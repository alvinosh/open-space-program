//! Shape presets: the forms craft files describe shapes in. Each becomes subparts of the
//! four kinds when loaded ([`ShapeDef::build`]), so nothing else knows about presets, and
//! new presets need nothing new downstream.
//!
//! Lengths are metres and angles degrees. Presets stand the part on z = 0 and run up +Z,
//! except a wing: its origin is the root's leading edge, its span runs along +X and its
//! chord back along −Z, and the side it lifts towards faces −Y.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use glam::{DMat3, DQuat, DVec2, DVec3};
use serde::{Deserialize, Serialize};

use super::{
    Fill, Geometry, Kind, ModelRef, Node, Subpart,
    curve::{Curve, Segment},
    lattice::Lattice,
    lofted::{Lofted, Station},
    revolved::{Region, Revolved, Wall},
};
use crate::{data::ShellProfile, skip::*};

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ShapeDef {
    CylinderTank(CylinderTankDef),
    ConeTank(ConeTankDef),
    Sphere(SphereDef),
    Torus(TorusDef),
    Shell(ShellDef),
    Ring(RingDef),
    Wing(WingDef),
    #[serde(rename = "box")]
    Cuboid(BoxDef),
    Prism(PrismDef),
    Beam(BeamDef),
    Truss(TrussDef),
    Revolved(RevolvedDef),
    Lofted(LoftedDef),
    Lattice(LatticeDef),
    Model(ModelDef),
    /// Several shapes, each placed in the part's frame.
    Compound(Vec<PlacedShapeDef>),
    /// A shape reflected across its YZ plane, X to −X: a left wing from a right one, for
    /// shapes that no turn brings onto their mirror image.
    Mirrored(Box<ShapeDef>),
}

/// A cylinder closed by domes at both ends: the usual propellant tank.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CylinderTankDef {
    pub diameter: f64,
    /// From the tip of one dome to the tip of the other.
    pub length: f64,
    /// Dome height over radius: √2:1 ellipsoidal heads by default, 0 flat, 1 hemispherical.
    #[serde(default = "default_dome", skip_serializing_if = "is_default_dome")]
    pub dome: f64,
    /// Height of a common bulkhead's rim above the aft dome's: it parts the tank into a
    /// `lower` and an `upper` cell, bulging down like the aft dome.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub common_bulkhead: Option<f64>,
    /// How far the wall runs on past each dome's rim: skirts that carry the stack's load
    /// around the domes.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub aft_skirt: f64,
    #[serde(default, skip_serializing_if = "is_zero")]
    pub forward_skirt: f64,
}

/// A frustum closed by domes, or by a point where a diameter is 0.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConeTankDef {
    /// From the bottom dome's tip to the top one's.
    pub length: f64,
    pub bottom_diameter: f64,
    pub top_diameter: f64,
    #[serde(default = "default_dome", skip_serializing_if = "is_default_dome")]
    pub dome: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SphereDef {
    pub diameter: f64,
}

/// A ring-shaped tank around the axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TorusDef {
    /// Across the outside.
    pub diameter: f64,
    /// Of the tube.
    pub tube_diameter: f64,
}

/// A thin wall of revolution through sections stacked from the bottom up: a nose cone,
/// adapter, interstage, skirt or fairing. Over part of a turn it is a fairing panel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellDef {
    pub bottom_diameter: f64,
    pub sections: Vec<ShellSection>,
    /// The angles it runs between; all the way round by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<[f64; 2]>,
    /// Flat walls closing its ends.
    #[serde(default, skip_serializing_if = "is_false")]
    pub cap_bottom: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub cap_top: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ShellSection {
    pub length: f64,
    /// By default the diameter below it; 0 closes it to a point.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_diameter: Option<f64>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub profile: ShellProfile,
}

/// A solid ring (or disc): a separation ring, a flange.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RingDef {
    pub diameter: f64,
    pub height: f64,
    /// From the outside in; a disc if it reaches the axis.
    #[serde(
        default = "default_ring_thickness",
        skip_serializing_if = "is_default_ring_thickness"
    )]
    pub thickness: f64,
}

/// A wing, tailplane or fin panel, tapered and swept.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WingDef {
    pub root_chord: f64,
    pub tip_chord: f64,
    pub span: f64,
    /// How far the tip's leading edge sits behind the root's.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub sweep: f64,
    /// Degrees the tip rises towards the lifting side.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub dihedral: f64,
    /// Degrees the tip's leading edge turns towards the lifting side, about its quarter
    /// chord.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub twist: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub airfoil: AirfoilDef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AirfoilDef {
    /// A NACA four-digit airfoil: thickness and camber over chord, and where along the
    /// chord the camber peaks.
    Naca {
        thickness: f64,
        #[serde(default, skip_serializing_if = "is_zero")]
        camber: f64,
        #[serde(
            default = "default_camber_position",
            skip_serializing_if = "is_default_camber_position"
        )]
        camber_position: f64,
    },
    /// A flat plate `thickness` thick whatever the chord, its edges bevelled over `bevel`
    /// of the chord: a rocket fin.
    Plate {
        thickness: f64,
        #[serde(default = "default_bevel", skip_serializing_if = "is_default_bevel")]
        bevel: f64,
    },
}

