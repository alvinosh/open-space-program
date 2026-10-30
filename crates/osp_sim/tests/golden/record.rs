//! How a craft is measured for the golden figures: shared by `examples/craft_golden.rs`,
//! which records them, and `tests/craft_golden.rs`, which holds the game to them.

use osp_mods::Registry;
use osp_sim::{
    aero::Flow,
    atmosphere::{Atmosphere, AtmosphereModel},
    attitude::mass_properties,
    data::{
        BatteryDef, EngineDef, LiveryDef, MaterialDef, PartCatalog, PodDef, RcsDef,
        ReactionWheelDef, ResourceDef, TankTypeDef,
    },
    propulsion::Propulsion,
};
use serde_json::{Value, json};

/// Altitude (m), Mach number, angle off the flow (degrees) and rotation command.
const FLOWS: [(f64, f64, f64, [f64; 3]); 3] = [
    (0.0, 0.3, 4.0, [0.5, 0.3, 0.2]),
    (10_000.0, 0.9, 2.0, [0.0, 0.0, 0.0]),
    (25_000.0, 2.5, 3.0, [-0.4, 0.0, 0.6]),
];

/// The part records of every mod, as the game loads them.
pub fn catalog(registry: &Registry) -> PartCatalog {
    let mut diagnostics = Vec::new();
    let catalog = PartCatalog {
        engines: registry.all::<EngineDef>("engines", &mut diagnostics),
        resources: registry.all::<ResourceDef>("resources", &mut diagnostics),
        materials: registry.all::<MaterialDef>("materials", &mut diagnostics),
        tank_types: registry.all::<TankTypeDef>("tank_types", &mut diagnostics),
        reaction_wheels: registry.all::<ReactionWheelDef>("reaction_wheels", &mut diagnostics),
        rcs: registry.all::<RcsDef>("rcs", &mut diagnostics),
        pods: registry.all::<PodDef>("pods", &mut diagnostics),
        batteries: registry.all::<BatteryDef>("batteries", &mut diagnostics),
        liveries: registry.all::<LiveryDef>("liveries", &mut diagnostics),
    };
    for d in &diagnostics {
        eprintln!("{d}");
    }
    catalog
}

/// The craft at liftoff and after each sequence. The air's loads are taken about the
/// centres of mass in `about`, phase by phase, where given, and about the craft's own
/// otherwise.
pub fn record(mut propulsion: Propulsion, about: &[f64]) -> Value {
    let mut phases = vec![phase("liftoff", &propulsion, about.first().copied())];
    while let Some(staged) = propulsion.activate_next_sequence() {
        let k = phases.len();
        phases.push(phase(
            &staged.messages.join("; "),
            &propulsion,
            about.get(k).copied(),
        ));
    }
    Value::Array(phases)
}

fn phase(after: &str, propulsion: &Propulsion, about: Option<f64>) -> Value {
    let stages: Vec<Value> = propulsion
        .stage_info()
        .iter()
        .zip(&propulsion.sections)
        .map(|(info, stage)| {
            json!({
                "name": stage.name,
                "dry": stage.dry_mass(),
                "wet": propulsion.section_mass(stage.id),
                "delta_v": info.delta_v_vac,
                "burn_time": info.burn_time,
            })
        })
        .collect();
    let mass = mass_properties(propulsion);
    let tensor = mass.inertia.tensor.to_cols_array();
    let shape = propulsion.shape();
    let about = about.map_or(mass.center_of_mass, |z| mass.center_of_mass.with_z(z));
    let atmosphere = Atmosphere {
        model: AtmosphereModel::Us1976,
        height: 140_000.0,
    };
    let loads: Vec<Value> = FLOWS
        .iter()
        .map(|&(altitude, mach, angle, controls)| {
            let air = atmosphere.sample(altitude);
            let angle = angle.to_radians();
            let speed = mach * air.speed_of_sound;
            let flow = Flow {
                velocity: glam::DVec3::new(0.0, angle.sin(), angle.cos()) * speed,
                angular_velocity: glam::DVec3::ZERO,
                air,
                plume: false,
                controls: controls.into(),
            };
            let loads = shape.loads_about(&flow, about);
            json!({ "force": loads.force.to_array(), "torque": loads.torque.to_array() })
        })
        .collect();
    json!({
        "after": after,
        "mass": propulsion.mass(),
        "stages": stages,
        "center_of_mass": mass.center_of_mass,
        "inertia": tensor,
        "outline": shape.outline(),
        "loads": loads,
    })
}
