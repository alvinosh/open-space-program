//! Prints what craft are made of, as the game loads them: per stage the wet and dry mass,
//! thrust, thrust-to-weight at ignition, vacuum delta-v and burn time, and the parts that
//! are newer than a given year.
//!
//! ```sh
//! cargo run -p osp_sim --example craft_report --release -- [--year 1961] [craft ids...]
//! ```
//!
//! Without ids it reports every craft of every mod. With a scenario id instead of a craft
//! id it reports that scenario's craft at the scenario's date.

use std::path::Path;

use osp_mods::Registry;
use osp_sim::{
    data::{
        BatteryDef, CraftDef, EngineDef, LiveryDef, MaterialDef, PartCatalog, PodDef, RcsDef,
        ReactionWheelDef, ResourceDef, ScenarioDef, TankTypeDef, TechNodeDef,
    },
    propulsion::Propulsion,
    tech::{TechTree, calendar_year},
    time::SimTime,
};

const G0: f64 = 9.80665;

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
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
    let tech = TechTree::new(registry.all::<TechNodeDef>("tech_nodes", &mut diagnostics));
    for d in &diagnostics {
        eprintln!("{d}");
    }

    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let mut year = None;
    if let Some(i) = args.iter().position(|a| a == "--year") {
        year = args.get(i + 1).and_then(|y| y.parse::<i32>().ok());
        args.drain(i..(i + 2).min(args.len()));
    }
    let mut jobs: Vec<(String, Option<i32>)> = Vec::new();
    if args.is_empty() {
        jobs.extend(registry_ids(registry, "craft").into_iter().map(|id| (id, year)));
    }
    for arg in args {
        if let Ok(s) = registry.get::<ScenarioDef>("scenarios", &arg) {
            let at = calendar_year(SimTime::from_seconds(s.start_time));
            println!("scenario {} ({}): {} in {at}", s.id, s.name, s.craft);
            jobs.push((s.craft, year.or(Some(at))));
        } else {
            jobs.push((arg, year));
        }
    }
    let mut failed = 0;
    for (id, year) in jobs {
        if !report(registry, &catalog, &tech, &id, year) {
            failed += 1;
        }
    }
    if failed > 0 {
        eprintln!("{failed} craft failed to load");
        std::process::exit(1);
    }
}

fn registry_ids(registry: &Registry, collection: &str) -> Vec<String> {
    registry.records(collection).map(|r| r.id.clone()).collect()
}

fn report(
    registry: &Registry,
    catalog: &PartCatalog,
    tech: &TechTree,
    id: &str,
    year: Option<i32>,
) -> bool {
    let craft: CraftDef = match registry.get("craft", id) {
        Ok(c) => c,
        Err(e) => {
            println!("{id}: {e}");
            return false;
        }
    };
    let propulsion = match Propulsion::from_craft(&craft, catalog) {
        Ok(p) => p,
        Err(e) => {
            println!("{id}: {e}");
            return false;
        }
    };
    let total = propulsion.mass();
    let name = if craft.name.is_empty() { &craft.id } else { &craft.name };
    println!("{name} [{id}]: {:.1} t", total / 1000.0);
    let info = propulsion.stage_info();
    for (stage, info) in propulsion.sections.iter().zip(&info) {
        let thrust_vac: f64 = propulsion.thrust_vac(stage.id);
        let thrust_sl: f64 = propulsion
            .engines_of(stage.id)
            .map(|e| e.def.thrust_vac * e.def.isp_sl / e.def.isp_vac.max(1.0))
            .sum();
        let twr = |thrust: f64| thrust / (info.start_mass * G0);
        let engines = summarize(propulsion.engines_of(stage.id).map(|e| e.def.id.as_str()));
        println!(
            "  {:<22} wet {:>8.1} t  dry {:>7.2} t  prop {:>8.1} t  TWR sl {:>4.2} vac {:>4.2}  \
             dv {:>5.0} m/s  burn {:>4.0} s  {}",
            stage.name,
            propulsion.section_mass(stage.id) / 1000.0,
            stage.dry_mass() / 1000.0,
            propulsion.propellant_mass(stage.id) / 1000.0,
            twr(thrust_sl),
            twr(thrust_vac),
            info.delta_v_vac,
            info.burn_time,
            engines
        );
    }
    let dv: f64 = info.iter().map(|i| i.delta_v_vac).sum();
    println!("  total vacuum delta-v {dv:.0} m/s");
    if let Some(year) = year {
        for a in tech.anachronisms(catalog, &craft, year) {
            println!("  ! {} `{}` is from {}, after {year}", a.kind, a.part, a.year);
        }
    }
    true
}

/// "3× RD-107, RD-108".
fn summarize<'a>(ids: impl Iterator<Item = &'a str>) -> String {
    let mut counts: Vec<(&str, usize)> = Vec::new();
    for id in ids {
        match counts.iter_mut().find(|(i, _)| *i == id) {
            Some((_, n)) => *n += 1,
            None => counts.push((id, 1)),
        }
    }
    counts
        .iter()
        .map(|(id, n)| if *n > 1 { format!("{n}x {id}") } else { id.to_string() })
        .collect::<Vec<_>>()
        .join(", ")
}
