//! Transfer planning: Lambert's problem and porkchop plots.
//!
//! [`lambert`] finds the conic between two positions in a given time (less than one
//! revolution) with universal variables: the time of flight rises monotonically with the
//! universal parameter z, so a Newton iteration kept inside a shrinking bracket always
//! converges, for ellipses and hyperbolas alike.
//!
//! [`porkchop`] solves it over a grid of departure times and flight times. Within one
//! sphere of influence (a vessel to a moon or another vessel) departure delta-v is the burn
//! from the vessel's own orbit. Between planets the departure leg starts at the planet, and
//! departure delta-v is the ejection burn from a circular parking orbit at the vessel's radius
//! (the Oberth effect included). Arrival delta-v is the relative speed on arrival, what
//! a rendezvous or capture has to take out. Transfers that pass below the central body's
//! safe radius are left out.
//!
//! Over windows of many parking orbits, where a departure time says little about where in
//! its orbit the vessel is, each cell instead takes the best departure within one parking
//! orbit of its time (`phase_search`): from low orbit the cheap departures to a moon on
//! another plane come only when the moon's arrival point lies in the parking plane, twice
//! a month, and this is what finds them.
//!
//! [`LaunchRequest`] answers the same question from the ground, where the plane of the parking orbit
//! is still to be chosen: when to leave and which way. A far target is reached where it will be on
//! arrival, so the orbit's plane has to hold that point, and any plane through the launch site and
//! that point will do: the cheapest is the one flown most nearly due east, twice a day. This works
//! from a site further from the equator than the target's orbit ever goes, which never passes under
//! it. A near target has to be matched all the way round, so the plane is its own, when the
//! site passes under it or comes closest.

use std::f64::consts::{PI, TAU};

use glam::DVec3;

use crate::{
    body::{Body, BodyId},
    kepler::stumpff,
    orbit::{Orbit, StateVector},
    plan::Burn,
    prediction::{PatchEnd, predict},
    rendezvous::closest_approaches,
    system::SolarSystem,
    time::SimTime,
};

/// Velocities at `r1` and `r2` of the conic around a body of gravitational parameter `mu`
/// that joins them in `tof` seconds, going prograde about `normal`, in less than one
/// revolution. `None` for straight-through geometries (180° or 0° apart) or no solution.
pub fn lambert(mu: f64, r1: DVec3, r2: DVec3, tof: f64, normal: DVec3) -> Option<(DVec3, DVec3)> {
    let (m1, m2) = (r1.length(), r2.length());
    if tof <= 0.0 || m1 == 0.0 || m2 == 0.0 {
        return None;
    }
    let cos = (r1.dot(r2) / (m1 * m2)).clamp(-1.0, 1.0);
    let mut sweep = cos.acos();
    if r1.cross(r2).dot(normal) < 0.0 {
        sweep = TAU - sweep;
    }
    // A is negative the long way round; near 0 the transfer plane is undefined.
    let a = sweep.sin() * (m1 * m2 / (1.0 - cos).max(1e-300)).sqrt();
    if a.abs() < 1e-9 * (m1 + m2) {
        return None;
    }
    let y = |z: f64| {
        let (c, s) = stumpff(z);
        m1 + m2 + a * (z * s - 1.0) / c.sqrt()
    };
    let time = |z: f64| {
        let y = y(z);
        if y < 0.0 {
            return None;
        }
        let (c, s) = stumpff(z);
        let chi = (y / c).sqrt();
        Some((chi * chi * chi * s + a * y.sqrt()) / mu.sqrt())
    };
    // dt/dz (Curtis, Orbital Mechanics, eq. 5.43), times √μ.
    let slope = |z: f64| {
        let y = y(z);
        let (c, s) = stumpff(z);
        if z.abs() < 1e-6 {
            std::f64::consts::SQRT_2 / 40.0 * y.powf(1.5)
                + a / 8.0 * (y.sqrt() + a * (1.0 / (2.0 * y)).sqrt())
        } else {
            (y / c).powf(1.5) * ((c - 1.5 * s / c) / (2.0 * z) + 0.75 * s * s / c)
                + a / 8.0 * (3.0 * s / c * y.sqrt() + a * (c / y).sqrt())
        }
    };
    // Below one revolution z < 4π²; deep hyperbolas are far below zero.
    let (mut lo, mut hi) = (-4.0 * PI * PI * 25.0, 4.0 * PI * PI * (1.0 - 1e-12));
    if time(hi).is_some_and(|t| t < tof) {
        return None;
    }
    let mut z = 0.0f64.clamp(lo, hi);
    for _ in 0..100 {
        let t = time(z);
        match t {
            Some(t) if t >= tof => hi = z,
            _ => lo = z,
        }
        let newton = t
            .map(|t| z - (t - tof) * mu.sqrt() / slope(z))
            .filter(|n| n.is_finite() && *n > lo && *n < hi);
        let next = newton.unwrap_or(0.5 * (lo + hi));
        let done =
            (next - z).abs() < 1e-13 * z.abs().max(1.0) || hi - lo < 1e-14 * hi.abs().max(1.0);
        z = next;
        if done {
            break;
        }
    }
    let y = y(z);
    if y <= 0.0 {
        return None;
    }
    let f = 1.0 - y / m1;
    let g = a * (y / mu).sqrt();
    let gdot = 1.0 - y / m2;
    Some(((r2 - f * r1) / g, (gdot * r2 - r1) / g))
}

