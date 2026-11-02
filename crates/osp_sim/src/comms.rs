//! Links between vessels and the ground: a deterministic budget, a line of sight and a delay.
//!
//! What reaches a receiver is what was sent, with both antennas' gain, less what the distance
//! spreads it by (the free-space loss); it is compared with the noise of the receiver over the
//! bandwidth of the data. The margin is how far it is over what a link needs. A link is there if
//! it has a margin and no body lies across the line. A vessel can relay through another that has
//! a link of its own.

use std::f64::consts::PI;

use glam::DVec3;

use crate::{
    body::BodyId, constants::SPEED_OF_LIGHT, data::GroundStationDef, environment::occulted_by,
    orbit::Orbit, system::SolarSystem, time::SimTime,
};

/// Boltzmann's constant, J/K.
const BOLTZMANN: f64 = 1.380_649e-23;
/// S-band, Hz: what Apollo and the stations here speak.
pub const S_BAND: f64 = 2.2e9;
/// How well a dish turns its area into gain.
const DISH_EFFICIENCY: f64 = 0.55;
/// Signal over noise per bit a link needs, as a share: about 10 dB.
const NEEDED_PER_BIT: f64 = 10.0;

/// A dish's gain at `frequency`, over an antenna that sends all ways alike, as a share.
pub fn dish_gain(diameter: f64, frequency: f64) -> f64 {
    let wavelength = SPEED_OF_LIGHT / frequency;
    DISH_EFFICIENCY * (PI * diameter / wavelength).powi(2)
}

/// As decibels.
pub fn decibels(share: f64) -> f64 {
    10.0 * share.log10()
}

/// What sends and receives on a vessel.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Radio {
    /// What it sends, W.
    pub power: f64,
    /// As a share over all ways alike.
    pub gain: f64,
    /// What it asks of a link, bit/s.
    pub data_rate: f64,
}

/// A ground station, ready to use.
#[derive(Debug, Clone, PartialEq)]
pub struct Station {
    pub id: String,
    pub name: String,
    pub body: BodyId,
    /// Where it stands in the body's own frame, m.
    pub place: DVec3,
    /// As a share over all ways alike.
    pub gain: f64,
    /// Kelvin.
    pub noise: f64,
}

impl Station {
    /// `def` on the body it names; `None` if the system has none of that name.
    pub fn new(def: &GroundStationDef, system: &SolarSystem) -> Option<Self> {
        let body = system.find(&def.body)?;
        let (lat, lon) = (def.latitude.to_radians(), def.longitude.to_radians());
        let up = DVec3::new(lat.cos() * lon.cos(), lat.cos() * lon.sin(), lat.sin());
        let radius = system.body(body).radius;
        Some(Self {
            id: def.id.clone(),
            name: if def.name.is_empty() {
                def.id.clone()
            } else {
                def.name.clone()
            },
            body,
            place: up * radius,
            gain: dish_gain(def.dish, S_BAND),
            noise: def.noise,
        })
    }

    /// Where it is at `time`, from body `frame`, m.
    pub fn position(&self, system: &SolarSystem, frame: BodyId, time: SimTime) -> DVec3 {
        let body = system.body(self.body);
        system.relative_state(self.body, frame, time).position + body.orientation(time) * self.place
    }
}

/// A link to the ground.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    /// Which station.
    pub station: String,
    /// How far it is over what a link needs, dB; positive for a link that works.
    pub margin: f64,
    /// How long a signal takes, one way, s.
    pub delay: f64,
}

/// How far over what it needs a signal is at `distance`, dB: what `radio` sends to a receiver
/// of `gain` and `noise` kelvin.
pub fn margin(radio: &Radio, gain: f64, noise: f64, distance: f64) -> f64 {
    let wavelength = SPEED_OF_LIGHT / S_BAND;
    let spread = (wavelength / (4.0 * PI * distance.max(1.0))).powi(2);
    let received = radio.power * radio.gain * gain * spread;
    let noise_density = BOLTZMANN * noise.max(1.0);
    decibels(received / (noise_density * radio.data_rate.max(1.0) * NEEDED_PER_BIT))
}

/// The best station that has a line of sight and a margin to `radio` at `position` from `frame`.
pub fn best_link(
    system: &SolarSystem,
    stations: &[Station],
    frame: BodyId,
    position: DVec3,
    radio: &Radio,
    time: SimTime,
) -> Option<Link> {
    stations
        .iter()
        .filter_map(|station| {
            let at = station.position(system, frame, time);
            if occulted_by(system, frame, position, at, time).is_some() {
                return None;
            }
            let distance = (at - position).length();
            let margin = margin(radio, station.gain, station.noise, distance);
            (margin > 0.0).then(|| Link {
                station: station.id.clone(),
                margin,
                delay: distance / SPEED_OF_LIGHT,
            })
        })
        .max_by(|a, b| a.margin.total_cmp(&b.margin))
}