impl Default for AirfoilDef {
    fn default() -> Self {
        AirfoilDef::Naca {
            thickness: 0.12,
            camber: 0.0,
            camber_position: default_camber_position(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BoxDef {
    /// Along X, Y and Z, centred on the axis.
    pub size: [f64; 3],
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill: Fill,
}

/// A regular prism along the axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PrismDef {
    pub sides: u32,
    /// Across the corners.
    pub diameter: f64,
    pub length: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill: Fill,
}

/// A straight structural member along the axis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BeamDef {
    pub length: f64,
    pub section: BeamSection,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BeamSection {
    Rectangle {
        width: f64,
        height: f64,
    },
    /// Flanges `width` wide and `flange` thick, `height` apart outside, on a `web` thick web.
    IBeam {
        width: f64,
        height: f64,
        flange: f64,
        web: f64,
    },
    Circle {
        diameter: f64,
    },
    Tube {
        diameter: f64,
        wall: f64,
    },
}

/// A truss on a frustum: rings of nodes, members along it and bracing between them.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TrussDef {
    pub bottom_diameter: f64,
    /// By default the bottom's.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub top_diameter: Option<f64>,
    pub length: f64,
    /// Bays along it.
    #[serde(default = "default_bays")]
    pub bays: u32,
    /// Nodes around each ring.
    #[serde(default = "default_around")]
    pub around: u32,
    #[serde(default, skip_serializing_if = "is_default")]
    pub bracing: Bracing,
    /// Members around each ring of nodes.
    #[serde(default = "default_true")]
    pub rings: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Bracing {
    /// Members along it and both diagonals across each panel.
    #[default]
    X,
    /// Zig-zag diagonals between rings of nodes a half step apart, no members along it.
    Warren,
    /// Members along it only.
    None,
}

/// Walls and regions written out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RevolvedDef {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub span: Option<[f64; 2]>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub walls: Vec<WallDef>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub regions: Vec<RegionDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WallDef {
    pub name: String,
    pub curve: CurveDef,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RegionDef {
    pub name: String,
    pub boundary: CurveDef,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill: Fill,
}

/// A curve from `start` ((radius, height)) through its segments, each from where the last
/// one ended.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CurveDef {
    pub start: [f64; 2],
    pub segments: Vec<SegmentDef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SegmentDef {
    /// A straight line to this point.
    Line([f64; 2]),
    /// Around the ellipse through the current point, to this angle (degrees from the radial
    /// direction towards +height).
    Ellipse {
        center: [f64; 2],
        radii: [f64; 2],
        to: f64,
    },
    /// A nose-cone curve to this point.
    Taper {
        to: [f64; 2],
        #[serde(default)]
        profile: ShellProfile,
    },
    Bezier {
        c1: [f64; 2],
        c2: [f64; 2],
        to: [f64; 2],
    },
}

/// A section written out and lofted through stations.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LoftedDef {
    #[serde(
        default = "default_loft_name",
        skip_serializing_if = "is_default_loft_name"
    )]
    pub name: String,
    pub section: SectionDef,
    pub stations: Vec<StationDef>,
    #[serde(default, skip_serializing_if = "is_default")]
    pub fill: Fill,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SectionDef {
    /// A unit chord along +X from the leading edge, the upper surface towards +Y.
    Airfoil(AirfoilDef),
    Rectangle {
        width: f64,
        height: f64,
    },
    Ellipse {
        width: f64,
        height: f64,
    },
    /// A regular polygon, `diameter` across its corners.
    Regular {
        sides: u32,
        diameter: f64,
    },
    IBeam {
        width: f64,
        height: f64,
        flange: f64,
        web: f64,
    },
    Polygon(Vec<[f64; 2]>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StationDef {
    pub z: f64,
    #[serde(default, skip_serializing_if = "is_default")]
    pub offset: [f64; 2],
    #[serde(default = "default_scale", skip_serializing_if = "is_default_scale")]
    pub scale: [f64; 2],
    /// Degrees.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub twist: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LatticeDef {
    pub nodes: Vec<[f64; 3]>,
    pub members: Vec<[usize; 2]>,
}

/// A glTF model, with a simple shape standing in for it in the air and in collisions.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ModelDef {
    /// Relative to the mod's root.
    pub path: String,
    #[serde(default = "default_scale_factor", skip_serializing_if = "is_one")]
    pub scale: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<Box<ShapeDef>>,
    /// Where other parts join it: a model has no natural nodes.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub nodes: Vec<NodeDef>,
}

/// A node written out, in the part's frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NodeDef {
    pub name: String,
    pub position: [f64; 3],
    /// Out of the part, the way the part joined there lies; need not be unit.
    pub direction: [f64; 3],
    /// Diameter of what fits there, m.
    #[serde(default, skip_serializing_if = "is_zero")]
    pub size: f64,
}

impl NodeDef {
    pub fn node(&self) -> Result<Node, String> {
        let (position, direction) = (DVec3::from(self.position), DVec3::from(self.direction));
        let Some(direction) = direction.try_normalize().filter(|_| position.is_finite()) else {
            return Err(format!(
                "node `{}` needs a place and a direction",
                self.name
            ));
        };
        if !non_negative(self.size) || self.name.is_empty() {
            return Err("a node needs a name, and a size that is not negative".into());
        }
        Ok(Node {
            name: self.name.clone(),
            position,
            direction,
            size: self.size,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PlacedShapeDef {
    /// Prefixes the names of its pieces.
    #[serde(default, skip_serializing_if = "is_default")]
    pub name: String,
    pub shape: ShapeDef,
    #[serde(default, skip_serializing_if = "is_default")]
    pub position: [f64; 3],
    /// Degrees about the part's X, then Y, then Z axis.
    #[serde(default, skip_serializing_if = "is_default")]
    pub rotation: [f64; 3],
    /// Inside the part and not drawn: room its equipment takes, which weighs.
    #[serde(default, skip_serializing_if = "is_false")]
    pub unseen: bool,
}

fn default_dome() -> f64 {
    std::f64::consts::FRAC_1_SQRT_2
}

fn is_default_loft_name(name: &str) -> bool {
    name == default_loft_name()
}

fn is_default_scale(scale: &[f64; 2]) -> bool {
    *scale == default_scale()
}

fn is_default_dome(x: &f64) -> bool {
    *x == default_dome()
}

fn is_default_ring_thickness(x: &f64) -> bool {
    *x == default_ring_thickness()
}

fn is_default_camber_position(x: &f64) -> bool {
    *x == default_camber_position()
}

fn is_default_bevel(x: &f64) -> bool {
    *x == default_bevel()
}

fn default_ring_thickness() -> f64 {
    0.05
}

fn default_camber_position() -> f64 {
    0.4
}

fn default_bevel() -> f64 {
    0.15
}

fn default_bays() -> u32 {
    1
}

fn default_around() -> u32 {
    8
}

fn default_true() -> bool {
    true
}

fn default_loft_name() -> String {
    "loft".into()
}

fn default_scale() -> [f64; 2] {
    [1.0, 1.0]
}

fn default_scale_factor() -> f64 {
    1.0
}

impl ShapeDef {
    /// The geometry this describes, or what is wrong with it.
    pub fn build(&self) -> Result<Geometry, String> {
        let (subparts, nodes) = self.pieces()?;
        let geometry = Geometry { subparts, nodes };
        let mut names: Vec<String> = geometry.elements().into_iter().map(|e| e.name).collect();
        names.sort();
        if let Some(w) = names.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!(
                "two pieces are both named `{}`: name the shapes of a compound",
                w[0]
            ));
        }
        let mut nodes: Vec<&str> = geometry.nodes.iter().map(|n| n.name.as_str()).collect();
        nodes.sort();
        if let Some(w) = nodes.windows(2).find(|w| w[0] == w[1]) {
            return Err(format!(
                "two nodes are both named `{}`: name the shapes of a compound",
                w[0]
            ));
        }
        Ok(geometry)
    }

    /// Its subparts and nodes.
    fn pieces(&self) -> Result<Pieces, String> {
        let one = |kind| Ok(natural(vec![Subpart::new("", kind)]));
        match self {
            // A tank stacks by its rims, whatever its domes do.
            ShapeDef::CylinderTank(d) => Ok(sized(
                one(Kind::Revolved(cylinder_tank(d)?))?,
                d.diameter,
                d.diameter,
            )),
            ShapeDef::ConeTank(d) => Ok(sized(
                one(Kind::Revolved(cone_tank(d)?))?,
                d.bottom_diameter,
                d.top_diameter,
            )),
            ShapeDef::Sphere(d) => {
                positive("a sphere's diameter", d.diameter)?;
                let r = d.diameter / 2.0;
                let side = ellipse(DVec2::new(0.0, r), DVec2::splat(r), -FRAC_PI_2, FRAC_PI_2);
                let sphere = one(Kind::Revolved(walled_cell(vec![side])?))?;
                Ok(sized(sphere, d.diameter, d.diameter))
            }
            ShapeDef::Torus(d) => {
                positive("a torus's tube diameter", d.tube_diameter)?;
                let r = d.tube_diameter / 2.0;
                let center = d.diameter / 2.0 - r;
                if center < r - 1e-12 {
                    return Err(format!(
                        "a torus {} m across is too small for its {} m tube to clear the axis",
                        d.diameter, d.tube_diameter
                    ));
                }
                let tube = ellipse(DVec2::new(center, r), DVec2::splat(r), -FRAC_PI_2, 1.5 * PI);
                let torus = one(Kind::Revolved(walled_cell(vec![tube])?))?;
                Ok(sized(torus, d.diameter, d.diameter))
            }
            ShapeDef::Shell(d) => one(Kind::Revolved(shell(d)?)),
            ShapeDef::Ring(d) => {
                positive("a ring's diameter", d.diameter)?;
                positive("a ring's height", d.height)?;
                positive("a ring's thickness", d.thickness)?;
                let (r, h) = (d.diameter / 2.0, d.height);
                let inner = (r - d.thickness).max(0.0);
                let p = DVec2::new;
                let corners = if inner > 1e-9 * r {
                    vec![
                        p(inner, 0.0),
                        p(r, 0.0),
                        p(r, h),
                        p(inner, h),
                        p(inner, 0.0),
                    ]
                } else {
                    vec![p(0.0, 0.0), p(r, 0.0), p(r, h), p(0.0, h)]
                };
                let region = region("ring", polyline(&corners)?, Fill::Solid);
                one(Kind::Revolved(Revolved::new(
                    0.0,
                    TAU,
                    Vec::new(),
                    vec![region],
                )?))
            }
            ShapeDef::Wing(d) => wing(d),
            ShapeDef::Cuboid(d) => {
                let [x, y, z] = d.size;
                positive("a box's size", x.min(y).min(z))?;
                let section = rectangle(x, y);
                one(Kind::Lofted(straight("box", section, z, d.fill)?))
            }
            ShapeDef::Prism(d) => {
                positive("a prism's diameter", d.diameter)?;
                positive("a prism's length", d.length)?;
                let section = regular(d.sides, d.diameter)?;
                one(Kind::Lofted(straight("prism", section, d.length, d.fill)?))
            }
            ShapeDef::Beam(d) => Ok(natural(beam(d)?)),
            ShapeDef::Truss(d) => one(Kind::Lattice(truss(d)?)),
            ShapeDef::Revolved(d) => {
                let (from, to) = span(d.span)?;
                let walls = d
                    .walls
                    .iter()
                    .map(|w| Ok(wall(&w.name, curve(&w.curve)?)))
                    .collect::<Result<_, String>>()?;
                let regions = d
                    .regions
                    .iter()
                    .map(|r| Ok(region(&r.name, curve(&r.boundary)?, r.fill)))
                    .collect::<Result<_, String>>()?;
                one(Kind::Revolved(Revolved::new(from, to, walls, regions)?))
            }
            ShapeDef::Lofted(d) => {
                let stations = d
                    .stations
                    .iter()
                    .map(|s| Station {
                        z: s.z,
                        offset: s.offset.into(),
                        scale: s.scale.into(),
                        twist: s.twist.to_radians(),
                    })
                    .collect();
                let section = section(&d.section)?;
                one(Kind::Lofted(Lofted::new(
                    d.name.clone(),
                    section,
                    stations,
                    d.fill,
                )?))
            }
            ShapeDef::Lattice(d) => {
                let nodes = d.nodes.iter().map(|&n| DVec3::from(n)).collect();
                one(Kind::Lattice(Lattice::new(
                    "lattice".into(),
                    nodes,
                    d.members.clone(),
                )?))
            }
            ShapeDef::Model(d) => {
                positive("a model's scale", d.scale)?;
                let model = ModelRef {
                    path: d.path.clone(),
                    scale: d.scale,
                };
                let mut subparts = vec![Subpart::new("", Kind::Model(model))];
                if let Some(proxy) = &d.proxy {
                    for mut sub in proxy.pieces()?.0 {
                        sub.proxy = true;
                        sub.name = join("proxy", &sub.name);
                        subparts.push(sub);
                    }
                }
                let nodes = d
                    .nodes
                    .iter()
                    .map(NodeDef::node)
                    .collect::<Result<_, _>>()?;
                Ok((subparts, nodes))
            }
            ShapeDef::Compound(shapes) => {
                if shapes.is_empty() {
                    return Err("a compound needs shapes".into());
                }
                let (mut subparts, mut nodes) = (Vec::new(), Vec::new());
                for placed in shapes {
                    let [x, y, z] = placed.rotation.map(f64::to_radians);
                    let rotation = DQuat::from_rotation_z(z)
                        * DQuat::from_rotation_y(y)
                        * DQuat::from_rotation_x(x);
                    let position = DVec3::from(placed.position);
                    let (subs, ends) = placed.shape.pieces()?;
                    for mut sub in subs {
                        sub.name = join(&placed.name, &sub.name);
                        sub.unseen |= placed.unseen;
                        sub.position = position + rotation * sub.position;
                        sub.rotation = rotation * sub.rotation;
                        subparts.push(sub);
                    }
                    nodes.extend(ends.into_iter().map(|n| Node {
                        name: join(&placed.name, &n.name),
                        position: position + rotation * n.position,
                        direction: rotation * n.direction,
                        size: n.size,
                    }));
                }
                Ok((subparts, nodes))
            }
            ShapeDef::Mirrored(shape) => {
                let (subparts, nodes) = shape.pieces()?;
                let subparts = subparts
                    .into_iter()
                    .map(mirrored)
                    .collect::<Result<_, _>>()?;
                let nodes = nodes
                    .into_iter()
                    .map(|n| Node {
                        position: flip(n.position),
                        direction: flip(n.direction),
                        ..n
                    })
                    .collect();
                Ok((subparts, nodes))
            }
        }
    }
}

/// X to −X.
fn flip(v: DVec3) -> DVec3 {
    DVec3::new(-v.x, v.y, v.z)
}

/// `sub` reflected across the part's YZ plane. A reflection is no turn, so each kind takes
/// a reflection of its own frame into its description (a loft's stations, a revolved span,
/// a lattice's nodes) and the turn is what is left: M R L, M the part's reflection and L
/// the kind's. A loft reflects along its stations, which leaves its section as it was: a
/// wing's airfoil still leads with its nose and lifts the same way.
fn mirrored(sub: Subpart) -> Result<Subpart, String> {
    let (kind, own) = match sub.kind {
        Kind::Lofted(l) => {
            let stations = l
                .stations()
                .iter()
                .rev()
                .map(|s| Station { z: -s.z, ..*s })
                .collect();
            let loft = Lofted::new(l.name.clone(), l.section().to_vec(), stations, l.fill)?;
            (Kind::Lofted(loft), DVec3::new(1.0, 1.0, -1.0))
        }
        Kind::Revolved(r) => {
            let (from, to) = r.span();
            let walls = r.walls().to_vec();
            let regions = r.regions().to_vec();
            let revolved = Revolved::new(-to, -from, walls, regions)?;
            (Kind::Revolved(revolved), DVec3::new(1.0, -1.0, 1.0))
        }
        Kind::Lattice(l) => {
            let nodes = l.nodes().iter().map(|&n| flip(n)).collect();
            let lattice = Lattice::new(l.name.clone(), nodes, l.members().to_vec())?;
            (Kind::Lattice(lattice), DVec3::new(-1.0, 1.0, 1.0))
        }
        Kind::Model(m) => return Err(format!("the model `{}` cannot be mirrored", m.path)),
    };
    let turn = DMat3::from_diagonal(DVec3::new(-1.0, 1.0, 1.0))
        * DMat3::from_quat(sub.rotation)
        * DMat3::from_diagonal(own);
    Ok(Subpart {
        position: flip(sub.position),
        rotation: DQuat::from_mat3(&turn).normalize(),
        kind,
        ..sub
    })
}

fn join(outer: &str, inner: &str) -> String {
    match (outer.is_empty(), inner.is_empty()) {
        (true, _) => inner.to_owned(),
        (false, true) => outer.to_owned(),
        (false, false) => format!("{outer}/{inner}"),
    }
}

type Pieces = (Vec<Subpart>, Vec<Node>);

/// Subparts with the natural nodes at the ends of each.
fn natural(subparts: Vec<Subpart>) -> Pieces {
    let nodes = subparts
        .iter()
        .flat_map(|sub| {
            let local = match &sub.kind {
                Kind::Revolved(r) => revolved_ends(r),
                Kind::Lofted(l) => lofted_ends(l),
                Kind::Lattice(l) => lattice_ends(l),
                Kind::Model(_) => Vec::new(),
            };
            local.into_iter().map(|n| Node {
                name: join(&sub.name, &n.name),
                position: sub.position + sub.rotation * n.position,
                direction: sub.rotation * n.direction,
                size: n.size,
            })
        })
        .collect();
    (subparts, nodes)
}

/// `pieces` with the sizes of its `bottom` and `top` nodes set.
fn sized((subparts, mut nodes): Pieces, bottom: f64, top: f64) -> Pieces {
    for node in &mut nodes {
        match node.name.as_str() {
            "bottom" => node.size = bottom,
            "top" => node.size = top,
            _ => {}
        }
    }
    (subparts, nodes)
}

fn end(name: &str, position: DVec3, direction: DVec3, size: f64) -> Node {
    Node {
        name: name.into(),
        position,
        direction,
        size,
    }
}

/// Nodes at the lowest and highest points of a revolved subpart, as wide as it is there: on
/// the axis, or for one turned through part of a turn, at the middle of its span.
fn revolved_ends(r: &Revolved) -> Vec<Node> {
    let curves: Vec<&Curve> = r
        .walls()
        .iter()
        .map(|w| &w.curve)
        .chain(r.regions().iter().map(|g| &g.boundary))
        .collect();
    let (lo, hi) = curves
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), c| {
            let (a, b) = c.heights();
            (lo.min(a), hi.max(b))
        });
    let widest = |z: f64| {
        curves
            .iter()
            .filter_map(|c| c.widest_at(z))
            .fold(0.0f64, f64::max)
    };
    let (from, to) = r.span();
    let middle = DVec2::from_angle((from + to) / 2.0);
    let at = |z: f64, radius: f64| match r.is_full() {
        true => DVec3::Z * z,
        false => (middle * radius).extend(z),
    };
    let (bottom, top) = (widest(lo), widest(hi));
    vec![
        end("bottom", at(lo, bottom), DVec3::NEG_Z, 2.0 * bottom),
        end("top", at(hi, top), DVec3::Z, 2.0 * top),
    ]
}

/// Nodes at a loft's first and last stations, at the section's origin, as wide as the
/// section reaches from it.
fn lofted_ends(l: &Lofted) -> Vec<Node> {
    let stations = l.stations();
    let at = |name: &str, s: &Station, direction| {
        let reach = l
            .section()
            .iter()
            .map(|p| (DVec2::from_angle(s.twist).rotate(*p * s.scale)).length())
            .fold(0.0f64, f64::max);
        end(name, s.offset.extend(s.z), direction, 2.0 * reach)
    };
    vec![
        at("bottom", &stations[0], DVec3::NEG_Z),
        at("top", &stations[stations.len() - 1], DVec3::Z),
    ]
}

/// Nodes on the axis at a lattice's lowest and highest nodes, as wide as they spread there.
fn lattice_ends(l: &Lattice) -> Vec<Node> {
    let (lo, hi) = l
        .nodes()
        .iter()
        .fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), p| {
            (lo.min(p.z), hi.max(p.z))
        });
    let near = 1e-9 * (hi - lo).max(1.0);
    let spread = |z: f64| {
        l.nodes()
            .iter()
            .filter(|p| (p.z - z).abs() <= near)
            .map(|p| p.truncate().length())
            .fold(0.0f64, f64::max)
    };
    vec![
        end("bottom", DVec3::Z * lo, DVec3::NEG_Z, 2.0 * spread(lo)),
        end("top", DVec3::Z * hi, DVec3::Z, 2.0 * spread(hi)),
    ]
}

