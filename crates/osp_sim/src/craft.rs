//! Craft: plain parts joined into a vessel (see `docs/vessel-overhaul.md`).
//!
//! A [`CraftDef`] lists parts, each a catalogue record (`template`) or a shape with
//! components, and each but the root held towards the root at one of another part's nodes
//! or surfaces (`on`). Loading one ([`Craft::build`]) builds every part
//! ([`crate::component`]) and joins them into an [`Assembly`], which places them.
//!
//! Until the physics runs on parts, the stages it runs on are gathered from a craft by
//! [`crate::parts::PartTree::from_craft`].

use std::{
    collections::{BTreeMap, BTreeSet},
    sync::Arc,
};

use glam::DVec3;

use crate::{
    assembly::{Assembly, AssemblyPart, End, Joint, Mount},
    component::{BuiltPart, Component, PartDef},
    data::{
        AssemblyDef, CraftDef, CraftPartDef, EngineDef, PartCatalog, PodDef, RcsDef,
        ReactionWheelDef, ShellProfile, SurfaceAtDef,
    },
    geometry::preset::{NodeDef, ShapeDef, ShellSection},
    propulsion::CraftError,
};

/// How far a fitted fairing keeps clear of what it holds, m.
const FAIRING_CLEARANCE: f64 = 0.1;
/// How long a fitted fairing's nose is, in diameters of its base.
const FAIRING_NOSE: f64 = 1.3;

/// A craft, built.
#[derive(Debug, Clone)]
pub struct Craft {
    pub id: String,
    pub name: String,
    /// In the order the craft lists them, which is the assembly's.
    pub parts: Vec<CraftPart>,
    pub assembly: Assembly,
}

/// One part of a craft, built.
#[derive(Debug, Clone)]
pub struct CraftPart {
    pub id: String,
    pub name: String,
    /// The catalogue record it was made from.
    pub template: Option<Template>,
    /// What it was built from: its template's shape and components and its own.
    pub def: PartDef,
    pub built: Arc<BuiltPart>,
}

/// A catalogue record a part was made from.
#[derive(Debug, Clone)]
pub enum Template {
    Engine(Box<EngineDef>),
    Pod(PodDef),
    Rcs(RcsDef),
    ReactionWheel(ReactionWheelDef),
}

impl Craft {
    /// Build `def` from `catalog`'s records, or say what is wrong with it.
    pub fn build(def: &CraftDef, catalog: &PartCatalog) -> Result<Self, CraftError> {
        let bad = |problem: String| CraftError::BadCraft(format!("{}: {problem}", def.id));
        let mut ids = BTreeSet::new();
        for part in &def.parts {
            if part.id.is_empty() || part.id.contains('.') || !ids.insert(part.id.as_str()) {
                return Err(bad(format!(
                    "part ids must be given, unique and without dots: `{}`",
                    part.id
                )));
            }
        }
        let mut parts = Vec::new();
        let mut joined = Vec::new();
        for part in &def.parts {
            let built = build_part(part, catalog)?;
            joined.push(built.built.assembly_part(part.id.clone()));
            parts.push(built);
        }
        let mut root = None;
        let mut joints = Vec::new();
        for (index, part) in def.parts.iter().enumerate() {
            let Some(on) = &part.on else {
                if let Some(other) = root.replace(index) {
                    return Err(bad(format!(
                        "`{}` and `{}` are both on nothing: only the root is",
                        def.parts[other].id, part.id
                    )));
                }
                continue;
            };
            let (holder, connector) = on.split_once('.').ok_or_else(|| {
                bad(format!(
                    "`{}` is on `{on}`: write `<part id>.<connector>`",
                    part.id
                ))
            })?;
            let parent = def
                .parts
                .iter()
                .position(|p| p.id == holder)
                .ok_or_else(|| {
                    bad(format!(
                        "`{}` is on `{holder}`, which is not a part",
                        part.id
                    ))
                })?;
            let near = mount(&joined[parent], connector, part)?;
            let far = mount(&joined[index], &part.by, part)?;
            let mut joint = Joint::new(
                End {
                    part: parent,
                    mount: near,
                },
                End {
                    part: index,
                    mount: far,
                },
            );
            joint.roll = part.roll.to_radians();
            joints.push(joint);
        }
        let root =
            root.ok_or_else(|| bad("every part is on another: one must be the root".into()))?;
        let assembly = Assembly::new(joined, joints, root).map_err(bad)?;
        Ok(Self {
            id: def.id.clone(),
            name: if def.name.is_empty() {
                def.id.clone()
            } else {
                def.name.clone()
            },
            parts,
            assembly,
        })
    }

