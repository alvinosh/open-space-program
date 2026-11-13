//! Links between vessels and the ground: a deterministic budget, a line of sight and a delay.
//!
//! What reaches a receiver is what was sent, with both antennas' gain, less what the distance
//! spreads it by (the free-space loss); it is compared with the noise of the receiver over the
//! bandwidth of the data. The margin is how far it is over what a link needs. A link is there if
//! it has a margin and no body lies across the line. A vessel out of reach of the ground can go
//! through a vessel with a relay antenna that has a link of its own, by as many hops as it takes.
//!
//! A link carries data at its radio's rate while it has [`FULL_RATE_MARGIN`] to spare, and slower
//! as the margin falls below that, down to a tenth at its edge ([`rate`]): so further away, slower.
//! Through relays the slowest hop sets it, and vessels sending through one relay share what it
//! passes on ([`shared_rates`]).

use std::f64::consts::PI;

use glam::DVec3;
use serde::{Deserialize, Serialize};

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
/// The margin a link sends at its radio's full rate with, dB: below it, the radio codes its data
/// more heavily to keep it whole, and sends that much slower.
pub const FULL_RATE_MARGIN: f64 = 10.0;

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
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct Radio {
    /// What it sends, W.
    pub power: f64,
    /// As a share over all ways alike.
    pub gain: f64,
    /// What it asks of a link, bit/s.
    pub data_rate: f64,
}

/// A ground station, ready to use.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
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
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Link {
    /// Which station.
    pub station: String,
    /// How far it is over what a link needs, dB; positive for a link that works.
    pub margin: f64,
    /// How long a signal takes, one way, s.
    pub delay: f64,
    /// How many relays it goes through.
    #[serde(default)]
    pub hops: u32,
    /// What it carries, bit/s: its slowest hop's [`rate`], before what it shares with other vessels
    /// going through the same relays.
    #[serde(default)]
    pub rate: f64,
    /// The vessels it goes through, nearest first, by their place in the list it was worked out
    /// from. Not saved: it is worked out again.
    #[serde(skip)]
    pub relays: Vec<usize>,
}

/// What `radio` sends at over a link `margin` dB over what it needs, bit/s: all of its rate with
/// [`FULL_RATE_MARGIN`] to spare, less below that, a tenth at the link's edge, and nothing past it.
pub fn rate(radio: &Radio, margin: f64) -> f64 {
    if margin <= 0.0 {
        return 0.0;
    }
    radio.data_rate * 10f64.powf((margin - FULL_RATE_MARGIN).min(0.0) / 10.0)
}

/// What each vessel with a link sends at, bit/s, when those `sending` all send at once: a relay
/// passes on no more than its own link carries, shared alike among everyone whose data goes through
/// it, its own included. `links` as [`spread_links`] works them out.
pub fn shared_rates(links: &[Option<Link>], sending: &[bool]) -> Vec<f64> {
    // How many send over each vessel's own link to the ground: itself, and those through it.
    let mut users = vec![0usize; links.len()];
    for (i, link) in links.iter().enumerate() {
        let Some(link) = link.as_ref().filter(|_| sending[i]) else {
            continue;
        };
        users[i] += 1;
        for &relay in &link.relays {
            users[relay] += 1;
        }
    }
    (links.iter().enumerate())
        .map(|(i, link)| {
            let Some(link) = link.as_ref().filter(|_| sending[i]) else {
                return 0.0;
            };
            let onward = |j: usize| {
                links[j]
                    .as_ref()
                    .map_or(0.0, |l| l.rate / users[j].max(1) as f64)
            };
            (link.relays.iter())
                .map(|&j| onward(j))
                .fold(link.rate / users[i].max(1) as f64, f64::min)
        })
        .collect()
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
                hops: 0,
                rate: rate(radio, margin),
                relays: Vec::new(),
            })
        })
        .max_by(|a, b| a.margin.total_cmp(&b.margin))
}

