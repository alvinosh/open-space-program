//! What a player can change about a part's shape, and where they take hold of it to change it: the
//! editor's handles, for every kind of shape, with no editor code for any one of them.
//!
//! [`sizes`] lists what can be changed (a tank's length and diameter, a fin's span and chords) and
//! how far, [`grips`] where on the part each is taken hold of and which way dragging it makes it
//! grow, and [`resize`] writes a new size into a part with what is mounted on it carried along: a
//! node on its wall stays on the wall, and one nearer an end stays as far from that end.
//! [`neighbour_widths`] gives the widths a part's widths can be matched to, those of the parts it
//! is joined to end to end.
//!
//! A part drawn by a model, or made from a catalogue record or by a sizer, has nothing here: what
//! it looks like is not what these sizes describe.

use glam::DVec3;

use crate::{
    assembly::Mount,
    craft::Craft,
    data::CraftPartDef,
    geometry::preset::{ShapeDef, ShellDef},
};

/// The smallest any size gets, m: a shape has to have some size to be built.
const LEAST: f64 = 0.05;
/// The biggest, m.
const MOST: f64 = 200.0;

/// One thing about a shape that can be changed, in metres.
#[derive(Debug, Clone, PartialEq)]
pub struct Size {
    /// The shape's own field: `length`, `diameter`, `top_diameter`, `span` and so on.
    pub name: &'static str,
    /// What the player reads.
    pub label: &'static str,
    pub value: f64,
    /// The least and the most it can be, as the rest of the shape stands.
    pub least: f64,
    pub most: f64,
    /// It is across the part's axis, so it can be matched to what the part is joined to.
    pub width: bool,
}

/// Where a part is taken hold of to change a size, or two.
#[derive(Debug, Clone, PartialEq)]
pub struct Grip {
    /// Where it is, in the part's frame.
    pub at: DVec3,
    /// What dragging it changes: one size, or two for a grip that moves in a plane.
    pub pulls: Vec<Pull>,
    /// It is out from the part's axis (+Z) on a part that is the same all the way round, so it can
    /// go round the axis to wherever it is easiest to see and take hold of.
    pub round: bool,
}

/// How dragging a grip one way changes one size.
#[derive(Debug, Clone, PartialEq)]
pub struct Pull {
    pub size: &'static str,
    /// The way the grip goes as the size grows, in the part's frame: unit.
    pub along: DVec3,
    /// How much the size grows for each metre the grip goes that way.
    pub rate: f64,
}

impl Grip {
    fn one(at: DVec3, size: &'static str, along: DVec3, rate: f64) -> Self {
        Self {
            at,
            pulls: vec![Pull { size, along, rate }],
            round: false,
        }
    }

    /// Out from the axis at height `z`, `radius` from it: a width, which grows twice as fast as the
    /// grip goes out.
    fn out(radius: f64, z: f64, size: &'static str) -> Self {
        Self {
            round: true,
            ..Self::one(DVec3::new(radius, 0.0, z), size, DVec3::X, 2.0)
        }
    }

    /// On the axis at height `z`, a length that grows as the grip goes up.
    fn up(z: f64, size: &'static str) -> Self {
        Self::one(DVec3::Z * z, size, DVec3::Z, 1.0)
    }

    fn mirrored(mut self) -> Self {
        self.at.x = -self.at.x;
        for pull in &mut self.pulls {
            pull.along.x = -pull.along.x;
        }
        self
    }
}

fn size(name: &'static str, label: &'static str, value: f64, width: bool) -> Size {
    Size {
        name,
        label,
        value,
        least: LEAST,
        most: MOST,
        width,
    }
}

