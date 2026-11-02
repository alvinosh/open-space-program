//! What a vessel's surroundings give it at a place and a time: sunlight, with eclipses by any
//! body, what a planet reflects and radiates, and whether another point can be seen.
//!
//! Sunlight falls off with the square of the distance from the Sun, and the share of the Sun's
//! disc that shows is the disc less any body in front of it: in umbra none, in penumbra part.
//! A body with an `albedo` reflects that share of the light on it, seen over the half of it that
//! faces the vessel, and radiates the rest back as a black body in balance with the Sun. On rails
//! [`next_light_change`] finds where a coast goes into and out of a shadow, as
//! [`crate::events`] finds where it changes sphere of influence.

use std::f64::consts::PI;

use glam::DVec3;

use crate::{body::BodyId, orbit::Orbit, system::SolarSystem, time::SimTime};

/// Sunlight at 1 AU, W/m².
pub const SOLAR_CONSTANT: f64 = 1361.0;
/// 1 AU, m.
pub const ASTRONOMICAL_UNIT: f64 = 1.495_978_707e11;

/// What the surroundings give at a place and a time.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Environment {
    /// Towards the Sun, a unit vector in inertial axes.
    pub sun_direction: DVec3,
    /// From the Sun, m.
    pub sun_distance: f64,
    /// Towards the Sun in the vessel's own axes, once a vessel has said how it is turned.
    pub sun_in_vessel: DVec3,
    /// The share of the Sun's disc that shows, 0..1: 0 in umbra, 1 in full light.
    pub sunlit: f64,
    /// Direct sunlight, with that share, W/m².
    pub solar_flux: f64,
    /// Sunlight off the body it is nearest, W/m² on a surface facing it.
    pub albedo_flux: f64,
    /// Heat radiated by that body, W/m² on a surface facing it.
    pub infrared_flux: f64,
}

impl Environment {
    /// Nothing at all: for a vessel with no system that wants to know.
    pub const DARK: Self = Self {
        sun_direction: DVec3::ZERO,
        sun_distance: 0.0,
        sun_in_vessel: DVec3::ZERO,
        sunlit: 0.0,
        solar_flux: 0.0,
        albedo_flux: 0.0,
        infrared_flux: 0.0,
    };
}

/// The sunlight at `distance` metres from the Sun, W/m².
pub fn solar_flux_at(distance: f64) -> f64 {
    SOLAR_CONSTANT * (ASTRONOMICAL_UNIT / distance.max(1.0)).powi(2)
}

/// The surroundings of a point `position` from body `frame` (inertial axes) at `time`.
pub fn at(system: &SolarSystem, frame: BodyId, position: DVec3, time: SimTime) -> Environment {
    let root = system.root();
    let sun = system.relative_state(root, frame, time).position - position;
    let sun_distance = sun.length();
    let sun_direction = sun.normalize_or_zero();
    let sunlit = sunlit_share(system, frame, position, sun, time);
    let full = solar_flux_at(sun_distance);

    let body = system.body(frame);
    let (mut albedo_flux, mut infrared_flux) = (0.0, 0.0);
    let distance = position.length();
    if frame != root && distance > 0.0 {
        // The whole of it as seen from here falls off like this; with the phase of what is lit.
        let view = (body.radius / distance.max(body.radius)).powi(2);
        let overhead = position / distance;
        let phase = (1.0 + overhead.dot(sun_direction)) / 2.0;
        let at_body = solar_flux_at(sun_distance);
        albedo_flux = at_body * body.albedo * view * phase;
        // In balance: it gives out over its whole surface what it takes in over its disc.
        infrared_flux = at_body * (1.0 - body.albedo) / 4.0 * view;
    }
    Environment {
        sun_direction,
        sun_distance,
        sun_in_vessel: sun_direction,
        sunlit,
        solar_flux: full * sunlit,
        albedo_flux,
        infrared_flux,
    }
}

