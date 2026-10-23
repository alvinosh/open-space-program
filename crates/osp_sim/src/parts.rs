//! The part tree: what a vessel is made of and how the pieces are joined.
//!
//! As in Kitten Space Agency, a vessel is a tree of parts. Every part but the root hangs
//! from its parent by a joint between two connectors, one on each part. A connector is
//! fixed (a node at one place, like either end of a stack) or a surface (anywhere on the
//! part, for things mounted on it), and says what it passes on: propellant, electricity,
//! or nothing until it is released. A joint carries what both its connectors carry.
//! Releasing a decoupler's joint splits the tree in two ([`PartTree::release`]): the side
//! with the root stays the vessel and the other flies on as a vessel of its own.
//!
//! Craft files still describe stages, so the tree is built from them
//! ([`PartTree::from_craft`]): each stage becomes a hull that carries the stage's parts,
//! hanging from a decoupler under the stage above. The stages the physics runs on
//! ([`crate::propulsion::Stage`]) are gathered back from the tree until parts take over
//! from them.
//!
//! Positions are in the assembly frame: metres from the base of the craft's bottom stage,
//! +Z up the stack to the nose, +X the pilot's right and +Y their feet, as in the vessel
//! frame. Parts keep their positions when the tree splits.

use std::collections::BTreeMap;

use glam::{DQuat, DVec2, DVec3};

use crate::{
    aero::Panel,
    data::{
        CraftDef, EngineDef, GearDef, PartCatalog, PodDef, RcsDef, ReactionWheelDef,
        StageAppearance, TankShape, WingDef,
    },
    gear::Leg,
    mass::Mass,
    propulsion::{CraftError, FAIRING_HALVES},
    structure::{FinSet, Tank},
};

/// Names a part for as long as it exists, through splits.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct PartId(pub u32);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConnectorKind {
    /// A node at one place on the part, joined to a fixed node of another part.
    Fixed,
    /// Anywhere on (or in) the part. What is mounted on it touches it with a surface
    /// connector of its own, which says where.
    Surface,
}

/// What crosses a connector.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Capabilities {
    /// Propellant flows through.
    pub fluid: bool,
    /// Electricity flows through.
    pub power: bool,
    /// The joint can be released in flight (a decoupler, a fairing's hinge).
    pub release: bool,
}

impl Capabilities {
    pub const NONE: Self = Self {
        fluid: false,
        power: false,
        release: false,
    };
    pub const PLUMBED: Self = Self {
        fluid: true,
        power: true,
        release: false,
    };
    pub const POWER: Self = Self {
        fluid: false,
        power: true,
        release: false,
    };
    pub const RELEASE: Self = Self {
        fluid: false,
        power: false,
        release: true,
    };

