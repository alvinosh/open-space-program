//! What wants the pilot's attention right now: chips over the flight view (`alerts.list`, in
//! `messages.json5`), with a tone as each one appears (see `crate::sound`). Each is on for as long
//! as the state it warns of lasts. Both can be turned off on the menu's Interface page.

use bevy::prelude::*;
use osp_sim::{constants::G0, systems::Crew, vessel::Vessel};
use osp_ui::{UiData, record};

use crate::{FrameSet, settings::InterfaceSettings, sim::Sim};

/// Share of its strength a structure is warned about at.
const STRUCTURE: f64 = 0.85;
/// Share of the propellant feeding the lit engines that is warned about.
const LOW_PROPELLANT: f64 = 0.05;
/// What a crew is warned of for, in g: sustained, the Saturn V peaked at about 4.
const HIGH_G: f64 = 6.0;
/// Seconds from the ground it is warned about, falling faster than `FALLING` m/s.
const GROUND: f64 = 10.0;
const FALLING: f64 = 20.0;

pub struct AlertsPlugin;

impl Plugin for AlertsPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<ActiveAlerts>()
            .add_systems(Update, update.in_set(FrameSet::Render));
    }
}

/// What is being warned of now.
#[derive(Resource, Default, Debug, PartialEq)]
pub struct ActiveAlerts(pub Vec<&'static str>);

/// What the alerts are worked out from.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
struct Readings {
    /// Load on the structure, as a share of its strength.
    structure: f64,
    no_power: bool,
    no_control: bool,
    /// It has a radio, and nothing to talk to.
    no_signal: bool,
    /// What is left of the propellant feeding the liquid engines that are lit.
    propellant: Option<f64>,
    /// Acceleration felt by a crew aboard, g.
    crew_g: Option<f64>,
    /// How far above the ground it is, m, and how fast it is falling, m/s; not when landed.
    height: Option<f64>,
    sink: f64,
}

/// The alerts `readings` call for, most urgent first.
fn judge(readings: &Readings) -> Vec<&'static str> {
    let mut alerts = Vec::new();
    if let Some(height) = readings.height
        && readings.sink > FALLING
        && height < GROUND * readings.sink
    {
        alerts.push("Ground");
    }
    if readings.structure > STRUCTURE {
        alerts.push("Structure near its limit");
    }
    if readings.crew_g.is_some_and(|g| g > HIGH_G) {
        alerts.push("High g");
    }
    if readings.no_power {
        alerts.push("Out of power");
    }
    if readings.no_control {
        alerts.push("No control");
    }
    if readings
        .propellant
        .is_some_and(|left| left < LOW_PROPELLANT)
    {
        alerts.push("Low propellant");
    }
    if readings.no_signal {
        alerts.push("No signal");
    }
    alerts
}

fn readings(vessel: &Vessel, sim: &Sim) -> Readings {
    let system = &sim.universe.system;
    let time = sim.universe.time();
    let landed = vessel.is_landed();
    let propulsion = &vessel.propulsion;
    let lit: Vec<_> = (propulsion.engines.iter())
        .filter(|e| e.level > 0.0 && !e.is_solid())
        .map(|e| e.part)
        .collect();
    let crewed = propulsion.systems.all::<Crew>().next().is_some();
    let state = vessel.state_at(system, time);
    let body = system.body(vessel.parent());
    let fixed = body.orientation(time).inverse() * state.position.normalize();
    Readings {
        structure: vessel.flight.structure_load,
        no_power: propulsion.systems.brownout(),
        no_control: !vessel.controlled(system),
        no_signal: vessel.radio().is_some() && vessel.link().is_none(),
        propellant: propulsion.feed_fraction(&lit),
        crew_g: crewed.then(|| vessel.flight.proper_acceleration / G0),
        height: (!landed)
            .then(|| state.position.length() - body.radius - body.terrain_height(fixed)),
        sink: -state.radial_speed(),
    }
}

fn update(
    sim: Res<Sim>,
    settings: Res<InterfaceSettings>,
    mut active: ResMut<ActiveAlerts>,
    mut data: ResMut<UiData>,
) {
    let vessel = sim.active_vessel();
    let now = if settings.alerts && !vessel.is_destroyed() {
        judge(&readings(vessel, &sim))
    } else {
        Vec::new()
    };
    if active.0 != now {
        data.set("alerts.any", !now.is_empty());
        let list: Vec<_> = (now.iter())
            .map(|&text| record([("text", text.into())]))
            .collect();
        data.set("alerts.list", list);
        active.0 = now;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_wrong_is_no_alerts() {
        assert!(judge(&Readings::default()).is_empty());
        let flying = Readings {
            structure: 0.5,
            propellant: Some(0.5),
            crew_g: Some(3.9),
            height: Some(30_000.0),
            sink: -300.0,
            ..default()
        };
        assert!(judge(&flying).is_empty());
    }

    #[test]
    fn each_state_has_its_alert() {
        let says = |readings: Readings| judge(&readings);
        let some = Readings::default();
        assert_eq!(
            says(Readings {
                structure: 0.9,
                ..some
            }),
            ["Structure near its limit"]
        );
        assert_eq!(
            says(Readings {
                no_power: true,
                ..some
            }),
            ["Out of power"]
        );
        assert_eq!(
            says(Readings {
                no_control: true,
                ..some
            }),
            ["No control"]
        );
        assert_eq!(
            says(Readings {
                no_signal: true,
                ..some
            }),
            ["No signal"]
        );
        assert_eq!(
            says(Readings {
                propellant: Some(0.02),
                ..some
            }),
            ["Low propellant"]
        );
        assert_eq!(
            says(Readings {
                crew_g: Some(7.0),
                ..some
            }),
            ["High g"]
        );
    }

    #[test]
    fn the_ground_is_only_called_when_it_is_coming_up_fast() {
        let falling = |height: f64, sink: f64| Readings {
            height: Some(height),
            sink,
            ..default()
        };
        assert_eq!(judge(&falling(1_000.0, 200.0)), ["Ground"]);
        // High up, slow under a parachute, climbing, or standing on it.
        assert!(judge(&falling(50_000.0, 200.0)).is_empty());
        assert!(judge(&falling(50.0, 8.0)).is_empty());
        assert!(judge(&falling(50.0, -40.0)).is_empty());
        assert!(
            judge(&Readings {
                height: None,
                sink: 100.0,
                ..default()
            })
            .is_empty()
        );
    }

    #[test]
    fn the_most_urgent_comes_first() {
        let readings = Readings {
            no_signal: true,
            structure: 0.95,
            height: Some(100.0),
            sink: 100.0,
            ..default()
        };
        assert_eq!(
            judge(&readings),
            ["Ground", "Structure near its limit", "No signal"]
        );
    }
}
