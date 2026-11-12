//! Radiation at a place, by kind: galactic cosmic rays, which a planet's magnetic field keeps some
//! of out and its bulk blocks from part of the sky; the particles its field holds in belts; and what
//! its ground gives off. Each kind goes through shielding differently, so they are kept apart until
//! a part's walls say how much of each gets in ([`Radiation::through`]).
//!
//! The rates are the bodies' (`radiation` in their records), in mSv/h for an unshielded person;
//! here they are Sv/s. A belt follows the field lines of a dipole on its body's spin axis: a point
//! `r` from the centre at magnetic latitude λ is on the line that crosses the equator `r / cos²λ`
//! out. Over a coast the dose is integrated along the orbit, a panel at a time, each short enough
//! for the vessel to move a fiftieth of its distance from the body, so a long step and many short
//! ones come to the same, belt crossings and all.

use glam::DVec3;

use crate::{
    body::BodyId, data::RadiationKindsDef, orbit::Orbit, system::SolarSystem, time::SimTime,
};

/// One mSv/h, Sv/s.
pub const MSV_PER_HOUR: f64 = 1e-3 / 3_600.0;

/// A figure for each kind of radiation: a rate (Sv/s), a dose (Sv), or the share of each that gets
/// through.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Radiation {
    pub cosmic: f64,
    pub trapped: f64,
    pub surface: f64,
}

impl Radiation {
    pub const NONE: Self = Self {
        cosmic: 0.0,
        trapped: 0.0,
        surface: 0.0,
    };

    pub fn total(&self) -> f64 {
        self.cosmic + self.trapped + self.surface
    }

    /// Each kind times the other's: a dose through the shares that get through.
    pub fn weighted(&self, shares: &Radiation) -> f64 {
        self.cosmic * shares.cosmic + self.trapped * shares.trapped + self.surface * shares.surface
    }

    fn plus(self, other: Radiation, times: f64) -> Self {
        Self {
            cosmic: self.cosmic + other.cosmic * times,
            trapped: self.trapped + other.trapped * times,
            surface: self.surface + other.surface * times,
        }
    }

    pub fn scaled(self, times: f64) -> Self {
        Self::NONE.plus(self, times)
    }

    /// The share of each kind that gets through `shielding` kg/m², each cut to 1/e by `lengths`.
    pub fn through(shielding: f64, lengths: &RadiationKindsDef) -> Self {
        let share = |length: f64| {
            if length > 0.0 {
                (-shielding.max(0.0) / length).exp()
            } else {
                0.0
            }
        };
        Self {
            cosmic: share(lengths.cosmic),
            trapped: share(lengths.trapped),
            surface: share(lengths.surface),
        }
    }
}

/// x²(3 - 2x) on 0..1: from 0 to 1 with no kink at either end.
fn smooth(x: f64) -> f64 {
    let x = x.clamp(0.0, 1.0);
    x * x * (3.0 - 2.0 * x)
}

/// The radiation at `position` from body `frame` (inertial axes), Sv/s by kind, before any
/// shielding, the air's included; and the air above it, kg/m², which shields like any wall.
pub fn at(system: &SolarSystem, frame: BodyId, position: DVec3) -> (Radiation, f64) {
    let root = system.root();
    let cosmic = (system.body(root).radiation.as_ref()).map_or(0.0, |r| r.cosmic) * MSV_PER_HOUR;
    let body = system.body(frame);
    let r = position.length();
    if frame == root || r <= 0.0 {
        return (
            Radiation {
                cosmic,
                ..Radiation::NONE
            },
            0.0,
        );
    }
    let near = (body.radius / r).min(1.0);
    // The share of the whole sky it fills: half of it on the ground.
    let sky = (1.0 - (1.0 - near * near).sqrt()) / 2.0;
    let mut out = Radiation {
        cosmic: cosmic * (1.0 - sky),
        ..Radiation::NONE
    };
    if let Some(def) = &body.radiation {
        out.cosmic *= 1.0 - def.magnetosphere * near * near;
        out.surface = def.surface * MSV_PER_HOUR * 2.0 * sky;
        let up = position / r;
        let sin_latitude = up.dot(body.pole()).clamp(-1.0, 1.0);
        let latitude = sin_latitude.asin().abs().to_degrees();
        let cos2 = 1.0 - sin_latitude * sin_latitude;
        let radii = r / body.radius;
        let shell = if cos2 > 1e-12 {
            radii / cos2
        } else {
            f64::INFINITY
        };
        let kept = if def.floor > 0.0 {
            smooth((radii - 1.0 - def.floor) / def.floor)
        } else {
            1.0
        };
        out.trapped = (def.belts.iter())
            .map(|belt| {
                let along = if shell <= belt.inner || shell >= belt.outer {
                    0.0
                } else if shell < belt.peak {
                    smooth((shell - belt.inner) / (belt.peak - belt.inner))
                } else {
                    smooth((belt.outer - shell) / (belt.outer - belt.peak))
                };
                let off = (latitude / belt.latitude.max(1e-6)).powi(2);
                belt.rate * MSV_PER_HOUR * along * (-off).exp() * kept
            })
            .sum();
    }
    let column = body.atmosphere.as_ref().map_or(0.0, |air| {
        let altitude = r - body.radius;
        if altitude >= air.height {
            return 0.0;
        }
        let gravity = body.gm / (body.radius * body.radius);
        air.sample(altitude.max(0.0)).pressure / gravity
    });
    (out, column)
}

