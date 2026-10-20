//! Procedural tanks and fins: sized from their material and what they are built for, and
//! checked against the loads they see in flight.
//!
//! A tank wall is as thin as its material allows at the design pressure (ullage pressure
//! plus the hydrostatic head of a full tank at the type's design acceleration) with the
//! type's safety factor, but never thinner than its minimum gauge. That gives the tank's
//! mass and its two strengths: the internal pressure at which the wall yields, and the
//! axial compression at which it buckles (a thin cylinder with the NASA SP-8007 knockdown,
//! stiffened by its grid and held taut by its own pressure, which is how balloon tanks
//! stand up at all). In flight a tank sees its ullage pressure plus the hydrostatic head of
//! what is left in it, and the weight, under the vessel's acceleration, of everything
//! stacked above it.
//!
//! Fins are solid plates. The air pushes on them in proportion to the angle of attack, and
//! each fin's root bends under that force. (The force does not turn the vessel yet: there
//! is no aerodynamic moment model.)

use std::f64::consts::PI;

use crate::{
    constants::G0,
    data::{FinDef, MaterialDef, PartCatalog, TankDef, TankShape, TankTypeDef},
    propulsion::{CraftError, Stage},
};

/// Dome height over radius: the classic √2:1 ellipsoidal head.
const DOME_RATIO: f64 = std::f64::consts::FRAC_1_SQRT_2;
/// Fins are bevelled at their edges: their volume and root section modulus over those of a
/// rectangular plate.
const FIN_SECTION_FILL: f64 = 0.85;
/// Normal force coefficient of a flat plate broadside to the flow.
const CROSSFLOW_DRAG: f64 = 1.2;
/// Fin drag: skin friction on both faces and the blunt edges' pressure drag.
const FIN_SKIN_FRICTION: f64 = 0.003;
const FIN_EDGE_DRAG: f64 = 1.0;

/// One resource a tank holds.
#[derive(Debug, Clone)]
pub struct TankContent {
    pub resource: String,
    /// kg when full.
    pub capacity: f64,
    /// kg/m³.
    pub density: f64,
}

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
    /// Wall thickness, m.
    pub wall: f64,
    /// Wall, stiffening and insulation, kg.
    pub dry_mass: f64,
    /// Operating (ullage) pressure, Pa.
    pub pressure: f64,
    /// Internal pressure at which the wall yields, Pa.
    pub max_pressure: f64,
    /// Axial compression it carries before the wall buckles or yields, N, with the help of
    /// its operating pressure. `None` for spheres, which are off the load path.
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
        let kind = find(&catalog.tank_types, &def.tank_type, |t| &t.id)
            .ok_or_else(|| CraftError::UnknownTankType(def.tank_type.clone()))?;
        let material = material(catalog, &kind.material)?;
        let bad = |problem: String| CraftError::BadTank {
            tank_type: kind.id.clone(),
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
        let radius = diameter / 2.0;
        let (length, volume, area) = match def.shape {
            TankShape::Cylinder => {
                let dome = DOME_RATIO * radius;
                let barrel = def.length - 2.0 * dome;
                if barrel < 0.0 {
                    return Err(bad(format!(
                        "{} m is shorter than its domes ({:.2} m)",
                        def.length,
                        2.0 * dome
                    )));
                }
                (
                    def.length,
                    PI * radius * radius * (barrel + 4.0 / 3.0 * dome),
                    2.0 * PI * radius * barrel + 2.0 * half_spheroid_area(radius, dome),
                )
            }
            TankShape::Sphere => (
                diameter,
                4.0 / 3.0 * PI * radius.powi(3),
                4.0 * PI * radius * radius,
            ),
        };

        // Fill the usable volume in the given mass ratio.
        let total: f64 = def.propellants.values().sum();
        if def.propellants.is_empty() || total <= 0.0 {
            return Err(bad("no propellants".into()));
        }
        let mut contents = Vec::new();
        for (name, ratio) in &def.propellants {
            let resource = find(&catalog.resources, name, |r| &r.id)
                .ok_or_else(|| CraftError::UnknownResource(name.clone()))?;
            if let Some(reason) = cannot_hold(kind, resource) {
                return Err(CraftError::CannotHold {
                    tank_type: kind.id.clone(),
                    resource: name.clone(),
                    reason,
                });
            }
            contents.push(TankContent {
                resource: name.clone(),
                capacity: ratio / total,
                density: resource.density,
            });
        }
        let utilization = kind.utilization.clamp(0.0, 1.0);
        let usable = volume * utilization;
        let mass = usable / contents.iter().map(|c| c.capacity / c.density).sum::<f64>();
        for content in &mut contents {
            content.capacity *= mass;
        }

        // A sphere's membrane stress is half a cylinder's hoop stress at the same pressure.
        let shape_factor = match def.shape {
            TankShape::Cylinder => 1.0,
            TankShape::Sphere => 2.0,
        };
        let full = |_: &str| 1.0;
        let design_load = def.design_load.unwrap_or(kind.design_load);
        let design = kind.pressure + hydrostatic(&contents, usable, length, design_load * G0, full);
        let sized = kind.safety_factor * design * radius / (shape_factor * material.yield_strength);
        let wall = sized.max(kind.min_gauge);
        let max_pressure = material.yield_strength * wall * shape_factor / radius;
        let dry_mass =
            area * (material.density * wall * kind.stiffening.max(1.0) + kind.insulation);
        let max_axial_load = (def.shape == TankShape::Cylinder).then(|| {
            // NASA SP-8007 knockdown for imperfect thin cylinders.
            let knockdown = 1.0 - 0.901 * (1.0 - (-(radius / wall).sqrt() / 16.0).exp());
            let buckling =
                knockdown * 0.605 * material.elastic_modulus * wall / radius * kind.buckling_factor;
            2.0 * PI * radius * wall * buckling.min(material.yield_strength)
                + PI * radius * radius * kind.pressure
        });
        Ok(Self {
            tank_type: kind.id.clone(),
            material: material.id.clone(),
            shape: def.shape,
            radius,
            length,
            base,
            volume,
            utilization,
            contents,
            wall,
            dry_mass,
            pressure: kind.pressure,
            max_pressure,
            max_axial_load,
        })
    }

    /// Internal pressure at the bottom of the tank, Pa, under `acceleration` (m/s² along
    /// the nose) with `fill` the fraction of each resource left.
    pub fn pressure_at(&self, acceleration: f64, fill: impl Fn(&str) -> f64) -> f64 {
        let usable = self.volume * self.utilization;
        self.pressure + hydrostatic(&self.contents, usable, self.length, acceleration, fill)
    }
}