/// What can be changed about `shape`, in the order a player would think of them.
pub fn sizes(shape: &ShapeDef) -> Vec<Size> {
    match shape {
        ShapeDef::CylinderTank(d) => {
            // The domes have to fit in its length.
            let domes = d.dome * d.diameter;
            vec![
                Size {
                    least: domes.max(LEAST),
                    ..size("length", "Length", d.length, false)
                },
                Size {
                    most: if d.dome > 0.0 {
                        (d.length / d.dome).min(MOST)
                    } else {
                        MOST
                    },
                    ..size("diameter", "Diameter", d.diameter, true)
                },
            ]
        }
        ShapeDef::ConeTank(d) => {
            let domes = |rb: f64, rt: f64| d.dome * (rb + rt) / 2.0;
            let room = |other: f64| {
                if d.dome > 0.0 {
                    (2.0 * d.length / d.dome - other).clamp(0.0, MOST)
                } else {
                    MOST
                }
            };
            vec![
                Size {
                    least: (domes(d.bottom_diameter, d.top_diameter) * 1.001).max(LEAST),
                    ..size("length", "Length", d.length, false)
                },
                Size {
                    least: 0.0,
                    most: room(d.top_diameter) * 0.999,
                    ..size(
                        "bottom_diameter",
                        "Bottom diameter",
                        d.bottom_diameter,
                        true,
                    )
                },
                Size {
                    least: 0.0,
                    most: room(d.bottom_diameter) * 0.999,
                    ..size("top_diameter", "Top diameter", d.top_diameter, true)
                },
            ]
        }
        ShapeDef::Sphere(d) => vec![size("diameter", "Diameter", d.diameter, true)],
        ShapeDef::Shell(d) if is_whole(d) => {
            let mut out = vec![
                size("length", "Length", shell_length(d), false),
                size(
                    "bottom_diameter",
                    "Bottom diameter",
                    d.bottom_diameter,
                    true,
                ),
            ];
            // A point stays a point: a nose's tip has nothing to widen.
            if shell_top(d) > 0.0 {
                out.push(size("top_diameter", "Top diameter", shell_top(d), true));
            }
            out
        }
        ShapeDef::Ring(d) => vec![
            size("diameter", "Diameter", d.diameter, true),
            Size {
                least: 0.005,
                ..size("height", "Height", d.height, false)
            },
        ],
        ShapeDef::Prism(d) => vec![
            size("length", "Length", d.length, false),
            size("diameter", "Diameter", d.diameter, true),
        ],
        ShapeDef::Cuboid(d) => vec![
            size("height", "Height", d.size[2], false),
            size("width", "Width", d.size[0], false),
            size("depth", "Depth", d.size[1], false),
        ],
        ShapeDef::Wing(d) => vec![
            size("span", "Span", d.span, false),
            size("root_chord", "Root chord", d.root_chord, false),
            Size {
                least: 0.0,
                ..size("tip_chord", "Tip chord", d.tip_chord, false)
            },
            Size {
                least: -MOST,
                ..size("sweep", "Sweep", d.sweep, false)
            },
        ],
        ShapeDef::Mirrored(inner) => sizes(inner),
        _ => Vec::new(),
    }
}

/// Set size `name` of `shape` to `value`, as it is: whether `shape` has such a size.
pub fn set(shape: &mut ShapeDef, name: &str, value: f64) -> bool {
    match (shape, name) {
        (ShapeDef::CylinderTank(d), "length") => d.length = value,
        (ShapeDef::CylinderTank(d), "diameter") => d.diameter = value,
        (ShapeDef::ConeTank(d), "length") => d.length = value,
        (ShapeDef::ConeTank(d), "bottom_diameter") => d.bottom_diameter = value,
        (ShapeDef::ConeTank(d), "top_diameter") => d.top_diameter = value,
        (ShapeDef::Sphere(d), "diameter") => d.diameter = value,
        (ShapeDef::Shell(d), "length") if is_whole(d) => {
            let scale = value / shell_length(d);
            for section in &mut d.sections {
                section.length *= scale;
            }
        }
        (ShapeDef::Shell(d), "bottom_diameter") if is_whole(d) => d.bottom_diameter = value,
        (ShapeDef::Shell(d), "top_diameter") if is_whole(d) && shell_top(d) > 0.0 => {
            if let Some(last) = d.sections.last_mut() {
                last.top_diameter = Some(value);
            }
        }
        (ShapeDef::Ring(d), "diameter") => d.diameter = value,
        (ShapeDef::Ring(d), "height") => d.height = value,
        (ShapeDef::Prism(d), "length") => d.length = value,
        (ShapeDef::Prism(d), "diameter") => d.diameter = value,
        (ShapeDef::Cuboid(d), "width") => d.size[0] = value,
        (ShapeDef::Cuboid(d), "depth") => d.size[1] = value,
        (ShapeDef::Cuboid(d), "height") => d.size[2] = value,
        (ShapeDef::Wing(d), "span") => d.span = value,
        (ShapeDef::Wing(d), "root_chord") => d.root_chord = value,
        (ShapeDef::Wing(d), "tip_chord") => d.tip_chord = value,
        (ShapeDef::Wing(d), "sweep") => d.sweep = value,
        (ShapeDef::Mirrored(inner), _) => return set(inner, name, value),
        _ => return false,
    }
    true
}

