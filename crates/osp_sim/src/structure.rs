//! Procedural tanks and fins: sized from their material and what they are built for, and
//! checked against the loads they see in flight.
//!
//! A stage's tank is a storage part ([`crate::component::storage`]): a cylinder with
//! elliptical domes, or a sphere, whose walls are sized by membrane theory for their design
//! pressure. That gives the tank's mass and its two strengths: the internal pressure at
//! which its weakest wall yields, and the axial compression at which its barrel buckles. In
//! flight a tank sees its ullage pressure plus the hydrostatic head of what is left in it,
//! and the weight, under the vessel's acceleration, of everything stacked above it.
//!
//! Fins are solid plates. The air pushes on them in proportion to the angle of attack
//! (which turns the vessel, see [`crate::aero`]), and each fin's root bends under that
//! force.

use std::f64::consts::PI;

pub use crate::component::TankContent;
use crate::{
    component::{BuiltPart, ComponentDef, PartDef, StorageDef, storage::head},
    data::{FinDef, MaterialDef, PartCatalog, TankDef, TankShape},
    geometry::preset::{CylinderTankDef, ShapeDef, SphereDef},
    parts::PartId,
    propulsion::{CraftError, Propulsion, Section},
};

/// Dome height over radius: the classic √2:1 ellipsoidal head, whose hoop stress vanishes
/// at its rim.
pub(crate) const DOME_RATIO: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Fins are bevelled at their edges: their volume and root section modulus over those of a
/// rectangular plate.
const FIN_SECTION_FILL: f64 = 0.85;
/// Normal force coefficient of a flat plate broadside to the flow.
const CROSSFLOW_DRAG: f64 = 1.2;
/// Fin drag: skin friction on both faces and the blunt edges' pressure drag.
const FIN_SKIN_FRICTION: f64 = 0.003;
const FIN_EDGE_DRAG: f64 = 1.0;

/// A procedural tank, sized for its type and contents.
#[derive(Debug, Clone)]
pub struct Tank {
    pub tank_type: String,
    pub material: String,
    pub shape: TankShape,
    /// Outer radius, m.
    pub radius: f64,
    /// Outer length (the diameter for a sphere), m.
    pub length: f64,
    /// Height of its base above the stage base, m.
    pub base: f64,
    /// Internal volume, m³, and the share of it that holds propellant.
    pub volume: f64,
    pub utilization: f64,
    pub contents: Vec<TankContent>,
    /// What it holds of each of its contents now, kg (full as built).
    pub amounts: Vec<f64>,
    /// The part it is and its section, once on a vessel.
    pub part: PartId,
    pub section: usize,
    /// Its thickest wall, m.
    pub wall: f64,
    /// Wall, stiffening and insulation, kg.
    pub dry_mass: f64,
    /// Operating (ullage) pressure, Pa.
    pub pressure: f64,
    /// Internal pressure at which its weakest wall yields, Pa.
    pub max_pressure: f64,
    /// Axial compression it carries before the wall buckles or yields, N, with the help of
    /// its operating pressure. `None` for spheres and cast solids, which are off the load
    /// path.
    pub max_axial_load: Option<f64>,
}

impl Tank {
    /// Size a tank of `def` in a stage of `stage_diameter`, its base `base` above the stage
    /// base.
    pub fn new(
        def: &TankDef,
        stage_diameter: f64,
        base: f64,
        catalog: &PartCatalog,
    ) -> Result<Self, CraftError> {
        let bad = |problem: String| CraftError::BadTank {
            tank_type: def.tank_type.clone(),
            problem,
        };
        let diameter = match (def.shape, def.diameter) {
            (_, Some(d)) => d,
            (TankShape::Cylinder, None) => stage_diameter,
            (TankShape::Sphere, None) => return Err(bad("a sphere needs a `diameter`".into())),
        };
        if diameter <= 0.0 {
            return Err(bad(format!("diameter {diameter} m")));
        }
        let (shape, length) = match def.shape {
            TankShape::Cylinder => (
                ShapeDef::CylinderTank(CylinderTankDef {
                    diameter,
                    length: def.length,
                    dome: DOME_RATIO,
                    common_bulkhead: None,
                    aft_skirt: 0.0,
                    forward_skirt: 0.0,
                }),
                def.length,
            ),
            TankShape::Sphere => (ShapeDef::Sphere(SphereDef { diameter }), diameter),
        };
        let part = PartDef {
            name: def.tank_type.clone(),
            shape,
            structure: Default::default(),
            components: vec![ComponentDef::Storage(StorageDef {
                cell: None,
                tank_type: def.tank_type.clone(),
                contents: def.propellants.clone(),
                design_load: def.design_load,
            })],
            nodes: Vec::new(),
            paint: Default::default(),
        }
        .build(catalog)
        .map_err(|e| match e {
            CraftError::BadPart { problem, .. } => bad(problem),
            e => e,
        })?;
        Ok(Self::of(&part, def.shape, diameter / 2.0, length, base))
    }