    /// The index of the part with this id.
    pub fn find(&self, id: &str) -> Option<usize> {
        self.parts.iter().position(|p| p.id == id)
    }
}

impl CraftDef {
    /// The fairing tool: fit each fairing to what stands above its base inside it. A
    /// fairing is the panels held on one part, each a shell over part of a turn whose
    /// `bottom` lets go; fitted, they are a barrel as wide as their base and as tall as
    /// needed under a tangent ogive nose [`FAIRING_NOSE`] diameters long, clear of
    /// everything by [`FAIRING_CLEARANCE`] (what reaches out to the rim it stands on, the
    /// barrel need only rise over). Returns the panels refitted; refuses a fairing too
    /// narrow for what it holds.
    pub fn fit_fairings(&mut self, catalog: &PartCatalog) -> Result<Vec<String>, CraftError> {
        let craft = Craft::build(self, catalog)?;
        let poses = craft.assembly.poses();
        let mut fairings: BTreeMap<&str, Vec<usize>> = BTreeMap::new();
        for (i, part) in self.parts.iter().enumerate() {
            let (Some(ShapeDef::Shell(shell)), Some(on)) = (&part.shape, &part.on) else {
                continue;
            };
            let partial = shell
                .span
                .is_some_and(|[a, b]| (b - a).abs() < 360.0 - 1e-9);
            let lets_go = (craft.parts[i].built.components.iter())
                .any(|c| matches!(c, Component::Decoupler(d) if d.node == "bottom"));
            if partial && lets_go {
                let holder = on.split_once('.').map_or(on.as_str(), |(p, _)| p);
                fairings.entry(holder).or_default().push(i);
            }
        }
        let mut fitted = Vec::new();
        let mut refits = Vec::new();
        for (holder, panels) in fairings {
            let Some(ShapeDef::Shell(shell)) = &self.parts[panels[0]].shape else {
                unreachable!("panels are shells");
            };
            // What stands in the fairing, in its frame: its axis +Z from the middle of its
            // base.
            let into = poses[panels[0]].inverse();
            let mut triangles = Vec::new();
            for (j, part) in craft.parts.iter().enumerate() {
                if !panels.contains(&j) {
                    let pose = into * poses[j];
                    crate::aero::outer_triangles(&part.built, |p| pose.point(p), &mut triangles);
                }
            }
            let radius = shell.bottom_diameter / 2.0;
            let sections = fit_fairing(&triangles, radius).ok_or_else(|| {
                CraftError::BadCraft(format!(
                    "{}: the fairing on `{holder}` is too narrow for what it holds",
                    self.id
                ))
            })?;
            refits.push((panels, sections));
        }
        for (panels, sections) in refits {
            for i in panels {
                if let Some(ShapeDef::Shell(shell)) = &mut self.parts[i].shape {
                    shell.sections = sections.clone();
                    fitted.push(self.parts[i].id.clone());
                }
            }
        }
        Ok(fitted)
    }

