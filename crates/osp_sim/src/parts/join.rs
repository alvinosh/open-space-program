//! What a join does to a part tree: nodes that are free to join at, re-rooting, moving parts
//! into another frame and hanging one tree from another. The rest of a join is in the propulsion
//! and the vessel.

use glam::DVec3;

use super::{Capabilities, Connector, ConnectorKind, Joint, JointWay, Part, PartId, PartTree};
use crate::{
    assembly::{Pose, node_frame},
    component::Component,
    propulsion::Port,
};

impl Part {
    /// Where it stands in the frame it was built in.
    pub(crate) fn as_built(&self) -> Pose {
        if self.frame == Pose::IDENTITY {
            Pose::new(self.position, self.rotation)
        } else {
            self.home
        }
    }

    /// Whether the part's connector `name` holds a joint.
    pub(crate) fn holds_joint(&self, tree: &PartTree, name: &str) -> bool {
        (self.connectors.iter().enumerate())
            .filter(|(_, c)| c.name == name)
            .any(|(k, _)| {
                self.joint.is_some_and(|j| j.connector == k)
                    || (tree.children(self.id)).any(|child| {
                        child
                            .joint
                            .is_some_and(|j| j.parent == self.id && j.parent_connector == k)
                    })
            })
    }
}

impl PartTree {
    /// The next part id and stage no part has.
    pub(crate) fn next_ids(&self) -> (u32, usize) {
        let part = self.parts.last().map_or(0, |p| p.id.0 + 1);
        let stage = self.parts.iter().map(|p| p.stage + 1).max().unwrap_or(0);
        (part, stage)
    }

    /// The frame section `stage` was built in, in the assembly frame.
    pub(crate) fn stage_frame(&self, stage: usize) -> Pose {
        self.stage(stage).next().map_or(Pose::IDENTITY, |p| p.frame)
    }

    /// The parts `id` hangs from as its craft was built: its parent unless that joint was turned
    /// round or made in flight, and the parts on it whose joint was turned round.
    pub(crate) fn holders(&self, id: PartId) -> impl Iterator<Item = &Part> {
        let parent = (self.get(id))
            .filter(|p| p.joint.is_some_and(|j| j.way == JointWay::AsBuilt))
            .and_then(|_| self.parent(id));
        let turned = if self.turned { &self.parts[..] } else { &[] };
        let children = turned.iter().filter(move |p| {
            p.joint
                .is_some_and(|j| j.parent == id && j.way == JointWay::Turned)
        });
        parent.into_iter().chain(children)
    }

    /// The parts hanging from `id` as its craft was built: those on it, but for the ones whose
    /// joint was turned round or made in flight, and its parent if its own joint was turned round.
    pub(crate) fn held(&self, id: PartId) -> impl Iterator<Item = &Part> {
        let parent = (self.get(id))
            .filter(|p| p.joint.is_some_and(|j| j.way == JointWay::Turned))
            .and_then(|_| self.parent(id));
        let children =
            (self.children(id)).filter(|p| p.joint.is_some_and(|j| j.way == JointWay::AsBuilt));
        parent.into_iter().chain(children)
    }

    /// The nodes that hold no joint, part by part: where something could join.
    pub fn free_nodes(&self) -> Vec<(PartId, String)> {
        let mut out = Vec::new();
        for part in &self.parts {
            let Some(built) = &part.built else { continue };
            for node in built.nodes.iter().chain(&built.geometry.nodes) {
                if !part.holds_joint(self, &node.name) {
                    out.push((part.id, node.name.clone()));
                }
            }
        }
        out
    }

    /// Where `port` is (its frame in the part's and the part's pose), and a connector for it that
    /// passes what the node passes and lets go. An error if there is no such node or it holds a
    /// joint.
    pub(crate) fn free_port(&self, port: Port) -> Result<(Pose, Pose, Connector), String> {
        let part = (self.get(port.part)).ok_or_else(|| format!("no part {}", port.part.0))?;
        let built = (part.built.as_ref())
            .ok_or_else(|| format!("`{}` was not built as a part with nodes", part.name))?;
        let node = (built.nodes.iter())
            .chain(&built.geometry.nodes)
            .find(|n| n.name == port.node)
            .ok_or_else(|| format!("`{}` has no node `{}`", part.name, port.node))?;
        if part.holds_joint(self, port.node) {
            return Err(format!(
                "node `{}` of `{}` holds a joint already",
                port.node, part.name
            ));
        }
        // As a craft's own joints: only a decoupler or a docking port says otherwise, and it may
        // not pass propellant.
        let crossfeed = built.components.iter().find_map(|c| match c {
            Component::Decoupler(d) if d.node == port.node => Some(d.crossfeed),
            Component::DockingPort(d) if d.node == port.node => Some(d.crossfeed),
            _ => None,
        });
        let connector = Connector {
            name: node.name.clone(),
            kind: ConnectorKind::Fixed,
            position: node.position,
            capabilities: Capabilities {
                fluid: crossfeed.unwrap_or(true),
                power: true,
                release: true,
            },
        };
        let pose = Pose::new(part.position, part.rotation);
        Ok((node_frame(node), pose, connector))
    }