    /// The tank a storage part of `shape` is, `radius` wide and `length` long, its base
    /// `base` above its stage's.
    pub(crate) fn of(
        part: &BuiltPart,
        shape: TankShape,
        radius: f64,
        length: f64,
        base: f64,
    ) -> Self {
        let storage = part.storages().next().expect("a storage part").clone();
        let wall = part.pieces.iter().map(|p| p.thickness).fold(0.0, f64::max);
        Self {
            tank_type: storage.tank_type,
            material: storage.material,
            shape,
            radius,
            length,
            base,
            volume: storage.volume,
            utilization: storage.utilization,
            amounts: storage.contents.iter().map(|c| c.capacity).collect(),
            contents: storage.contents,
            part: PartId(u32::MAX),
            section: usize::MAX,
            wall,
            dry_mass: part.mass.mass,
            pressure: storage.pressure,
            max_pressure: storage.max_pressure,
            max_axial_load: storage.max_axial_load,
        }
    }

    /// The share of `resource` left in it, 0..1 (0 if it holds none).
    pub fn fill(&self, resource: &str) -> f64 {
        self.contents
            .iter()
            .zip(&self.amounts)
            .find(|(c, _)| c.resource == resource)
            .map_or(0.0, |(c, a)| (a / c.capacity.max(1e-9)).clamp(0.0, 1.0))
    }

    /// What it holds of `resource` now, kg.
    pub fn held(&self, resource: &str) -> f64 {
        self.contents
            .iter()
            .zip(&self.amounts)
            .filter(|(c, _)| c.resource == resource)
            .map(|(_, a)| a)
            .sum()
    }

    /// Internal pressure at the bottom of the tank, Pa, under `acceleration` (m/s² along
    /// the nose) with `fill` the fraction of each resource left.
    pub fn pressure_at(&self, acceleration: f64, fill: impl Fn(&str) -> f64) -> f64 {
        self.pressure + head(&self.contents, acceleration, fill)
    }
}

/// A stage's fins, sized from their geometry and material.
#[derive(Debug, Clone)]
pub struct FinSet {
    pub def: FinDef,
    /// Planform area of one fin, m².
    pub area: f64,
    /// All the fins together, kg.
    pub mass: f64,
    /// Normal force one fin takes before its root yields, N.
    pub max_force: f64,
}

impl FinSet {
    pub fn new(def: &FinDef, catalog: &PartCatalog) -> Result<Self, CraftError> {
        let material = material(catalog, &def.material)?;
        let bad = |problem: &str| CraftError::BadFins(problem.into());
        if def.count < 2 {
            return Err(bad("a fin set needs at least two fins"));
        }
        if def.root_chord <= 0.0 || def.tip_chord < 0.0 || def.span <= 0.0 {
            return Err(bad("chords and span must be positive"));
        }
        if def.thickness <= 0.0 || def.thickness > 0.5 * def.root_chord {
            return Err(bad(
                "thickness must be positive and thin against the root chord",
            ));
        }
        let area = (def.root_chord + def.tip_chord) / 2.0 * def.span;
        let mass = def.count as f64 * material.density * area * def.thickness * FIN_SECTION_FILL;
        let section_modulus = FIN_SECTION_FILL * def.root_chord * def.thickness.powi(2) / 6.0;
        let mut fins = Self {
            def: def.clone(),
            area,
            mass,
            max_force: 0.0,
        };
        fins.max_force = material.yield_strength * section_modulus / fins.centroid().0;
        Ok(fins)
    }