/// Where `shape` is taken hold of to change its sizes, in its frame.
pub fn grips(shape: &ShapeDef) -> Vec<Grip> {
    let Some((low, high)) = extent(shape) else {
        return Vec::new();
    };
    let middle = (low.z + high.z) / 2.0;
    match shape {
        ShapeDef::CylinderTank(d) => vec![
            Grip::up(high.z, "length"),
            Grip::out(d.diameter / 2.0, middle, "diameter"),
        ],
        ShapeDef::ConeTank(d) => {
            let (rb, rt) = (d.bottom_diameter / 2.0, d.top_diameter / 2.0);
            // At the rims, where the side meets the domes.
            let (bottom, top) = (d.dome * rb, d.length - d.dome * rt);
            vec![
                Grip::up(high.z, "length"),
                Grip::out(rb, bottom, "bottom_diameter"),
                Grip::out(rt, top, "top_diameter"),
            ]
        }
        ShapeDef::Sphere(d) => vec![Grip::out(d.diameter / 2.0, middle, "diameter")],
        ShapeDef::Shell(d) if is_whole(d) => {
            let mut out = vec![
                Grip::up(high.z, "length"),
                Grip::out(d.bottom_diameter / 2.0, low.z, "bottom_diameter"),
            ];
            if shell_top(d) > 0.0 {
                out.push(Grip::out(shell_top(d) / 2.0, high.z, "top_diameter"));
            }
            out
        }
        ShapeDef::Ring(d) => vec![
            Grip::up(high.z, "height"),
            Grip::out(d.diameter / 2.0, middle, "diameter"),
        ],
        ShapeDef::Prism(d) => vec![
            Grip::up(high.z, "length"),
            Grip::out(d.diameter / 2.0, middle, "diameter"),
        ],
        ShapeDef::Cuboid(d) => {
            let [x, y, _] = d.size;
            vec![
                Grip::up(high.z, "height"),
                Grip::one(DVec3::new(x / 2.0, 0.0, middle), "width", DVec3::X, 2.0),
                Grip::one(DVec3::new(0.0, y / 2.0, middle), "depth", DVec3::Y, 2.0),
            ]
        }
        ShapeDef::Wing(d) => {
            // The span runs out along +X and the chord back along −Z from the leading edge; the
            // tip rises by the dihedral as it goes out.
            let rise = d.dihedral.to_radians();
            let out = DVec3::new(1.0, -rise.tan(), 0.0);
            let tip = out * d.span;
            let back = DVec3::NEG_Z;
            vec![
                Grip {
                    at: tip + back * d.sweep,
                    pulls: vec![
                        Pull {
                            size: "span",
                            along: out.normalize(),
                            rate: rise.cos(),
                        },
                        Pull {
                            size: "sweep",
                            along: back,
                            rate: 1.0,
                        },
                    ],
                    round: false,
                },
                Grip::one(tip + back * (d.sweep + d.tip_chord), "tip_chord", back, 1.0),
                Grip::one(back * d.root_chord, "root_chord", back, 1.0),
            ]
        }
        ShapeDef::Mirrored(inner) => grips(inner).into_iter().map(Grip::mirrored).collect(),
        _ => Vec::new(),
    }
}

/// The sizes of `part`, if it is drawn as its shape: none for one made from a catalogue record or
/// by a sizer, or drawn by a model.
pub fn part_sizes(part: &CraftPartDef) -> Vec<Size> {
    match drawn_shape(part) {
        Some(shape) => sizes(shape),
        None => Vec::new(),
    }
}

/// Where `part` is taken hold of, if it is drawn as its shape.
pub fn part_grips(part: &CraftPartDef) -> Vec<Grip> {
    match drawn_shape(part) {
        Some(shape) => grips(shape),
        None => Vec::new(),
    }
}

