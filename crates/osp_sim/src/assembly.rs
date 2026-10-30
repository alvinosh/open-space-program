//! How parts join into a vessel: their connectors, the joints between them and the tree the
//! joints make (see `docs/vessel-overhaul.md`).
//!
//! A connector is a node of the part's geometry (a place, a direction out of the part and a
//! size, see [`crate::geometry::Node`]) or one side of one of its surfaces. A joint joins
//! two connectors, at least one a node. Node to node, the two meet facing each other; a node
//! on a surface meets it at the point its surface coordinates give, standing off along its
//! normal and facing into it. Either way the second part can be turned about the line they
//! meet along (`roll`), and a hinged or sliding joint moves. A node holds one joint, a
//! surface any number.
//!
//! Joints have no direction. The assembly's root decides which way the tree runs: placing
//! parts starts there, rerooting changes no joint, and when a joint opens the side with the
//! root stays ([`Assembly::split`]). Parts are placed from the joints alone, so whatever
//! hangs on a part moves with it when the part is resized.
//!
//! Struts and fuel lines are not joints: they will be links alongside the tree.

use std::{
    collections::{BTreeMap, BTreeSet},
    ops::Mul,
    sync::Arc,
};

use glam::{DMat3, DQuat, DVec3};
use serde::{Deserialize, Serialize};

use crate::geometry::{Geometry, Node};

/// Where something is and how it is turned, in some frame.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Pose {
    pub position: DVec3,
    pub rotation: DQuat,
}

impl Pose {
    pub const IDENTITY: Self = Self {
        position: DVec3::ZERO,
        rotation: DQuat::IDENTITY,
    };

    pub fn new(position: DVec3, rotation: DQuat) -> Self {
        Self { position, rotation }
    }

    pub fn inverse(self) -> Self {
        let rotation = self.rotation.inverse();
        Self {
            position: -(rotation * self.position),
            rotation,
        }
    }

    /// A point of this frame in the outer one.
    pub fn point(self, p: DVec3) -> DVec3 {
        self.position + self.rotation * p
    }
}

impl Mul for Pose {
    type Output = Self;

    /// `other`, given in this frame, in the outer one.
    fn mul(self, other: Self) -> Self {
        Self {
            position: self.point(other.position),
            rotation: (self.rotation * other.rotation).normalize(),
        }
    }
}

/// Which side of a surface: the front faces out of a tank or shell.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Side {
    #[default]
    Front,
    Back,
}

/// What passes through a connector, and so through a joint whose connectors both pass it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Carries {
    pub fluid: bool,
    pub power: bool,
    pub data: bool,
}

impl Carries {
    pub const NOTHING: Self = Self {
        fluid: false,
        power: false,
        data: false,
    };
    /// What a node passes unless its part says otherwise: everything.
    pub const NODE: Self = Self {
        fluid: true,
        power: true,
        data: true,
    };
    /// What a surface passes unless its part says otherwise: power and data, not
    /// propellant.
    pub const SURFACE: Self = Self {
        fluid: false,
        power: true,
        data: true,
    };

    pub fn both(self, other: Self) -> Self {
        Self {
            fluid: self.fluid && other.fluid,
            power: self.power && other.power,
            data: self.data && other.data,
        }
    }
}

/// Where a joint holds a part.
#[derive(Debug, Clone, PartialEq)]
pub enum Mount {
    /// At one of its nodes.
    Node(String),
    /// On a surface, `u` and `v` (each 0 to 1) its coordinates there (see
    /// [`Geometry::surface_point`]), `standoff` metres out along its normal.
    Surface {
        surface: String,
        side: Side,
        u: f64,
        v: f64,
        standoff: f64,
    },
}

impl Mount {
    /// The node's or surface's name.
    pub fn name(&self) -> &str {
        match self {
            Mount::Node(name) => name,
            Mount::Surface { surface, .. } => surface,
        }
    }
}

/// One end of a joint: a part, by index, and where the joint holds it.
#[derive(Debug, Clone, PartialEq)]
pub struct End {
    pub part: usize,
    pub mount: Mount,
}

/// How a joint can move.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Motion {
    #[default]
    Rigid,
    /// Turns about the joint's X axis: a control surface, a fairing petal.
    Hinge,
    /// Slides along the line the parts meet along: a landing leg's strut.
    Slide,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Joint {
    pub ends: [End; 2],
    /// How far the second end is turned about the line they meet along, rad.
    pub roll: f64,
    pub motion: Motion,
    /// How far a hinge is turned (rad), or a slide drawn out (m).
    pub travel: f64,
}

impl Joint {
    pub fn new(a: End, b: End) -> Self {
        Self {
            ends: [a, b],
            roll: 0.0,
            motion: Motion::Rigid,
            travel: 0.0,
        }
    }