    /// Add copies of `assembly`'s parts, their ids prefixed with `prefix`, held at the
    /// assembly's free connector `on` one of this craft's (`"<part id>.<connector>"`), `at`
    /// and `roll` as a part's. The joints from the free connector back to the assembly's
    /// root are written the other way round, so that every part is on one nearer the
    /// craft's root; joints have no direction, so the parts sit as they did.
    pub fn place(
        &mut self,
        assembly: &AssemblyDef,
        prefix: &str,
        on: &str,
        at: Option<SurfaceAtDef>,
        roll: f64,
    ) -> Result<(), CraftError> {
        let bad = |problem: String| CraftError::BadCraft(format!("{}: {problem}", self.id));
        let renamed = |id: &str| format!("{prefix}{id}");
        let mut parts: Vec<CraftPartDef> = assembly
            .parts
            .iter()
            .map(|p| {
                let mut copy = p.clone();
                copy.id = renamed(&p.id);
                copy.on = p.on.as_ref().map(|on| match on.split_once('.') {
                    Some((part, connector)) => format!("{}.{connector}", renamed(part)),
                    None => on.clone(),
                });
                copy
            })
            .collect();
        let (free, connector) = assembly
            .free
            .split_once('.')
            .ok_or_else(|| bad(format!("assembly `{}` has no free connector", assembly.id)))?;
        let mut current = renamed(free);
        let mut hold = (Some(on.to_string()), connector.to_string(), at, roll);
        loop {
            let part = parts
                .iter_mut()
                .find(|p| p.id == current)
                .ok_or_else(|| bad(format!("assembly `{}` has no part `{free}`", assembly.id)))?;
            let was = (part.on.take(), part.by.clone(), part.at, part.roll);
            (part.on, part.by, part.at, part.roll) = hold;
            let Some(held) = was.0 else {
                break;
            };
            let (parent, at_parent) = held
                .split_once('.')
                .ok_or_else(|| bad(format!("`{held}` names no connector")))?;
            hold = (
                Some(format!("{current}.{}", was.1)),
                at_parent.to_string(),
                was.2,
                was.3,
            );
            current = parent.to_string();
        }
        if let Some(clash) = parts
            .iter()
            .find(|p| self.parts.iter().any(|q| q.id == p.id))
        {
            return Err(bad(format!("it already has a part `{}`", clash.id)));
        }
        self.parts.extend(parts);
        Ok(())
    }
}

/// The sections of a fairing `radius` wide at its base that holds `enclosed` (triangles in
/// its frame, its base at z = 0): a barrel under a tangent ogive (see
/// [`CraftDef::fit_fairings`]). `None` if something above the base reaches out past it.
pub fn fit_fairing(enclosed: &[[DVec3; 3]], radius: f64) -> Option<Vec<ShellSection>> {
    let nose = FAIRING_NOSE * 2.0 * radius;
    // The tangent ogive's arc: its radius, and how high above the barrel it is `r` out.
    let arc = (radius * radius + nose * nose) / (2.0 * radius);
    let height_at = |r: f64| {
        let x = r + arc - radius;
        (arc * arc - x * x).max(0.0).sqrt()
    };
    let mut barrel: f64 = 0.0;
    for t in enclosed {
        let top = t.iter().map(|v| v.z).fold(f64::NEG_INFINITY, f64::max);
        // Below the base (meshes are single precision).
        if top <= 1e-3 {
            continue;
        }
        // Out at the rim (what the panels stand on) it only has to be under the barrel.
        let reach = t.iter().map(|v| v.truncate().length()).fold(0.0, f64::max);
        if reach > radius + 1e-3 {
            return None;
        }
        let need = (reach + FAIRING_CLEARANCE).min(radius);
        barrel = barrel.max(top + FAIRING_CLEARANCE - height_at(need));
    }
    let r6 = |x: f64| (x * 1e6).round() / 1e6;
    Some(vec![
        ShellSection {
            length: r6(barrel.max(0.2 * radius)),
            top_diameter: None,
            profile: ShellProfile::Conic,
        },
        ShellSection {
            length: r6(nose),
            top_diameter: Some(0.0),
            profile: ShellProfile::Ogive,
        },
    ])
}

/// `part` built from its template or its shape.
fn build_part(part: &CraftPartDef, catalog: &PartCatalog) -> Result<CraftPart, CraftError> {
    let bad = |problem: &str| CraftError::BadPart {
        part: part.id.clone(),
        problem: problem.into(),
    };
    let (template, mut def) = match (&part.template, &part.shape) {
        (Some(id), None) => {
            if part.structure != Default::default() {
                return Err(bad("a template's record says what it is made of"));
            }
            let template = template(id, catalog)?;
            let def = match &template {
                Template::Engine(e) => PartDef::engine(e)?,
                Template::Pod(p) => PartDef::pod(p),
                Template::Rcs(r) => PartDef::rcs(r),
                Template::ReactionWheel(w) => PartDef::reaction_wheel(w),
            };
            (Some(template), def)
        }
        (None, Some(shape)) => (
            None,
            PartDef {
                name: String::new(),
                shape: shape.clone(),
                structure: part.structure.clone(),
                components: Vec::new(),
                nodes: Vec::new(),
                paint: part.paint.clone(),
            },
        ),
        _ => return Err(bad("a part has a `template` or a `shape`")),
    };
    def.name = part
        .name
        .clone()
        .unwrap_or_else(|| part.template.clone().unwrap_or_else(|| part.id.clone()));
    def.components.extend(part.components.iter().cloned());
    def.nodes.extend(part.nodes.iter().cloned());
    let built = def.build(catalog)?;
    Ok(CraftPart {
        id: part.id.clone(),
        name: def.name.clone(),
        template,
        def,
        built: Arc::new(built),
    })
}

