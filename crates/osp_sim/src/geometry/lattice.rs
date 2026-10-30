//! Lattices: straight members between nodes, like a truss. Each member is measured by
//! length, so a lattice's mass follows the mass per metre of its members.

use glam::DVec3;

use super::moments::Moments;

#[derive(Debug, Clone)]
pub struct Lattice {
    pub name: String,
    nodes: Vec<DVec3>,
    members: Vec<[usize; 2]>,
    moments: Moments,
}

impl Lattice {
    pub fn new(name: String, nodes: Vec<DVec3>, members: Vec<[usize; 2]>) -> Result<Self, String> {
        if members.is_empty() {
            return Err(format!("`{name}`: a lattice needs members"));
        }
        for &[a, b] in &members {
            let (Some(p), Some(q)) = (nodes.get(a), nodes.get(b)) else {
                return Err(format!(
                    "`{name}`: a member joins nodes {a} and {b}, which are not all there"
                ));
            };
            if !(p.is_finite() && q.is_finite()) || p == q {
                return Err(format!(
                    "`{name}`: the member from node {a} to node {b} has no length"
                ));
            }
        }
        let moments = members
            .iter()
            .map(|&[a, b]| Moments::segment(nodes[a], nodes[b]))
            .sum();
        Ok(Self {
            name,
            nodes,
            members,
            moments,
        })
    }

    pub fn nodes(&self) -> &[DVec3] {
        &self.nodes
    }

    pub fn members(&self) -> &[[usize; 2]] {
        &self.members
    }

    /// All its members, by length.
    pub fn moments(&self) -> Moments {
        self.moments
    }
}