    /// How the second end's frame sits in the first's: turned by `roll`, moved by the
    /// joint's travel, and facing back.
    pub fn transform(&self) -> Pose {
        let travel = match self.motion {
            Motion::Rigid => Pose::IDENTITY,
            Motion::Hinge => Pose::new(DVec3::ZERO, DQuat::from_rotation_x(self.travel)),
            Motion::Slide => Pose::new(DVec3::Z * self.travel, DQuat::IDENTITY),
        };
        let face = Pose::new(DVec3::ZERO, DQuat::from_rotation_x(std::f64::consts::PI));
        Pose::new(DVec3::ZERO, DQuat::from_rotation_z(self.roll)) * travel * face
    }
}

/// The roll of a rigid joint that puts the part at its far end at `pose`, in the frame of
/// the part at its near end, given where each part is held (`near` and `far`, in their own
/// frames, from [`AssemblyPart::frame`]). `None` if the two cannot meet there.
pub fn roll_to(near: Pose, far: Pose, pose: Pose) -> Option<f64> {
    let across = near.inverse() * pose * far;
    let size = 1.0 + near.position.length() + far.position.length() + pose.position.length();
    let turn = across.rotation * DQuat::from_rotation_x(std::f64::consts::PI).inverse();
    (across.position.length() <= 1e-9 * size && turn.x.abs() < 1e-9 && turn.y.abs() < 1e-9)
        .then(|| 2.0 * turn.z.atan2(turn.w))
}

/// A part as the assembly sees it: a shape and where it can be joined.
#[derive(Debug, Clone)]
pub struct AssemblyPart {
    /// Unique within the vessel; kept when the vessel splits.
    pub id: String,
    pub geometry: Arc<Geometry>,
    /// Nodes besides its geometry's own: a template's, or one added in the editor.
    pub extra_nodes: Vec<Node>,
    /// What its connectors pass, by node or surface name, where that differs from
    /// [`Carries::NODE`] and [`Carries::SURFACE`].
    pub carries: BTreeMap<String, Carries>,
    /// Connectors whose joints can be let go in flight: a decoupler's.
    pub releases: BTreeSet<String>,
    /// Things can be mounted on the back of its surfaces, inside it: it is a container.
    pub inside: bool,
}

impl AssemblyPart {
    pub fn new(id: impl Into<String>, geometry: Arc<Geometry>) -> Self {
        Self {
            id: id.into(),
            geometry,
            extra_nodes: Vec::new(),
            carries: BTreeMap::new(),
            releases: BTreeSet::new(),
            inside: false,
        }
    }

    /// Its geometry's nodes and its extra ones.
    pub fn nodes(&self) -> impl Iterator<Item = &Node> {
        self.geometry.nodes.iter().chain(&self.extra_nodes)
    }

    pub fn node(&self, name: &str) -> Option<&Node> {
        self.nodes().find(|n| n.name == name)
    }

    /// What `mount` passes.
    pub fn carries(&self, mount: &Mount) -> Carries {
        let usual = match mount {
            Mount::Node(_) => Carries::NODE,
            Mount::Surface { .. } => Carries::SURFACE,
        };
        self.carries.get(mount.name()).copied().unwrap_or(usual)
    }

    /// The frame of `mount` in the part's: its origin where the joint holds the part and
    /// its +Z out of the part. Around that axis +X follows the part's +X (or +Y, for a node
    /// along X) on a node, and the surface's u direction on a surface.
    pub fn frame(&self, mount: &Mount) -> Option<Pose> {
        match mount {
            Mount::Node(name) => {
                let node = self.node(name)?;
                Some(Pose::new(node.position, facing(node.direction, &[])))
            }
            Mount::Surface {
                surface,
                side,
                u,
                v,
                standoff,
            } => {
                let p = self.geometry.surface_point(surface, *u, *v)?;
                let normal = match side {
                    Side::Front => p.normal,
                    Side::Back => -p.normal,
                };
                if !normal.is_normalized() {
                    return None;
                }
                let rotation = facing(normal, &[p.along_u, p.along_v]);
                Some(Pose::new(p.position + normal * *standoff, rotation))
            }
        }
    }
}

/// A rotation whose +Z is `z` (unit) and whose +X is the first of `along`, then +X, then
/// +Y, that does not lie close to `z`, made square to it.
fn facing(z: DVec3, along: &[DVec3]) -> DQuat {
    let x = along
        .iter()
        .chain(&[DVec3::X, DVec3::Y])
        .find_map(|&d| {
            let square = d - z * z.dot(d);
            (square.length() > 0.1 * d.length()).then(|| square.normalize())
        })
        .expect("+X and +Y cannot both lie along one direction");
    DQuat::from_mat3(&DMat3::from_cols(x, z.cross(x), z))
}