/// The share of the Sun's disc that shows from `position` from `frame`, the Sun `sun` from
/// there: the disc less whichever body covers most of it.
fn sunlit_share(
    system: &SolarSystem,
    frame: BodyId,
    position: DVec3,
    sun: DVec3,
    time: SimTime,
) -> f64 {
    let root = system.root();
    let sun_distance = sun.length();
    if frame == root && position.length() < system.body(root).radius {
        return 1.0;
    }
    let sun_angle = (system.body(root).radius / sun_distance.max(1.0))
        .min(1.0)
        .asin();
    let mut share: f64 = 1.0;
    for body in system.bodies().iter().filter(|b| b.id != root) {
        let to_body = system.relative_state(body.id, frame, time).position - position;
        let distance = to_body.length();
        if distance >= sun_distance || distance <= 0.0 {
            continue;
        }
        // Inside it, which only a tunnel could be.
        if distance < body.radius {
            return 0.0;
        }
        let body_angle = (body.radius / distance).asin();
        let apart = to_body.angle_between(sun);
        share = share.min(disc_left(sun_angle, body_angle, apart));
        if share <= 0.0 {
            return 0.0;
        }
    }
    share
}

/// The share of a disc of angular radius `a` left uncovered by a disc of radius `b`, their
/// centres `d` apart (small angles: flat circles).
fn disc_left(a: f64, b: f64, d: f64) -> f64 {
    if d >= a + b {
        return 1.0;
    }
    if d <= b - a {
        return 0.0;
    }
    if d <= a - b {
        return 1.0 - (b / a).powi(2);
    }
    // The lens two circles share.
    let (a2, b2) = (a * a, b * b);
    let lens = a2 * ((d * d + a2 - b2) / (2.0 * d * a)).clamp(-1.0, 1.0).acos()
        + b2 * ((d * d + b2 - a2) / (2.0 * d * b)).clamp(-1.0, 1.0).acos()
        - 0.5
            * ((a + b - d) * (d + a - b) * (d - a + b) * (d + a + b))
                .max(0.0)
                .sqrt();
    (1.0 - lens / (PI * a2)).clamp(0.0, 1.0)
}

/// The body whose disc lies across the straight line from `from` to `to`, both given from
/// `frame`, if any: what stops a radio link. Points inside a body's radius (a ground station on
/// it) count as at its surface.
pub fn occulted_by(
    system: &SolarSystem,
    frame: BodyId,
    from: DVec3,
    to: DVec3,
    time: SimTime,
) -> Option<BodyId> {
    let line = to - from;
    let length = line.length();
    if length <= 0.0 {
        return None;
    }
    let along = line / length;
    system.bodies().iter().find_map(|body| {
        let center = system.relative_state(body.id, frame, time).position;
        let to_center = center - from;
        let t = to_center.dot(along);
        if t <= 0.0 || t >= length {
            return None;
        }
        let miss = (to_center - along * t).length();
        // A little under the surface, so that a station on the ground sees what is overhead.
        (miss < body.radius * (1.0 - 1e-6)).then_some(body.id)
    })
}

/// Steps a coast is looked at in, as a share of its period: a low orbit's shadow is a third of
/// it, and any step shorter than the shortest stretch of light or dark finds them all.
const LIGHT_SAMPLES: usize = 48;