fn drawn_shape(part: &CraftPartDef) -> Option<&ShapeDef> {
    let alone = part.template.is_none() && part.procedural.is_none() && part.model.is_none();
    part.shape.as_ref().filter(|_| alone)
}

/// Set size `name` of `part`'s shape to `value`, kept between its least and most, and carry its
/// own nodes (and where its mass sits) along with the shape: what is mounted on its wall stays on
/// the wall, and what is nearer an end stays as far from that end. Says why if it cannot.
pub fn resize(part: &mut CraftPartDef, name: &str, value: f64) -> Result<(), String> {
    let sizes = part_sizes(part);
    let Some(size) = sizes.iter().find(|s| s.name == name) else {
        return Err(format!("`{}` has no size `{name}` to change", part.id));
    };
    if !value.is_finite() {
        return Err(format!("{} has to be a number", size.label));
    }
    let value = value.clamp(size.least, size.most);
    let Some(shape) = part.shape.as_mut() else {
        unreachable!("it has sizes, so it has a shape");
    };
    let before = shape.clone();
    set(shape, name, value);
    shape.build()?;
    carry_nodes(part, &before);
    Ok(())
}

/// Carry `part`'s own nodes (and where its mass sits) from where they were on `before`, the shape
/// it had, to the same places on the shape it has now: on the wall stays on the wall, nearer an end
/// stays as far from that end. Nothing moves for shapes that have no box to go by.
pub fn carry_nodes(part: &mut CraftPartDef, before: &ShapeDef) {
    let (Some(before), Some(after)) = (extent(before), part.shape.as_ref().and_then(extent)) else {
        return;
    };
    let carry = |at: [f64; 3]| carried(DVec3::from(at), before, after).to_array();
    for node in &mut part.nodes {
        node.position = carry(node.position);
    }
    if let Some(center) = &mut part.mass_center {
        *center = carry(*center);
    }
}

/// `at`, on a shape that filled `before` and now fills `after`: as far out on each side across its
/// axis for the shape's width there, and as far from the nearer end.
fn carried(at: DVec3, before: (DVec3, DVec3), after: (DVec3, DVec3)) -> DVec3 {
    let ((low, high), (new_low, new_high)) = (before, after);
    let across = |v: f64, low: f64, high: f64, new_low: f64, new_high: f64| {
        // The ratio first, so that what does not change comes out exactly as it was.
        if v > 0.0 && high > 0.0 {
            v * (new_high / high)
        } else if v < 0.0 && low < 0.0 {
            v * (new_low / low)
        } else {
            v
        }
    };
    let z = if at.z > (low.z + high.z) / 2.0 {
        at.z + new_high.z - high.z
    } else {
        at.z + new_low.z - low.z
    };
    DVec3::new(
        across(at.x, low.x, high.x, new_low.x, new_high.x),
        across(at.y, low.y, high.y, new_low.y, new_high.y),
        z,
    )
}

/// The box `shape` fills in its frame, from its own sizes, for those that can be resized.
fn extent(shape: &ShapeDef) -> Option<(DVec3, DVec3)> {
    let round = |radius: f64, height: f64| {
        Some((
            DVec3::new(-radius, -radius, 0.0),
            DVec3::new(radius, radius, height),
        ))
    };
    match shape {
        ShapeDef::CylinderTank(d) => {
            let r = d.diameter / 2.0;
            let dome = d.dome * r;
            let barrel = (d.length - 2.0 * dome).max(0.0);
            let aft = dome.max(d.aft_skirt);
            round(r, aft + barrel + dome.max(d.forward_skirt))
        }
        ShapeDef::ConeTank(d) => round(d.bottom_diameter.max(d.top_diameter) / 2.0, d.length),
        ShapeDef::Sphere(d) => round(d.diameter / 2.0, d.diameter),
        ShapeDef::Shell(d) if is_whole(d) => {
            let widest = (d.sections.iter())
                .filter_map(|s| s.top_diameter)
                .fold(d.bottom_diameter, f64::max);
            round(widest / 2.0, shell_length(d))
        }
        ShapeDef::Ring(d) => round(d.diameter / 2.0, d.height),
        ShapeDef::Prism(d) => round(d.diameter / 2.0, d.length),
        ShapeDef::Cuboid(d) => {
            let [x, y, z] = d.size;
            Some((
                DVec3::new(-x / 2.0, -y / 2.0, 0.0),
                DVec3::new(x / 2.0, y / 2.0, z),
            ))
        }
        ShapeDef::Wing(d) => {
            let back = d.root_chord.max(d.sweep + d.tip_chord);
            Some((DVec3::new(0.0, 0.0, -back), DVec3::new(d.span, 0.0, 0.0)))
        }
        ShapeDef::Mirrored(inner) => {
            let (low, high) = extent(inner)?;
            Some((
                DVec3::new(-high.x, low.y, low.z),
                DVec3::new(-low.x, high.y, high.z),
            ))
        }
        _ => None,
    }
}