/// The panel a coast is integrated over from a state at `position` moving at `velocity`: a
/// fiftieth of the way from the body, between a second and ten minutes.
fn panel(position: DVec3, velocity: DVec3) -> f64 {
    let speed = velocity.length();
    if speed <= 0.0 {
        return 600.0;
    }
    (0.02 * position.length() / speed).clamp(1.0, 600.0)
}

/// A coast's panels from `start` for `span` seconds: each one's start, length, and the radiation
/// at its start, middle and end (shared with the next).
struct Panels<'a> {
    system: &'a SolarSystem,
    frame: BodyId,
    orbit: &'a Orbit,
    start: SimTime,
}

impl Panels<'_> {
    fn rate(&self, t: f64) -> Radiation {
        let position = self.orbit.state_at(self.start.add_seconds(t)).position;
        at(self.system, self.frame, position).0
    }

    /// What falls over `t..t + h`, Sv by kind, with the rate at `t` known: Simpson's rule.
    fn over(&self, t: f64, h: f64, first: Radiation) -> (Radiation, Radiation) {
        let middle = self.rate(t + h / 2.0);
        let last = self.rate(t + h);
        let dose = (first.plus(middle, 4.0).plus(last, 1.0)).scaled(h / 6.0);
        (dose, last)
    }

    fn length(&self, t: f64) -> f64 {
        let state = self.orbit.state_at(self.start.add_seconds(t));
        panel(state.position, state.velocity)
    }
}

/// What falls on a vessel coasting `orbit` (about `frame`) from `start` for `span` seconds, Sv by
/// kind, before any shielding.
pub fn over_coast(
    system: &SolarSystem,
    frame: BodyId,
    orbit: &Orbit,
    start: SimTime,
    span: f64,
) -> Radiation {
    let panels = Panels {
        system,
        frame,
        orbit,
        start,
    };
    let mut total = Radiation::NONE;
    let mut t = 0.0;
    let mut first = panels.rate(0.0);
    while t < span {
        let h = panels.length(t).min(span - t);
        let (dose, last) = panels.over(t, h, first);
        total = total.plus(dose, 1.0);
        (t, first) = (t + h, last);
    }
    total
}