fn positive(what: &str, value: f64) -> Result<(), String> {
    if value > 0.0 && value.is_finite() {
        Ok(())
    } else {
        Err(format!("{what} must be positive, not {value}"))
    }
}

/// At least 0, and a number.
fn non_negative(value: f64) -> bool {
    value >= 0.0
}

fn span(degrees: Option<[f64; 2]>) -> Result<(f64, f64), String> {
    match degrees {
        None => Ok((0.0, TAU)),
        Some([from, to]) if to > from && to - from <= 360.0 => {
            Ok((from.to_radians(), to.to_radians()))
        }
        Some([from, to]) => Err(format!(
            "a span runs from one angle to a larger one, at most 360° on: not {from}° to {to}°"
        )),
    }
}

fn wall(name: &str, curve: Curve) -> Wall {
    Wall {
        name: name.into(),
        curve,
    }
}

fn region(name: &str, boundary: Curve, fill: Fill) -> Region {
    Region {
        name: name.into(),
        boundary,
        fill,
    }
}

fn line(from: DVec2, to: DVec2) -> Segment {
    Segment::Line { from, to }
}

fn ellipse(center: DVec2, radii: DVec2, from: f64, to: f64) -> Segment {
    Segment::Ellipse {
        center,
        radii,
        from,
        to,
    }
}