/// A relay: is there a line of sight and a margin between two radios, with the first's power and
/// gain to the second's gain, on an ordinary receiver?
pub fn relay_link(
    system: &SolarSystem,
    frame: BodyId,
    from: (DVec3, &Radio),
    to: (DVec3, &Radio),
    time: SimTime,
) -> Option<f64> {
    if occulted_by(system, frame, from.0, to.0, time).is_some() {
        return None;
    }
    let distance = (to.0 - from.0).length();
    let margin = margin(from.1, to.1.gain, RELAY_NOISE, distance);
    (margin > 0.0).then_some(distance / SPEED_OF_LIGHT)
}

/// A vessel's receiver, K.
const RELAY_NOISE: f64 = 500.0;

/// Steps a coast is looked at in, as a share of its period.
const LINK_SAMPLES: usize = 96;

/// The first moment after `after` and before `until` that `orbit` gains or loses a link.
pub fn next_link_change(
    system: &SolarSystem,
    stations: &[Station],
    frame: BodyId,
    orbit: &Orbit,
    radio: &Radio,
    after: SimTime,
    until: SimTime,
) -> Option<SimTime> {
    let linked = |t: SimTime| {
        best_link(
            system,
            stations,
            frame,
            orbit.state_at(t).position,
            radio,
            t,
        )
        .is_some()
    };
    let span = until.seconds_since(after);
    if span <= 0.0 {
        return None;
    }
    let period = orbit.period().filter(|p| p.is_finite() && *p > 0.0);
    let step = period.map_or(600.0, |p| (p / LINK_SAMPLES as f64).min(600.0));
    let first = linked(after);
    let mut t0 = 0.0;
    while t0 < span {
        let t1 = (t0 + step).min(span);
        if linked(after.add_seconds(t1)) != first {
            let (mut low, mut high) = (t0, t1);
            while high - low > 0.5 {
                let middle = (low + high) / 2.0;
                if linked(after.add_seconds(middle)) == first {
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
    use crate::{system::tests::rss, test_data::records};

    fn stations(system: &SolarSystem) -> Vec<Station> {
        let defs: Vec<GroundStationDef> = records(include_str!(
            "../../../assets/core/ground_stations/stations.json5"
        ));
        defs.iter()
            .map(|d| Station::new(d, system).expect("on a body there is"))
            .collect()
    }

    /// The CSM's S-band: about 20 W into four 0.8 m dishes, 51.2 kbit/s of telemetry.
    fn apollo() -> Radio {
        Radio {
            power: 20.0,
            gain: dish_gain(0.8, S_BAND),
            data_rate: 51_200.0,
        }
    }

    #[test]
    fn a_dish_has_the_gain_it_should() {
        // A 26 m dish at S-band was about 53 dB, and the CSM's about 22 dB.
        let big = decibels(dish_gain(26.0, S_BAND));
        assert!((big - 53.0).abs() < 2.0, "{big}");
        let small = decibels(dish_gain(0.8, S_BAND));
        assert!((small - 22.0).abs() < 3.0, "{small}");
    }

    #[test]
    fn the_csms_dish_reaches_the_ground_from_the_moon_and_a_whip_does_not() {
        let dsn = dish_gain(26.0, S_BAND);
        let moon = 384_400_000.0;
        let dish = margin(&apollo(), dsn, 50.0, moon);
        assert!(dish > 10.0 && dish < 40.0, "{dish}");
        let whip = Radio {
            gain: 1.0,
            ..apollo()
        };
        assert!(margin(&whip, dsn, 50.0, moon) < dish - 15.0);
        // Twice as far costs 6 dB.
        let far = margin(&apollo(), dsn, 50.0, 2.0 * moon);
        assert!((dish - far - 6.02).abs() < 0.01, "{}", dish - far);
    }

    #[test]
    fn a_signal_from_the_moon_takes_a_little_over_a_second() {
        let system = rss();
        let stations = stations(&system);
        let t = SimTime::J2000;
        let moon = system.find("Moon").unwrap();
        let earth = system.find("Earth").unwrap();
        // On the Earth's side of the Moon, 200 km up.
        let to_earth = system.relative_state(earth, moon, t).position.normalize();
        let position = to_earth * (system.body(moon).radius + 200_000.0);
        // Some station sees it whenever the Earth turns one under it; try the turn of a day.
        let link = (0..24)
            .map(|h| t.add_seconds(3600.0 * f64::from(h)))
            .find_map(|at| best_link(&system, &stations, moon, position, &apollo(), at))
            .expect("a station comes round");
        assert!((link.delay - 1.28).abs() < 0.1, "{link:?}");
        assert!(link.margin > 0.0);
    }

    #[test]
    fn the_far_side_of_the_moon_has_no_link() {
        let system = rss();
        let stations = stations(&system);
        let t = SimTime::J2000;
        let moon = system.find("Moon").unwrap();
        let earth = system.find("Earth").unwrap();
        let away = -system.relative_state(earth, moon, t).position.normalize();
        let position = away * (system.body(moon).radius + 110_000.0);
        for h in 0..24 {
            let at = t.add_seconds(3600.0 * f64::from(h));
            assert_eq!(
                best_link(&system, &stations, moon, position, &apollo(), at),
                None
            );
        }
    }

    /// An orbit round the Moon goes behind it and comes out again, about as long as the geometry
    /// says for the far side's arc.
    #[test]
    fn an_orbit_loses_and_gains_the_link_behind_the_moon() {
        use crate::orbit::StateVector;
        let system = rss();
        let stations = stations(&system);
        let t = SimTime::J2000;
        let moon = system.find("Moon").unwrap();
        let earth = system.find("Earth").unwrap();
        let body = system.body(moon);
        let radius = body.radius + 110_000.0;
        let towards = system.relative_state(earth, moon, t).position.normalize();
        let side = towards.cross(DVec3::Z).normalize();
        let start = StateVector {
            position: towards * radius,
            velocity: side * (body.gm / radius).sqrt(),
        };
        let orbit = Orbit::new(body.gm, t, start);
        let period = orbit.period().unwrap();
        let until = t.add_seconds(3.0 * period);
        let radio = apollo();
        let lost = next_link_change(&system, &stations, moon, &orbit, &radio, t, until);
        let lost = lost.expect("it goes behind");
        let found = next_link_change(&system, &stations, moon, &orbit, &radio, lost, until)
            .expect("it comes out");
        let dark = found.seconds_since(lost);
        // A little under half the orbit: the Moon hides what lies behind its disc as seen from
        // the Earth, a little less than half of the circle.
        assert!(
            dark > 0.3 * period && dark < 0.5 * period,
            "{dark} of {period}"
        );
    }

    /// A small uncrewed vessel with a dish, in `orbit` round the Earth, over the station.
    mod policy {
        use super::*;
        use crate::{
            data::{CraftDef, OrbitStartDef},
            plan::Burn,
            test_data::records,
            universe::Universe,
            vessel::{Vessel, tests::catalog},
        };

        fn def(crew: bool) -> CraftDef {
            let crew = if crew { ", { crew: { seats: 1 } }" } else { "" };
            records(&format!(
                r#"{{ id: "probe", name: "", parts: [
                    {{ id: "bus", shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 1 }}],
                        cap_bottom: true, cap_top: true }} }},
                      structure: {{ mass: 100 }},
                      components: [{{ container: {{}} }},
                        {{ antenna: {{ diameter: 1, power: 20, data_rate: 1000 }} }},
                        {{ avionics: {{ controlled_mass: 1000 }} }} {crew}] }}
                ] }}"#
            ))
            .remove(0)
        }

        /// A vessel `crew`ed or not, 42 000 km up, with the Earth's stations, one of which always
        /// sees it from there.
        fn universe(crew: bool) -> Universe {
            let system = rss();
            let t = SimTime::J2000;
            let start = OrbitStartDef {
                body: "Earth".into(),
                periapsis_altitude: 35_629e3,
                apoapsis_altitude: 35_629e3,
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: 0.0,
                spin: [0.0; 3],
            };
            let vessel = Vessel::in_orbit(&def(crew), &catalog(), &system, &start, t).unwrap();
            let defs: Vec<GroundStationDef> = records(include_str!(
                "../../../assets/core/ground_stations/stations.json5"
            ));
            let mut universe = Universe::new(system, vec![vessel], t);
            universe.focus = Some(universe.vessels[0].id());
            universe.set_stations(&defs);
            universe
        }

        fn burn(time: SimTime) -> Burn {
            Burn {
                time,
                delta_v: DVec3::X,
            }
        }

        #[test]
        fn an_uncrewed_vessel_needs_a_link_and_a_plan_arrives_late() {
            // In sight of a station: it answers, and what is sent takes the light's time to
            // arrive.
            let mut u = universe(false);
            let link = u.vessels[0].link().cloned().expect("a station in sight");
            assert!(link.delay > 0.11 && link.delay < 0.16, "{link:?}");
            assert!(u.vessels[0].commandable());
            let now = u.time();
            assert!(u.vessels[0].upload_burn(burn(now.add_seconds(1000.0)), now));
            assert!(u.vessels[0].plan.is_empty());
            // Not there yet a tick later, there after the delay.
            u.step_physics();
            assert!(u.vessels[0].plan.is_empty());
            for _ in 0..10 {
                u.step_physics();
            }
            assert_eq!(u.vessels[0].plan.burns().len(), 1);

            // With nobody to talk to it does not answer.
            let mut u = universe(false);
            u.set_stations(&[]);
            assert!(u.vessels[0].link().is_none());
            assert!(!u.vessels[0].commandable());
            let before = u.vessels[0].propulsion.throttle();
            u.vessels[0].set_throttle(1.0);
            assert_eq!(u.vessels[0].propulsion.throttle(), before);
            let now = u.time();
            assert!(!u.vessels[0].upload_burn(burn(now.add_seconds(1000.0)), now));
            assert!(u.vessels[0].plan.is_empty());
        }

        #[test]
        fn a_crew_answers_without_a_link() {
            let mut u = universe(true);
            u.set_stations(&[]);
            assert!(u.vessels[0].commandable());
            let now = u.time();
            assert!(u.vessels[0].upload_burn(burn(now.add_seconds(1000.0)), now));
            assert_eq!(u.vessels[0].plan.burns().len(), 1);
        }
    }
}