/// A hop between two radios: how long a signal takes and the margin of the weaker way, if there
/// is a line of sight and a margin both ways on ordinary receivers.
pub fn relay_link(
    system: &SolarSystem,
    frame: BodyId,
    from: (DVec3, &Radio),
    to: (DVec3, &Radio),
    time: SimTime,
) -> Option<(f64, f64)> {
    if occulted_by(system, frame, from.0, to.0, time).is_some() {
        return None;
    }
    let distance = (to.0 - from.0).length();
    let weaker = margin(from.1, to.1.gain, RELAY_NOISE, distance).min(margin(
        to.1,
        from.1.gain,
        RELAY_NOISE,
        distance,
    ));
    (weaker > 0.0).then_some((distance / SPEED_OF_LIGHT, weaker))
}

/// What is on the air: where a vessel is, its best radio, and its best relay antenna, if each is on.
#[derive(Debug, Clone, Copy)]
pub struct OnTheAir {
    pub position: DVec3,
    pub radio: Option<Radio>,
    pub relay: Option<Radio>,
}

/// Each vessel's link, from those that reach the ground themselves (`direct`), spreading out through
/// relays one hop at a time so each gets the fewest hops, the shortest way among those. A hop's
/// margin is the weakest on the way. Positions are from `frame`.
pub fn spread_links(
    system: &SolarSystem,
    frame: BodyId,
    vessels: &[OnTheAir],
    direct: Vec<Option<Link>>,
    time: SimTime,
) -> Vec<Option<Link>> {
    let mut links = direct;
    for _ in 0..vessels.len() {
        let mut found = Vec::new();
        for (i, vessel) in vessels.iter().enumerate() {
            let Some(radio) = vessel.radio.as_ref().filter(|_| links[i].is_none()) else {
                continue;
            };
            let through = (vessels.iter().zip(&links).enumerate())
                .filter(|(j, _)| *j != i)
                .filter_map(|(j, (other, link))| {
                    let (link, relay) = (link.as_ref()?, other.relay.as_ref()?);
                    let (delay, margin) = relay_link(
                        system,
                        frame,
                        (vessel.position, radio),
                        (other.position, relay),
                        time,
                    )?;
                    Some(Link {
                        station: link.station.clone(),
                        margin: margin.min(link.margin),
                        delay: delay + link.delay,
                        hops: link.hops + 1,
                        rate: rate(radio, margin).min(link.rate),
                        relays: std::iter::once(j)
                            .chain(link.relays.iter().copied())
                            .collect(),
                    })
                })
                .min_by(|a, b| a.delay.total_cmp(&b.delay));
            if let Some(link) = through {
                found.push((i, link));
            }
        }
        if found.is_empty() {
            break;
        }
        for (i, link) in found {
            links[i] = Some(link);
        }
    }
    links
}

