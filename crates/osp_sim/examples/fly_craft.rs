//! Flies a craft from a file off the pad as a player's first go does: SAS on, full throttle, next
//! sequence, then hands off. Prints the flight each second: height, speed, Mach, pitch, angle of
//! attack, how fast it turns, what the controls ask, and thrust to weight. Made to look into what a
//! playtest saw (`target/playtest/<name>/craft.json`).
//!
//! ```sh
//! cargo run -p osp_sim --example fly_craft --release -- target/playtest/first_rocket/craft.json [seconds] [--realistic]
//! ```

use std::path::Path;

use glam::DVec3;
use osp_sim::{
    aero::Flow,
    attitude,
    data::{BodyDef, CraftDef, GroundStationDef, LaunchSiteDef, ScenarioDef, SystemDef},
    universe::Universe,
};

#[path = "../tests/golden/record.rs"]
#[allow(dead_code)]
mod record;

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let file = args.first().expect("a craft file");
    let seconds: f64 = args.get(1).and_then(|s| s.parse().ok()).unwrap_or(30.0);
    let realistic = args.iter().any(|a| a == "--realistic");
    let sas = !args.iter().any(|a| a == "--no-sas");
    let throttle: f64 = (args.iter())
        .find_map(|a| a.strip_prefix("--throttle=")?.parse().ok())
        .unwrap_or(1.0);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let mods = root.join("mods");
    let content = osp_mods::load(
        &root.join("assets/core"),
        realistic.then_some(mods.as_path()),
    );
    let registry = &content.registry;
    let catalog = record::catalog(registry);
    let craft: CraftDef =
        serde_json::from_str(&std::fs::read_to_string(file).expect("the file")).expect("a craft");
    let system = SystemDef {
        bodies: registry.all::<BodyDef>("bodies", &mut Vec::new()),
    };
    let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut Vec::new());
    let scenario: ScenarioDef = registry.get("scenarios", "sounder").unwrap();
    if args.iter().any(|a| a == "--stability") {
        stability(&craft, &catalog);
        return;
    }
    let craft_of = |_: &str| -> Result<CraftDef, String> { Ok(craft.clone()) };
    let mut universe =
        Universe::new_game(&system, Vec::new(), &sites, &catalog, craft_of, &scenario).unwrap();
    // As the game does: a probe core is flown by radio, through the ground stations.
    universe.set_stations(&registry.all::<GroundStationDef>("ground_stations", &mut Vec::new()));
    universe.step_physics();
    let id = universe.focus.unwrap();
    let index = universe.find_vessel(id).unwrap();
    let time = universe.time();
    let vessel = &mut universe.vessels[index];
    vessel.set_sas(&universe.system, time, sas);
    vessel.set_throttle(throttle);
    universe.activate_next_stage(index);
    let start = universe.time();

    println!(
        "{:>5} {:>8} {:>6} {:>5} {:>7} {:>7} {:>8} {:>18} {:>5} {:>8} {:>7} {:>7}",
        "t s",
        "height m",
        "v m/s",
        "Mach",
        "pitch°",
        "AoA°",
        "turn°/s",
        "control p/y/r",
        "TWR",
        "q kPa",
        "CG m",
        "margin"
    );
    let ticks = (seconds * 50.0) as usize;
    for tick in 0..=ticks {
        if tick > 0 {
            universe.step_physics();
        }
        if tick % 50 != 0 {
            continue;
        }
        let Some(index) = universe.find_vessel(id) else {
            println!("the vessel is gone");
            break;
        };
        let time = universe.time();
        let vessel = &universe.vessels[index];
        let state = vessel.state_at(&universe.system, time);
        let body = universe.system.body(vessel.parent());
        let up = state.position.normalize();
        let attitude = vessel.attitude_state(&universe.system, time);
        let pitch = attitude.nose().dot(up).clamp(-1.0, 1.0).asin().to_degrees();
        let turn = attitude.angular_velocity.length().to_degrees();
        let flight = &vessel.flight;
        let weight = flight.mass * body.gm / state.position.length_squared();
        let c = flight.control;
        // Static margin: how far the centre of pressure sits behind the centre of mass, at 2°
        // off the airflow at this height and speed, m (negative: ahead, unstable).
        let height = state.position.length() - body.radius;
        let air = (body.atmosphere.as_ref()).map(|a| a.sample(height));
        let mass = attitude::mass_properties(&vessel.propulsion);
        let speed = (flight.mach * air.map_or(0.0, |a| a.speed_of_sound)).max(30.0);
        let margin = air.filter(|a| a.density > 0.0).map(|air| {
            let alpha = 2f64.to_radians();
            let flow = Flow {
                velocity: DVec3::new(alpha.sin(), 0.0, alpha.cos()) * speed,
                angular_velocity: DVec3::ZERO,
                air,
                plume: flight.thrust > 0.0,
                controls: DVec3::ZERO,
            };
            let loads = vessel
                .propulsion
                .shape()
                .loads_about(&flow, mass.center_of_mass);
            loads.torque.y / -loads.force.x
        });
        println!(
            "{:>5.0} {:>8.0} {:>6.0} {:>5.2} {:>7.1} {:>7.1} {:>8.1} {:>6.2}/{:>5.2}/{:>5.2} {:>5.2} {:>8.1} {:>7.2} {:>7.2}",
            time.seconds_since(start),
            state.position.length() - body.radius,
            state.velocity.length(),
            flight.mach,
            pitch,
            flight.angle_of_attack.to_degrees(),
            turn,
            c.x,
            c.y,
            c.z,
            flight.thrust / weight.max(1.0),
            flight.dynamic_pressure / 1000.0,
            mass.center_of_mass.z,
            margin.unwrap_or(f64::NAN),
        );
    }
}