fn polyline(points: &[DVec2]) -> Result<Curve, String> {
    Curve::new(points.windows(2).map(|w| line(w[0], w[1])).collect())
}

/// A closed curve as both a wall named `wall` and a cell named `tank`.
fn walled_cell(segments: Vec<Segment>) -> Result<Revolved, String> {
    let curve = Curve::new(segments)?;
    Revolved::new(
        0.0,
        TAU,
        vec![wall("wall", curve.clone())],
        vec![region("tank", curve, Fill::Cell)],
    )
}

/// A dome `height` tall on a rim `radius` wide at height `rim`, bulging down (from the axis
/// out to the rim) or up (from the rim in to the axis); flat if `height` is 0.
fn head(radius: f64, height: f64, rim: f64, up: bool) -> Segment {
    let (axis, edge) = (DVec2::new(0.0, rim), DVec2::new(radius, rim));
    match (height > 0.0, up) {
        (true, false) => ellipse(axis, DVec2::new(radius, height), -FRAC_PI_2, 0.0),
        (true, true) => ellipse(axis, DVec2::new(radius, height), 0.0, FRAC_PI_2),
        (false, false) => line(axis, edge),
        (false, true) => line(edge, axis),
    }
}

/// `segment` run the other way.
fn reversed(segment: Segment) -> Segment {
    match segment {
        Segment::Line { from, to } => line(to, from),
        Segment::Ellipse {
            center,
            radii,
            from,
            to,
        } => ellipse(center, radii, to, from),
        other => other,
    }
}