/// The catalogue record `id` names: an engine, pod, RCS block or reaction wheel.
fn template(id: &str, catalog: &PartCatalog) -> Result<Template, CraftError> {
    let mut found = Vec::new();
    found.extend(
        catalog
            .engines
            .iter()
            .filter(|r| r.id == id)
            .map(|r| Template::Engine(Box::new(r.clone()))),
    );
    found.extend(
        catalog
            .pods
            .iter()
            .filter(|r| r.id == id)
            .cloned()
            .map(Template::Pod),
    );
    found.extend(
        catalog
            .rcs
            .iter()
            .filter(|r| r.id == id)
            .cloned()
            .map(Template::Rcs),
    );
    found.extend(
        catalog
            .reaction_wheels
            .iter()
            .filter(|r| r.id == id)
            .cloned()
            .map(Template::ReactionWheel),
    );
    match found.len() {
        0 => Err(CraftError::UnknownTemplate(id.into())),
        1 => Ok(found.remove(0)),
        _ => Err(CraftError::BadCraft(format!(
            "`{id}` names records in more than one collection"
        ))),
    }
}

/// Where `part`'s joint holds `holder`: its node `name`, or its surface `name` at `part`'s
/// `at`.
fn mount(holder: &AssemblyPart, name: &str, part: &CraftPartDef) -> Result<Mount, CraftError> {
    let bad = |problem: String| CraftError::BadPart {
        part: part.id.clone(),
        problem,
    };
    if holder.node(name).is_some() {
        return Ok(Mount::Node(name.into()));
    }
    if holder.geometry.surfaces().iter().any(|s| s == name) {
        let at = part.at.ok_or_else(|| {
            bad(format!(
                "it is held on surface `{name}`: say where with `at`"
            ))
        })?;
        return Ok(Mount::Surface {
            surface: name.into(),
            side: at.side,
            u: at.u,
            v: at.v,
            standoff: at.standoff,
        });
    }
    Err(bad(format!(
        "`{}` has no node or surface `{name}`",
        holder.id
    )))
}