/// What to plan: departing from `departure` (both orbits around the same central body)
/// within `[depart_from, depart_from + depart_span]` to reach `arrival` after a flight of
/// `tof_min..tof_max` seconds.
#[derive(Debug, Clone, Copy)]
pub struct TransferRequest {
    pub departure: Orbit,
    pub arrival: Orbit,
    pub depart_from: SimTime,
    pub depart_span: f64,
    pub tof_min: f64,
    pub tof_max: f64,
    pub columns: usize,
    pub rows: usize,
    /// Leaving from orbit around a planet whose heliocentric orbit is `departure`: its
    /// gravitational parameter and the parking orbit radius.
    pub parking: Option<(f64, f64)>,
    /// Transfers passing closer to the central body than this are left out, m.
    pub floor: f64,
    /// Search departures within ± half this period (the parking orbit's) around each
    /// cell's time for the cheapest.
    pub phase_search: Option<f64>,
}

/// One transfer in the grid.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Transfer {
    pub departure: SimTime,
    pub tof: f64,
    /// Departure delta-v (ejection burn between planets), m/s.
    pub depart_dv: f64,
    /// Speed relative to the target on arrival, m/s.
    pub arrive_dv: f64,
    /// Velocity needed at departure (relative to the central body).
    pub velocity: DVec3,
    /// Speed beyond escape leaving the departure planet, m/s (between planets).
    pub excess: Option<DVec3>,
}

impl Transfer {
    pub fn total(&self) -> f64 {
        self.depart_dv + self.arrive_dv
    }
}

/// Departure times across, flight times up; row-major from the shortest flight.
#[derive(Debug, Clone)]
pub struct Porkchop {
    pub request: TransferRequest,
    pub cells: Vec<Option<Transfer>>,
}

impl Porkchop {
    pub fn cell(&self, column: usize, row: usize) -> Option<&Transfer> {
        self.cells
            .get(row * self.request.columns + column)
            .and_then(Option::as_ref)
    }

    /// The cheapest transfer in total delta-v and where it is in the grid.
    pub fn best(&self) -> Option<(usize, usize, &Transfer)> {
        let columns = self.request.columns;
        self.cells
            .iter()
            .enumerate()
            .filter_map(|(i, c)| Some((i % columns, i / columns, c.as_ref()?)))
            .min_by(|a, b| a.2.total().total_cmp(&b.2.total()))
    }
}

/// Solve every cell of the request's grid.
pub fn porkchop(request: &TransferRequest) -> Porkchop {
    let r = request;
    let mu = r.departure.mu();
    let normal = r.departure.normal();
    let mut cells = Vec::with_capacity(r.columns * r.rows);
    for row in 0..r.rows {
        let tof = r.tof_min + (r.tof_max - r.tof_min) * row as f64 / (r.rows - 1).max(1) as f64;
        for column in 0..r.columns {
            let share = column as f64 / (r.columns - 1).max(1) as f64;
            let departure = r.depart_from.add_seconds(r.depart_span * share);
            let cell = match r.phase_search {
                Some(period) => best_phase(r, mu, normal, departure, tof, period),
                None => solve(r, mu, normal, departure, tof),
            };
            cells.push(cell);
        }
    }
    Porkchop {
        request: *request,
        cells,
    }
}

/// Coarse samples across the parking orbit, then golden-section refinement around the best.
fn best_phase(
    r: &TransferRequest,
    mu: f64,
    normal: DVec3,
    around: SimTime,
    tof: f64,
    period: f64,
) -> Option<Transfer> {
    const SAMPLES: usize = 12;
    let at = |offset: f64| solve(r, mu, normal, around.add_seconds(offset), tof);
    let cost = |t: &Option<Transfer>| t.map_or(f64::INFINITY, |t| t.total());
    let step = period / SAMPLES as f64;
    let offsets: Vec<f64> = (0..SAMPLES)
        .map(|k| (k as f64 - SAMPLES as f64 / 2.0) * step)
        .collect();
    let (k, mut best) = offsets
        .iter()
        .map(|&o| at(o))
        .enumerate()
        .min_by(|a, b| cost(&a.1).total_cmp(&cost(&b.1)))?;
    best?;
    let (mut lo, mut hi) = (offsets[k] - step, offsets[k] + step);
    let ratio = 0.618_033_988_75;
    for _ in 0..14 {
        let (a, b) = (hi - ratio * (hi - lo), lo + ratio * (hi - lo));
        let (ta, tb) = (at(a), at(b));
        if cost(&ta) < cost(&tb) {
            hi = b;
            if cost(&ta) < cost(&best) {
                best = ta;
            }
        } else {
            lo = a;
            if cost(&tb) < cost(&best) {
                best = tb;
            }
        }
    }
    best
}

fn solve(
    r: &TransferRequest,
    mu: f64,
    normal: DVec3,
    departure: SimTime,
    tof: f64,
) -> Option<Transfer> {
    let arrival = departure.add_seconds(tof);
    let from = r.departure.state_at(departure);
    let to = r.arrival.state_at(arrival);
    let (v1, v2) = lambert(mu, from.position, to.position, tof, normal)?;
    // Passing below the safe radius on the way: no good.
    let path = Orbit::new(mu, departure, StateVector::new(from.position, v1));
    if path.periapsis() < r.floor
        && path
            .next_time_at_true_anomaly(0.0, departure)
            .is_some_and(|t| t < arrival)
    {
        return None;
    }
    let excess = v1 - from.velocity;
    let depart_dv = match r.parking {
        Some((gm, radius)) => {
            let escape = 2.0 * gm / radius;
            (excess.length_squared() + escape).sqrt() - (gm / radius).sqrt()
        }
        None => excess.length(),
    };
    Some(Transfer {
        departure,
        tof,
        depart_dv,
        arrive_dv: (to.velocity - v2).length(),
        velocity: v1,
        excess: r.parking.map(|_| excess),
    })
}