    /// Where the fin's area is centred: its distance out from the root and its height above
    /// the stage base, m.
    pub fn centroid(&self) -> (f64, f64) {
        let d = &self.def;
        let out = d.span / 3.0 * (d.root_chord + 2.0 * d.tip_chord) / (d.root_chord + d.tip_chord);
        let t = out / d.span;
        let chord = d.root_chord + (d.tip_chord - d.root_chord) * t;
        let leading_edge = d.root_chord - d.sweep * t;
        (out, leading_edge - chord / 2.0)
    }

    /// Aspect ratio of a fin and its mirror image in the hull.
    pub fn aspect_ratio(&self) -> f64 {
        2.0 * self.def.span.powi(2) / self.area
    }

    /// Where the air's push on a fin acts: its distance out from the root and its height
    /// above the stage base, m. A quarter of the mean aerodynamic chord back from its leading
    /// edge subsonic (Barrowman), moving back to half of it by Mach 2.
    pub fn aerodynamic_center(&self, mach: f64) -> (f64, f64) {
        let d = &self.def;
        let (root, tip) = (d.root_chord, d.tip_chord);
        let out = d.span / 3.0 * (root + 2.0 * tip) / (root + tip);
        let mean_chord = 2.0 / 3.0 * (root + tip - root * tip / (root + tip));
        let leading_edge = root - d.sweep * out / d.span;
        let share = 0.25 + 0.25 * (mach - 1.0).clamp(0.0, 1.0);
        (out, leading_edge - share * mean_chord)
    }

    /// Normal force coefficient of one fin (on its planform area) meeting the air at
    /// `angle_of_attack`, rad, signed like it: linear at small angles, a flat plate's
    /// crossflow drag broadside, and nothing flying straight backwards.
    pub fn normal_coefficient(&self, mach: f64, angle_of_attack: f64) -> f64 {
        let d = &self.def;
        let mid_chord_sweep = (d.sweep + (d.tip_chord - d.root_chord) / 2.0) / d.span;
        let slope = lift_slope(self.aspect_ratio(), mid_chord_sweep, mach);
        let (sin, cos) = angle_of_attack.sin_cos();
        slope * sin * cos.abs() + CROSSFLOW_DRAG * sin * sin.abs()
    }

    /// Normal force on one fin meeting the air at `angle_of_attack`, N.
    pub fn normal_force(&self, dynamic_pressure: f64, mach: f64, angle_of_attack: f64) -> f64 {
        dynamic_pressure * self.area * self.normal_coefficient(mach, angle_of_attack).abs()
    }

    /// Drag area (C_D · A) of the whole set, m².
    pub fn drag_area(&self) -> f64 {
        let d = &self.def;
        d.count as f64
            * (2.0 * self.area * FIN_SKIN_FRICTION + d.span * d.thickness * FIN_EDGE_DRAG)
    }
}

/// Lift-curve slope per radian: DATCOM's subsonic formula (Prandtl–Glauert compressibility,
/// `tan_sweep` the mid-chord sweep); supersonic, Ackeret's 4/β, bounded at low aspect ratio
/// by slender-wing theory's πA/2 (which both meet at Mach 1); and a blend between.
pub(crate) fn lift_slope(aspect: f64, tan_sweep: f64, mach: f64) -> f64 {
    let subsonic = |m: f64| {
        let beta2 = 1.0 - m * m;
        2.0 * PI * aspect / (2.0 + (4.0 + aspect * aspect * (beta2 + tan_sweep.powi(2))).sqrt())
    };
    let supersonic = |m: f64| {
        let beta = (m * m - 1.0).sqrt();
        1.0 / ((beta / 4.0).powi(2) + (2.0 / (PI * aspect)).powi(2)).sqrt()
    };
    match mach {
        m if m <= 0.8 => subsonic(m),
        m if m >= 1.2 => supersonic(m),
        m => {
            let t = (m - 0.8) / 0.4;
            subsonic(0.8) * (1.0 - t) + supersonic(1.2) * t
        }
    }
}