fn cylinder_tank(d: &CylinderTankDef) -> Result<Revolved, String> {
    positive("a cylinder tank's diameter", d.diameter)?;
    positive("a cylinder tank's length", d.length)?;
    if !(d.dome >= 0.0 && d.dome.is_finite()) || d.aft_skirt < 0.0 || d.forward_skirt < 0.0 {
        return Err("a cylinder tank's dome and skirts cannot be negative".into());
    }
    let r = d.diameter / 2.0;
    let dome = d.dome * r;
    let barrel = d.length - 2.0 * dome;
    if barrel < -1e-9 * d.length {
        return Err(format!(
            "a {} m cylinder tank is shorter than its domes ({:.3} m)",
            d.length,
            2.0 * dome
        ));
    }
    let barrel = barrel.max(0.0);
    // Heights of the aft and forward rims, where the barrel meets the domes.
    let aft = dome.max(d.aft_skirt);
    let forward = aft + barrel;
    let p = DVec2::new;
    let side = |from: f64, to: f64| line(p(r, from), p(r, to));
    let mut walls = Vec::new();
    if d.aft_skirt > 0.0 {
        walls.push(wall(
            "aft_skirt",
            Curve::new(vec![side(aft - d.aft_skirt, aft)])?,
        ));
    }
    walls.push(wall(
        "aft_dome",
        Curve::new(vec![head(r, dome, aft, false)])?,
    ));
    if barrel > 0.0 {
        walls.push(wall("barrel", Curve::new(vec![side(aft, forward)])?));
    }
    walls.push(wall(
        "forward_dome",
        Curve::new(vec![head(r, dome, forward, true)])?,
    ));
    if d.forward_skirt > 0.0 {
        let top = forward + d.forward_skirt;
        walls.push(wall("forward_skirt", Curve::new(vec![side(forward, top)])?));
    }
    // A cell's boundary: up from the axis through `bottom`, up the barrel and in through
    // `top`.
    let cell = |bottom: Segment, from: f64, to: f64, top: Segment| {
        let mut segments = vec![bottom];
        if to > from {
            segments.push(side(from, to));
        }
        segments.push(top);
        Curve::new(segments)
    };
    let regions = match d.common_bulkhead {
        None => vec![region(
            "tank",
            cell(
                head(r, dome, aft, false),
                aft,
                forward,
                head(r, dome, forward, true),
            )?,
            Fill::Cell,
        )],
        Some(c) => {
            if !(c > 0.0 && (c < barrel || (dome > 0.0 && c <= barrel))) {
                return Err(format!(
                    "a common bulkhead sits within the barrel, above the aft rim: {c} m is not \
                     between 0 and {barrel} m"
                ));
            }
            let bulkhead = head(r, dome, aft + c, false);
            walls.push(wall("bulkhead", Curve::new(vec![bulkhead.clone()])?));
            let lower = cell(
                head(r, dome, aft, false),
                aft,
                aft + c,
                reversed(bulkhead.clone()),
            )?;
            let upper = cell(bulkhead, aft + c, forward, head(r, dome, forward, true))?;
            vec![
                region("lower", lower, Fill::Cell),
                region("upper", upper, Fill::Cell),
            ]
        }
    };
    Revolved::new(0.0, TAU, walls, regions)
}