/// The burn that starts `transfer` from `orbit` (same sphere of influence).
pub fn departure_burn(orbit: &Orbit, transfer: &Transfer) -> Option<Burn> {
    let state = orbit.state_at(transfer.departure);
    Burn::from_inertial(
        transfer.departure,
        &state,
        transfer.velocity - state.velocity,
    )
}

/// The burn from a near-circular `parking` orbit that leaves the planet with hyperbolic
/// excess velocity `excess` (inertial), at the pass nearest `around`.
///
/// The escape hyperbola's asymptote lies θ∞ = acos(−1/e) ahead of its periapsis, so the
/// burn is where the vessel is θ∞ behind the excess direction (projected into the parking
/// plane), prograde, to the speed that leaves exactly `excess` at infinity. Excess out of the
/// parking plane is not steered for; the burn can be refined in the flight plan.
pub fn ejection_burn(parking: &Orbit, excess: DVec3, around: SimTime) -> Option<Burn> {
    let normal = parking.normal();
    let direction = (excess - normal * excess.dot(normal)).try_normalize()?;
    let mu = parking.mu();
    let v_inf = excess.length();
    let radius = parking.semi_latus_rectum() / (1.0 + parking.eccentricity());
    let e = 1.0 + radius * v_inf * v_inf / mu;
    let behind = (-1.0 / e).acos();
    let periapsis = glam::DQuat::from_axis_angle(normal, -behind) * direction;
    let half = parking.period().unwrap_or(0.0) / 2.0;
    let time = parking.next_time_at(periapsis, around.add_seconds(-half))?;
    let state = parking.state_at(time);
    let r = state.position.length();
    let speed = (v_inf * v_inf + 2.0 * mu / r).sqrt();
    let wanted = state.velocity.normalize() * speed;
    Burn::from_inertial(time, &state, wanted - state.velocity)
}

/// The trajectory a vessel is on, for refining a planned burn against the full patched-conic
/// prediction.
#[derive(Debug, Clone, Copy)]
pub struct Course<'a> {
    pub system: &'a SolarSystem,
    pub frame: BodyId,
    pub orbit: Orbit,
    pub from: SimTime,
    pub horizon: SimTime,
}

/// Adjust burn `index` of `burns` so the trajectory after it passes `target` with
/// periapsis `radius` from its centre, or as close as it can get, changing the burn as
/// little as possible. While there is no encounter yet it steers on the closest approach
/// instead. Minimum-norm Newton steps on finite-difference gradients through the full
/// prediction, so SOI changes and the target's motion are all accounted for; prograde is
/// usually the strongest lever, as it moves the arrival time. Returns the burn and the
/// radius achieved.
pub fn aim_periapsis(
    course: &Course,
    burns: &[Burn],
    index: usize,
    target: BodyId,
    radius: f64,
) -> Option<(Burn, f64)> {
    let target_body = course.system.body(target);
    let measure = |burn: &Burn| -> Option<f64> {
        let mut list = burns.to_vec();
        *list.get_mut(index)? = *burn;
        let patches = predict(
            course.system,
            course.frame,
            course.orbit,
            course.from,
            course.horizon,
            16,
            &list,
        );
        let after = patches.iter().position(
            |p| matches!(p.ending, Some(PatchEnd::Burn { index: i, .. }) if i == index),
        )? + 1;
        let legs = patches.get(after..)?;
        if let Some(arrival) = legs.iter().find(|p| p.frame == target) {
            return Some(arrival.orbit.periapsis());
        }
        // No encounter: the closest the legs around the target's parent come to it.
        let parent = target_body.parent?;
        legs.iter()
            .filter(|p| p.frame == parent)
            .filter_map(|p| {
                let relative = |t: SimTime| {
                    course.system.relative_state(target, parent, t) - p.orbit.state_at(t)
                };
                let step = target_body.orbit?.period()? / 2_000.0;
                let mut found = closest_approaches(relative, p.start, p.end, step, 1);
                // Closing all the way to the end of the leg counts too.
                let end = relative(p.end).position.length();
                Some(found.pop().map_or(end, |a| a.distance.min(end)))
            })
            .reduce(f64::min)
    };
    let mut burn = *burns.get(index)?;
    let mut miss = measure(&burn)? - radius;
    for _ in 0..20 {
        if miss.abs() < 1e-4 * radius.max(1.0) + 50.0 {
            break;
        }
        // Gradient over prograde, normal and radial, in m per m/s.
        let h = 0.01;
        let mut gradient = DVec3::ZERO;
        for axis in 0..3 {
            let mut nudged = burn;
            nudged.delta_v[axis] += h;
            gradient[axis] = (measure(&nudged)? - radius - miss) / h;
        }
        let norm = gradient.length_squared();
        if norm == 0.0 {
            break;
        }
        // Newton along the gradient, a few m/s at most per step, halved until it helps.
        let mut scale = (-miss / norm).clamp(-20.0 / norm.sqrt(), 20.0 / norm.sqrt());
        let mut improved = false;
        for _ in 0..8 {
            let mut next = burn;
            next.delta_v += scale * gradient;
            if let Some(m) = measure(&next).map(|r| r - radius)
                && m.abs() < miss.abs()
            {
                (burn, miss) = (next, m);
                improved = true;
                break;
            }
            scale /= 2.0;
        }
        if !improved {
            break;
        }
    }
    Some((burn, miss + radius))
}