/// How the air pushes the craft as it stands on the pad, 2° off the airflow at 5 km, by Mach: the
/// whole craft, its body alone and its lifting surfaces alone (the difference), each as a normal
/// force coefficient per radian on the body's frontal area and where it acts above the base.
fn stability(craft: &CraftDef, catalog: &osp_sim::data::PartCatalog) {
    use osp_sim::{atmosphere::AtmosphereSample, propulsion::Propulsion};
    let whole = Propulsion::from_craft(craft, catalog).expect("the craft builds");
    let mut bare = craft.clone();
    bare.parts.retain(|p| {
        p.shape
            .as_ref()
            .is_none_or(|s| !format!("{s:?}").starts_with("Wing"))
    });
    let body = Propulsion::from_craft(&bare, catalog).expect("the body builds");
    let mass = attitude::mass_properties(&whole);
    let cg = mass.center_of_mass;
    let air = AtmosphereSample {
        pressure: 54_000.0,
        density: 0.736,
        temperature: 255.7,
        speed_of_sound: 320.5,
    };
    let area = whole.shape().reference_area();
    let alpha = 2f64.to_radians();
    println!(
        "centre of mass {:.2} m up, {} parts, {} without wings",
        cg.z,
        craft.parts.len(),
        bare.parts.len()
    );
    println!(
        "outline (height m, radius m): {:?}",
        whole
            .shape()
            .outline()
            .iter()
            .map(|(z, r)| format!("{z:.2}:{r:.2}"))
            .collect::<Vec<_>>()
    );
    println!(
        "{:>5} {:>9} {:>8} {:>9} {:>8} {:>9} {:>8} {:>8}",
        "Mach", "CNa all", "CP all", "CNa body", "CP body", "CNa fins", "CP fins", "margin"
    );
    for mach in [0.3, 0.6, 0.8, 0.95, 1.1, 1.3, 1.5, 1.7, 2.0, 2.5, 3.0] {
        let speed = mach * air.speed_of_sound;
        let q = 0.5 * air.density * speed * speed;
        let flow = Flow {
            velocity: DVec3::new(alpha.sin(), 0.0, alpha.cos()) * speed,
            angular_velocity: DVec3::ZERO,
            air,
            plume: true,
            controls: DVec3::ZERO,
        };
        let all = whole.shape().loads_about(&flow, cg);
        let hull = body.shape().loads_about(&flow, cg);
        let (n_all, m_all) = (-all.force.x, all.torque.y);
        let (n_body, m_body) = (-hull.force.x, hull.torque.y);
        let (n_fins, m_fins) = (n_all - n_body, m_all - m_body);
        let coefficient = |n: f64| n / (q * area * alpha);
        // Torque y = (CG - CP) * N for a force pushing back across the flow.
        let cp = |n: f64, m: f64| cg.z - m / n;
        println!(
            "{mach:>5.2} {:>9.2} {:>8.2} {:>9.2} {:>8.2} {:>9.2} {:>8.2} {:>8.2}",
            coefficient(n_all),
            cp(n_all, m_all),
            coefficient(n_body),
            cp(n_body, m_body),
            coefficient(n_fins),
            cp(n_fins, m_fins),
            cg.z - cp(n_all, m_all),
        );
    }
}
