//! What every cell checks, never just "it loaded": the sim refuses with a rule or builds
//! the craft; every part and component is kept; the mass, centre of mass and inertia are
//! the sum of the parts'; the air sees what is outside the skin and nothing inside it;
//! saving and loading give the same craft; and the craft stands on the pad for five seconds.

use osp_sim::{
    DVec3,
    aero::Flow,
    atmosphere::{Atmosphere, AtmosphereModel},
    attitude::mass_properties,
    component::Component,
    craft::Craft,
    data::CraftDef,
    mass::Mass,
    propulsion::Propulsion,
    structure::{self, Conditions},
    universe::{PHYSICS_STEP, Universe},
};
use serde::Serialize;

use crate::{
    cell::{self, Aero, Built, Column, Row},
    content,
};

/// The text the gatherer refuses with when the physics cannot hold a craft yet. A refusal
/// that says this is a limit of the sim, not a rule of building.
pub const LIMIT: &str = "until the physics runs on parts";

/// How close sums must agree, relative to their size.
const EXACT: f64 = 1e-9;
/// Standard gravity, m/s².
const G: f64 = osp_sim::constants::G0;
/// How much an outline's radius may change, m, before the air has seen something: stations
/// at other heights give the same shape to within this.
const OUTLINE: f64 = 1e-3;
/// How much the loads in a flow may change, as a share of their size, before the air has seen
/// something: a station more or less changes the sum a little.
const LOADS: f64 = 1e-3;
/// Seconds on the pad.
const PAD_SECONDS: f64 = 5.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Check {
    /// It builds, or is refused with the rule that stops it.
    Loads,
    /// Every part and component is in the physics.
    Kept,
    /// Mass is the sum of the parts' (and their propellant).
    Mass,
    /// Each part is where the assembly puts it, and the centre of mass and inertia are the
    /// parts' sum.
    Balance,
    /// Outside the skin the air sees it; inside a container or fairing it does not.
    Aero,
    /// Saving and loading give the same craft.
    Save,
    /// Five seconds on the pad give no NaN, and the weight is in the pad's load.
    Pad,
}

impl Check {
    pub const ALL: [Check; 7] = [
        Check::Loads,
        Check::Kept,
        Check::Mass,
        Check::Balance,
        Check::Aero,
        Check::Save,
        Check::Pad,
    ];

    pub fn title(self) -> &'static str {
        match self {
            Check::Loads => "Loads or refuses with a rule",
            Check::Kept => "Every part and component is kept",
            Check::Mass => "Mass is the sum of the parts",
            Check::Balance => "Centre of mass and inertia are the parts' sum",
            Check::Aero => "The air sees the outside, not the inside",
            Check::Save => "Save and load give the same craft",
            Check::Pad => "Five seconds on the pad",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize)]
#[serde(tag = "result", content = "detail", rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail(String),
    /// Nothing to check here, and why.
    Skip(String),
}

/// How a cell was refused.
#[derive(Debug, Clone, Serialize)]
pub struct Refusal {
    /// A limit of the sim ("until the physics runs on parts"), not a rule of building.
    pub limit: bool,
    pub reason: String,
}

#[derive(Debug, Serialize)]
pub struct CellResult {
    pub refusal: Option<Refusal>,
    pub checks: Vec<(Check, Outcome)>,
}

impl CellResult {
    pub fn failed(&self) -> Vec<Check> {
        self.checks
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Fail(_)))
            .map(|(c, _)| *c)
            .collect()
    }

    /// Refused by a rule: nothing to run, nothing wrong.
    pub fn grey(&self) -> bool {
        self.refusal.as_ref().is_some_and(|r| !r.limit)
    }
}

/// Where cells are flown: one universe, started afresh for each.
pub struct Pad {
    universe: Option<Universe>,
}

impl Pad {
    pub fn new() -> Self {
        Self { universe: None }
    }
}