/// Time of a Hohmann transfer between circular orbits of radii `r1` and `r2`.
pub fn hohmann_time(mu: f64, r1: f64, r2: f64) -> f64 {
    let a = 0.5 * (r1 + r2);
    PI * (a * a * a / mu).sqrt()
}

/// Time for two orbits' phase to come round again.
pub fn synodic_period(p1: f64, p2: f64) -> f64 {
    let rate = (1.0 / p1 - 1.0 / p2).abs();
    if rate > 0.0 {
        1.0 / rate
    } else {
        f64::INFINITY
    }
}

/// A target further out than this, in radii of the parking orbit, is reached where it will be rather
/// than matched in its plane.
const FAR_TARGET: f64 = 5.0;
/// How far above what it has to clear a parking orbit is, m.
const PARKING_HEIGHT: f64 = 60e3;
/// How many times are tried per turn of the ground under the launch.
const LAUNCH_STEPS: usize = 96;
/// Launch times are found to about this share of the step.
const REFINE_STEPS: usize = 30;
/// When the best and worst times of a turn are this close, rad, no time is better than another.
const FLAT: f64 = 0.1 * PI / 180.0;

/// Leaving the ground at a time, for a target in orbit around the same body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaunchWindow {
    pub time: SimTime,
    /// The compass heading to fly over the ground, rad clockwise from north.
    pub heading: f64,
    /// Of the orbit it leads to, against the body's equator, rad.
    pub inclination: f64,
    /// How far the plane the launch can reach is from the one it wants, rad: 0 when the site is in
    /// that plane, as it always is for a far target.
    pub miss: f64,
    /// How much more speed over the ground this orbit takes than one flown due east, m/s: what the
    /// body's rotation does not give it.
    pub extra: f64,
}

/// A launch from where a vessel stands to a target in orbit around the same body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct LaunchRequest {
    pub body: BodyId,
    /// Where it stands, body-fixed.
    pub site: DVec3,
    pub arrival: Orbit,
    /// Radius of the orbit it goes to first, m.
    pub parking: f64,
    /// The flight from that orbit to the target, s.
    pub flight: f64,
    /// From the launch to leaving that orbit, s.
    pub lead: f64,
}

impl LaunchRequest {
    /// `None` if the target's orbit is not a closed one.
    pub fn new(body: &Body, id: BodyId, site: DVec3, arrival: Orbit) -> Option<Self> {
        let parking = body.coast_floor() + PARKING_HEIGHT;
        let far = arrival.semi_major_axis();
        arrival.period()?;
        (far > 0.0).then(|| Self {
            body: id,
            site,
            arrival,
            parking,
            flight: hohmann_time(body.gm, parking, far),
            lead: TAU * (parking * parking * parking / body.gm).sqrt(),
        })
    }

    /// Is the target reached where it will be, rather than matched in its plane?
    pub fn is_far(&self) -> bool {
        self.arrival.semi_major_axis() > FAR_TARGET * self.parking
    }

    /// The plane to go into from the site at `time`, by its normal, and how far that is from the one
    /// wanted, rad.
    fn plane(&self, system: &SolarSystem, time: SimTime) -> (DVec3, f64) {
        let up = system.body(self.body).orientation(time) * self.site.normalize_or_zero();
        let along = self.arrival.normal();
        if self.is_far() {
            let there = time.add_seconds(self.lead + self.flight);
            let normal = up.cross(self.arrival.state_at(there).position);
            let normal = normal.normalize_or_zero();
            // Going round the way it does.
            let sign = if normal.dot(along) < 0.0 { -1.0 } else { 1.0 };
            (normal * sign, 0.0)
        } else {
            let off = along.dot(up);
            let normal = (along - up * off).normalize_or_zero();
            (normal, off.clamp(-1.0, 1.0).asin().abs())
        }
    }

    /// What is left to wish for at `time`: how far a near target's plane is, and how much a far one
    /// is flown away from the rotation that helps, as the inclination this makes.
    fn cost(&self, system: &SolarSystem, time: SimTime) -> f64 {
        let (normal, miss) = self.plane(system, time);
        if self.is_far() {
            let pole = system.body(self.body).pole();
            normal.dot(pole).abs().clamp(0.0, 1.0).acos()
        } else {
            miss
        }
    }

    /// Leaving at `time`: which way, and into what.
    pub fn at(&self, system: &SolarSystem, time: SimTime) -> LaunchWindow {
        let body = system.body(self.body);
        let up = body.orientation(time) * self.site.normalize_or_zero();
        let (normal, miss) = self.plane(system, time);
        let pole = body.pole();
        let east = pole.cross(up).normalize_or_zero();
        let north = up.cross(east);
        // Over the ground the orbit's speed has the ground's own taken off it.
        let speed = (body.gm / self.parking).sqrt();
        let ground = body.surface_velocity(up * self.site.length());
        let over_ground = normal.cross(up) * speed - ground;
        LaunchWindow {
            time,
            heading: over_ground
                .dot(east)
                .atan2(over_ground.dot(north))
                .rem_euclid(TAU),
            inclination: normal.dot(pole).clamp(-1.0, 1.0).acos(),
            miss,
            extra: over_ground.length() - (speed - ground.length()),
        }
    }