/// Parts joined into a tree, from a root.
#[derive(Debug, Clone)]
pub struct Assembly {
    parts: Vec<AssemblyPart>,
    joints: Vec<Joint>,
    root: usize,
    /// Each joint's ends' frames, in their parts' frames.
    frames: Vec<[Pose; 2]>,
}

impl Assembly {
    pub fn new(parts: Vec<AssemblyPart>, joints: Vec<Joint>, root: usize) -> Result<Self, String> {
        if root >= parts.len() {
            return Err("an assembly needs its root among its parts".into());
        }
        let mut ids = BTreeSet::new();
        for part in &parts {
            if part.id.is_empty() || !ids.insert(part.id.as_str()) {
                return Err(format!("part ids must be given and unique: `{}`", part.id));
            }
            let mut names = BTreeSet::new();
            if let Some(n) = part.nodes().find(|n| !names.insert(n.name.as_str())) {
                return Err(format!("`{}` has two nodes named `{}`", part.id, n.name));
            }
        }
        let mut held = BTreeSet::new();
        let mut frames = Vec::with_capacity(joints.len());
        for joint in &joints {
            let [a, b] = &joint.ends;
            let id = |end: &End| parts.get(end.part).map_or("?", |p| p.id.as_str());
            let what = format!("the joint between `{}` and `{}`", id(a), id(b));
            if a.part >= parts.len() || b.part >= parts.len() || a.part == b.part {
                return Err(format!("{what} must join two parts of the assembly"));
            }
            if !matches!(a.mount, Mount::Node(_)) && !matches!(b.mount, Mount::Node(_)) {
                return Err(format!("{what} joins two surfaces: one end must be a node"));
            }
            if !(joint.roll.is_finite() && joint.travel.is_finite()) {
                return Err(format!("{what} has a roll or travel that is not a number"));
            }
            let mut pair = [Pose::IDENTITY; 2];
            for (k, end) in joint.ends.iter().enumerate() {
                let part = &parts[end.part];
                if let Mount::Surface { u, v, standoff, .. } = &end.mount
                    && !((0.0..=1.0).contains(u) && (0.0..=1.0).contains(v) && standoff.is_finite())
                {
                    return Err(format!("{what}: surface coordinates run from 0 to 1"));
                }
                if let Mount::Surface {
                    side: Side::Back, ..
                } = &end.mount
                    && !part.inside
                {
                    return Err(format!(
                        "{what}: `{}` is not a container, so nothing mounts inside it",
                        part.id
                    ));
                }
                if let Mount::Node(name) = &end.mount
                    && !held.insert((end.part, name.clone()))
                {
                    return Err(format!(
                        "node `{name}` of `{}` holds two joints; a node holds one",
                        part.id
                    ));
                }
                pair[k] = part.frame(&end.mount).ok_or_else(|| {
                    format!(
                        "{what}: `{}` has no `{}` to join at",
                        part.id,
                        end.mount.name()
                    )
                })?;
            }
            frames.push(pair);
        }
        let assembly = Self {
            parts,
            joints,
            root,
            frames,
        };
        let reached = assembly.order();
        if let Some(stray) = (0..assembly.parts.len()).find(|&p| !reached.contains(&p)) {
            return Err(format!(
                "`{}` is not joined to the rest",
                assembly.parts[stray].id
            ));
        }
        if assembly.joints.len() != assembly.parts.len() - 1 {
            return Err(
                "the joints close a loop: a vessel's joints make a tree (struts and fuel lines \
                 will be links)"
                    .into(),
            );
        }
        Ok(assembly)
    }

    pub fn parts(&self) -> &[AssemblyPart] {
        &self.parts
    }

    pub fn joints(&self) -> &[Joint] {
        &self.joints
    }

    pub fn root(&self) -> usize {
        self.root
    }