/// The tallest liquid column's pressure: each resource fills its own compartment, a share
/// of the tank's length.
fn hydrostatic(
    contents: &[TankContent],
    usable: f64,
    length: f64,
    acceleration: f64,
    fill: impl Fn(&str) -> f64,
) -> f64 {
    contents
        .iter()
        .map(|c| {
            let column = length * c.capacity / c.density / usable * fill(&c.resource);
            c.density * acceleration.max(0.0) * column
        })
        .fold(0.0, f64::max)
}

fn cannot_hold(kind: &TankTypeDef, resource: &crate::data::ResourceDef) -> Option<String> {
    if let Some(allowed) = &kind.propellants
        && !allowed.contains(&resource.id)
    {
        return Some(format!("it only holds {}", allowed.join(", ")));
    }
    if resource.temperature < kind.min_temperature {
        return Some(format!(
            "it is stored at {:.0} K and the tank is only insulated down to {:.0} K",
            resource.temperature, kind.min_temperature
        ));
    }
    if resource.pressure > kind.pressure {
        return Some(format!(
            "it is stored at {:.0} bar and the tank is rated for {:.1} bar",
            resource.pressure / 1e5,
            kind.pressure / 1e5
        ));
    }
    None
}

/// Surface of half an oblate spheroid with equatorial radius `a` and height `c` (< a).
fn half_spheroid_area(a: f64, c: f64) -> f64 {
    let e = (1.0 - (c / a).powi(2)).sqrt();
    if e < 1e-9 {
        return 2.0 * PI * a * a;
    }
    PI * a * a + PI * c * c / (2.0 * e) * ((1.0 + e) / (1.0 - e)).ln()
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

    /// Normal force on one fin meeting the air at `angle_of_attack`, N.
    pub fn normal_force(&self, dynamic_pressure: f64, mach: f64, angle_of_attack: f64) -> f64 {
        let d = &self.def;
        let mid_chord_sweep = (d.sweep + (d.tip_chord - d.root_chord) / 2.0) / d.span;
        let slope = lift_slope(self.aspect_ratio(), mid_chord_sweep, mach);
        // Linear at small angles, a flat plate's crossflow drag broadside, and nothing
        // flying straight backwards.
        let a = angle_of_attack.abs().min(PI);
        let coefficient = slope * (a.sin() * a.cos()).abs() + CROSSFLOW_DRAG * a.sin().powi(2);
        dynamic_pressure * self.area * coefficient
    }

    /// Drag area (C_D · A) of the whole set, m².
    pub fn drag_area(&self) -> f64 {
        let d = &self.def;
        d.count as f64
            * (2.0 * self.area * FIN_SKIN_FRICTION + d.span * d.thickness * FIN_EDGE_DRAG)
    }
}