    /// The best times to leave after `after`, soonest first, up to `count` of them. None at all if
    /// no time is better than another.
    pub fn windows(&self, system: &SolarSystem, after: SimTime, count: usize) -> Vec<LaunchWindow> {
        let rate = system.body(self.body).angular_velocity().length();
        // A body that does not turn still sees a far target go round.
        let turn = if rate > 0.0 {
            TAU / rate
        } else {
            self.arrival.period().unwrap_or(0.0)
        };
        if turn <= 0.0 {
            return Vec::new();
        }
        let step = turn / LAUNCH_STEPS as f64;
        let at = |index: usize| after.add_seconds(step * index as f64);
        let cost = |index: usize| self.cost(system, at(index));
        let first_turn = (0..=LAUNCH_STEPS).map(&cost);
        let (low, high) = first_turn.fold((f64::INFINITY, 0.0f64), |(low, high), c| {
            (low.min(c), high.max(c))
        });
        if high - low < FLAT {
            return Vec::new();
        }
        let mut found = Vec::new();
        // One a turn at the least, however many are asked for.
        let steps = LAUNCH_STEPS * (count + 1);
        let mut around = [cost(0), cost(1), cost(2)];
        for index in 1..steps {
            if found.len() == count {
                break;
            }
            if around[1] <= around[0] && around[1] < around[2] {
                let best = self.refine(system, at(index - 1), step * 2.0);
                found.push(self.at(system, best));
            }
            around = [around[1], around[2], cost(index + 2)];
        }
        found
    }