fn cone_tank(d: &ConeTankDef) -> Result<Revolved, String> {
    positive("a cone tank's length", d.length)?;
    let (rb, rt) = (d.bottom_diameter / 2.0, d.top_diameter / 2.0);
    if !(non_negative(rb) && non_negative(rt) && non_negative(d.dome)) || rb + rt == 0.0 {
        return Err(
            "a cone tank's diameters and dome cannot be negative, nor both diameters 0".into(),
        );
    }
    let (db, dt) = (d.dome * rb, d.dome * rt);
    let barrel = d.length - db - dt;
    if barrel <= 0.0 {
        return Err(format!(
            "a {} m cone tank is no longer than its domes",
            d.length
        ));
    }
    let p = DVec2::new;
    let side = line(p(rb, db), p(rt, db + barrel));
    let mut walls = Vec::new();
    let mut boundary = Vec::new();
    if rb > 0.0 {
        let aft = head(rb, db, db, false);
        walls.push(wall("aft_dome", Curve::new(vec![aft.clone()])?));
        boundary.push(aft);
    }
    walls.push(wall("barrel", Curve::new(vec![side.clone()])?));
    boundary.push(side);
    if rt > 0.0 {
        let forward = head(rt, dt, db + barrel, true);
        walls.push(wall("forward_dome", Curve::new(vec![forward.clone()])?));
        boundary.push(forward);
    }
    let tank = region("tank", Curve::new(boundary)?, Fill::Cell);
    Revolved::new(0.0, TAU, walls, vec![tank])
}

fn shell(d: &ShellDef) -> Result<Revolved, String> {
    if d.sections.is_empty() {
        return Err("a shell needs sections".into());
    }
    if !non_negative(d.bottom_diameter) {
        return Err("a shell's diameter cannot be negative".into());
    }
    let (mut r, mut z) = (d.bottom_diameter / 2.0, 0.0);
    let mut segments = Vec::new();
    for section in &d.sections {
        positive("a shell section's length", section.length)?;
        let top = section.top_diameter.map_or(r, |t| t / 2.0);
        if !non_negative(top) || r + top == 0.0 {
            return Err("a shell section's diameters cannot be negative, nor both 0".into());
        }
        let (from, to) = (DVec2::new(r, z), DVec2::new(top, z + section.length));
        segments.push(if section.profile == ShellProfile::Conic || top == r {
            line(from, to)
        } else {
            Segment::Taper {
                from,
                to,
                profile: section.profile,
            }
        });
        (r, z) = (top, z + section.length);
    }
    let bottom = d.bottom_diameter / 2.0;
    let mut walls = vec![wall("wall", Curve::new(segments)?)];
    if d.cap_bottom && bottom > 0.0 {
        let cap = line(DVec2::ZERO, DVec2::new(bottom, 0.0));
        walls.push(wall("bottom", Curve::new(vec![cap])?));
    }
    if d.cap_top && r > 0.0 {
        let cap = line(DVec2::new(r, z), DVec2::new(0.0, z));
        walls.push(wall("top", Curve::new(vec![cap])?));
    }
    let (from, to) = span(d.span)?;
    Revolved::new(from, to, walls, Vec::new())
}

fn wing(d: &WingDef) -> Result<Pieces, String> {
    positive("a wing's root chord", d.root_chord)?;
    positive("a wing's span", d.span)?;
    if !non_negative(d.tip_chord) {
        return Err("a wing's tip chord cannot be negative".into());
    }
    // The airfoil about its quarter chord, lifting towards −y.
    let section: Vec<DVec2> = airfoil(&d.airfoil)?
        .into_iter()
        .map(|p| DVec2::new(p.x - 0.25, -p.y))
        .collect();
    let thick = |chord: f64| match d.airfoil {
        AirfoilDef::Naca { .. } => DVec2::splat(chord),
        AirfoilDef::Plate { .. } => DVec2::new(chord, 1.0),
    };
    let root = Station {
        z: 0.0,
        offset: DVec2::new(0.25 * d.root_chord, 0.0),
        scale: thick(d.root_chord),
        twist: 0.0,
    };
    let tip = Station {
        z: d.span,
        offset: DVec2::new(
            d.sweep + 0.25 * d.tip_chord,
            -d.span * d.dihedral.to_radians().tan(),
        ),
        scale: thick(d.tip_chord),
        twist: d.twist.to_radians(),
    };
    // The tip's leading edge, before the loft is turned.
    let leading = DVec2::from_angle(tip.twist).rotate(DVec2::new(-0.25, 0.0) * tip.scale);
    let tip_edge = (leading + tip.offset).extend(d.span);
    let loft = Lofted::new("wing".into(), section, vec![root, tip], Fill::Solid)?;
    // The loft runs up its z: turn that to the span (+X), its x (the chord) to −Z.
    let turn = DQuat::from_rotation_y(FRAC_PI_2);
    let mut sub = Subpart::new("", Kind::Lofted(loft));
    sub.rotation = turn;
    // Its root meets the hull at its leading edge; its tip can carry a tip tank or a pod.
    let nodes = vec![
        end("root", DVec3::ZERO, DVec3::NEG_X, d.root_chord),
        end("tip", turn * tip_edge, DVec3::X, d.tip_chord),
    ];
    Ok((vec![sub], nodes))
}