/// How far `radio` reaches a receiver of `gain` and `noise` kelvin, m: where its margin runs out.
pub fn reach(radio: &Radio, gain: f64, noise: f64) -> f64 {
    let wavelength = SPEED_OF_LIGHT / S_BAND;
    let needed = BOLTZMANN * noise.max(1.0) * radio.data_rate.max(1.0) * NEEDED_PER_BIT;
    wavelength / (4.0 * PI) * (radio.power * radio.gain * gain / needed).sqrt()
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

    /// Around the Moon: a probe on the far side, a relay high over the limb that sees it, and a
    /// relay out to the side that sees that one and the Earth.
    fn far_side(far_relay: bool) -> (Vec<OnTheAir>, SolarSystem, SimTime, BodyId) {
        let system = rss();
        let t = SimTime::J2000;
        let moon = system.find("Moon").unwrap();
        let earth = system.find("Earth").unwrap();
        let away = -system.relative_state(earth, moon, t).position.normalize();
        let side = away.cross(DVec3::Z).normalize();
        let r = system.body(moon).radius;
        let dish = Radio {
            power: 20.0,
            gain: dish_gain(1.0, S_BAND),
            data_rate: 10_000.0,
        };
        let vessels = vec![
            OnTheAir {
                position: away * (r + 110_000.0),
                radio: Some(dish),
                relay: None,
            },
            OnTheAir {
                position: away * (2.0 * r) + side * (3.0 * r),
                radio: Some(dish),
                relay: far_relay.then_some(dish),
            },
            OnTheAir {
                position: -away * (3.0 * r) + side * (1.5 * r),
                radio: Some(dish),
                relay: Some(dish),
            },
        ];
        (vessels, system, t, moon)
    }

    fn ground() -> Link {
        Link {
            station: "goldstone".into(),
            margin: 9.0,
            delay: 1.3,
            hops: 0,
            rate: 1000.0,
            relays: Vec::new(),
        }
    }

    #[test]
    fn a_probe_behind_the_moon_reaches_the_ground_through_two_relays() {
        let (vessels, system, t, moon) = far_side(true);
        let direct = vec![None, None, Some(ground())];
        let links = spread_links(&system, moon, &vessels, direct, t);
        let probe = links[0].as_ref().expect("through the relays");
        assert_eq!(probe.hops, 2);
        assert_eq!(links[1].as_ref().unwrap().hops, 1);
        assert_eq!(links[2].as_ref().unwrap().hops, 0);
        assert_eq!(probe.station, "goldstone");
        assert!(probe.delay > 1.3 && probe.delay < 1.3 + 0.2, "{probe:?}");
        assert!(probe.margin <= 9.0);
    }

    #[test]
    fn only_a_relay_antenna_passes_signals_on() {
        let (vessels, system, t, moon) = far_side(false);
        let direct = vec![None, None, Some(ground())];
        let links = spread_links(&system, moon, &vessels, direct, t);
        assert!(links[0].is_none(), "the one in the middle is no relay");
        assert_eq!(
            links[1].as_ref().unwrap().hops,
            1,
            "it reaches the relay itself"
        );
    }

    #[test]
    fn no_chain_of_relays_helps_with_nothing_on_the_ground() {
        let (vessels, system, t, moon) = far_side(true);
        let links = spread_links(&system, moon, &vessels, vec![None, None, None], t);
        assert!(links.iter().all(Option::is_none));
    }

    #[test]
    fn a_hop_has_to_work_both_ways() {
        let (mut vessels, system, t, moon) = far_side(true);
        // Far too faint to answer: the relay hears it and it does not hear the relay.
        vessels[1].relay = Some(Radio {
            power: 1e-9,
            ..vessels[1].relay.unwrap()
        });
        let direct = vec![None, None, Some(ground())];
        let links = spread_links(&system, moon, &vessels, direct, t);
        assert!(links[0].is_none());
    }

    #[test]
    fn reach_is_where_the_margin_runs_out() {
        let dsn = dish_gain(26.0, S_BAND);
        let far = reach(&apollo(), dsn, 50.0);
        assert!(margin(&apollo(), dsn, 50.0, 0.99 * far) > 0.0);
        assert!(margin(&apollo(), dsn, 50.0, 1.01 * far) < 0.0);
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
            data::{CraftDef, OrbitStartDef, RulesDef},
            plan::Burn,
            systems::Antenna,
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

        /// Two vessels in the same high orbit, 100 km apart: `probe` with a radio too faint for the
        /// ground, and another with a big dish that reaches it, a relay if `relay` says so.
        fn probe_and_dish(relay: bool) -> Universe {
            let system = rss();
            let t = SimTime::J2000;
            let vessel = |antenna: String, mean_anomaly: f64| {
                let craft: CraftDef = records(&format!(
                    r#"{{ id: "x", name: "", parts: [{{ id: "bus",
                        shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 1 }}],
                            cap_bottom: true, cap_top: true }} }},
                        structure: {{ mass: 100 }},
                        components: [{{ container: {{}} }}, {{ antenna: {antenna} }},
                            {{ avionics: {{ controlled_mass: 1000 }} }}] }}] }}"#
                ))
                .remove(0);
                let start = OrbitStartDef {
                    body: "Earth".into(),
                    periapsis_altitude: 35_629e3,
                    apoapsis_altitude: 35_629e3,
                    inclination: 0.0,
                    longitude_of_ascending_node: 0.0,
                    plane: None,
                    argument_of_periapsis: 0.0,
                    mean_anomaly,
                    spin: [0.0; 3],
                };
                Vessel::in_orbit(&craft, &catalog(), &system, &start, t).unwrap()
            };
            // 10 dB short of the best station there is, at the nearest it could be.
            let unit = Radio {
                power: 1.0,
                gain: 1.0,
                data_rate: 1000.0,
            };
            let at_one_watt = margin(&unit, dish_gain(64.0, S_BAND), 20.0, 35_629e3);
            let power = 10f64.powf((-10.0 - at_one_watt) / 10.0);
            let probe = vessel(format!("{{ power: {power:e}, data_rate: 1000 }}"), 0.0);
            let dish = vessel(
                format!("{{ diameter: 10, power: 100, data_rate: 1000, relay: {relay} }}"),
                100_000.0 / 42_000_000.0,
            );
            let defs: Vec<GroundStationDef> = records(include_str!(
                "../../../assets/core/ground_stations/stations.json5"
            ));
            let mut universe = Universe::new(system, vec![probe, dish], t);
            universe.set_stations(&defs);
            universe
        }

        #[test]
        fn a_faint_probe_reaches_the_ground_only_through_a_working_relay() {
            let mut universe = probe_and_dish(true);
            universe.update_links();
            assert_eq!(universe.vessels[1].link().map(|l| l.hops), Some(0));
            let link = universe.vessels[0].link().expect("through the relay");
            assert_eq!(link.hops, 1);
            assert!(universe.vessels[0].commandable());

            // Shut the relay down and there is nothing to go through.
            let systems = &mut universe.vessels[1].propulsion.systems;
            let (part, _) = systems.all::<Antenna>().next().unwrap();
            assert!(systems.act(part, crate::systems::ANTENNA_OFF).is_some());
            universe.update_links();
            assert!(universe.vessels[0].link().is_none());
            assert!(!universe.vessels[0].commandable());
        }

        #[test]
        fn a_probe_core_with_no_radio_answers_only_in_a_casual_game() {
            let system = rss();
            let start = OrbitStartDef {
                body: "Earth".into(),
                periapsis_altitude: 200e3,
                apoapsis_altitude: 200e3,
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: 0.0,
                spin: [0.0; 3],
            };
            let craft = |crew: &str, probe: bool| -> Vessel {
                let def: CraftDef = records(&format!(
                    r#"{{ id: "x", name: "", parts: [{{ id: "bus",
                        shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 1 }}],
                            cap_bottom: true, cap_top: true }} }},
                        structure: {{ mass: 100 }},
                        components: [{{ container: {{}} }},
                            {{ avionics: {{ controlled_mass: 1000, needs_link: {probe} }} }} {crew}] }}] }}"#
                ))
                .remove(0);
                Vessel::in_orbit(&def, &catalog(), &system, &start, SimTime::J2000).unwrap()
            };
            let realistic = RulesDef::default();
            let casual = RulesDef {
                avionics: false,
                ..RulesDef::default()
            };
            let mut probe = craft("", true);
            assert!(probe.commandable(), "outside a game it flies as it did");
            probe.propulsion.require_control_under(&realistic);
            assert!(!probe.commandable());
            probe.propulsion.require_control_under(&casual);
            assert!(probe.commandable());
            let mut crewed = craft(", { crew: { seats: 1 } }", true);
            crewed.propulsion.require_control_under(&realistic);
            assert!(crewed.commandable(), "someone aboard needs no radio");
            // A launcher's own flight computer is not a probe core.
            let mut launcher = craft("", false);
            launcher.propulsion.require_control_under(&realistic);
            assert!(launcher.commandable());
        }

        /// Telemetry only sends: a sounding rocket with it flies whether it has a link or not, a
        /// probe core cannot be told anything through it, and its data still goes home over it.
        #[test]
        fn telemetry_sends_data_but_carries_no_commands() {
            let system = rss();
            let t = SimTime::J2000;
            let vessel = |probe: bool| -> Vessel {
                let def: CraftDef = records(&format!(
                    r#"{{ id: "x", name: "", parts: [{{ id: "bus",
                        shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 1 }}],
                            cap_bottom: true, cap_top: true }} }},
                        structure: {{ mass: 100 }},
                        components: [{{ container: {{}} }},
                            {{ antenna: {{ power: 1, data_rate: 512, telemetry: true }} }},
                            {{ avionics: {{ controlled_mass: 1000, needs_link: {probe} }} }}] }}] }}"#
                ))
                .remove(0);
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
                let mut v = Vessel::in_orbit(&def, &catalog(), &system, &start, t).unwrap();
                v.propulsion.require_control_under(&RulesDef::default());
                v
            };
            let mut rocket = vessel(false);
            assert!(rocket.commandable(), "no link, and nothing waits on one");
            let antenna = rocket.propulsion.systems.all::<Antenna>().next().unwrap().1;
            assert!(antenna.telemetry && !antenna.relay);
            rocket.set_link(Some(ground()));
            assert!(rocket.commandable());
            let mut probe = vessel(true);
            probe.set_link(Some(ground()));
            assert!(!probe.commandable(), "telemetry hears nothing");
            let now = probe.coasting().map_or(t, |_| t);
            assert!(!probe.upload_burn(burn(now.add_seconds(100.0)), now));
        }

        #[test]
        fn a_dish_that_is_no_relay_passes_nothing_on() {
            let mut universe = probe_and_dish(false);
            universe.update_links();
            assert!(universe.vessels[1].link().is_some());
            assert!(universe.vessels[0].link().is_none());
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

    /// Data sent home: how fast, how a pass over a station sends what it does at any warp, and
    /// relays shared.
    mod sending {
        use super::*;
        use crate::{
            career::Career,
            data::{CraftDef, OrbitStartDef},
            systems::Held,
            test_data::records,
            universe::Universe,
            vessel::{Vessel, tests::catalog},
        };

        /// A probe with a 1 m dish of `data_rate` bit/s, an RTG to keep it on and a drive holding
        /// `bytes` of one whole set of data worth [`WORTH`], `altitude` up over the Earth's equator, alone with the
        /// ground `stations`, sending at the links' rates if `rates` says so, in a career.
        fn probe(
            altitude: f64,
            data_rate: f64,
            bytes: f64,
            stations: &str,
            rates: bool,
        ) -> Universe {
            let system = rss();
            let t = SimTime::J2000;
            let craft: CraftDef = records(&format!(
                r#"{{ id: "probe", name: "Probe", parts: [{{ id: "bus",
                    shape: {{ shell: {{ bottom_diameter: 1, sections: [{{ length: 1 }}],
                        cap_bottom: true, cap_top: true }} }},
                    structure: {{ mass: 100 }},
                    components: [{{ container: {{}} }},
                        {{ antenna: {{ diameter: 1, power: 20, data_rate: {data_rate} }} }},
                        {{ rtg: {{ power: 500 }} }}, {{ drive: {{ data: 1e9 }} }}] }}] }}"#
            ))
            .remove(0);
            let start = OrbitStartDef {
                body: "Earth".into(),
                periapsis_altitude: altitude,
                apoapsis_altitude: altitude,
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: 0.0,
                spin: [0.0; 3],
            };
            let mut vessel = Vessel::in_orbit(&craft, &catalog(), &system, &start, t).unwrap();
            put(&mut vessel, bytes);
            let mut universe = Universe::new(system, vec![vessel], t);
            universe.focus = Some(universe.vessels[0].id());
            universe.set_stations(&records::<GroundStationDef>(stations));
            universe.data_rates = rates;
            universe.career = Some(Career {
                tree: "t".into(),
                science: 0.0,
                funds: 0.0,
                facilities: Default::default(),
                unlocked: Default::default(),
                returned: Default::default(),
            });
            universe
        }

        /// Put a whole set of `bytes` of data worth [`WORTH`] on `vessel`'s drive.
        fn put(vessel: &mut Vessel, bytes: f64) {
            let systems = &mut vessel.propulsion.systems;
            let (part, _) = systems.drives().next().unwrap();
            let drive = systems.on_mut(part).iter_mut().find_map(|s| s.drive_mut());
            let held = Held {
                amount: bytes,
                whole: bytes,
                sample: false,
                worth: WORTH,
            };
            assert_eq!(drive.unwrap().put("measure@Earth.space_low", held), 0.0);
        }

        /// What the probe's set of data is worth.
        const WORTH: f64 = 10.0;

        /// The ground stations as they are.
        const STATIONS: &str = include_str!("../../../assets/core/ground_stations/stations.json5");
        /// One dish on the equator.
        const ONE_ON_THE_EQUATOR: &str =
            r#"[{ id: "eq", body: "Earth", latitude: 0, longitude: 0, dish: 26, noise: 50 }]"#;

        fn left(universe: &Universe) -> f64 {
            universe.vessels[0].propulsion.systems.sendable()
        }

        fn returned(universe: &Universe) -> f64 {
            let career = universe.career.as_ref().unwrap();
            career.returned.values().sum()
        }

        /// On rails up to `seconds` from now, in jumps no longer than `longest`.
        fn fly(universe: &mut Universe, seconds: f64, longest: f64) {
            let end = universe.time().add_seconds(seconds);
            while universe.time() < end {
                let to = end.min(universe.time().add_seconds(longest));
                universe.advance_rails(to);
            }
        }

        #[test]
        fn the_rate_falls_with_distance_past_the_margin_to_spare() {
            let dsn = dish_gain(26.0, S_BAND);
            let radio = apollo();
            let near = margin(&radio, dsn, 50.0, 1e7);
            assert!(near > FULL_RATE_MARGIN);
            assert_eq!(rate(&radio, near), radio.data_rate);
            let edge = reach(&radio, dsn, 50.0);
            let at = |share: f64| rate(&radio, margin(&radio, dsn, 50.0, share * edge));
            // Full rate nearer than a third of the way to the edge (10 dB to spare), then as the
            // square of the distance, a tenth at the edge.
            assert_eq!(at(0.3), radio.data_rate);
            let ratio = at(0.9) / at(0.5);
            assert!((ratio - (0.5f64 / 0.9).powi(2)).abs() < 1e-9, "{ratio}");
            assert!((at(0.999_999) - 0.1 * radio.data_rate).abs() < 1.0);
            assert_eq!(at(1.01), 0.0);
        }

        /// High over the Earth, always in sight of a station: 50 kB at the link's rate takes 50 kB
        /// over the rate, and comes home a whole set.
        #[test]
        fn a_file_takes_its_size_over_the_rate() {
            let mut universe = probe(35_629e3, 1000.0, 50e3, STATIONS, true);
            let rate = universe.vessels[0].link().expect("in sight").rate;
            assert_eq!(rate, 1000.0);
            let takes = 50e3 * 8.0 / rate;
            fly(&mut universe, takes / 2.0, 60.0);
            assert!((left(&universe) - 25e3).abs() < 1.0, "{}", left(&universe));
            assert!((returned(&universe) - 0.5).abs() < 1e-4);
            fly(&mut universe, takes, 60.0);
            assert_eq!(left(&universe), 0.0);
            assert!((returned(&universe) - 1.0).abs() < 1e-9);
        }

        /// In a game that does not send at rates it all goes at once, as soon as there is a link.
        #[test]
        fn without_rates_it_all_goes_with_the_link() {
            let mut universe = probe(35_629e3, 1000.0, 50e3, STATIONS, false);
            universe.advance_rails(universe.time().add_seconds(1.0));
            assert_eq!(left(&universe), 0.0);
            assert!((returned(&universe) - 1.0).abs() < 1e-9);
            // With no station, nothing.
            let mut universe = probe(35_629e3, 1000.0, 50e3, "[]", false);
            fly(&mut universe, 1000.0, 100.0);
            assert_eq!(left(&universe), 50e3);
        }

        /// Low over the equator with one station on it, over two orbits: nothing goes while the
        /// station is out of sight, it goes on where it left off on the next pass, and nothing is
        /// lost. One long jump sends what jumps of ten seconds do.
        #[test]
        fn losing_the_link_pauses_a_file_and_nothing_is_lost_at_any_warp() {
            let mut sent = Vec::new();
            for longest in [1e9, 10.0] {
                let mut universe = probe(400e3, 1000.0, 1e6, ONE_ON_THE_EQUATOR, true);
                let period = universe.vessels[0].coasting().unwrap().period().unwrap();
                let (mut linked, mut dark) = (0, 0);
                let end = universe.time().add_seconds(2.0 * period);
                while universe.time() < end {
                    let before = left(&universe);
                    let to = end.min(universe.time().add_seconds(longest));
                    universe.advance_rails(to);
                    if longest < 1e9 {
                        // A jump that ends without a link and started without one sends nothing.
                        if universe.vessels[0].link().is_none() && before == left(&universe) {
                            dark += 1;
                        } else {
                            linked += 1;
                        }
                    }
                }
                let gone = 1e6 - left(&universe);
                assert!(
                    (returned(&universe) * 1e6 - gone).abs() < 1e-3,
                    "nothing lost"
                );
                if longest < 1e9 {
                    // Two passes of a few minutes each, in two orbits.
                    assert!(dark > 4 * linked, "{dark} dark, {linked} linked");
                    assert!(linked > 0);
                }
                sent.push(gone);
            }
            assert!(sent[0] > 10e3, "{sent:?}");
            assert!((sent[0] - sent[1]).abs() < 0.01 * sent[1], "{sent:?}");
        }

        /// A measurement sent home from orbit earns what it is worth, the same again half that, and
        /// the science unlocks a node whose engine the parts list then has.
        #[test]
        fn data_sent_home_earns_science_less_each_time_and_unlocks_the_tree() {
            let catalog = catalog();
            let tree = crate::career::tests::tree(&catalog);
            let mut universe = probe(35_629e3, 1000.0, 50e3, STATIONS, false);
            universe.career = Some(Career::new(&tree));
            let science = |u: &Universe| u.career.as_ref().unwrap().science;
            universe.advance_rails(universe.time().add_seconds(1.0));
            assert!((science(&universe) - WORTH).abs() < 1e-9);
            let said = universe.vessels[0].log.last().map(|e| e.message.clone());
            assert_eq!(said.as_deref(), Some("Data sent home: 10.0 science"));

            put(&mut universe.vessels[0], 50e3);
            universe.advance_rails(universe.time().add_seconds(1.0));
            assert!((science(&universe) - 1.5 * WORTH).abs() < 1e-9);

            let career = universe.career.as_mut().unwrap();
            assert!(!career.has_entry(&tree, "engine:A-10"));
            career.unlock("rocketry", &tree).unwrap();
            assert!(career.has_entry(&tree, "engine:A-10"));
            assert!((career.science - 1.5 * WORTH + 5.0).abs() < 1e-9);
        }

        /// A relay passes on no more than its own link carries: two probes through it share that,
        /// alike, and with the relay sending too it is a third each.
        #[test]
        fn two_probes_through_one_relay_share_its_rate() {
            let link = |rate: f64, relays: Vec<usize>| {
                Some(Link {
                    rate,
                    relays,
                    ..ground()
                })
            };
            let links = vec![
                link(800.0, vec![2]),
                link(800.0, vec![2]),
                link(1000.0, vec![]),
            ];
            assert_eq!(
                shared_rates(&links, &[true, true, false]),
                [500.0, 500.0, 0.0]
            );
            assert_eq!(
                shared_rates(&links, &[true, false, false]),
                [800.0, 0.0, 0.0]
            );
            let third = 1000.0 / 3.0;
            assert_eq!(
                shared_rates(&links, &[true, true, true]),
                [third, third, third]
            );
            // Through two relays the second one's share counts too.
            let chain = vec![
                link(800.0, vec![1, 2]),
                link(900.0, vec![2]),
                link(1000.0, vec![]),
            ];
            assert_eq!(
                shared_rates(&chain, &[true, true, false]),
                [450.0, 450.0, 0.0]
            );
        }
    }
}