    /// The least cost in the `span` after `from`, by golden section.
    fn refine(&self, system: &SolarSystem, from: SimTime, span: f64) -> SimTime {
        const SHARE: f64 = 0.618_033_988_749_894_9;
        let cost = |offset: f64| self.cost(system, from.add_seconds(offset));
        let (mut low, mut high) = (0.0, span);
        for _ in 0..REFINE_STEPS {
            let near = high - (high - low) * SHARE;
            let far = low + (high - low) * SHARE;
            if cost(near) < cost(far) {
                high = far;
            } else {
                low = near;
            }
        }
        from.add_seconds((low + high) / 2.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 3.986_004_418e14;
    const SUN: f64 = 1.327_124_4e20;

    #[test]
    fn lambert_recovers_known_orbits() {
        // Ellipse, short way and long way, and a hyperbola.
        let cases = [
            (
                DVec3::new(7e6, 0.0, 0.0),
                DVec3::new(0.0, 8.2e3, 1.0e3),
                1_800.0,
            ),
            (
                DVec3::new(7e6, 0.0, 0.0),
                DVec3::new(0.0, 8.2e3, 1.0e3),
                6_000.0,
            ),
            (
                DVec3::new(7e6, 0.0, 0.0),
                DVec3::new(0.0, 12e3, 0.0),
                3_000.0,
            ),
        ];
        for (position, velocity, dt) in cases {
            let orbit = Orbit::new(MU, SimTime::J2000, StateVector::new(position, velocity));
            let end = orbit.state_at(SimTime::J2000.add_seconds(dt));
            let (v1, v2) = lambert(MU, position, end.position, dt, orbit.normal()).unwrap();
            assert!(
                (v1 - velocity).length() < 1e-6 * velocity.length(),
                "{dt}: {v1} {velocity}"
            );
            assert!((v2 - end.velocity).length() < 1e-6 * end.velocity.length());
        }
    }

    #[test]
    fn hohmann_is_the_cheapest_coplanar_transfer() {
        // LEO to a circular orbit at the Moon's distance, timed for a Hohmann half-ellipse.
        let (r1, r2) = (6_671e3, 384_400e3);
        let circle = |r: f64, phase: f64| {
            let v = (MU / r).sqrt();
            Orbit::new(
                MU,
                SimTime::J2000,
                StateVector::new(
                    DVec3::new(phase.cos(), phase.sin(), 0.0) * r,
                    DVec3::new(-phase.sin(), phase.cos(), 0.0) * v,
                ),
            )
        };
        let t_h = hohmann_time(MU, r1, r2);
        let n2 = (MU / (r2 * r2 * r2)).sqrt();
        // The target leads by π − n2·t_h when the chaser leaves at phase 0.
        let lead = PI - n2 * t_h;
        let departure = circle(r1, 0.0);
        let arrival = circle(r2, lead);
        let period = departure.period().unwrap();
        let request = TransferRequest {
            departure,
            arrival,
            depart_from: SimTime::J2000.add_seconds(-0.1 * period),
            depart_span: 0.2 * period,
            tof_min: 0.9 * t_h,
            tof_max: 1.1 * t_h,
            columns: 41,
            rows: 41,
            parking: None,
            floor: 6_471e3,
            phase_search: None,
        };
        let chop = porkchop(&request);
        let (_, _, best) = chop.best().unwrap();
        let hohmann = (MU / r1).sqrt() * ((2.0 * r2 / (r1 + r2)).sqrt() - 1.0);
        let capture = (MU / r2).sqrt() * (1.0 - (2.0 * r1 / (r1 + r2)).sqrt());
        assert!(
            best.depart_dv > hohmann - 1.0 && best.depart_dv < hohmann + 15.0,
            "{}",
            best.depart_dv
        );
        assert!(
            (best.arrive_dv - capture).abs() < 30.0,
            "{} {capture}",
            best.arrive_dv
        );
        assert!(best.departure.seconds_since(SimTime::J2000).abs() < 0.02 * period);
        // The burn it makes is prograde.
        let burn = departure_burn(&departure, best).unwrap();
        assert!(burn.delta_v.x > 0.99 * burn.delta_v.length());
    }

    #[test]
    fn earth_to_mars_window_costs_about_3_6_km_s() {
        // Circular coplanar planets: the classic Hohmann numbers.
        let au = 1.495_978_707e11;
        let (r1, r2) = (au, 1.5237 * au);
        let circle = |r: f64, phase: f64| {
            let v = (SUN / r).sqrt();
            Orbit::new(
                SUN,
                SimTime::J2000,
                StateVector::new(
                    DVec3::new(phase.cos(), phase.sin(), 0.0) * r,
                    DVec3::new(-phase.sin(), phase.cos(), 0.0) * v,
                ),
            )
        };
        let t_h = hohmann_time(SUN, r1, r2);
        let lead = PI - (SUN / (r2 * r2 * r2)).sqrt() * t_h;
        let request = TransferRequest {
            departure: circle(r1, 0.0),
            arrival: circle(r2, lead),
            depart_from: SimTime::J2000.add_seconds(-20.0 * 86_400.0),
            depart_span: 40.0 * 86_400.0,
            tof_min: 0.8 * t_h,
            tof_max: 1.2 * t_h,
            columns: 41,
            rows: 41,
            parking: Some((MU, 6_671e3)),
            floor: 7e8,
            phase_search: None,
        };
        let chop = porkchop(&request);
        let (_, _, best) = chop.best().unwrap();
        // From LEO about 3.6 km/s; Mars arrival about 2.6 km/s relative.
        assert!(
            (best.depart_dv - 3_600.0).abs() < 100.0,
            "{}",
            best.depart_dv
        );
        assert!(
            (best.arrive_dv - 2_650.0).abs() < 150.0,
            "{}",
            best.arrive_dv
        );
        assert!(best.excess.unwrap().length() > 2_800.0);
    }

    #[test]
    fn ejection_burn_leaves_along_the_excess() {
        let r = 6_671e3;
        let v = (MU / r).sqrt();
        let parking = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(DVec3::new(r, 0.0, 0.0), DVec3::new(0.0, v, 0.0)),
        );
        let excess = DVec3::new(-1.0, 2.0, 0.0).normalize() * 3_000.0;
        let burn = ejection_burn(&parking, excess, SimTime::J2000.add_seconds(4_000.0)).unwrap();
        let state = parking.state_at(burn.time);
        let after = Orbit::new(
            MU,
            burn.time,
            StateVector::new(
                state.position,
                state.velocity + burn.inertial(&state).unwrap(),
            ),
        );
        // Far out, the vessel leaves at 3 km/s along the excess direction.
        let far = after.state_at(burn.time.add_seconds(30.0 * 86_400.0));
        assert!(
            (far.velocity.length() - 3_000.0).abs() < 20.0,
            "{}",
            far.velocity.length()
        );
        assert!(
            far.velocity.angle_between(excess) < 0.01,
            "{}",
            far.velocity.angle_between(excess)
        );
        // Within half an orbit of the time asked for.
        let period = parking.period().unwrap();
        assert!(
            burn.time
                .seconds_since(SimTime::J2000.add_seconds(4_000.0))
                .abs()
                <= period / 2.0
        );
    }

    #[test]
    fn phase_search_finds_the_cheap_departures_to_a_tilted_moon() {
        // LEO tilted 30° to a Moon-like circular orbit: over half a month the cheapest
        // departure is close to the coplanar Hohmann cost, far below a plane change.
        let (r1, r2) = (6_671e3, 384_400e3);
        let tilt = 30f64.to_radians();
        let leo = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(
                DVec3::new(r1, 0.0, 0.0),
                DVec3::new(0.0, tilt.cos(), tilt.sin()) * (MU / r1).sqrt(),
            ),
        );
        let moon = Orbit::new(
            MU,
            SimTime::J2000,
            StateVector::new(
                DVec3::new(0.0, r2, 0.0),
                DVec3::new(-(MU / r2).sqrt(), 0.0, 0.0),
            ),
        );
        let t_h = hohmann_time(MU, r1, r2);
        let request = TransferRequest {
            departure: leo,
            arrival: moon,
            depart_from: SimTime::J2000,
            depart_span: moon.period().unwrap() / 2.0,
            tof_min: 0.7 * t_h,
            tof_max: 1.3 * t_h,
            columns: 48,
            rows: 7,
            parking: None,
            floor: 6_471e3,
            phase_search: leo.period(),
        };
        let chop = porkchop(&request);
        let (_, _, best) = chop.best().unwrap();
        let hohmann = (MU / r1).sqrt() * ((2.0 * r2 / (r1 + r2)).sqrt() - 1.0);
        assert!(
            best.depart_dv < hohmann + 120.0,
            "{} vs {hohmann}",
            best.depart_dv
        );
        // The burn made from it is (nearly) in the parking plane.
        let burn = departure_burn(&leo, best).unwrap();
        assert!(
            burn.delta_v.y.abs() < 0.1 * burn.delta_v.length(),
            "{:?}",
            burn.delta_v
        );
    }