/// Builds the cell's craft and runs every check on it.
pub fn run(row: &Row, column: Column, pad: &mut Pad) -> CellResult {
    let refused = |limit: bool, reason: String| CellResult {
        refusal: Some(Refusal {
            limit,
            reason: reason.clone(),
        }),
        checks: Check::ALL
            .iter()
            .map(|&c| {
                let outcome = match c {
                    Check::Loads if limit => Outcome::Fail(reason.clone()),
                    Check::Loads => Outcome::Pass,
                    _ => Outcome::Skip("it did not load".into()),
                };
                (c, outcome)
            })
            .collect(),
    };
    let built = match cell::build(row, column) {
        Ok(built) => built,
        Err(reason) => return refused(reason.contains(LIMIT), reason),
    };
    let catalog = &content::get().catalog;
    let craft = match Craft::build(&built.def, catalog) {
        Ok(craft) => craft,
        Err(e) => return refused(e.to_string().contains(LIMIT), e.to_string()),
    };
    let propulsion = match Propulsion::from_craft(&built.def, catalog) {
        Ok(p) => p,
        Err(e) => return refused(e.to_string().contains(LIMIT), e.to_string()),
    };
    let mut checks = vec![(Check::Loads, Outcome::Pass)];
    checks.push((Check::Kept, kept(&craft, &propulsion)));
    checks.push((Check::Mass, mass(&craft, &propulsion)));
    checks.push((Check::Balance, balance(&craft, &propulsion)));
    checks.push((Check::Aero, aero(row, column, &built, &propulsion)));
    checks.push((Check::Save, save(&built.def, &craft)));
    checks.push((Check::Pad, on_the_pad(&built, &craft, &propulsion, pad)));
    CellResult {
        refusal: None,
        checks,
    }
}

fn outcome(problems: Vec<String>) -> Outcome {
    if problems.is_empty() {
        Outcome::Pass
    } else {
        Outcome::Fail(problems.join("; "))
    }
}

fn near(a: f64, b: f64) -> bool {
    (a - b).abs() <= EXACT * (1.0 + a.abs().max(b.abs()))
}

/// Every part is in the physics, with what it does.
pub fn kept(craft: &Craft, p: &Propulsion) -> Outcome {
    let mut problems = Vec::new();
    if p.parts().parts().len() != craft.parts.len() {
        problems.push(format!(
            "{} parts in, {} in the physics",
            craft.parts.len(),
            p.parts().parts().len()
        ));
    }
    let count = |f: fn(&Component) -> bool| {
        craft
            .parts
            .iter()
            .flat_map(|part| &part.built.components)
            .filter(|c| f(c))
            .count()
    };
    let mut want = |what: &str, wanted: usize, got: usize| {
        if got < wanted {
            problems.push(format!(
                "{wanted} {what} in the craft, {got} in the physics"
            ));
        }
    };
    want(
        "storages",
        count(|c| matches!(c, Component::Storage(_))),
        p.tanks.len(),
    );
    want(
        "engines",
        count(|c| matches!(c, Component::Engine(_))),
        p.engines.len(),
    );
    want(
        "RCS blocks",
        count(|c| matches!(c, Component::Rcs(_))),
        p.rcs.len(),
    );
    let wheels = count(|c| matches!(c, Component::ReactionWheel(_)));
    let torque: f64 = p.sections.iter().map(|s| s.wheel_torque).sum();
    want(
        "reaction wheels",
        wheels,
        usize::from(torque > 0.0) * wheels,
    );
    let wings = count(|c| matches!(c, Component::LiftingSurface(_)));
    want(
        "lifting surfaces",
        wings,
        p.sections.iter().map(|s| s.wings.len()).sum(),
    );
    let legs = count(|c| matches!(c, Component::Gear(_)));
    want(
        "gear legs",
        legs,
        p.sections.iter().map(|s| s.gear.len()).sum(),
    );
    let seats: u32 = craft
        .parts
        .iter()
        .flat_map(|part| &part.built.components)
        .filter_map(|c| match c {
            Component::Crew { seats } => Some(*seats),
            _ => None,
        })
        .sum();
    let crew: u32 = p
        .sections
        .iter()
        .filter_map(|s| s.pod.as_ref())
        .map(|pod| pod.crew)
        .sum();
    if seats > crew {
        problems.push(format!(
            "{seats} crew seats in the craft, {crew} in the physics"
        ));
    }
    let charge: f64 = craft
        .parts
        .iter()
        .flat_map(|part| &part.built.components)
        .filter_map(|c| match c {
            Component::Battery(b) => Some(b.charge),
            _ => None,
        })
        .sum();
    let stored: f64 = p
        .sections
        .iter()
        .filter_map(|s| s.pod.as_ref())
        .map(|pod| pod.electric_charge)
        .sum();
    if charge > stored + 1e-9 {
        problems.push(format!(
            "{charge:.0} kJ of battery in the craft, {stored:.0} kJ in the physics"
        ));
    }
    outcome(problems)
}