/// Lift-curve slope per radian: DATCOM's subsonic formula (Prandtl–Glauert compressibility,
/// `tan_sweep` the mid-chord sweep), Ackeret's supersonic one, and a blend between.
fn lift_slope(aspect: f64, tan_sweep: f64, mach: f64) -> f64 {
    let subsonic = |m: f64| {
        let beta2 = 1.0 - m * m;
        2.0 * PI * aspect / (2.0 + (4.0 + aspect * aspect * (beta2 + tan_sweep.powi(2))).sqrt())
    };
    let supersonic = |m: f64| 4.0 / (m * m - 1.0).sqrt();
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

/// The load on every tank and fin set of a stack (bottom first) as a fraction of its
/// strength.
pub fn loads(stages: &[Stage], conditions: &Conditions) -> Vec<PartLoad> {
    let mut out = Vec::new();
    // Mass of the stages above the current one.
    let mut above = 0.0;
    for (s, stage) in stages.iter().enumerate().rev() {
        let fill = |name: &str| {
            stage
                .propellants
                .iter()
                .find(|p| p.name == name)
                .map_or(0.0, |p| p.amount / p.capacity.max(1e-9))
        };
        // Cylinders from the top down: each one's wall carries everything stacked above its
        // base except its own propellant, which rests on its own aft dome.
        let mut tanks_above = 0.0;
        for (k, tank) in stage.tanks.iter().enumerate().rev() {
            let burst = tank.pressure_at(conditions.acceleration, fill) / tank.max_pressure;
            let mut load = PartLoad {
                stage: s,
                part: Part::Tank(k),
                fraction: burst,
                mode: Mode::Burst,
            };
            if let Some(max_load) = tank.max_axial_load {
                let share_above = (1.0 - tank.base / stage.length.max(1e-9)).clamp(0.0, 1.0);
                let mass = above + stage.structure_mass * share_above + tanks_above + tank.dry_mass;
                let compression = mass * conditions.acceleration + conditions.drag;
                let buckling = compression.max(0.0) / max_load;
                if buckling > load.fraction {
                    load.fraction = buckling;
                    load.mode = Mode::Buckling;
                }
                let propellant: f64 = tank
                    .contents
                    .iter()
                    .map(|c| c.capacity * fill(&c.resource))
                    .sum();
                tanks_above += tank.dry_mass + propellant;
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
        above += stage.mass();
    }
    out
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
        data::CraftDef,
        propulsion::Propulsion,
        vessel::tests::{catalog, pathfinder},
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
            ..Default::default()
        };
        let booster_tank = |p: &Propulsion, conditions: &Conditions| {
            loads(&p.stages, conditions)
                .into_iter()
                .find(|l| l.stage == 0 && l.part == Part::Tank(0))
                .unwrap()
        };
        assert!(loads(&p.stages, &on_pad).iter().all(|l| l.fraction < 1.0));
        // In free fall only its own pressure loads it, well within its safety factor.
        let floating = booster_tank(&p, &Conditions::default());
        assert_eq!(floating.mode, Mode::Burst);
        assert!(floating.fraction < 1.0 / 1.25 + 1e-9, "{floating:?}");
        // A payload a hundred times too heavy crushes it on the pad.
        p.stages[2].structure_mass *= 100.0;
        let crushed = booster_tank(&p, &on_pad);
        assert_eq!(crushed.mode, Mode::Buckling);
        assert!(crushed.fraction > 1.0, "{crushed:?}");
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
        let mut craft: CraftDef = pathfinder();
        craft.stages[1].length = 5.0;
        let err = Propulsion::from_craft(&craft, &catalog());
        assert!(matches!(err, Err(CraftError::BadStage { .. })), "{err:?}");
    }
}