/// A node written out from one placed in a part's frame, its place rounded to a nanometre.
pub(crate) fn node_def(name: &str, node: &crate::geometry::Node) -> NodeDef {
    // Over a power of ten, so that each is the double nearest its decimal.
    let round = |v: glam::DVec3, per: f64| v.to_array().map(|x| (x * per).round() / per + 0.0);
    NodeDef {
        name: name.into(),
        position: round(node.position, 1e9),
        direction: round(node.direction, 1e12),
        size: node.size,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{propulsion::Propulsion, test_data::records, vessel::tests::catalog};

    /// The triangles of a closed drum `radius` round the axis from `z0` to `z1`.
    fn drum(radius: f64, z0: f64, z1: f64) -> Vec<[DVec3; 3]> {
        let p = |k: usize, z: f64| {
            let a = std::f64::consts::TAU * k as f64 / 48.0;
            DVec3::new(radius * a.cos(), radius * a.sin(), z)
        };
        (0..48)
            .flat_map(|k| {
                let (a, b, c, d) = (p(k, z0), p(k + 1, z0), p(k, z1), p(k + 1, z1));
                [
                    [a, b, d],
                    [a, d, c],
                    [DVec3::Z * z0, b, a],
                    [DVec3::Z * z1, c, d],
                ]
            })
            .collect()
    }

    #[test]
    fn a_fitted_fairing_clears_what_it_holds() {
        let held = drum(0.8, 0.4, 3.0);
        let radius = 1.2;
        let sections = fit_fairing(&held, radius).unwrap();
        let [barrel, nose] = sections.as_slice() else {
            panic!("{sections:?}")
        };
        assert_eq!(nose.profile, ShellProfile::Ogive);
        assert!((nose.length - FAIRING_NOSE * 2.0 * radius).abs() < 1e-6);
        // The fairing's radius at each height: the barrel's, then the ogive's arc.
        let arc = (radius * radius + nose.length * nose.length) / (2.0 * radius);
        let wall = |z: f64| {
            let s = z - barrel.length;
            if s <= 0.0 {
                radius
            } else {
                (arc * arc - s * s).max(0.0).sqrt() - (arc - radius)
            }
        };
        let clearance = held
            .iter()
            .flatten()
            .map(|v| wall(v.z) - v.truncate().length())
            .fold(f64::INFINITY, f64::min);
        assert!(
            (FAIRING_CLEARANCE - 1e-3..2.0 * FAIRING_CLEARANCE).contains(&clearance),
            "{clearance}"
        );
        // Something wider than the fairing does not fit.
        assert!(fit_fairing(&drum(1.25, 0.4, 3.0), radius).is_none());
    }

    #[test]
    fn fitting_a_craft_s_fairings_keeps_it_flying() {
        let mut craft: CraftDef =
            records(include_str!("../../../assets/core/craft/pathfinder.json5")).remove(0);
        let fitted = craft.fit_fairings(&catalog()).unwrap();
        assert_eq!(fitted.len(), 2, "{fitted:?}");
        let p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        assert!(p.sections.iter().any(|s| s.fairing.is_some()));
    }

    #[test]
    fn the_modding_guides_craft_flies() {
        let guide = include_str!("../../../docs/modding.md");
        let section = &guide[guide.find("### Craft").unwrap()..];
        let start = section.find("```json5\n").unwrap() + "```json5\n".len();
        let text = &section[start..start + section[start..].find("```").unwrap()];
        let craft: CraftDef = records(text).remove(0);
        let p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        let names: Vec<&str> = p.sections.iter().map(|s| s.name.as_str()).collect();
        assert_eq!(names, ["booster", "cabin"]);
        assert!(p.sections[1].pod.is_some() && p.sections[0].engines == 1);
    }

    #[test]
    fn a_placed_assembly_sits_as_the_same_parts_written_out() {
        // A nose: a cone on a short ring, rooted at the cone and held by the ring's bottom.
        let nose: AssemblyDef = records(
            "{ id: 'nose', free: 'ring.bottom', parts: [
                { id: 'cone', shape: { shell: { bottom_diameter: 1.9,
                  sections: [{ length: 1.5, top_diameter: 0 }] } } },
                { id: 'ring', on: 'cone.bottom', by: 'top', roll: 30,
                  shape: { ring: { diameter: 1.9, height: 0.2 } } } ] }",
        )
        .remove(0);
        let guide = include_str!("../../../docs/modding.md");
        let section = &guide[guide.find("### Craft").unwrap()..];
        let start = section.find("```json5\n").unwrap() + "```json5\n".len();
        let hop: CraftDef =
            records(&section[start..start + section[start..].find("```").unwrap()]).remove(0);
        let mut placed = hop.clone();
        placed
            .place(&nose, "nose_", "cabin.top", None, 0.0)
            .unwrap();
        // The same, written by hand from the craft's root out.
        let mut written = hop.clone();
        written.parts.extend(records::<CraftPartDef>(
            "[{ id: 'nose_ring', on: 'cabin.top',
                shape: { ring: { diameter: 1.9, height: 0.2 } } },
              { id: 'nose_cone', on: 'nose_ring.top', roll: 30,
                shape: { shell: { bottom_diameter: 1.9,
                  sections: [{ length: 1.5, top_diameter: 0 }] } } }]",
        ));
        let poses = |craft: &CraftDef| {
            let built = Craft::build(craft, &catalog()).unwrap();
            let poses = built.assembly.poses();
            let mut by_id: Vec<_> = built
                .parts
                .iter()
                .map(|p| p.id.clone())
                .zip(poses)
                .collect();
            by_id.sort_by(|a, b| a.0.cmp(&b.0));
            by_id
        };
        let (a, b) = (poses(&placed), poses(&written));
        assert_eq!(a.len(), b.len());
        for ((ida, pa), (idb, pb)) in a.iter().zip(&b) {
            assert_eq!(ida, idb);
            assert!((pa.position - pb.position).length() < 1e-9, "{ida}");
            assert!(pa.rotation.dot(pb.rotation).abs() > 1.0 - 1e-12, "{ida}");
        }
        // Twice under one prefix is refused.
        let err = placed
            .place(&nose, "nose_", "capsule.top", None, 0.0)
            .unwrap_err();
        assert!(err.to_string().contains("already has"), "{err}");
    }
}