/// What a part weighs with its propellant, kg: independently of the physics.
fn wet(part: &osp_sim::craft::CraftPart) -> f64 {
    let propellant: f64 = part
        .built
        .storages()
        .flat_map(|s| &s.contents)
        .map(|c| c.capacity)
        .sum();
    part.built.mass.mass + propellant
}

pub fn mass(craft: &Craft, p: &Propulsion) -> Outcome {
    let want: f64 = craft.parts.iter().map(wet).sum();
    let got = p.mass();
    let ok = near(want, got) && near(mass_properties(p).mass, got);
    outcome(if ok {
        Vec::new()
    } else {
        vec![format!(
            "the parts weigh {want:.3} kg, the physics {got:.3} kg"
        )]
    })
}

pub fn balance(craft: &Craft, p: &Propulsion) -> Outcome {
    let poses = craft.assembly.poses();
    let tree = p.parts();
    let mut problems = Vec::new();
    let shift = tree.parts()[0].position - poses[0].position;
    for (i, part) in tree.parts().iter().enumerate() {
        let off = (part.position - poses[i].position - shift).length();
        let turn = part.rotation.dot(poses[i].rotation).abs();
        if off > 1e-9 || turn < 1.0 - 1e-12 {
            problems.push(format!(
                "`{}` sits {off:.3} m and {:.1}° from where the assembly puts it",
                craft.parts[i].id,
                2.0 * turn.min(1.0).acos().to_degrees()
            ));
        }
    }
    let sum: Mass = craft
        .parts
        .iter()
        .zip(&poses)
        .map(|(part, pose)| part.built.mass.placed(pose.position, pose.rotation))
        .sum();
    let got = tree.dry_mass();
    if !near(sum.mass, got.mass) {
        problems.push(format!(
            "dry mass {:.3} kg against {:.3} kg",
            sum.mass, got.mass
        ));
    } else {
        let centre = (got.center - sum.center - shift).length();
        let scale = 1.0 + sum.center.length();
        if centre > EXACT * scale {
            problems.push(format!("centre of mass {centre:.2e} m off"));
        }
        let a = sum.inertia_about(sum.center);
        let b = got.inertia_about(got.center);
        let diff = (a - b)
            .to_cols_array()
            .iter()
            .fold(0.0_f64, |m, x| m.max(x.abs()));
        let size = a
            .to_cols_array()
            .iter()
            .fold(1.0_f64, |m, x| m.max(x.abs()));
        if diff > 1e-6 * size {
            problems.push(format!(
                "inertia differs by {:.1e} of its size",
                diff / size
            ));
        }
    }
    outcome(problems)
}

/// What the air feels of a craft: its outline, and its loads in two flows.
#[derive(Debug)]
struct Signature {
    outline: Vec<(f64, f64)>,
    loads: Vec<[f64; 6]>,
}