    /// What a joint between connectors with these carries: what both pass on. It can be
    /// released if either can.
    pub fn joined(self, other: Self) -> Self {
        Self {
            fluid: self.fluid && other.fluid,
            power: self.power && other.power,
            release: self.release || other.release,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Connector {
    pub name: &'static str,
    pub kind: ConnectorKind,
    /// In the part's frame, m: where a fixed node meets the other part's, or where a
    /// mounted part touches its parent's surface.
    pub position: DVec3,
    pub capabilities: Capabilities,
}

impl Connector {
    fn fixed(name: &'static str, position: DVec3, capabilities: Capabilities) -> Self {
        Self {
            name,
            kind: ConnectorKind::Fixed,
            position,
            capabilities,
        }
    }

    fn surface(name: &'static str, capabilities: Capabilities) -> Self {
        Self {
            name,
            kind: ConnectorKind::Surface,
            position: DVec3::ZERO,
            capabilities,
        }
    }
}

/// How a part hangs from its parent: which of the parent's connectors holds which of its
/// own (indices into their `connectors`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Joint {
    pub parent: PartId,
    pub parent_connector: usize,
    pub connector: usize,
}

#[derive(Debug, Clone)]
pub enum PartKind {
    /// A stage's body: the shell and the structure, avionics or payload inside it, which
    /// carries the rest of the stage. Its frame's origin is the stage base.
    Hull {
        /// kg, spread along it.
        structure_mass: f64,
        diameter: f64,
        length: f64,
        appearance: StageAppearance,
    },
    /// Holds its stage under the one above until released; massless.
    Decoupler,
    /// A procedural tank; its origin is its base.
    Tank(Tank),
    /// Propellant ratios normalised. Its origin is the gimbal pivot, nozzle down −Z.
    Engine(EngineDef),
    /// Propellant ratios normalised. Its origin is on the hull, +X outward.
    Rcs(RcsDef),
    ReactionWheel(ReactionWheelDef),
    /// A crew cabin; its origin is its base.
    Pod(PodDef),
    /// A set of fins around the stage base.
    Fins(FinSet),
    /// Its origin is the root's leading edge, +X along the span.
    Wing(WingDef),
    /// Its origin is the strut's mount, +X outward.
    Gear(GearDef),
    /// One half of a payload fairing, a thin shell around its stage facing `angle` (rad
    /// about the axis from +X). Its origin is the stage base.
    FairingHalf {
        mass: f64,
        angle: f64,
        radius: f64,
        length: f64,
    },
}

#[derive(Debug, Clone)]
pub struct Part {
    pub id: PartId,
    pub name: String,
    pub kind: PartKind,
    /// The craft file stage it belongs to ([`crate::propulsion::Stage::id`]), while the
    /// physics runs on stages.
    pub stage: usize,
    /// Its origin in the assembly frame, m, and how its axes are turned there.
    pub position: DVec3,
    pub rotation: DQuat,
    pub connectors: Vec<Connector>,
    /// How it hangs from its parent; `None` for the root.
    pub joint: Option<Joint>,
}

impl Part {
    /// Where connector `index` is in the assembly frame.
    pub fn connector_position(&self, index: usize) -> DVec3 {
        self.position + self.rotation * self.connectors[index].position
    }
}

/// A hull's connectors: the ends of the stack, and its surface for what it carries.
const HULL_TOP: usize = 0;
const HULL_BOTTOM: usize = 1;
const HULL_SURFACE: usize = 2;
/// A decoupler's connectors: the side it releases, and the side it stays on.
const DECOUPLER_TOP: usize = 0;
const DECOUPLER_BOTTOM: usize = 1;

/// The parts of a vessel.
#[derive(Debug, Clone, Default)]
pub struct PartTree {
    /// By id.
    parts: Vec<Part>,
}

impl PartTree {
    /// The tree a craft file's stages describe. Each stage becomes a hull, with its parts
    /// mounted on the hull's surface; each stage but the top one hangs from a decoupler
    /// under the stage above, so the top stage's hull is the root.
    pub fn from_craft(craft: &CraftDef, catalog: &PartCatalog) -> Result<Self, CraftError> {
        if craft.stages.is_empty() {
            return Err(CraftError::NoStages);
        }
        let mut tree = Self::default();
        let mut hulls = Vec::new();
        let mut decouplers = Vec::new();
        let mut base = 0.0;
        for (stage, def) in craft.stages.iter().enumerate() {
            let bad = |problem: String| CraftError::BadStage {
                stage: def.name.clone(),
                problem,
            };
            let hull = tree.add(
                stage,
                def.name.clone(),
                PartKind::Hull {
                    structure_mass: def.structure_mass,
                    diameter: def.diameter,
                    length: def.length,
                    appearance: def.appearance.clone(),
                },
                DVec3::Z * base,
                DQuat::IDENTITY,
                vec![
                    Connector::fixed("top", DVec3::Z * def.length, Capabilities::PLUMBED),
                    Connector::fixed("bottom", DVec3::ZERO, Capabilities::PLUMBED),
                    Connector::surface("surface", Capabilities::PLUMBED),
                ],
            );
            hulls.push(hull);
            // Each of the stage's parts is mounted on the hull, turned `angle` about the
            // axis (from +X towards +Y).
            let mut mount = |name: String, kind, position, angle: f64, carries| {
                let rotation = DQuat::from_rotation_z(angle);
                let connectors = vec![Connector::surface("mount", carries)];
                let id = tree.add(stage, name, kind, position, rotation, connectors);
                tree.part_mut(id).joint = Some(Joint {
                    parent: hull,
                    parent_connector: HULL_SURFACE,
                    connector: 0,
                });
            };
            let mut engines = Vec::new();
            for name in &def.engines {
                let mut engine = catalog
                    .engines
                    .iter()
                    .find(|e| &e.id == name)
                    .cloned()
                    .ok_or_else(|| CraftError::UnknownEngine(name.clone()))?;
                normalise(&mut engine.propellants);
                engines.push(engine);
            }
            let layout = engine_layout(engines.len(), def.diameter / 2.0);
            for (engine, at) in engines.into_iter().zip(layout) {
                let name = engine.id.clone();
                let plumbed = Capabilities::PLUMBED;
                mount(
                    name,
                    PartKind::Engine(engine),
                    at.extend(base),
                    0.0,
                    plumbed,
                );
            }
            for name in &def.reaction_wheels {
                let wheel = catalog
                    .reaction_wheels
                    .iter()
                    .find(|w| &w.id == name)
                    .cloned()
                    .ok_or_else(|| CraftError::UnknownReactionWheel(name.clone()))?;
                let middle = DVec3::Z * (base + def.length / 2.0);
                let kind = PartKind::ReactionWheel(wheel);
                mount(name.clone(), kind, middle, 0.0, Capabilities::POWER);
            }
            // Evenly around the stage's forward rim.
            for (i, name) in def.rcs.iter().enumerate() {
                let mut block = catalog
                    .rcs
                    .iter()
                    .find(|b| &b.id == name)
                    .cloned()
                    .ok_or_else(|| CraftError::UnknownRcs(name.clone()))?;
                normalise(&mut block.propellants);
                let angle = std::f64::consts::TAU * i as f64 / def.rcs.len() as f64;
                let at = DVec2::from_angle(angle) * def.diameter / 2.0;
                let kind = PartKind::Rcs(block);
                let plumbed = Capabilities::PLUMBED;
                mount(
                    name.clone(),
                    kind,
                    at.extend(base + def.length),
                    angle,
                    plumbed,
                );
            }
            if let Some(name) = &def.pod {
                let pod = catalog
                    .pods
                    .iter()
                    .find(|p| &p.id == name)
                    .cloned()
                    .ok_or_else(|| CraftError::UnknownPod(name.clone()))?;
                if pod.height > def.length {
                    return Err(bad(format!(
                        "the {} pod is {} m tall, the stage only {} m",
                        pod.id, pod.height, def.length
                    )));
                }
                let top = DVec3::Z * (base + def.length - pod.height);
                mount(
                    name.clone(),
                    PartKind::Pod(pod),
                    top,
                    0.0,
                    Capabilities::POWER,
                );
            }
            if let Some(fins) = &def.fins {
                let kind = PartKind::Fins(FinSet::new(fins, catalog)?);
                mount(
                    "Fins".into(),
                    kind,
                    DVec3::Z * base,
                    0.0,
                    Capabilities::NONE,
                );
            }
            for wing in &def.wings {
                check_wing(wing)?;
                let angle = wing.angle.to_radians();
                let at = DVec2::from_angle(angle) * def.diameter / 2.0;
                let root = at.extend(base + wing.position);
                let kind = PartKind::Wing(wing.clone());
                mount(wing.name.clone(), kind, root, angle, Capabilities::NONE);
            }
            for leg in &def.gear {
                if leg.length <= 0.0 || leg.wheel_radius <= 0.0 || leg.stroke <= 0.0 {
                    return Err(bad(format!(
                        "gear `{}`: length, wheel radius and stroke must be positive",
                        leg.name
                    )));
                }
                if leg.load <= 0.0 || leg.mass < 0.0 {
                    return Err(bad(format!(
                        "gear `{}`: it needs a load to be sprung for, and no negative mass",
                        leg.name
                    )));
                }
                let angle = leg.angle.to_radians();
                let at = DVec2::from_angle(angle) * def.diameter / 2.0;
                let strut = at.extend(base + leg.position);
                let kind = PartKind::Gear(leg.clone());
                mount(leg.name.clone(), kind, strut, angle, Capabilities::NONE);
            }
            // Cylinders stack up from the stage base unless placed higher, each above the
            // one before; spheres sit inside, at the base unless placed.
            let mut stacked = 0.0;
            for tank_def in &def.tanks {
                let cylinder = tank_def.shape == TankShape::Cylinder;
                let at = tank_def
                    .position
                    .unwrap_or(if cylinder { stacked } else { 0.0 });
                if at < 0.0 || (cylinder && at < stacked - 1e-9) {
                    return Err(bad(format!(
                        "a tank placed {at} m up is below the stage base or the tank before it"
                    )));
                }
                let tank = Tank::new(tank_def, def.diameter, at, catalog)?;
                if 2.0 * tank.radius > def.diameter + 1e-9 {
                    return Err(bad(format!(
                        "a {} m tank does not fit its {} m diameter",
                        2.0 * tank.radius,
                        def.diameter
                    )));
                }
                if at + tank.length > def.length + 1e-9 {
                    return Err(bad(format!(
                        "its tanks reach {} m up, the stage is only {} m long",
                        at + tank.length,
                        def.length
                    )));
                }
                if cylinder {
                    stacked = at + tank.length;
                }
                let name = tank.tank_type.clone();
                let bottom = DVec3::Z * (base + at);
                mount(
                    name,
                    PartKind::Tank(tank),
                    bottom,
                    0.0,
                    Capabilities::PLUMBED,
                );
            }
            if let Some(fairing) = &def.fairing {
                for angle in FAIRING_HALVES {
                    let kind = PartKind::FairingHalf {
                        mass: fairing.mass / 2.0,
                        angle,
                        radius: def.diameter / 2.0,
                        length: def.length,
                    };
                    let release = Capabilities::RELEASE;
                    mount("Fairing".into(), kind, DVec3::Z * base, angle, release);
                }
            }
            if stage + 1 < craft.stages.len() {
                let top = DVec3::Z * (base + def.length);
                decouplers.push(tree.add(
                    stage,
                    "Decoupler".into(),
                    PartKind::Decoupler,
                    top,
                    DQuat::IDENTITY,
                    vec![
                        Connector::fixed("top", DVec3::ZERO, Capabilities::RELEASE),
                        Connector::fixed("bottom", DVec3::ZERO, Capabilities::POWER),
                    ],
                ));
            }
            base += def.length;
        }
        for (stage, &decoupler) in decouplers.iter().enumerate() {
            tree.part_mut(decoupler).joint = Some(Joint {
                parent: hulls[stage + 1],
                parent_connector: HULL_BOTTOM,
                connector: DECOUPLER_TOP,
            });
            tree.part_mut(hulls[stage]).joint = Some(Joint {
                parent: decoupler,
                parent_connector: DECOUPLER_BOTTOM,
                connector: HULL_TOP,
            });
        }
        Ok(tree)
    }

    /// A new part, not joined to anything yet.
    fn add(
        &mut self,
        stage: usize,
        name: String,
        kind: PartKind,
        position: DVec3,
        rotation: DQuat,
        connectors: Vec<Connector>,
    ) -> PartId {
        let id = PartId(self.parts.last().map_or(0, |p| p.id.0 + 1));
        self.parts.push(Part {
            id,
            name,
            kind,
            stage,
            position,
            rotation,
            connectors,
            joint: None,
        });
        id
    }

    fn part_mut(&mut self, id: PartId) -> &mut Part {
        let index = self.index(id).expect("part on the tree");
        &mut self.parts[index]
    }

    fn index(&self, id: PartId) -> Option<usize> {
        self.parts.binary_search_by_key(&id, |p| p.id).ok()
    }

    /// Every part, by id.
    pub fn parts(&self) -> &[Part] {
        &self.parts
    }

    pub fn get(&self, id: PartId) -> Option<&Part> {
        self.index(id).map(|i| &self.parts[i])
    }

    /// The part everything else hangs from.
    pub fn root(&self) -> Option<&Part> {
        self.parts.iter().find(|p| p.joint.is_none())
    }

    pub fn parent(&self, id: PartId) -> Option<&Part> {
        self.get(self.get(id)?.joint?.parent)
    }

    /// The parts hanging from `id` itself.
    pub fn children(&self, id: PartId) -> impl Iterator<Item = &Part> {
        self.parts
            .iter()
            .filter(move |p| p.joint.is_some_and(|j| j.parent == id))
    }

    /// `id` and everything hanging from it, `id` first.
    pub fn subtree(&self, id: PartId) -> Vec<PartId> {
        if self.get(id).is_none() {
            return Vec::new();
        }
        let mut found = vec![id];
        let mut next = 0;
        while let Some(&parent) = found.get(next) {
            found.extend(self.children(parent).map(|p| p.id));
            next += 1;
        }
        found
    }

    /// The parts of craft stage `stage` still on the tree.
    pub fn stage(&self, stage: usize) -> impl Iterator<Item = &Part> {
        self.parts.iter().filter(move |p| p.stage == stage)
    }

    /// What the joint `id` hangs from carries; `None` for the root.
    pub fn joint_capabilities(&self, id: PartId) -> Option<Capabilities> {
        let part = self.get(id)?;
        let joint = part.joint?;
        let parent = self.get(joint.parent)?;
        let theirs = parent.connectors[joint.parent_connector].capabilities;
        Some(theirs.joined(part.connectors[joint.connector].capabilities))
    }

    /// Release the joint `id` hangs from, if it can be released: `id` and everything
    /// hanging from it leave as a tree of their own, `id` its root.
    pub fn release(&mut self, id: PartId) -> Option<PartTree> {
        self.joint_capabilities(id).filter(|c| c.release)?;
        self.break_off(id)
    }

    /// Take `id` and everything hanging from it off the tree, whatever held it (it tore
    /// off), as a tree of its own. Not the root.
    pub fn break_off(&mut self, id: PartId) -> Option<PartTree> {
        self.get(id)?.joint?;
        let gone = self.subtree(id);
        let (parts, kept) = std::mem::take(&mut self.parts)
            .into_iter()
            .partition(|p| gone.contains(&p.id));
        self.parts = kept;
        let mut tree = PartTree { parts };
        tree.part_mut(id).joint = None;
        Some(tree)
    }

    /// Parts that propellant can flow between through joints that carry it, each group
    /// in id order. Parts with no plumbing are in none.
    pub fn fluid_groups(&self) -> Vec<Vec<PartId>> {
        let mut groups: BTreeMap<PartId, Vec<PartId>> = BTreeMap::new();
        let plumbed = |p: &Part| p.connectors.iter().any(|c| c.capabilities.fluid);
        for part in self.parts.iter().filter(|p| plumbed(p)) {
            // In a tree, the part a group reaches highest up names it.
            let mut top = part;
            while self.joint_capabilities(top.id).is_some_and(|c| c.fluid)
                && let Some(parent) = self.parent(top.id)
            {
                top = parent;
            }
            groups.entry(top.id).or_default().push(part.id);
        }
        groups.into_values().collect()
    }

    /// Where the stack stands: the base of its lowest hull (or of a fairing half on its
    /// own), m up the assembly frame.
    pub fn base(&self) -> f64 {
        self.parts
            .iter()
            .filter(|p| matches!(p.kind, PartKind::Hull { .. } | PartKind::FairingHalf { .. }))
            .map(|p| p.position.z)
            .reduce(f64::min)
            .unwrap_or(0.0)
    }

    /// How everything but the propellant is spread, in the assembly frame.
    pub fn dry_mass(&self) -> Mass {
        self.parts.iter().map(|p| self.mass_of(p)).sum()
    }

    /// The dry mass of `part`, in the assembly frame. Each part is a simple shape: a hull a
    /// solid cylinder, a tank its wall, an engine, RCS block or reaction wheel a point, a
    /// pod a solid frustum, a fairing half a half shell, fins and wings flat plates and gear
    /// legs points halfway down their struts.
    fn mass_of(&self, part: &Part) -> Mass {
        // What is mounted on a hull stands out from it.
        let (hull_radius, hull_base) = match self.parent(part.id) {
            Some(Part {
                kind: PartKind::Hull { diameter, .. },
                position,
                ..
            }) => (diameter / 2.0, position.z),
            _ => (0.0, part.position.z),
        };
        let local = match &part.kind {
            PartKind::Hull {
                structure_mass,
                diameter,
                length,
                ..
            } => Mass::cylinder(*structure_mass, diameter / 2.0, *length),
            PartKind::Decoupler => Mass::ZERO,
            PartKind::Tank(tank) => match tank.shape {
                TankShape::Cylinder => Mass::tube(tank.dry_mass, tank.radius, tank.length),
                TankShape::Sphere => Mass::spherical_shell(tank.dry_mass, tank.radius)
                    .placed(DVec3::Z * tank.radius, DQuat::IDENTITY),
            },
            PartKind::Engine(def) => Mass::point(def.mass, DVec3::ZERO),
            PartKind::Rcs(def) => Mass::point(def.mass, DVec3::ZERO),
            PartKind::ReactionWheel(def) => Mass::point(def.mass, DVec3::ZERO),
            PartKind::Pod(def) => Mass::frustum(
                def.mass,
                def.diameter / 2.0,
                def.top_diameter() / 2.0,
                def.height,
            ),
            PartKind::FairingHalf {
                mass,
                radius,
                length,
                ..
            } => Mass::half_tube(*mass, *radius, *length),
            PartKind::Fins(fins) => return fin_mass(fins, hull_radius, hull_base),
            PartKind::Wing(def) => return wing_mass(def, hull_radius, hull_base),
            PartKind::Gear(def) => {
                return Leg::of(def, hull_radius, hull_base)
                    .iter()
                    .map(|leg| Mass::point(leg.mass, leg.axle(leg.length / 2.0)))
                    .sum();
            }
        };
        local.placed(part.position, part.rotation)
    }
}

/// A fin set's fins around a hull of `hull_radius` standing at `base`: each a plate from
/// mid-chord at its root to mid-chord at its tip, its trailing edge along the base.
fn fin_mass(fins: &FinSet, hull_radius: f64, base: f64) -> Mass {
    let def = &fins.def;
    let (out, height) = fins.centroid();
    let chord = fins.area / def.span;
    let rise = (def.root_chord - def.tip_chord) / 2.0 - def.sweep;
    let each = fins.mass / def.count as f64;
    (0..def.count)
        .map(|k| {
            let (s, c) = (std::f64::consts::TAU * k as f64 / def.count as f64).sin_cos();
            let outward = DVec3::new(c, s, 0.0);
            let center = outward * (hull_radius + out) + DVec3::Z * (base + height);
            let span = outward * def.span + DVec3::Z * rise;
            Mass::plate(each, center, span, DVec3::Z * chord)
        })
        .sum()
}

/// A wing's panels on a hull of `hull_radius` standing at `base`: plates from mid-chord at
/// the root to mid-chord at the tip, as wide as their mean chord along the hull.
fn wing_mass(wing: &WingDef, hull_radius: f64, base: f64) -> Mass {
    let panels = Panel::of(wing, hull_radius, base);
    let each = wing.mass / panels.len() as f64;
    panels
        .iter()
        .map(|p| {
            let a = p.root - DVec3::Z * p.root_chord / 2.0;
            let b = p.tip - DVec3::Z * p.tip_chord / 2.0;
            let chord = (p.root_chord + p.tip_chord) / 2.0;
            Mass::plate(each, (a + b) / 2.0, b - a, DVec3::Z * chord)
        })
        .sum()
}

/// Mixture ratios as shares of one.
fn normalise(propellants: &mut BTreeMap<String, f64>) {
    let total: f64 = propellants.values().sum();
    for ratio in propellants.values_mut() {
        *ratio /= total;
    }
}

/// Engine positions for a stage of the given radius, seen from behind: a single engine on
/// the axis, small clusters on a ring, five as a cross (like the Saturn V's first stages),
/// and seven or more as a ring around a central engine (like a Falcon 9 octaweb). Off-axis
/// engines can roll the vessel by gimballing sideways.
fn engine_layout(count: usize, radius: f64) -> Vec<DVec2> {
    let ring = |n: usize, r: f64| {
        (0..n).map(move |i| DVec2::from_angle(std::f64::consts::TAU * i as f64 / n as f64) * r)
    };
    match count {
        0 => Vec::new(),
        1 => vec![DVec2::ZERO],
        5 => std::iter::once(DVec2::ZERO)
            .chain(ring(4, 0.76 * radius))
            .collect(),
        2..=6 => ring(count, 0.5 * radius).collect(),
        _ => std::iter::once(DVec2::ZERO)
            .chain(ring(count - 1, 0.65 * radius))
            .collect(),
    }
}

/// A wing must have a shape, a thickness that is an airfoil's, and its control surface
/// must fit on it.
fn check_wing(wing: &WingDef) -> Result<(), CraftError> {
    let bad = |problem: &str| {
        Err(CraftError::BadWing {
            wing: wing.name.clone(),
            problem: problem.into(),
        })
    };
    if wing.root_chord <= 0.0 || wing.tip_chord < 0.0 || wing.span <= 0.0 {
        return bad("chords and span must be positive");
    }
    if !(0.0..=0.5).contains(&wing.thickness) || wing.thickness == 0.0 {
        return bad("thickness is a share of the chord, above 0 and at most 0.5");
    }
    if wing.mass < 0.0 {
        return bad("mass cannot be negative");
    }
    if let Some(c) = &wing.control {
        if !(c.chord > 0.0 && c.chord <= 1.0) {
            return bad("the control surface's chord is a share of the wing's, up to 1");
        }
        if !(0.0 <= c.span[0] && c.span[0] < c.span[1] && c.span[1] <= 1.0) {
            return bad(
                "the control surface's span runs from a share to a larger one, within 0..1",
            );
        }
        if !(c.deflection > 0.0 && c.deflection <= 60.0) {
            return bad("the control surface deflects more than 0 and at most 60 degrees");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        propulsion::Propulsion,
        test_data::records,
        vessel::tests::{catalog, pathfinder},
    };

    fn stock_craft() -> Vec<CraftDef> {
        [
            include_str!("../../../assets/core/craft/comet.json5"),
            include_str!("../../../assets/core/craft/glider.json5"),
            include_str!("../../../assets/core/craft/lunar_upper.json5"),
            include_str!("../../../assets/core/craft/pathfinder.json5"),
            include_str!("../../../assets/core/craft/pathfinder_crew.json5"),
            include_str!("../../../assets/core/craft/sounder.json5"),
            include_str!("../../../assets/core/craft/upper_stage.json5"),
        ]
        .into_iter()
        .map(|text| records(text).remove(0))
        .collect()
    }

    fn named<'a>(tree: &'a PartTree, name: &str) -> &'a Part {
        tree.parts().iter().find(|p| p.name == name).unwrap()
    }

    fn sorted(mut ids: Vec<PartId>) -> Vec<PartId> {
        ids.sort();
        ids
    }

    #[test]
    fn stages_hang_from_decouplers_under_the_stage_above() {
        let tree = PartTree::from_craft(&pathfinder(), &catalog()).unwrap();
        let root = tree.root().unwrap();
        assert_eq!(root.name, "Payload");
        // Payload ← decoupler ← Upper Stage ← decoupler ← Booster.
        let upper = named(&tree, "Upper Stage");
        let booster = named(&tree, "Booster");
        let between = |hull: &Part| {
            let decoupler = tree.parent(hull.id).unwrap();
            assert!(matches!(decoupler.kind, PartKind::Decoupler));
            assert_eq!(decoupler.stage, hull.stage);
            tree.parent(decoupler.id).unwrap().id
        };
        assert_eq!(between(upper), root.id);
        assert_eq!(between(booster), upper.id);
        // The joint above a decoupler lets go and carries nothing; the one below it carries
        // power but no propellant.
        let decoupler = tree.parent(booster.id).unwrap();
        assert_eq!(
            tree.joint_capabilities(decoupler.id),
            Some(Capabilities::RELEASE)
        );
        assert_eq!(
            tree.joint_capabilities(booster.id),
            Some(Capabilities::POWER)
        );
        // The stage's parts are mounted on its hull.
        let on_booster: Vec<_> = tree.children(booster.id).map(|p| p.name.as_str()).collect();
        assert_eq!(
            on_booster.iter().filter(|&&n| n == "K-1 Booster").count(),
            9
        );
        assert!(on_booster.contains(&"Isogrid Al-Li"));
        assert_eq!(
            tree.children(root.id)
                .filter(|p| p.name == "Fairing")
                .count(),
            2
        );
        assert_eq!(tree.subtree(root.id).len(), tree.parts().len());
    }

    #[test]
    fn joints_join_connectors_of_a_kind_and_fixed_nodes_meet() {
        for craft in stock_craft() {
            let tree = PartTree::from_craft(&craft, &catalog()).unwrap();
            assert_eq!(tree.parts().iter().filter(|p| p.joint.is_none()).count(), 1);
            for part in tree.parts() {
                let Some(joint) = part.joint else {
                    continue;
                };
                let parent = tree.get(joint.parent).unwrap();
                let (theirs, ours) = (
                    &parent.connectors[joint.parent_connector],
                    &part.connectors[joint.connector],
                );
                assert_eq!(theirs.kind, ours.kind, "{}: {}", craft.id, part.name);
                if ours.kind == ConnectorKind::Fixed {
                    let gap = parent.connector_position(joint.parent_connector)
                        - part.connector_position(joint.connector);
                    assert!(gap.length() < 1e-9, "{}: {} {gap}", craft.id, part.name);
                }
            }
        }
    }

    #[test]
    fn propellant_flows_within_a_stage_and_not_across_decouplers() {
        for craft in stock_craft() {
            let tree = PartTree::from_craft(&craft, &catalog()).unwrap();
            let groups = tree.fluid_groups();
            // One group per stage, holding its hull, tanks, engines and RCS.
            assert_eq!(groups.len(), craft.stages.len(), "{}", craft.id);
            for group in &groups {
                let stage = tree.get(group[0]).unwrap().stage;
                let plumbed: Vec<PartId> = tree
                    .stage(stage)
                    .filter(|p| {
                        matches!(
                            p.kind,
                            PartKind::Hull { .. }
                                | PartKind::Tank(_)
                                | PartKind::Engine(_)
                                | PartKind::Rcs(_)
                        )
                    })
                    .map(|p| p.id)
                    .collect();
                assert_eq!(*group, plumbed, "{} stage {stage}", craft.id);
            }
        }
    }

    #[test]
    fn the_parts_weigh_what_the_stages_do() {
        for craft in stock_craft() {
            let p = Propulsion::from_craft(&craft, &catalog()).unwrap();
            let stages: f64 = p.stages.iter().map(|s| s.dry_mass()).sum();
            let parts = p.parts().dry_mass().mass;
            assert!((parts - stages).abs() < 1e-9 * stages, "{}", craft.id);
        }
    }

    #[test]
    fn a_tank_can_sit_higher_up_its_stage() {
        let mut craft = crate::vessel::tests::comet();
        let tree = PartTree::from_craft(&craft, &catalog()).unwrap();
        let tank = named(&tree, "Stiffened Al 2219");
        assert_eq!(tank.position.z, 2.4);
        // But inside its stage, and clear of the tank before it.
        let bad = |craft: &CraftDef| {
            matches!(
                PartTree::from_craft(craft, &catalog()),
                Err(CraftError::BadStage { .. })
            )
        };
        for position in [-0.1, 5.0] {
            craft.stages[0].tanks[0].position = Some(position);
            assert!(bad(&craft), "{position}");
        }
        craft.stages[0].tanks[0].position = Some(1.0);
        let mut above = craft.stages[0].tanks[0].clone();
        above.length = 1.0;
        above.position = Some(3.0);
        craft.stages[0].tanks.push(above.clone());
        assert!(bad(&craft));
        craft.stages[0].tanks[1].position = Some(3.2);
        assert!(!bad(&craft));
    }

    #[test]
    fn releasing_a_decoupler_splits_the_tree() {
        let mut tree = PartTree::from_craft(&pathfinder(), &catalog()).unwrap();
        let everything = sorted(tree.parts().iter().map(|p| p.id).collect());
        let upper = named(&tree, "Upper Stage").id;
        let decoupler = tree.parent(upper).unwrap().id;
        // Only joints that can be released are: not an engine's, not the root's.
        let engine = named(&tree, "K-1 Vacuum").id;
        assert!(tree.release(engine).is_none());
        assert!(tree.release(tree.root().unwrap().id).is_none());
        assert!(tree.release(upper).is_none());

        let below = tree.release(decoupler).unwrap();
        assert_eq!(below.root().unwrap().id, decoupler);
        // The booster went along with the upper stage; the payload stayed.
        assert!(below.stage(0).next().is_some() && below.stage(1).next().is_some());
        assert!(tree.stage(0).next().is_none() && tree.stage(1).next().is_none());
        let mut both: Vec<PartId> = tree.parts().iter().map(|p| p.id).collect();
        both.extend(below.parts().iter().map(|p| p.id));
        assert_eq!(sorted(both), everything);
        for half in [&tree, &below] {
            assert_eq!(
                half.subtree(half.root().unwrap().id).len(),
                half.parts().len()
            );
        }
        // Released once, it is gone.
        assert!(tree.release(decoupler).is_none());
    }

    #[test]
    fn staging_takes_the_parts_with_the_stages() {
        let mut p = Propulsion::from_craft(&pathfinder(), &catalog()).unwrap();
        let everything = sorted(p.parts().parts().iter().map(|p| p.id).collect());
        let hull_stages = |tree: &PartTree| -> Vec<usize> {
            tree.parts()
                .iter()
                .filter(|p| matches!(p.kind, PartKind::Hull { .. }))
                .map(|p| p.stage)
                .collect()
        };
        let mut released = Vec::new();
        while let Some(staged) = p.activate_next_sequence() {
            for r in staged.released {
                let ids: Vec<usize> = r.stages.iter().map(|s| s.id).collect();
                if r.parts.root().is_some_and(|p| p.name == "Fairing") {
                    assert_eq!(r.parts.parts().len(), 1);
                } else {
                    assert_eq!(hull_stages(&r.parts), ids);
                }
                released.push(r.parts);
            }
            let ids: Vec<usize> = p.stages.iter().map(|s| s.id).collect();
            assert_eq!(hull_stages(p.parts()), ids);
        }
        // Booster, upper stage and both fairing halves came off; the payload is what is
        // left.
        assert_eq!(released.len(), 4);
        assert_eq!(p.parts().root().unwrap().name, "Payload");
        assert!(p.parts().parts().iter().all(|p| p.name != "Fairing"));
        let mut all: Vec<PartId> = p.parts().parts().iter().map(|p| p.id).collect();
        all.extend(released.iter().flat_map(|t| t.parts().iter().map(|p| p.id)));
        assert_eq!(sorted(all), everything);
    }
}