/// What the structure feels this tick.
#[derive(Debug, Clone, Copy, Default)]
pub struct Conditions {
    /// Proper acceleration along the nose, m/s²: positive when pushed forward, e.g. by
    /// thrust, or by the ground (1 g) standing on the pad.
    pub acceleration: f64,
    /// Drag along the axis, N. It acts on the nose, so it compresses the whole stack.
    pub drag: f64,
    pub dynamic_pressure: f64,
    pub mach: f64,
    /// Angle between the nose and the airflow, rad.
    pub angle_of_attack: f64,
    /// Standing on the ground, which takes what the engines do not hold up.
    pub landed: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Part {
    /// Index into the stage's tanks.
    Tank(usize),
    Fins,
}

/// What a part's load threatens to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Mode {
    /// Tank wall crushed by the stack above.
    Buckling,
    /// Tank wall yielding under internal pressure.
    Burst,
    /// Fin root bending.
    Bending,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PartLoad {
    /// Index into the stack, bottom first.
    pub stage: usize,
    pub part: Part,
    /// Load over strength: beyond 1 the part fails.
    pub fraction: f64,
    pub mode: Mode,
}

/// The axial load at the joint each of `stages` (a vessel's, in firing order) hangs from,
/// N, compression positive: what the stage and everything hanging from it push into it, their
/// engines' thrust and the ground under those standing on it, less what accelerating them
/// takes. For a stack, that is the weight (or inertia) of the stages above plus the drag on
/// the nose; for a booster beside its core, its thrust less its own inertia. The root hangs
/// from nothing: its load is the drag on its nose.
pub fn joint_loads(propulsion: &Propulsion, conditions: &Conditions) -> Vec<f64> {
    let stages = &propulsion.sections;
    let a = conditions.acceleration;
    let mass = |s: &Section| propulsion.section_mass(s.id);
    let thrust = |s: &Section| -> f64 {
        propulsion
            .engines_of(s.id)
            .map(|e| e.thrust * e.deflection.length().cos())
            .sum()
    };
    // On the ground it takes the rest, shared by the stages standing on it by their mass.
    let standing: Vec<bool> = crate::propulsion::placed(stages)
        .map(|(height, _)| height < 0.01)
        .collect();
    let total: f64 = stages.iter().map(mass).sum();
    let on_ground: f64 = stages
        .iter()
        .zip(&standing)
        .filter(|(_, s)| **s)
        .map(|(s, _)| mass(s))
        .sum();
    let ground = if conditions.landed {
        (total * a - stages.iter().map(thrust).sum::<f64>()).max(0.0)
    } else {
        0.0
    };
    let push = |k: usize| {
        let s = &stages[k];
        let held = if standing[k] && on_ground > 0.0 {
            ground * mass(s) / on_ground
        } else {
            0.0
        };
        thrust(s) + held - mass(s) * a
    };
    let hangs_from = |k: usize, root: usize| {
        let mut at = &stages[k];
        loop {
            if at.id == stages[root].id {
                return true;
            }
            match stages.iter().find(|t| Some(t.id) == at.parent) {
                Some(next) => at = next,
                None => return false,
            }
        }
    };
    (0..stages.len())
        .map(|s| {
            let held = stages.iter().any(|t| Some(t.id) == stages[s].parent);
            if !held {
                return conditions.drag;
            }
            (0..stages.len())
                .filter(|&k| hangs_from(k, s))
                .map(push)
                .sum()
        })
        .collect()
}

/// The load on every tank and fin set of `stages` (a vessel's, in firing order) as a
/// fraction of its strength. A stage's tanks carry the load at the joint it hangs from
/// ([`joint_loads`]), and the structure and tanks of the stage above them. Tanks whose
/// barrels stand side by side carry it together, each its share by strength.
pub fn loads(propulsion: &Propulsion, conditions: &Conditions) -> Vec<PartLoad> {
    let stages = &propulsion.sections;
    let mut out = Vec::new();
    let joints = joint_loads(propulsion, conditions);
    for (s, stage) in stages.iter().enumerate().rev() {
        let tanks: Vec<&Tank> = propulsion.tanks_of(stage.id).collect();
        // The cylinders from the top down, in rows of those side by side: each row's walls
        // carry everything stacked above its base except their own propellant, which rests
        // on their own aft domes.
        let mut cylinders: Vec<usize> = (0..tanks.len())
            .filter(|&k| tanks[k].max_axial_load.is_some())
            .collect();
        cylinders.sort_by(|&a, &b| barrel(tanks[b]).0.total_cmp(&barrel(tanks[a]).0));
        let mut rows: Vec<Vec<usize>> = Vec::new();
        let mut bottom = f64::INFINITY;
        for k in cylinders {
            let (lo, hi) = barrel(tanks[k]);
            match rows.last_mut() {
                Some(row) if hi > bottom => row.push(k),
                _ => rows.push(vec![k]),
            }
            bottom = lo;
        }
        let mut buckling = vec![None; tanks.len()];
        let mut tanks_above = 0.0;
        for row in rows {
            let base = row
                .iter()
                .map(|&k| tanks[k].base)
                .fold(f64::INFINITY, f64::min);
            let share_above = (1.0 - base / stage.length.max(1e-9)).clamp(0.0, 1.0);
            let dry: f64 = row.iter().map(|&k| tanks[k].dry_mass).sum();
            let mass = stage.structure_mass * share_above + tanks_above + dry;
            let compression = joints[s] + mass * conditions.acceleration;
            let strength: f64 = row.iter().filter_map(|&k| tanks[k].max_axial_load).sum();
            for &k in &row {
                buckling[k] = Some(compression.max(0.0) / strength);
                let tank = tanks[k];
                let propellant: f64 = tank
                    .contents
                    .iter()
                    .map(|c| c.capacity * tank.fill(&c.resource))
                    .sum();
                tanks_above += tank.dry_mass + propellant;
            }
        }
        for (k, tank) in tanks.iter().enumerate().rev() {
            let fill = |name: &str| tank.fill(name);
            let burst = tank.pressure_at(conditions.acceleration, fill) / tank.max_pressure;
            let mut load = PartLoad {
                stage: s,
                part: Part::Tank(k),
                fraction: burst,
                mode: Mode::Burst,
            };
            if let Some(buckling) = buckling[k]
                && buckling > load.fraction
            {
                load.fraction = buckling;
                load.mode = Mode::Buckling;
            }
            out.push(load);
        }
        if let Some(fins) = &stage.fins {
            let force = fins.normal_force(
                conditions.dynamic_pressure,
                conditions.mach,
                conditions.angle_of_attack,
            );
            out.push(PartLoad {
                stage: s,
                part: Part::Fins,
                fraction: force / fins.max_force,
                mode: Mode::Bending,
            });
        }
    }
    out
}

/// Where a cylinder's straight wall runs, between its domes: its bottom and top above the
/// stage base, m.
fn barrel(tank: &Tank) -> (f64, f64) {
    let dome = tank.radius * DOME_RATIO;
    (tank.base + dome, tank.base + tank.length - dome)
}

fn find<'a, T>(records: &'a [T], id: &str, key: impl Fn(&T) -> &String) -> Option<&'a T> {
    records.iter().find(|r| key(r) == id)
}