fn signature(p: &Propulsion) -> Signature {
    let shape = p.shape();
    let atmosphere = Atmosphere {
        model: AtmosphereModel::Us1976,
        height: 140_000.0,
    };
    let loads = [(0.0, 0.3, 4.0_f64), (10_000.0, 0.9, 2.0)]
        .iter()
        .map(|&(altitude, mach, angle)| {
            let air = atmosphere.sample(altitude);
            let angle = angle.to_radians();
            let flow = Flow {
                velocity: DVec3::new(0.0, angle.sin(), angle.cos()) * mach * air.speed_of_sound,
                angular_velocity: DVec3::ZERO,
                air,
                plume: false,
                controls: DVec3::ZERO,
            };
            let l = shape.loads_about(&flow, DVec3::new(0.0, 0.0, 1.5));
            let (f, t) = (l.force, l.torque);
            [f.x, f.y, f.z, t.x, t.y, t.z]
        })
        .collect();
    Signature {
        outline: shape.outline(),
        loads,
    }
}

impl Signature {
    /// Where the two differ: in the outline's size, in the loads, or not at all. Extra
    /// stations that leave the outline as it was do not count.
    fn differs(&self, other: &Signature) -> Option<String> {
        let outline = outlines_differ(&self.outline, &other.outline);
        if outline > OUTLINE {
            return Some(format!("its outline moves by {:.1} mm", outline * 1e3));
        }
        let loads = (self
            .loads
            .iter()
            .flatten()
            .zip(other.loads.iter().flatten()))
        .map(|(a, b)| (a - b).abs() / (1.0 + a.abs().max(b.abs())))
        .fold(0.0_f64, f64::max);
        (loads > LOADS).then(|| format!("its loads change by {loads:.1e} of their size"))
    }
}

/// How much two outlines (height, radius) differ in radius, halfway between the heights
/// either has a station at.
fn outlines_differ(a: &[(f64, f64)], b: &[(f64, f64)]) -> f64 {
    let radius = |outline: &[(f64, f64)], z: f64| -> f64 {
        outline
            .windows(2)
            .find(|w| w[0].0 < z && z <= w[1].0)
            .map_or(0.0, |w| {
                let t = (z - w[0].0) / (w[1].0 - w[0].0);
                w[0].1 + t * (w[1].1 - w[0].1)
            })
    };
    let mut heights: Vec<f64> = a.iter().chain(b).map(|p| p.0).collect();
    heights.sort_by(f64::total_cmp);
    heights.dedup_by(|x, y| (*x - *y).abs() < 1e-9);
    heights
        .windows(2)
        .map(|w| (w[0] + w[1]) / 2.0)
        .map(|z| (radius(a, z) - radius(b, z)).abs())
        .fold(0.0, f64::max)
}

pub fn aero(row: &Row, column: Column, built: &Built, with: &Propulsion) -> Outcome {
    let catalog = &content::get().catalog;
    let without = match Propulsion::from_craft(&built.baseline, catalog) {
        Ok(p) => p,
        Err(e) => return Outcome::Skip(format!("the craft without it does not load: {e}")),
    };
    let change = signature(with).differs(&signature(&without));
    if column.exposed() {
        match (row.aero, change) {
            (Aero::Apart, _) => Outcome::Skip("an engine is modelled apart from the body".into()),
            (_, Some(_)) => Outcome::Pass,
            (_, None) => Outcome::Fail("it is outside the skin and the air does not see it".into()),
        }
    } else {
        match change {
            None => Outcome::Pass,
            Some(what) => Outcome::Fail(format!("it is inside, and {what}")),
        }
    }
}

pub fn save(def: &CraftDef, craft: &Craft) -> Outcome {
    let catalog = &content::get().catalog;
    let text = serde_json::to_string(def).expect("a craft is written");
    let loaded: CraftDef = match json5::from_str(&text) {
        Ok(loaded) => loaded,
        Err(e) => return Outcome::Fail(format!("the saved craft does not load: {e}")),
    };
    let (a, b) = (
        serde_json::to_value(def).unwrap(),
        serde_json::to_value(&loaded).unwrap(),
    );
    if a != b {
        return Outcome::Fail("the loaded craft is not the saved one".into());
    }
    let Ok(again) = Craft::build(&loaded, catalog) else {
        return Outcome::Fail("the loaded craft does not build".into());
    };
    let (p, q) = (craft.assembly.poses(), again.assembly.poses());
    let same = p.len() == q.len()
        && p.iter().zip(&q).all(|(a, b)| {
            (a.position - b.position).length() < 1e-12
                && a.rotation.dot(b.rotation).abs() > 1.0 - 1e-12
        });
    if same {
        Outcome::Pass
    } else {
        Outcome::Fail("the loaded craft sits differently".into())
    }
}