    #[test]
    fn aiming_a_moon_transfer_at_a_periapsis() {
        // A Lambert departure aimed at the Moon's centre, then aimed 100 km above it.
        let system = crate::system::tests::rss();
        let earth = system.find("Earth").unwrap();
        let moon = system.find("Moon").unwrap();
        let mu = system.body(earth).gm;
        let r1 = system.body(earth).radius + 300e3;
        let moon_orbit = system.body(moon).orbit.unwrap();
        // A parking orbit in the Moon's plane.
        let normal = moon_orbit.normal();
        let x = normal.any_orthonormal_vector();
        let leo = Orbit::new(
            mu,
            SimTime::J2000,
            StateVector::new(x * r1, normal.cross(x) * (mu / r1).sqrt()),
        );
        let t_h = hohmann_time(mu, r1, moon_orbit.semi_major_axis());
        let request = TransferRequest {
            departure: leo,
            arrival: moon_orbit,
            depart_from: SimTime::J2000.add_seconds(600.0),
            depart_span: 5.0 * 86_400.0,
            tof_min: 0.8 * t_h,
            tof_max: 1.1 * t_h,
            columns: 24,
            rows: 8,
            parking: None,
            floor: system.body(earth).coast_floor(),
            phase_search: leo.period(),
        };
        let chop = porkchop(&request);
        let (_, _, best) = chop.best().unwrap();
        let burns = [departure_burn(&leo, best).unwrap()];
        let course = Course {
            system: &system,
            frame: earth,
            orbit: leo,
            from: SimTime::J2000,
            horizon: best.departure.add_seconds(2.0 * best.tof),
        };
        let goal = system.body(moon).radius + 100e3;
        let (aimed, reached) = aim_periapsis(&course, &burns, 0, moon, goal).unwrap();
        assert!(
            (reached - goal).abs() < 1e3,
            "{} km off",
            (reached - goal) / 1e3
        );
        // A few tens of m/s at most changed the burn.
        let change = (aimed.delta_v - burns[0].delta_v).length();
        assert!(change < 100.0, "{change}");
    }

    #[test]
    fn synodic_period_of_earth_and_mars() {
        let s = synodic_period(365.25, 686.98);
        assert!((s - 779.9).abs() < 1.0, "{s}");
    }

    /// The real Earth and Moon of the base game, and a site at the Cape's latitude.
    fn earth_and_moon() -> (SolarSystem, BodyId, Orbit, DVec3) {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let text = std::fs::read_to_string(root.join("bodies/solar_system.json5")).unwrap();
        let bodies = json5::from_str(&text).unwrap();
        let system = SolarSystem::from_def(&crate::data::SystemDef { bodies }).unwrap();
        let earth = system.find("Earth").unwrap();
        let moon = system.body(system.find("Moon").unwrap()).orbit.unwrap();
        let site = Body::surface_normal_fixed(28.6083f64.to_radians(), -80.6f64.to_radians());
        (system, earth, moon, site * 6_371e3)
    }

    /// The circular parking orbit `window` leads to, as it passes over the site.
    fn parking_orbit(system: &SolarSystem, request: &LaunchRequest, time: SimTime) -> Orbit {
        let body = system.body(request.body);
        let up = body.orientation(time) * request.site.normalize();
        let (normal, _) = request.plane(system, time);
        let speed = (body.gm / request.parking).sqrt();
        let state = StateVector::new(up * request.parking, normal.cross(up) * speed);
        Orbit::new(body.gm, time, state)
    }

    /// The same launch flown due east whatever the time, as a player with no window to go by would.
    fn due_east(system: &SolarSystem, request: &LaunchRequest, time: SimTime) -> Orbit {
        let body = system.body(request.body);
        let up = body.orientation(time) * request.site.normalize();
        let east = body.pole().cross(up).normalize();
        let speed = (body.gm / request.parking).sqrt();
        let state = StateVector::new(up * request.parking, east * speed);
        Orbit::new(body.gm, time, state)
    }

    /// What the planner would find to the Moon from `orbit` soon after reaching it: the cheapest
    /// total leaving within two of its orbits, over the flight times it plots.
    fn cheapest_soon(
        system: &SolarSystem,
        request: &LaunchRequest,
        orbit: Orbit,
        start: SimTime,
    ) -> f64 {
        let period = orbit.period().unwrap();
        let chop = porkchop(&TransferRequest {
            departure: orbit,
            arrival: request.arrival,
            depart_from: start,
            depart_span: 2.0 * period,
            tof_min: 0.5 * request.flight,
            tof_max: 1.5 * request.flight,
            columns: 12,
            rows: 24,
            parking: None,
            floor: system.body(request.body).coast_floor(),
            phase_search: Some(period),
        });
        chop.best().map_or(f64::INFINITY, |(_, _, t)| t.total())
    }

    #[test]
    fn a_launch_for_the_moon_goes_twice_a_day_flying_due_east() {
        let (system, earth, arrival, site) = earth_and_moon();
        let request = LaunchRequest::new(system.body(earth), earth, site, arrival).unwrap();
        assert!(request.is_far());
        let start = SimTime::from_seconds(21_600.0);
        let windows = request.windows(&system, start, 4);
        assert_eq!(windows.len(), 4);
        assert!(windows[0].time > start);
        for pair in windows.windows(2) {
            assert!(pair[1].time > pair[0].time);
        }
        // Not evenly: how far apart the two of a day are depends on how far the Moon is from the
        // equator. Each comes round a day later, and the Moon has moved on.
        let again = windows[2].time.seconds_since(windows[0].time) / 3_600.0;
        assert!((24.0..=26.0).contains(&again), "{again} h");
        for window in &windows {
            // As low as the Cape's latitude allows, flown due east, taking all the rotation has.
            let inclination = window.inclination.to_degrees();
            assert!((inclination - 28.6).abs() < 0.1, "{inclination}");
            let heading = window.heading.to_degrees();
            assert!((heading - 90.0).abs() < 0.5, "{heading}");
            assert!(window.extra.abs() < 1.0, "{} m/s", window.extra);
            assert_eq!(window.miss, 0.0);
        }
    }