    /// Make `id` the root: the joints between it and the old root turn round, and the rest stay
    /// as they are. Returns the joints that turned, each as the part that named it (the one
    /// hanging from it) and the part that names it now.
    pub(crate) fn reroot(&mut self, id: PartId) -> Vec<(PartId, PartId)> {
        let mut path = vec![id];
        while let Some(joint) = self.get(path[path.len() - 1]).and_then(|p| p.joint) {
            path.push(joint.parent);
        }
        let joints: Vec<Option<Joint>> = path
            .iter()
            .map(|&p| self.get(p).and_then(|p| p.joint))
            .collect();
        self.part_mut(id).joint = None;
        let mut renamed = Vec::new();
        for (w, joint) in path.windows(2).zip(joints) {
            let (child, parent) = (w[0], w[1]);
            let joint = joint.expect("every part on the way up hangs from the next");
            self.part_mut(parent).joint = Some(Joint {
                parent: child,
                parent_connector: joint.connector,
                connector: joint.parent_connector,
                way: match joint.way {
                    JointWay::AsBuilt => JointWay::Turned,
                    JointWay::Turned => JointWay::AsBuilt,
                    JointWay::Join => JointWay::Join,
                },
            });
            renamed.push((child, parent));
        }
        self.turned =
            (self.parts.iter()).any(|p| p.joint.is_some_and(|j| j.way == JointWay::Turned));
        renamed
    }

    /// Call every part and stage what these say. They keep their order only if `part` does,
    /// so they are put back in order.
    pub(crate) fn renumber(
        &mut self,
        part: impl Fn(PartId) -> PartId,
        stage: impl Fn(usize) -> usize,
    ) {
        for p in &mut self.parts {
            p.id = part(p.id);
            p.stage = stage(p.stage);
            if let Some(joint) = &mut p.joint {
                joint.parent = part(joint.parent);
            }
        }
        self.parts.sort_by_key(|p| p.id);
    }

    /// Move every part by `pose`, which then is part of how the frame it was built in sits here.
    pub(crate) fn move_parts(&mut self, pose: Pose) {
        for p in &mut self.parts {
            if p.frame == Pose::IDENTITY {
                p.home = Pose::new(p.position, p.rotation);
            }
            p.position = pose.point(p.position);
            p.rotation = (pose.rotation * p.rotation).normalize();
            p.frame = pose * p.frame;
        }
    }

    /// Move every part back by the frame `home`: those that were in it are exactly where they
    /// stood in the frame they were built in, whatever rounding the journey did.
    pub(crate) fn move_home(&mut self, home: Pose) {
        let back = home.inverse();
        for p in &mut self.parts {
            if p.frame == home {
                (p.position, p.rotation) = (p.home.position, p.home.rotation);
                p.frame = Pose::IDENTITY;
            } else {
                p.position = back.point(p.position);
                p.rotation = (back.rotation * p.rotation).normalize();
                p.frame = back * p.frame;
            }
        }
    }

    /// Hang `other`, whose root is `to`, from this tree's part `at`, by these new connectors.
    pub(crate) fn graft(
        &mut self,
        mut other: PartTree,
        at: PartId,
        mine: Connector,
        to: PartId,
        theirs: Connector,
    ) {
        let parent_connector = self.part_mut(at).add_connector(mine);
        let connector = other.part_mut(to).add_connector(theirs);
        other.part_mut(to).joint = Some(Joint {
            parent: at,
            parent_connector,
            connector,
            way: JointWay::Join,
        });
        self.turned |= other.turned;
        self.parts.append(&mut other.parts);
        self.parts.sort_by_key(|p| p.id);
    }

    /// Take a connector no joint holds off `id`.
    pub(super) fn forget_connector(&mut self, id: PartId, index: usize) {
        self.part_mut(id).connectors.remove(index);
        for part in &mut self.parts {
            let Some(joint) = &mut part.joint else {
                continue;
            };
            if part.id == id && joint.connector > index {
                joint.connector -= 1;
            }
            if joint.parent == id && joint.parent_connector > index {
                joint.parent_connector -= 1;
            }
        }
    }
}

impl Part {
    fn add_connector(&mut self, connector: Connector) -> usize {
        self.connectors.push(connector);
        self.connectors.len() - 1
    }
}

/// The way a node with frame `node`, on a part at `pose`, faces out of the part.
pub(crate) fn node_out(pose: Pose, node: Pose) -> DVec3 {
    pose.rotation * (node.rotation * DVec3::Z)
}