/// Five seconds on the pad at the Cape, and the load the pad takes.
pub fn on_the_pad(built: &Built, craft: &Craft, p: &Propulsion, pad: &mut Pad) -> Outcome {
    let content = content::get();
    let def = built.def.clone();
    let craft_of = |_: &str| Ok::<_, String>(def.clone());
    let mut scenario = content.scenario.clone();
    scenario.craft = def.id.clone();
    let started = match &mut pad.universe {
        Some(universe) => universe.restart(&content.sites, &content.catalog, craft_of, &scenario),
        None => Universe::new_game(
            &content.system,
            Vec::new(),
            &content.sites,
            &content.catalog,
            craft_of,
            &scenario,
        )
        .map(|universe| pad.universe = Some(universe)),
    };
    if let Err(e) = started {
        return Outcome::Fail(format!("it does not start on the pad: {e}"));
    }
    let universe = pad.universe.as_mut().unwrap();
    for _ in 0..(PAD_SECONDS / PHYSICS_STEP.as_seconds()).round() as u32 {
        universe.step_physics();
    }
    let focus = universe.focus.expect("a vessel to fly");
    let now = universe.time();
    let vessel = universe.vessel(focus).expect("the vessel is there");
    let state = vessel.state_at(&universe.system, now);
    let finite = |v: DVec3| v.is_finite();
    let mut problems = Vec::new();
    let flight = vessel.flight;
    if !(finite(state.position) && finite(state.velocity)) {
        problems.push("its state is not finite".to_string());
    }
    let numbers = [
        flight.mass,
        flight.proper_acceleration,
        flight.structure_load,
        flight.thrust,
    ];
    if numbers.iter().any(|x| !x.is_finite()) {
        problems.push("its instruments read NaN".to_string());
    }
    if vessel.is_destroyed() {
        problems.push("it broke up on the pad".to_string());
    } else if !vessel.is_landed() {
        problems.push("it did not stay on the pad".to_string());
    }
    // The weight is in the load: the joint each stage hangs from carries what is above it.
    let on_pad = Conditions {
        acceleration: G,
        landed: true,
        ..Conditions::default()
    };
    let (sections, tree) = (&vessel.propulsion.sections, vessel.propulsion.parts());
    let stage_mass: Vec<f64> = sections
        .iter()
        .map(|s| {
            tree.parts()
                .iter()
                .zip(&craft.parts)
                .filter(|(part, _)| part.stage == s.id)
                .map(|(_, part)| wet(part))
                .sum()
        })
        .collect();
    let total: f64 = stage_mass.iter().sum();
    let loads = structure::joint_loads(&vessel.propulsion, &on_pad);
    for (k, section) in sections.iter().enumerate() {
        if !sections.iter().any(|t| Some(t.id) == section.parent) {
            continue;
        }
        // The section and everything hanging from it.
        let hangs = |mut at: usize| loop {
            if sections[at].id == section.id {
                return true;
            }
            match sections
                .iter()
                .position(|t| Some(t.id) == sections[at].parent)
            {
                Some(next) => at = next,
                None => return false,
            }
        };
        let below: f64 = (0..sections.len())
            .filter(|&j| hangs(j))
            .map(|j| stage_mass[j])
            .sum();
        let want = G * (total - below);
        if (loads[k] - want).abs() > 1e-6 * (1.0 + want.abs()) {
            problems.push(format!(
                "`{}` hangs from a joint that carries {:.0} N, the weight above it is {want:.0} N",
                section.name, loads[k]
            ));
        }
    }
    let _ = p;
    outcome(problems)
}