/// The first moment after `after` and before `until` that coasting `orbit` has given `dose` Sv,
/// each kind weighted by `shares` (what gets through a part's walls). It is found to a
/// millisecond, and never before the dose is reached by the same sums a step to it does.
pub fn time_to_dose(
    system: &SolarSystem,
    frame: BodyId,
    orbit: &Orbit,
    after: SimTime,
    until: SimTime,
    shares: &Radiation,
    dose: f64,
) -> Option<SimTime> {
    let span = until.seconds_since(after);
    if span <= 0.0 || dose <= 0.0 {
        return None;
    }
    let panels = Panels {
        system,
        frame,
        orbit,
        start: after,
    };
    let mut taken = 0.0;
    let mut t = 0.0;
    let mut first = panels.rate(0.0);
    while t < span {
        let h = panels.length(t).min(span - t);
        let (got, last) = panels.over(t, h, first);
        if taken + got.weighted(shares) >= dose {
            // Within this panel: the shortest stretch of it that gives the rest.
            let (mut low, mut high) = (0.0, h);
            while high - low > 1e-3 {
                let middle = (low + high) / 2.0;
                let part = panels.over(t, middle, first).0.weighted(shares);
                if taken + part >= dose {
                    high = middle;
                } else {
                    low = middle;
                }
            }
            return Some(after.add_seconds(t + high));
        }
        taken += got.weighted(shares);
        (t, first) = (t + h, last);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        orbit::{Orbit, StateVector},
        system::tests::rss,
        time::SimTime,
    };

    const DAY: f64 = 86_400.0;

    /// Sv a day, by kind, at a point `altitude` m over `body`'s equator, or at `latitude` degrees.
    fn daily(system: &SolarSystem, body: &str, altitude: f64, latitude: f64) -> Radiation {
        let id = system.find(body).unwrap();
        let b = system.body(id);
        let pole = b.pole();
        let across = pole.any_orthonormal_vector();
        let up =
            (across * latitude.to_radians().cos() + pole * latitude.to_radians().sin()).normalize();
        at(system, id, up * (b.radius + altitude)).0.scaled(DAY)
    }

    #[test]
    fn the_belts_are_far_worse_than_low_orbit_and_the_ground_is_kinder_than_orbit() {
        let system = rss();
        let low = daily(&system, "Earth", 400e3, 0.0);
        let station = daily(&system, "Earth", 400e3, 50.0);
        let belt = daily(&system, "Earth", 3_500e3, 0.0);
        // Low orbit is cosmic rays only, much of them kept out: about a third of a mSv a day.
        assert_eq!(low.trapped, 0.0);
        assert_eq!(
            station.trapped, 0.0,
            "the air takes the belts' particles this low"
        );
        assert!(low.total() > 0.2e-3 && low.total() < 0.6e-3, "{low:?}");
        assert!(belt.total() > 1_000.0 * low.total(), "{belt:?} {low:?}");
        // The outer belt is there, off the equator less of it.
        let outer = daily(&system, "Earth", 3.5 * 6_371e3, 0.0);
        let off = daily(&system, "Earth", 3.5 * 6_371e3, 20.0);
        assert!(outer.trapped > 0.0 && off.trapped < outer.trapped);

        let orbit = daily(&system, "Moon", 100e3, 0.0);
        let ground = daily(&system, "Moon", 0.0, 0.0);
        assert!(ground.total() < orbit.total(), "{ground:?} {orbit:?}");
        assert!(
            ground.surface > orbit.surface,
            "the ground gives off more on it"
        );
        // In the air, at the bottom of it, next to nothing.
        let earth = system.find("Earth").unwrap();
        let radius = system.body(earth).radius;
        let (_, column) = at(&system, earth, DVec3::X * radius);
        assert!(column > 10_000.0, "{column} kg/m²");
    }

    #[test]
    fn shielding_lets_less_through_and_least_of_the_belts() {
        let lengths = RadiationKindsDef {
            cosmic: 400.0,
            trapped: 20.0,
            surface: 100.0,
        };
        let thin = Radiation::through(20.0, &lengths);
        let thick = Radiation::through(100.0, &lengths);
        assert!(thick.cosmic < thin.cosmic && thick.trapped < thin.trapped);
        assert!(thick.trapped < thick.surface && thick.surface < thick.cosmic);
        assert!((thin.trapped - (-1.0_f64).exp()).abs() < 1e-12);
    }

    /// A transfer orbit through both belts: a day of it in one step comes to what it does a minute
    /// at a time, and the dose is reached where the sums say.
    #[test]
    fn a_coast_through_the_belts_gives_the_same_dose_in_one_step_or_many() {
        let system = rss();
        let earth = system.find("Earth").unwrap();
        let body = system.body(earth);
        let (perigee, apogee) = (body.radius + 300e3, body.radius + 35_786e3);
        let a = (perigee + apogee) / 2.0;
        let speed = (body.gm * (2.0 / perigee - 1.0 / a)).sqrt();
        let start = SimTime::J2000;
        let state = StateVector::new(DVec3::X * perigee, DVec3::Y * speed);
        let orbit = Orbit::new(body.gm, start, state);
        let whole = over_coast(&system, earth, &orbit, start, DAY);
        let mut parts = Radiation::NONE;
        for k in 0..1_440 {
            let t = start.add_seconds(60.0 * f64::from(k));
            parts = parts.plus(over_coast(&system, earth, &orbit, t, 60.0), 1.0);
        }
        assert!(whole.trapped > 0.0);
        for (one, many) in [
            (whole.cosmic, parts.cosmic),
            (whole.trapped, parts.trapped),
            (whole.surface, parts.surface),
        ] {
            assert!((one - many).abs() <= 1e-6 * one.max(1e-12), "{one} {many}");
        }
        let shares = Radiation::through(
            50.0,
            &RadiationKindsDef {
                cosmic: 400.0,
                trapped: 20.0,
                surface: 100.0,
            },
        );
        let half = whole.weighted(&shares) / 2.0;
        let until = start.add_seconds(DAY);
        let when = time_to_dose(&system, earth, &orbit, start, until, &shares, half).unwrap();
        let got = over_coast(&system, earth, &orbit, start, when.seconds_since(start));
        assert!(got.weighted(&shares) >= half);
        let before = over_coast(
            &system,
            earth,
            &orbit,
            start,
            when.seconds_since(start) - 0.01,
        );
        assert!(before.weighted(&shares) < half);
    }
}