fn beam(d: &BeamDef) -> Result<Vec<Subpart>, String> {
    positive("a beam's length", d.length)?;
    let length = d.length;
    let solid = |section| {
        Ok(vec![Subpart::new(
            "",
            Kind::Lofted(straight("beam", section, length, Fill::Solid)?),
        )])
    };
    let round = |outer: f64, inner: f64| -> Result<Vec<Subpart>, String> {
        let p = DVec2::new;
        let corners = if inner > 0.0 {
            vec![
                p(inner, 0.0),
                p(outer, 0.0),
                p(outer, length),
                p(inner, length),
                p(inner, 0.0),
            ]
        } else {
            vec![p(0.0, 0.0), p(outer, 0.0), p(outer, length), p(0.0, length)]
        };
        let beam = region("beam", polyline(&corners)?, Fill::Solid);
        let revolved = Revolved::new(0.0, TAU, Vec::new(), vec![beam])?;
        Ok(vec![Subpart::new("", Kind::Revolved(revolved))])
    };
    match d.section {
        BeamSection::Rectangle { width, height } => {
            positive("a beam's width and height", width.min(height))?;
            solid(rectangle(width, height))
        }
        BeamSection::IBeam {
            width,
            height,
            flange,
            web,
        } => solid(i_beam(width, height, flange, web)?),
        BeamSection::Circle { diameter } => {
            positive("a beam's diameter", diameter)?;
            round(diameter / 2.0, 0.0)
        }
        BeamSection::Tube { diameter, wall } => {
            positive("a tube's diameter", diameter)?;
            positive("a tube's wall", wall)?;
            round(diameter / 2.0, (diameter / 2.0 - wall).max(0.0))
        }
    }
}

fn truss(d: &TrussDef) -> Result<Lattice, String> {
    positive("a truss's diameter", d.bottom_diameter)?;
    positive("a truss's length", d.length)?;
    let top = d.top_diameter.unwrap_or(d.bottom_diameter);
    positive("a truss's top diameter", top)?;
    if d.bays == 0 || d.around < 3 {
        return Err("a truss needs at least one bay and three nodes around".into());
    }
    let (bays, n) = (d.bays as usize, d.around as usize);
    let step = TAU / n as f64;
    let warren = d.bracing == Bracing::Warren;
    let mut nodes = Vec::new();
    for k in 0..=bays {
        let share = k as f64 / bays as f64;
        let radius = (d.bottom_diameter + (top - d.bottom_diameter) * share) / 2.0;
        let shift = if warren && k % 2 == 1 {
            step / 2.0
        } else {
            0.0
        };
        for i in 0..n {
            let (sin, cos) = (step * i as f64 + shift).sin_cos();
            nodes.push(DVec3::new(radius * cos, radius * sin, d.length * share));
        }
    }
    let at = |k: usize, i: usize| k * n + i % n;
    let mut members = Vec::new();
    if d.rings {
        for k in 0..=bays {
            members.extend((0..n).map(|i| [at(k, i), at(k, i + 1)]));
        }
    }
    for k in 0..bays {
        for i in 0..n {
            match d.bracing {
                Bracing::X => members.extend([
                    [at(k, i), at(k + 1, i)],
                    [at(k, i), at(k + 1, i + 1)],
                    [at(k, i + 1), at(k + 1, i)],
                ]),
                Bracing::None => members.push([at(k, i), at(k + 1, i)]),
                // Each node of the shifted ring sits between two of the other.
                Bracing::Warren if k % 2 == 0 => {
                    members.extend([[at(k, i), at(k + 1, i)], [at(k + 1, i), at(k, i + 1)]])
                }
                Bracing::Warren => {
                    members.extend([[at(k, i), at(k + 1, i)], [at(k, i), at(k + 1, i + 1)]])
                }
            }
        }
    }
    Lattice::new("truss".into(), nodes, members)
}

/// A section lofted straight up from z = 0 to `length`.
fn straight(name: &str, section: Vec<DVec2>, length: f64, fill: Fill) -> Result<Lofted, String> {
    let station = |z| Station {
        z,
        offset: DVec2::ZERO,
        scale: DVec2::ONE,
        twist: 0.0,
    };
    Lofted::new(
        name.into(),
        section,
        vec![station(0.0), station(length)],
        fill,
    )
}

fn rectangle(width: f64, height: f64) -> Vec<DVec2> {
    let (x, y) = (width / 2.0, height / 2.0);
    vec![
        DVec2::new(-x, -y),
        DVec2::new(x, -y),
        DVec2::new(x, y),
        DVec2::new(-x, y),
    ]
}

fn regular(sides: u32, diameter: f64) -> Result<Vec<DVec2>, String> {
    if sides < 3 {
        return Err("a regular polygon needs three sides or more".into());
    }
    positive("a polygon's diameter", diameter)?;
    Ok((0..sides)
        .map(|i| DVec2::from_angle(TAU * i as f64 / sides as f64) * diameter / 2.0)
        .collect())
}