    /// The index of the part with this id.
    pub fn find(&self, id: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.id == id)
    }

    /// Make `part` the root. No joint changes.
    pub fn reroot(&mut self, part: usize) {
        assert!(
            part < self.parts.len(),
            "rerooting at a part not in the assembly"
        );
        self.root = part;
    }

    /// Set how far a hinge is turned or a slide drawn out.
    pub fn set_travel(&mut self, joint: usize, travel: f64) {
        self.joints[joint].travel = travel;
    }

    /// The joints of each part.
    fn neighbours(&self) -> Vec<Vec<usize>> {
        let mut out = vec![Vec::new(); self.parts.len()];
        for (j, joint) in self.joints.iter().enumerate() {
            for end in &joint.ends {
                out[end.part].push(j);
            }
        }
        out
    }

    /// The parts reachable from the root, root first, each after the part it hangs from.
    pub fn order(&self) -> Vec<usize> {
        let neighbours = self.neighbours();
        let mut seen = vec![false; self.parts.len()];
        let mut order = vec![self.root];
        seen[self.root] = true;
        let mut next = 0;
        while let Some(&part) = order.get(next) {
            for &j in &neighbours[part] {
                let other = self.other_end(j, part);
                if !seen[other] {
                    seen[other] = true;
                    order.push(other);
                }
            }
            next += 1;
        }
        order
    }

    /// The part at the far end of joint `j` from `part`.
    fn other_end(&self, j: usize, part: usize) -> usize {
        let [a, b] = &self.joints[j].ends;
        if a.part == part { b.part } else { a.part }
    }

    /// For each part, the joint it hangs from and the part at that joint's other end, towards
    /// the root; `None` for the root.
    pub fn parents(&self) -> Vec<Option<(usize, usize)>> {
        let neighbours = self.neighbours();
        let mut parents = vec![None; self.parts.len()];
        for part in self.order() {
            for &j in &neighbours[part] {
                let child = self.other_end(j, part);
                if child != self.root && parents[child].is_none() {
                    parents[child] = Some((j, part));
                }
            }
        }
        parents
    }

    /// Every part's pose in the root's frame.
    pub fn poses(&self) -> Vec<Pose> {
        let mut poses = vec![Pose::IDENTITY; self.parts.len()];
        let parents = self.parents();
        for part in self.order().into_iter().skip(1) {
            let (j, parent) = parents[part].expect("every part but the root hangs from one");
            let joint = &self.joints[j];
            let [near, far] = if joint.ends[0].part == parent {
                [0, 1]
            } else {
                [1, 0]
            };
            let across = match near {
                0 => joint.transform(),
                _ => joint.transform().inverse(),
            };
            let frames = &self.frames[j];
            poses[part] = poses[parent] * frames[near] * across * frames[far].inverse();
        }
        poses
    }

    /// Where joint `j` holds each of its two ends, in that end's part's frame (see
    /// [`AssemblyPart::frame`]).
    pub fn frames(&self, j: usize) -> [Pose; 2] {
        self.frames[j]
    }

    /// What joint `j` passes: what both its connectors do.
    pub fn carries(&self, j: usize) -> Carries {
        let [a, b] = &self.joints[j].ends;
        self.parts[a.part]
            .carries(&a.mount)
            .both(self.parts[b.part].carries(&b.mount))
    }

    /// Whether joint `j` can be let go in flight: one of its connectors releases.
    pub fn releasable(&self, j: usize) -> bool {
        self.joints[j]
            .ends
            .iter()
            .any(|e| self.parts[e.part].releases.contains(e.mount.name()))
    }

    /// Let go of joint `j` if it can be: [`Self::split`].
    pub fn release(&mut self, j: usize) -> Option<Assembly> {
        self.releasable(j).then(|| self.split(j))
    }

    /// Open joint `j`, whatever held it (it tore): the side with the root stays, and the
    /// other side leaves as an assembly of its own, rooted where the joint held it.
    pub fn split(&mut self, j: usize) -> Assembly {
        let opened = self.joints.remove(j);
        self.frames.remove(j);
        let kept: BTreeSet<usize> = self.order().into_iter().collect();
        let far = if kept.contains(&opened.ends[0].part) {
            opened.ends[1].part
        } else {
            opened.ends[0].part
        };
        let (mut stay, mut leave) = (Vec::new(), Vec::new());
        let mut index = vec![0; self.parts.len()];
        for (i, part) in std::mem::take(&mut self.parts).into_iter().enumerate() {
            let side = if kept.contains(&i) {
                &mut stay
            } else {
                &mut leave
            };
            index[i] = side.len();
            side.push(part);
        }
        let (mut stay_joints, mut leave_joints) = (Vec::new(), Vec::new());
        let joints = std::mem::take(&mut self.joints);
        let frames = std::mem::take(&mut self.frames);
        for (mut joint, pair) in joints.into_iter().zip(frames) {
            let staying = kept.contains(&joint.ends[0].part);
            for end in &mut joint.ends {
                end.part = index[end.part];
            }
            let side = if staying {
                &mut stay_joints
            } else {
                &mut leave_joints
            };
            side.push((joint, pair));
        }
        let unzip = |pairs: Vec<(Joint, [Pose; 2])>| pairs.into_iter().unzip();
        (self.joints, self.frames) = unzip(stay_joints);
        self.parts = stay;
        self.root = index[self.root];
        let (joints, frames) = unzip(leave_joints);
        Assembly {
            parts: leave,
            joints,
            root: index[far],
            frames,
        }
    }
}

#[cfg(test)]
mod tests;