/// A shell all the way round: a part of a turn is a fairing's panel, which the fairing tool sizes.
fn is_whole(d: &ShellDef) -> bool {
    d.span.is_none_or(|[a, b]| (b - a).abs() >= 360.0 - 1e-9)
}

fn shell_length(d: &ShellDef) -> f64 {
    d.sections.iter().map(|s| s.length).sum()
}

/// How wide a shell is at its top.
fn shell_top(d: &ShellDef) -> f64 {
    (d.sections.iter()).fold(d.bottom_diameter, |below, s| {
        s.top_diameter.unwrap_or(below)
    })
}

/// The widths of the nodes `part` of `craft` is joined to end to end, each with the name of the
/// part it is on: what a width of `part` can be matched to.
pub fn neighbour_widths(craft: &Craft, part: usize) -> Vec<(f64, String)> {
    let mut out: Vec<(f64, String)> = Vec::new();
    for joint in craft.assembly.joints() {
        let [a, b] = &joint.ends;
        let other = if a.part == part {
            b
        } else if b.part == part {
            a
        } else {
            continue;
        };
        let (Mount::Node(name), Some(Mount::Node(_))) = (
            &other.mount,
            (joint.ends.iter())
                .find(|e| e.part == part)
                .map(|e| &e.mount),
        ) else {
            continue;
        };
        let Some(node) = craft.assembly.parts()[other.part].node(name) else {
            continue;
        };
        if node.size > 0.0 && !out.iter().any(|(w, _)| (w - node.size).abs() < 1e-6) {
            out.push((node.size, craft.parts[other.part].name.clone()));
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        data::{CraftDef, PartCatalog},
        geometry::preset::NodeDef,
    };

    fn shape(text: &str) -> ShapeDef {
        json5::from_str(text).unwrap()
    }

    /// One of every shape there are sizes for.
    fn every_shape() -> Vec<ShapeDef> {
        [
            "{ cylinder_tank: { diameter: 3, length: 8 } }",
            "{ cylinder_tank: { diameter: 10.1, length: 31.2, aft_skirt: 3.6, forward_skirt: 3.6, thrust_plate: true } }",
            "{ cone_tank: { length: 4, bottom_diameter: 3, top_diameter: 1.5 } }",
            "{ sphere: { diameter: 2 } }",
            "{ shell: { bottom_diameter: 3.7, sections: [{ length: 6 }, { length: 4.8, top_diameter: 0, profile: 'ogive' }] } }",
            "{ shell: { bottom_diameter: 6.6, sections: [{ length: 8.5, top_diameter: 3.9 }] } }",
            "{ ring: { diameter: 1.9, height: 0.08, thickness: 0.04 } }",
            "{ prism: { sides: 6, diameter: 1.2, length: 3 } }",
            "{ box: { size: [1.6, 1.2, 4.5] } }",
            "{ wing: { root_chord: 2.4, tip_chord: 1, span: 1.1, sweep: 1.2, airfoil: { plate: { thickness: 0.02 } } } }",
            "{ wing: { root_chord: 3, tip_chord: 1, span: 5, sweep: 1.5, dihedral: 6 } }",
            "{ mirrored: { wing: { root_chord: 3, tip_chord: 1, span: 5, sweep: 1.5, dihedral: 6 } } }",
        ]
        .into_iter()
        .map(shape)
        .collect()
    }

    fn value(shape: &ShapeDef, name: &str) -> f64 {
        sizes(shape).iter().find(|s| s.name == name).unwrap().value
    }

    #[test]
    fn every_size_reads_back_what_was_set_and_the_shape_still_builds() {
        for shape in every_shape() {
            for size in sizes(&shape) {
                let mut changed = shape.clone();
                let new = size.value * 1.1 + 0.01;
                assert!(set(&mut changed, size.name, new), "{shape:?} {}", size.name);
                assert!(
                    (value(&changed, size.name) - new).abs() < 1e-9,
                    "{shape:?}: {} reads back wrong",
                    size.name
                );
                changed
                    .build()
                    .unwrap_or_else(|e| panic!("{shape:?} {}: {e}", size.name));
            }
            assert!(!set(&mut shape.clone(), "colour", 1.0));
        }
    }

    fn grip_at(shape: &ShapeDef, index: usize) -> DVec3 {
        grips(shape)[index].at
    }

    #[test]
    fn a_grip_moves_with_the_size_it_changes_at_its_rate() {
        // So that a grip dragged a metre on the screen changes its size by its rate, and stays
        // under the pointer.
        for shape in every_shape() {
            for (index, grip) in grips(&shape).iter().enumerate() {
                assert!(!grip.pulls.is_empty());
                for pull in &grip.pulls {
                    assert!((pull.along.length() - 1.0).abs() < 1e-9);
                    let step = 0.1;
                    let mut grown = shape.clone();
                    set(&mut grown, pull.size, value(&shape, pull.size) + step);
                    let moved = grip_at(&grown, index) - grip.at;
                    let along = moved.dot(pull.along);
                    assert!(
                        (along * pull.rate - step).abs() < 1e-6,
                        "{shape:?}: `{}` moved its grip {along} m, at {} a metre",
                        pull.size,
                        pull.rate
                    );
                }
            }
        }
    }

    #[test]
    fn every_grip_changes_a_size_the_shape_has() {
        for shape in every_shape() {
            let names: Vec<&str> = sizes(&shape).iter().map(|s| s.name).collect();
            let pulled: Vec<&str> = (grips(&shape).iter())
                .flat_map(|g| g.pulls.iter().map(|p| p.size))
                .collect();
            for name in &names {
                assert!(pulled.contains(name), "{shape:?}: nothing grips `{name}`");
            }
            assert!(pulled.iter().all(|p| names.contains(p)));
        }
    }

    #[test]
    fn a_mirrored_wing_is_gripped_on_the_other_side() {
        let right = shape("{ wing: { root_chord: 3, tip_chord: 1, span: 5, sweep: 1.5 } }");
        let left = ShapeDef::Mirrored(Box::new(right.clone()));
        let (right, left) = (grips(&right), grips(&left));
        assert!(right[0].at.x > 4.9 && left[0].at.x < -4.9);
        assert_eq!(right[0].at.z, left[0].at.z);
        assert!(left[0].pulls[0].along.x < 0.0);
    }

    #[test]
    fn a_nose_has_no_tip_to_widen_and_a_fairing_panel_nothing_to_size() {
        let nose = shape(
            "{ shell: { bottom_diameter: 2, sections: [{ length: 3, top_diameter: 0, profile: 'ogive' }] } }",
        );
        assert!(sizes(&nose).iter().all(|s| s.name != "top_diameter"));
        let panel = shape(
            "{ shell: { bottom_diameter: 3.7, sections: [{ length: 11 }], span: [-90, 90] } }",
        );
        assert!(sizes(&panel).is_empty() && grips(&panel).is_empty());
    }

    #[test]
    fn a_longer_shell_keeps_the_shape_of_its_sections() {
        let mut fairing = shape(
            "{ shell: { bottom_diameter: 3.7, sections: [{ length: 6 }, { length: 4, top_diameter: 0, profile: 'ogive' }] } }",
        );
        set(&mut fairing, "length", 20.0);
        let ShapeDef::Shell(d) = &fairing else {
            unreachable!()
        };
        assert!((d.sections[0].length - 12.0).abs() < 1e-9);
        assert!((d.sections[1].length - 8.0).abs() < 1e-9);
        assert_eq!(d.sections[1].top_diameter, Some(0.0));
    }

    /// A tank with an engine under it, fins on its wall near the bottom, and a node near the top.
    fn tank() -> CraftPartDef {
        let node = |name: &str, position: [f64; 3], direction: [f64; 3]| NodeDef {
            name: name.into(),
            position,
            direction,
            size: 1.0,
        };
        CraftPartDef {
            id: "tank".into(),
            shape: Some(shape("{ cylinder_tank: { diameter: 4, length: 20 } }")),
            nodes: vec![
                node("engine", [1.0, 0.0, 0.0], [0.0, 0.0, -1.0]),
                node("fin", [2.0, 0.0, 3.0], [1.0, 0.0, 0.0]),
                node("other_fin", [0.0, -2.0, 3.0], [0.0, -1.0, 0.0]),
                node("antenna", [2.0, 0.0, 19.0], [1.0, 0.0, 0.0]),
            ],
            mass_center: Some([0.0, 0.0, 12.0]),
            ..json5::from_str("{ id: 'tank' }").unwrap()
        }
    }

    fn at(part: &CraftPartDef, name: &str) -> DVec3 {
        let node = part.nodes.iter().find(|n| n.name == name).unwrap();
        DVec3::from(node.position)
    }

    #[test]
    fn a_longer_tank_takes_what_is_near_its_top_up_with_it() {
        let mut part = tank();
        let top = |part: &CraftPartDef| grips(part.shape.as_ref().unwrap())[0].at.z;
        let before = top(&part);
        resize(&mut part, "length", 25.0).unwrap();
        assert!((top(&part) - before - 5.0).abs() < 1e-9);
        assert_eq!(at(&part, "engine"), DVec3::new(1.0, 0.0, 0.0));
        assert_eq!(at(&part, "fin"), DVec3::new(2.0, 0.0, 3.0));
        assert_eq!(at(&part, "antenna"), DVec3::new(2.0, 0.0, 24.0));
        assert_eq!(part.mass_center, Some([0.0, 0.0, 17.0]));
    }

    #[test]
    fn a_wider_tank_keeps_what_is_on_its_wall_on_the_wall() {
        let mut part = tank();
        resize(&mut part, "diameter", 6.0).unwrap();
        assert!((at(&part, "fin") - DVec3::new(3.0, 0.0, 3.0)).length() < 1e-9);
        assert!((at(&part, "other_fin") - DVec3::new(0.0, -3.0, 3.0)).length() < 1e-9);
        // An engine part way out goes out as far for the width.
        assert!((at(&part, "engine").x - 1.5).abs() < 1e-9);
        // Its length runs from dome to dome, so its top stays where it was.
        assert!((at(&part, "antenna") - DVec3::new(3.0, 0.0, 19.0)).length() < 1e-9);
    }

    #[test]
    fn a_size_is_kept_inside_what_the_shape_allows() {
        let mut part = tank();
        resize(&mut part, "length", 0.5).unwrap();
        let length = value(part.shape.as_ref().unwrap(), "length");
        // √2:1 domes on a 4 m tank need 2.83 m between them.
        assert!((length - 4.0 * std::f64::consts::FRAC_1_SQRT_2).abs() < 1e-9);
        part.shape.as_ref().unwrap().build().unwrap();
        assert!(resize(&mut part, "span", 2.0).is_err());
        assert!(resize(&mut part, "length", f64::NAN).is_err());
        let mut engine = part.clone();
        engine.template = Some("Some engine".into());
        assert!(part_sizes(&engine).is_empty() && part_grips(&engine).is_empty());
    }

    #[test]
    fn a_part_can_be_matched_to_what_it_stands_on_and_what_stands_on_it() {
        let craft: CraftDef = json5::from_str(
            "{ id: 'stack', parts: [
                { id: 'top', shape: { cylinder_tank: { diameter: 2, length: 5 } } },
                { id: 'middle', on: 'top.bottom', by: 'top', shape: { cone_tank: { length: 3, bottom_diameter: 3, top_diameter: 2 } } },
                { id: 'bottom', name: 'Booster', on: 'middle.bottom', by: 'top', shape: { cylinder_tank: { diameter: 3, length: 10 } } },
            ] }",
        )
        .unwrap();
        let catalog: PartCatalog = json5::from_str("{ engines: [] }").unwrap();
        let built = Craft::build(&craft, &catalog).unwrap();
        let mut widths = neighbour_widths(&built, 1);
        widths.sort_by(|a, b| a.0.total_cmp(&b.0));
        assert_eq!(widths, [(2.0, "top".into()), (3.0, "Booster".into())]);
        assert_eq!(neighbour_widths(&built, 0), [(2.0, "middle".into())]);
    }
}