/// The first moment after `after` and before `until` that `orbit` goes into or out of a shadow:
/// the share of the Sun that shows crosses a half.
pub fn next_light_change(
    system: &SolarSystem,
    frame: BodyId,
    orbit: &Orbit,
    after: SimTime,
    until: SimTime,
) -> Option<SimTime> {
    let lit = |t: SimTime| {
        let position = orbit.state_at(t).position;
        at(system, frame, position, t).sunlit >= 0.5
    };
    let span = until.seconds_since(after);
    if span <= 0.0 {
        return None;
    }
    // Ten-minute steps at most, and fine enough for the orbit.
    let period = orbit.period().filter(|p| p.is_finite() && *p > 0.0);
    let step = period.map_or(600.0, |p| (p / LIGHT_SAMPLES as f64).min(600.0));
    let first = lit(after);
    let mut t0 = 0.0;
    while t0 < span {
        let t1 = (t0 + step).min(span);
        if lit(after.add_seconds(t1)) != first {
            // Bisect to a tenth of a second.
            let (mut low, mut high) = (t0, t1);
            while high - low > 0.1 {
                let middle = (low + high) / 2.0;
                if lit(after.add_seconds(middle)) == first {
                    low = middle;
                } else {
                    high = middle;
                }
            }
            return Some(after.add_seconds(high));
        }
        t0 = t1;
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::system::tests::rss;

    fn earth(system: &SolarSystem) -> BodyId {
        system.find("Earth").unwrap()
    }

    /// Where the Earth's shadow points at `t`, as a unit vector from the Earth.
    fn antisun(system: &SolarSystem, t: SimTime) -> DVec3 {
        -system
            .relative_state(system.root(), earth(system), t)
            .position
            .normalize()
    }

    #[test]
    fn sunlight_is_the_solar_constant_at_the_earth_and_falls_with_distance() {
        let system = rss();
        let t = SimTime::J2000;
        let out = -antisun(&system, t) * 42_000_000.0;
        let here = at(&system, earth(&system), out, t);
        assert!(
            here.solar_flux > 1300.0 && here.solar_flux < 1430.0,
            "{here:?}"
        );
        assert_eq!(here.sunlit, 1.0);
        let mars = system.find("Mars").unwrap();
        let sunward = system
            .relative_state(system.root(), mars, t)
            .position
            .normalize();
        let there = at(&system, mars, sunward * 20_000_000.0, t);
        assert!(
            there.solar_flux > 480.0 && there.solar_flux < 720.0,
            "{there:?}"
        );
    }

    #[test]
    fn the_earth_casts_an_umbra_and_a_penumbra() {
        let system = rss();
        let t = SimTime::J2000;
        let (id, away) = (earth(&system), antisun(&system, t));
        // Close behind it, the Sun is gone.
        let dark = at(&system, id, away * 7_000_000.0, t);
        assert_eq!((dark.sunlit, dark.solar_flux), (0.0, 0.0));
        // Towards the Sun it is full.
        assert_eq!(at(&system, id, -away * 7_000_000.0, t).sunlit, 1.0);
        // At the edge of the shadow, 1000 km out, some of the Sun shows: the penumbra.
        let across = away.cross(DVec3::Z).normalize();
        let distance = 1_000_000.0 * 100.0;
        let radius = system.body(id).radius;
        let mut partial = false;
        for k in 0..2000 {
            let off = radius * (0.5 + f64::from(k) * 0.0005);
            let share = at(&system, id, away * distance + across * off, t).sunlit;
            partial |= share > 0.01 && share < 0.99;
        }
        assert!(partial, "no penumbra found");
    }

    /// A circular orbit 185 km up, in the plane of the shadow's axis: the longest eclipse there
    /// can be is an arc of 2 asin(R / r), 37 of its 88 minutes, and any tilt to the Sun makes it
    /// less.
    #[test]
    fn a_parking_orbits_eclipse_matches_the_geometry() {
        let system = rss();
        let t = SimTime::J2000;
        let (id, away) = (earth(&system), antisun(&system, t));
        let body = system.body(id);
        let radius = body.radius + 185_000.0;
        let across = away.cross(DVec3::Z).normalize();
        let period = 2.0 * PI * (radius.powi(3) / body.gm).sqrt();
        assert!((period / 60.0 - 88.0).abs() < 2.0, "{period}");
        let samples = 3600;
        let mut dark = 0;
        for k in 0..samples {
            let angle = 2.0 * PI * f64::from(k) / f64::from(samples);
            let position = (away * angle.cos() + across * angle.sin()) * radius;
            if at(&system, id, position, t).sunlit < 0.5 {
                dark += 1;
            }
        }
        let minutes = period / 60.0 * f64::from(dark) / f64::from(samples);
        let geometry = period / 60.0 * (body.radius / radius).asin() / PI;
        assert!((minutes - geometry).abs() < 0.5, "{minutes} of {geometry}");
        assert!(minutes < 38.0, "{minutes}");
        // Tilted 30° out of the Sun's line it is shorter.
        let tilted = (away * 30f64.to_radians().cos()
            + across.cross(away) * 30f64.to_radians().sin())
        .normalize();
        let mut dark = 0;
        for k in 0..samples {
            let angle = 2.0 * PI * f64::from(k) / f64::from(samples);
            let position = (tilted * angle.cos() + across * angle.sin()) * radius;
            if at(&system, id, position, t).sunlit < 0.5 {
                dark += 1;
            }
        }
        let shorter = period / 60.0 * f64::from(dark) / f64::from(samples);
        assert!(shorter < minutes - 1.0, "{shorter} of {minutes}");
    }

    #[test]
    fn a_planet_gives_off_light_and_heat_by_its_albedo() {
        let system = rss();
        let t = SimTime::J2000;
        let (id, away) = (earth(&system), antisun(&system, t));
        let radius = system.body(id).radius + 300_000.0;
        // Over the day side it reflects a third or so; over the night side nothing, and both
        // radiate alike.
        let day = at(&system, id, -away * radius, t);
        let night = at(&system, id, away * radius, t);
        assert!(
            day.albedo_flux > 300.0 && day.albedo_flux < 450.0,
            "{day:?}"
        );
        assert!(night.albedo_flux < 1e-6, "{night:?}");
        assert!((day.infrared_flux / night.infrared_flux - 1.0).abs() < 1e-3);
        assert!(
            night.infrared_flux > 180.0 && night.infrared_flux < 260.0,
            "{night:?}"
        );
    }

    #[test]
    fn a_body_in_the_way_stops_the_line_of_sight() {
        let system = rss();
        let t = SimTime::J2000;
        let id = earth(&system);
        let radius = system.body(id).radius;
        let (a, b) = (DVec3::X * 2.0 * radius, DVec3::NEG_X * 2.0 * radius);
        assert_eq!(occulted_by(&system, id, a, b, t), Some(id));
        let c = DVec3::Y * 2.0 * radius;
        assert_eq!(occulted_by(&system, id, a, c, t), None);
        // A station on the ground sees straight up.
        let ground = DVec3::X * radius;
        assert_eq!(occulted_by(&system, id, ground, a, t), None);
    }

    #[test]
    fn a_coast_finds_where_it_goes_into_and_out_of_the_shadow() {
        let system = rss();
        let t = SimTime::J2000;
        let id = earth(&system);
        let body = system.body(id);
        let away = antisun(&system, t);
        let radius = body.radius + 185_000.0;
        let speed = (body.gm / radius).sqrt();
        // Starting in the light on the Sun's side, in the plane of the shadow's axis.
        let across = away.cross(DVec3::Z).normalize();
        let start = crate::orbit::StateVector {
            position: -away * radius,
            velocity: across * speed,
        };
        let orbit = Orbit::new(body.gm, t, start);
        let period = orbit.period().unwrap();
        let until = t.add_seconds(2.0 * period);
        let enter = next_light_change(&system, id, &orbit, t, until).unwrap();
        let leave = next_light_change(&system, id, &orbit, enter, until).unwrap();
        let dark = leave.seconds_since(enter);
        let want = period * (body.radius / radius).asin() / PI;
        assert!((dark - want).abs() < 60.0, "{dark} of {want}");
        let there = |at_t: SimTime| at(&system, id, orbit.state_at(at_t).position, at_t).sunlit;
        assert_eq!(there(t), 1.0);
        assert!(there(enter.add_seconds(10.0)) < 0.5);
        assert!(there(leave.add_seconds(10.0)) > 0.5);
    }
}