fn i_beam(width: f64, height: f64, flange: f64, web: f64) -> Result<Vec<DVec2>, String> {
    positive("an I-beam's width and height", width.min(height))?;
    positive("an I-beam's flanges and web", flange.min(web))?;
    if 2.0 * flange >= height || web >= width {
        return Err(
            "an I-beam's flanges must leave room for its web, and the web be narrower".into(),
        );
    }
    let (x, y, w, f) = (width / 2.0, height / 2.0, web / 2.0, height / 2.0 - flange);
    Ok([
        (-x, -y),
        (x, -y),
        (x, -f),
        (w, -f),
        (w, f),
        (x, f),
        (x, y),
        (-x, y),
        (-x, f),
        (-w, f),
        (-w, -f),
        (-x, -f),
    ]
    .map(|(a, b)| DVec2::new(a, b))
    .to_vec())
}

fn section(d: &SectionDef) -> Result<Vec<DVec2>, String> {
    match d {
        SectionDef::Airfoil(a) => airfoil(a),
        SectionDef::Rectangle { width, height } => {
            positive("a section's width and height", width.min(*height))?;
            Ok(rectangle(*width, *height))
        }
        SectionDef::Ellipse { width, height } => {
            positive("a section's width and height", width.min(*height))?;
            let radii = DVec2::new(width / 2.0, height / 2.0);
            Ok((0..48)
                .map(|i| DVec2::from_angle(TAU * i as f64 / 48.0) * radii)
                .collect())
        }
        SectionDef::Regular { sides, diameter } => regular(*sides, *diameter),
        SectionDef::IBeam {
            width,
            height,
            flange,
            web,
        } => i_beam(*width, *height, *flange, *web),
        SectionDef::Polygon(points) => Ok(points.iter().map(|&p| DVec2::from(p)).collect()),
    }
}

/// An airfoil on a unit chord along +X from its leading edge, the upper surface towards +Y.
/// A plate's thickness is in metres, as stations scale it by the chord along X only.
fn airfoil(d: &AirfoilDef) -> Result<Vec<DVec2>, String> {
    match *d {
        AirfoilDef::Naca {
            thickness,
            camber,
            camber_position,
        } => {
            if !(thickness > 0.0 && thickness <= 0.5) {
                return Err(
                    "a NACA airfoil's thickness is a share of its chord, above 0 and at most 0.5"
                        .into(),
                );
            }
            if !((0.0..=0.1).contains(&camber) && camber_position > 0.0 && camber_position < 1.0) {
                return Err(
                    "a NACA airfoil's camber is at most 0.1, peaking within its chord".into(),
                );
            }
            Ok(naca(thickness, camber, camber_position))
        }
        AirfoilDef::Plate { thickness, bevel } => {
            positive("a plate's thickness", thickness)?;
            if !(bevel > 0.0 && bevel < 0.5) {
                return Err("a plate's bevel is a share of its chord, between 0 and 0.5".into());
            }
            let (b, t) = (bevel, thickness / 2.0);
            Ok([
                (0.0, 0.0),
                (b, -t),
                (1.0 - b, -t),
                (1.0, 0.0),
                (1.0 - b, t),
                (b, t),
            ]
            .map(|(x, y)| DVec2::new(x, y))
            .to_vec())
        }
    }
}

/// A NACA four-digit airfoil with a closed trailing edge, its points bunched towards the
/// edges: from the trailing edge along the lower surface to the leading edge and back along
/// the upper one.
fn naca(thickness: f64, camber: f64, position: f64) -> Vec<DVec2> {
    const POINTS: usize = 32;
    let surface = |x: f64, upper: bool| {
        let half = 5.0
            * thickness
            * (0.2969 * x.sqrt() - 0.1260 * x - 0.3516 * x * x + 0.2843 * x.powi(3)
                - 0.1036 * x.powi(4));
        let (mean, slope) = if x < position {
            (
                camber / (position * position) * (2.0 * position * x - x * x),
                2.0 * camber / (position * position) * (position - x),
            )
        } else {
            let q = (1.0 - position).powi(2);
            (
                camber / q * ((1.0 - 2.0 * position) + 2.0 * position * x - x * x),
                2.0 * camber / q * (position - x),
            )
        };
        let (sin, cos) = slope.atan().sin_cos();
        let side = if upper { 1.0 } else { -1.0 };
        DVec2::new(x - side * half * sin, mean + side * half * cos)
    };
    let x = |i: usize| (1.0 - (PI * i as f64 / POINTS as f64).cos()) / 2.0;
    let mut points: Vec<DVec2> = (1..POINTS).rev().map(|i| surface(x(i), false)).collect();
    points.push(DVec2::ZERO);
    points.extend((1..POINTS).map(|i| surface(x(i), true)));
    points.push(DVec2::new(1.0, 0.0));
    points
}

/// A curve written out: each segment from where the last one ended.
fn curve(d: &CurveDef) -> Result<Curve, String> {
    let mut at = DVec2::from(d.start);
    let mut segments = Vec::new();
    for s in &d.segments {
        let segment = match *s {
            SegmentDef::Line(to) => line(at, to.into()),
            SegmentDef::Ellipse { center, radii, to } => {
                let (center, radii) = (DVec2::from(center), DVec2::from(radii));
                if !(radii.x > 0.0 && radii.y > 0.0) {
                    return Err(format!("an ellipse needs positive radii, not {radii}"));
                }
                let on = (at - center) / radii;
                if (on.length() - 1.0).abs() > 1e-6 {
                    return Err(format!(
                        "{at} is not on the ellipse around {center} with radii {radii}"
                    ));
                }
                // Of the angles the start could be named by, the nearest that still turns.
                let to = to.to_radians();
                let angle = on.y.atan2(on.x);
                let from = [angle - TAU, angle, angle + TAU]
                    .into_iter()
                    .filter(|a| (to - a).abs() > 1e-9 && (to - a).abs() <= TAU + 1e-9)
                    .min_by(|a, b| (to - a).abs().total_cmp(&(to - b).abs()))
                    .ok_or("an ellipse must turn through some angle")?;
                ellipse(center, radii, from, to)
            }
            SegmentDef::Taper { to, profile } => Segment::Taper {
                from: at,
                to: to.into(),
                profile,
            },
            SegmentDef::Bezier { c1, c2, to } => Segment::Bezier {
                points: [at, c1.into(), c2.into(), to.into()],
            },
        };
        at = segment.point(1.0);
        segments.push(segment);
    }
    Curve::new(segments)
}