    #[test]
    fn the_moon_is_cheap_to_reach_from_a_window_and_from_the_heading_given_at_any_time() {
        let (system, earth, arrival, site) = earth_and_moon();
        let request = LaunchRequest::new(system.body(earth), earth, site, arrival).unwrap();
        // A fortnight apart: the Moon on opposite sides of the Earth.
        for start in [21_600.0, 21_600.0 + 14.0 * 86_400.0].map(SimTime::from_seconds) {
            for window in request.windows(&system, start, 2) {
                let orbit = parking_orbit(&system, &request, window.time);
                let cost = cheapest_soon(&system, &request, orbit, window.time);
                assert!(cost < 4_150.0, "{cost} m/s from a window");
            }
            let mut worst_east = 0.0f64;
            for hour in 0..25 {
                let time = start.add_seconds(f64::from(hour) * 3_600.0);
                let orbit = parking_orbit(&system, &request, time);
                let cost = cheapest_soon(&system, &request, orbit, time);
                assert!(cost < 4_400.0, "{cost} m/s at hour {hour}");
                let east = due_east(&system, &request, time);
                worst_east = worst_east.max(cheapest_soon(&system, &request, east, time));
            }
            // Flown due east at the wrong time, leaving soon costs twice as much.
            assert!(worst_east > 8_000.0, "{worst_east} m/s");
        }
    }

    #[test]
    fn going_other_than_due_east_costs_what_the_rotation_no_longer_gives() {
        let (system, earth, arrival, site) = earth_and_moon();
        let request = LaunchRequest::new(system.body(earth), earth, site, arrival).unwrap();
        let start = SimTime::from_seconds(21_600.0);
        let window = request.windows(&system, start, 1)[0];
        // An hour or two off it costs next to nothing, and near-polar nearly all the 408 m/s.
        let off = request.at(&system, window.time.add_seconds(-2.0 * 3_600.0));
        assert!(off.extra < 30.0, "{} m/s", off.extra);
        let polar = request.at(&system, start);
        assert!(polar.inclination.to_degrees() > 80.0);
        assert!((350.0..450.0).contains(&polar.extra), "{} m/s", polar.extra);
    }

    /// A circular orbit at this inclination to `body`'s equator.
    fn inclined(body: &Body, degrees: f64) -> Orbit {
        let radius = 6_771e3;
        let speed = (body.gm / radius).sqrt();
        let (sin, cos) = degrees.to_radians().sin_cos();
        let to_equator = glam::DQuat::from_rotation_arc(DVec3::Z, body.pole());
        let state = StateVector::new(
            to_equator * DVec3::X * radius,
            to_equator * DVec3::new(0.0, cos, sin) * speed,
        );
        Orbit::new(body.gm, SimTime::J2000, state)
    }

    #[test]
    fn a_near_target_is_matched_in_its_plane_when_the_site_passes_under_it() {
        let (system, earth, _, site) = earth_and_moon();
        let body = system.body(earth);
        // Steeper than the site's latitude: it passes under it twice a turn, once going north and
        // once going south, and the launch is into just that plane.
        let request = LaunchRequest::new(body, earth, site, inclined(body, 51.6)).unwrap();
        assert!(!request.is_far());
        let start = SimTime::from_seconds(21_600.0);
        let windows = request.windows(&system, start, 2);
        assert_eq!(windows.len(), 2);
        let (one, other) = (
            windows[0].heading.to_degrees(),
            windows[1].heading.to_degrees(),
        );
        assert!((one - 90.0) * (other - 90.0) < 0.0, "{one} {other}");
        for window in &windows {
            assert!(window.miss.to_degrees() < 0.01, "{}", window.miss);
            assert!((window.inclination.to_degrees() - 51.6).abs() < 0.01);
        }
        // Shallower than the latitude: it never does, so once a turn it comes closest, due east.
        let request = LaunchRequest::new(body, earth, site, inclined(body, 5.0)).unwrap();
        let windows = request.windows(&system, start, 2);
        assert_eq!(windows.len(), 2);
        let apart = windows[1].time.seconds_since(windows[0].time) / 3_600.0;
        assert!((apart - 23.93).abs() < 0.1, "{apart} h apart");
        for window in &windows {
            let miss = window.miss.to_degrees();
            assert!((miss - 23.6).abs() < 0.1, "{miss}");
            assert!((window.heading.to_degrees() - 90.0).abs() < 0.5);
        }
    }

    #[test]
    fn a_site_on_the_equator_under_an_equatorial_orbit_has_no_better_time() {
        let (system, earth, _, _) = earth_and_moon();
        let body = system.body(earth);
        let site = Body::surface_normal_fixed(0.0, 0.0) * body.radius;
        let request = LaunchRequest::new(body, earth, site, inclined(body, 0.0)).unwrap();
        let start = SimTime::from_seconds(21_600.0);
        assert!(request.windows(&system, start, 4).is_empty());
        assert!((request.at(&system, start).heading.to_degrees() - 90.0).abs() < 1e-6);
    }
}