fn material<'a>(catalog: &'a PartCatalog, id: &str) -> Result<&'a MaterialDef, CraftError> {
    find(&catalog.materials, id, |m| &m.id).ok_or_else(|| CraftError::UnknownMaterial(id.into()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        constants::G0,
        propulsion::Propulsion,
        vessel::tests::{catalog, pathfinder, staged_pathfinder},
    };

    fn tank(
        tank_type: &str,
        shape: TankShape,
        length: f64,
        propellants: &[(&str, f64)],
    ) -> TankDef {
        TankDef {
            tank_type: tank_type.into(),
            shape,
            length,
            position: None,
            diameter: (shape == TankShape::Sphere).then_some(1.35),
            design_load: None,
            propellants: propellants
                .iter()
                .map(|(n, r)| (n.to_string(), *r))
                .collect(),
        }
    }

    const KEROLOX: &[(&str, f64)] = &[("Kerosene", 1.0), ("LqdOxygen", 2.36)];

    #[test]
    fn a_tank_holds_its_usable_volume_in_the_mixture_ratio() {
        let t = Tank::new(
            &tank("Isogrid Al-Li", TankShape::Cylinder, 9.6, KEROLOX),
            3.7,
            0.0,
            &catalog(),
        )
        .unwrap();
        let (r, dome) = (1.85, 1.85 * DOME_RATIO);
        let expected = PI * r * r * (9.6 - 2.0 * dome) + 4.0 / 3.0 * PI * r * r * dome;
        assert!((t.volume - expected).abs() < 1e-9);
        let used: f64 = t.contents.iter().map(|c| c.capacity / c.density).sum();
        assert!((used - t.utilization * t.volume).abs() < 1e-9);
        let (k, o) = (&t.contents[0], &t.contents[1]);
        assert!((o.capacity / k.capacity - 2.36).abs() < 1e-9, "{t:?}");
    }

    #[test]
    fn walls_are_sized_by_pressure_or_minimum_gauge() {
        let c = catalog();
        // The big booster tank's wall is set by its design pressure, a few millimetres of
        // aluminium-lithium like a real kerolox first stage.
        let booster = Tank::new(
            &tank("Isogrid Al-Li", TankShape::Cylinder, 38.0, KEROLOX),
            3.7,
            0.0,
            &c,
        )
        .unwrap();
        assert!(
            booster.wall > 0.003 && booster.wall < 0.006,
            "{}",
            booster.wall
        );
        let kind = c
            .tank_types
            .iter()
            .find(|t| t.id == "Isogrid Al-Li")
            .unwrap();
        // It yields at the safety factor times its design pressure.
        let full = booster.pressure_at(kind.design_load * G0, |_| 1.0);
        assert!((booster.max_pressure / full - kind.safety_factor).abs() < 1e-9);
        // A short tank's wall would be thinner than can be built.
        let short = Tank::new(
            &tank("Isogrid Al-Li", TankShape::Cylinder, 3.0, KEROLOX),
            3.7,
            0.0,
            &c,
        )
        .unwrap();
        assert_eq!(short.wall, kind.min_gauge);
        assert!(
            short.max_pressure
                > kind.safety_factor * short.pressure_at(kind.design_load * G0, |_| 1.0)
        );
    }

    #[test]
    fn a_pressurant_bottle_is_as_efficient_as_real_ones() {
        let bottle = Tank::new(
            &tank("COPV", TankShape::Sphere, 0.0, &[("Nitrogen", 1.0)]),
            3.7,
            0.0,
            &catalog(),
        )
        .unwrap();
        // Performance factor P·V/W of real composite bottles: 20–40 km.
        let pv_w = bottle.pressure * bottle.volume / (bottle.dry_mass * G0);
        assert!(pv_w > 20e3 && pv_w < 40e3, "{pv_w}");
        assert!(bottle.max_axial_load.is_none());
    }

    #[test]
    fn tanks_refuse_what_they_cannot_hold() {
        let c = catalog();
        let hydrogen = [("LqdHydrogen", 1.0)];
        let err = Tank::new(
            &tank("Isogrid Al-Li", TankShape::Cylinder, 9.0, &hydrogen),
            3.7,
            0.0,
            &c,
        );
        assert!(matches!(err, Err(CraftError::CannotHold { .. })), "{err:?}");
        Tank::new(
            &tank("Insulated Al-Li", TankShape::Cylinder, 9.0, &hydrogen),
            3.7,
            0.0,
            &c,
        )
        .unwrap();
        let gas = [("Nitrogen", 1.0)];
        let err = Tank::new(
            &tank("Isogrid Al-Li", TankShape::Cylinder, 9.0, &gas),
            3.7,
            0.0,
            &c,
        );
        assert!(matches!(err, Err(CraftError::CannotHold { .. })), "{err:?}");
        let err = Tank::new(
            &tank("Isogrid Al-Li", TankShape::Cylinder, 2.0, KEROLOX),
            3.7,
            0.0,
            &c,
        );
        assert!(matches!(err, Err(CraftError::BadTank { .. })), "{err:?}");
    }

    #[test]
    fn pressure_holds_a_balloon_tank_up() {
        let t = Tank::new(
            &tank("Balloon Stainless", TankShape::Cylinder, 10.0, KEROLOX),
            3.0,
            0.0,
            &catalog(),
        )
        .unwrap();
        let from_pressure = PI * t.radius * t.radius * t.pressure;
        // Its foil-thin wall alone carries only a few percent of that.
        let load = t.max_axial_load.unwrap();
        assert!(
            load > from_pressure && load < 1.1 * from_pressure,
            "{load} {from_pressure}"
        );
    }

    #[test]
    fn the_stack_weighs_on_the_booster_tank() {
        let mut p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        let on_pad = Conditions {
            acceleration: G0,
            landed: true,
            ..Default::default()
        };
        let booster_tank = |p: &Propulsion, conditions: &Conditions| {
            loads(p, conditions)
                .into_iter()
                .find(|l| l.stage == 0 && l.part == Part::Tank(0))
                .unwrap()
        };
        assert!(loads(&p, &on_pad).iter().all(|l| l.fraction < 1.0));
        // In free fall only its own pressure loads it, well within its safety factor.
        let floating = booster_tank(&p, &Conditions::default());
        assert_eq!(floating.mode, Mode::Burst);
        assert!(floating.fraction < 1.0 / 1.25 + 1e-9, "{floating:?}");
        // A payload a hundred times too heavy crushes it on the pad.
        p.sections[2].structure_mass *= 100.0;
        let crushed = booster_tank(&p, &on_pad);
        assert_eq!(crushed.mode, Mode::Buckling);
        assert!(crushed.fraction > 1.0, "{crushed:?}");
    }

    #[test]
    fn tanks_side_by_side_share_the_stack() {
        let mut p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        // A payload heavy enough to work the booster tank's walls harder than its pressure.
        p.sections[2].structure_mass *= 300.0;
        let on_pad = Conditions {
            acceleration: G0,
            landed: true,
            ..Default::default()
        };
        let booster_tanks = |p: &Propulsion| -> Vec<PartLoad> {
            loads(p, &on_pad)
                .into_iter()
                .filter(|l| l.stage == 0 && l.mode == Mode::Buckling)
                .collect()
        };
        let alone = booster_tanks(&p)[0].fraction;
        // A second booster tank beside the first, as Proton's outer tanks stand round its
        // core: neither stands on the other, and they carry the stack above together.
        let twin = p.tanks.iter().find(|t| t.section == 0).unwrap().clone();
        p.tanks.push(twin);
        let shared = booster_tanks(&p);
        assert_eq!(shared.len(), 2);
        assert_eq!(shared[0].fraction, shared[1].fraction);
        assert!(
            shared[0].fraction > 0.5 * alone && shared[0].fraction < 0.6 * alone,
            "{shared:?} {alone}"
        );
    }

    #[test]
    fn a_cast_grain_leaves_the_stack_to_its_motor_case() {
        let mut c = catalog();
        let mut grain = c
            .tank_types
            .iter()
            .find(|t| t.id == "Composite")
            .unwrap()
            .clone();
        grain.id = "Grain".into();
        grain.pressure = 0.0;
        grain.cast = true;
        c.tank_types.push(grain);
        let solid = &[("Kerosene", 1.0)];
        let t = Tank::new(
            &tank("Grain", TankShape::Cylinder, 8.0, solid),
            1.2,
            0.0,
            &c,
        )
        .unwrap();
        // However hard the motor pushes, the grain presses on nothing, and the case carries
        // the stack.
        assert_eq!(t.pressure_at(20.0 * G0, |_| 1.0), 0.0);
        assert!(t.max_axial_load.is_none());
        // Its liner is as thin as can be built.
        let kind = c.tank_types.iter().find(|t| t.id == "Grain").unwrap();
        assert_eq!(t.wall, kind.min_gauge);
    }

    #[test]
    fn fins_bend_with_dynamic_pressure_and_angle() {
        let def = FinDef {
            count: 4,
            root_chord: 2.0,
            tip_chord: 1.0,
            span: 1.0,
            sweep: 0.8,
            thickness: 0.01,
            material: "Al 2219-T87".into(),
        };
        let fins = FinSet::new(&def, &catalog()).unwrap();
        assert!((fins.area - 1.5).abs() < 1e-12);
        // Solid aluminium: 4 × 1.5 m² × 1 cm.
        assert!(fins.mass > 100.0 && fins.mass < 200.0, "{}", fins.mass);
        let (out, height) = fins.centroid();
        assert!(out > 0.0 && out < 0.5 && height > 0.0 && height < 2.0);
        let force = |q, alpha: f64| fins.normal_force(q, 0.6, alpha.to_radians());
        assert_eq!(force(30e3, 0.0), 0.0);
        assert!((force(30e3, 2.0) / force(15e3, 2.0) - 2.0).abs() < 1e-12);
        // At max-Q a couple of degrees is fine, a hard turn is not.
        assert!(force(30e3, 2.0) < fins.max_force);
        assert!(force(30e3, 20.0) > fins.max_force);
        // Faster than sound the slope falls.
        assert!(fins.normal_force(30e3, 3.0, 0.05) < fins.normal_force(30e3, 0.6, 0.05));
    }

    #[test]
    fn stages_must_fit_their_tanks() {
        let mut craft = staged_pathfinder();
        craft.stages[1].length = 5.0;
        let err = Propulsion::from_staged(&craft, &catalog());
        assert!(matches!(err, Err(CraftError::BadStage { .. })), "{err:?}");
    }
}
