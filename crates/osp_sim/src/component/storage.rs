//! Storage: a cell of a part's geometry that holds resources, and the tank walls around it,
//! sized for the pressure they hold.
//!
//! The walls are as thin as their material allows at the design pressure (the tank type's
//! operating pressure plus the hydrostatic head of the full tank at its design
//! acceleration) with the type's safety factor, but never thinner than its minimum gauge.
//! Each wall of revolution is sized by membrane theory: along its curve, the meridional
//! and hoop stress resultants follow from the pressure and the curve's two radii of
//! curvature, so one method sizes cylinders, cones, domes, spheres and tori. Where a wall
//! runs level (a flat bulkhead) it carries the pressure in bending instead, as a plate, and
//! so do a loft's skin and a sector tank's flat sides. Straight walls along the stack also
//! carry its weight: they buckle as thin cylinders (or cones) with the NASA SP-8007
//! knockdown, stiffened by their grid and held taut by their own pressure, which is how
//! balloon tanks stand up at all.

use std::{collections::BTreeMap, f64::consts::PI};

use glam::{DVec2, DVec3};
use serde::{Deserialize, Serialize};

use crate::{
    constants::G0,
    data::{MaterialDef, PartCatalog, ResourceDef, TankTypeDef},
    geometry::{
        Fill, Geometry, Kind,
        curve::{Curve, Segment},
        revolved::Revolved,
    },
    propulsion::CraftError,
};

/// Points along each segment of a wall at which its stress is found, besides its ends.
const SAMPLES: usize = 64;
/// β in σ = β p L² / t², the most a flat plate L across is stressed under pressure p: a
/// disc of radius L simply supported at its rim (ν = 0.3), and a long strip L wide.
const DISC: f64 = 3.0 * 3.3 / 8.0;
const STRIP: f64 = 0.75;
/// The classical buckling stress of a thin cylinder over E t / R, 1/√(3(1 − ν²)) at ν = 0.3.
const CLASSICAL_BUCKLING: f64 = 0.605;

/// Resources held in one of the part's cells.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct StorageDef {
    /// The region or loft it fills, which must be a cell; by default the part's only one.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cell: Option<String>,
    /// Id of a `tank_types` record: how its walls are built.
    #[serde(rename = "type")]
    pub tank_type: String,
    /// Resource -> mass ratio (normalised): it is filled to its usable volume in this
    /// ratio.
    pub contents: BTreeMap<String, f64>,
    /// Acceleration, in g, at which the full tank's hydrostatic pressure is designed for;
    /// the tank type's by default. An upper stage rides its booster's peak acceleration
    /// with full tanks, so it needs more than a first stage, which is only full at liftoff.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub design_load: Option<f64>,
}

/// One resource a tank holds.
#[derive(Debug, Clone)]
pub struct TankContent {
    pub resource: String,
    /// kg when full.
    pub capacity: f64,
    /// kg/m³.
    pub density: f64,
    /// A compressed gas: it fills the tank instead of settling in it.
    pub gas: bool,
    /// How high it stands when full, settled at the bottom of the cell along the part's +Z,
    /// m: the column its hydrostatic pressure comes from. 0 for a gas or a cast solid.
    pub column: f64,
}

/// A storage, sized for its cell.
#[derive(Debug, Clone)]
pub struct Storage {
    /// The cell's element name.
    pub cell: String,
    pub tank_type: String,
    pub material: String,
    /// The cell's volume, m³, and the share of it that holds resources.
    pub volume: f64,
    pub utilization: f64,
    pub contents: Vec<TankContent>,
    /// Operating (ullage) pressure, Pa.
    pub pressure: f64,
    /// What its walls are built for: the operating pressure plus the hydrostatic head of
    /// the full tank at its design acceleration, Pa.
    pub design_pressure: f64,
    /// Internal pressure at which its weakest wall yields, Pa.
    pub max_pressure: f64,
    /// Axial compression its straight walls carry before buckling or yielding, N, with the
    /// help of its operating pressure. `None` with no straight wall along the stack (a
    /// sphere, a torus) or for a cast solid's liner, off the load path.
    pub max_axial_load: Option<f64>,
    /// The walls around its cell, by element name.
    pub walls: Vec<String>,
}

impl Storage {
    /// Internal pressure at the bottom of the cell, Pa, under `acceleration` (m/s² along
    /// the part's +Z) with `fill` the share of each resource left: each column falls as it
    /// drains.
    pub fn pressure_at(&self, acceleration: f64, fill: impl Fn(&str) -> f64) -> f64 {
        self.pressure + head(&self.contents, acceleration, fill)
    }
}

/// The tallest column's pressure: as if each resource had the bottom of the cell.
pub(crate) fn head(contents: &[TankContent], acceleration: f64, fill: impl Fn(&str) -> f64) -> f64 {
    contents
        .iter()
        .map(|c| c.density * acceleration.max(0.0) * c.column * fill(&c.resource))
        .fold(0.0, f64::max)
}

/// How a wall around a cell carries the cell's pressure.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
struct Load {
    /// The largest stress resultant its membrane carries per unit pressure, m.
    membrane: f64,
    /// β L² for its widest flat stretch, which carries the pressure as a plate, m².
    plate: f64,
}

impl Load {
    /// The thinnest wall that stays below `stress` at `pressure`.
    fn thickness(self, pressure: f64, stress: f64) -> f64 {
        (pressure * self.membrane / stress).max((pressure * self.plate / stress).sqrt())
    }

    /// The pressure at which a wall `thickness` thick yields at `stress`.
    fn burst(self, thickness: f64, stress: f64) -> f64 {
        let membrane = if self.membrane > 0.0 {
            stress * thickness / self.membrane
        } else {
            f64::INFINITY
        };
        let plate = if self.plate > 0.0 {
            stress * thickness * thickness / self.plate
        } else {
            f64::INFINITY
        };
        membrane.min(plate)
    }
}

/// A tank wall as one storage sizes it.
#[derive(Debug, Clone)]
pub(crate) struct SizedWall {
    pub element: String,
    pub material: String,
    /// m.
    pub thickness: f64,
    /// Wall, stiffening and insulation, kg/m².
    pub areal_density: f64,
    /// The material's yield strength, Pa.
    pub strength: f64,
    load: Load,
}

/// A storage and the walls it asks for.
#[derive(Debug, Clone)]
pub(crate) struct Sized {
    pub storage: Storage,
    pub walls: Vec<SizedWall>,
}

impl Sized {
    /// Its burst pressure once its walls are as built: each as thick as the storage that
    /// needs it thickest asked for (a common bulkhead serves two).
    pub fn built(&mut self, built: &BTreeMap<String, SizedWall>) {
        self.storage.max_pressure = self
            .walls
            .iter()
            .map(|own| {
                let wall = built.get(&own.element).unwrap_or(own);
                own.load.burst(wall.thickness, wall.strength)
            })
            .fold(f64::INFINITY, f64::min);
    }
}

impl StorageDef {
    /// Size it in `geometry`.
    pub(crate) fn size(
        &self,
        geometry: &Geometry,
        catalog: &PartCatalog,
    ) -> Result<Sized, CraftError> {
        let kind = catalog
            .tank_types
            .iter()
            .find(|t| t.id == self.tank_type)
            .ok_or_else(|| CraftError::UnknownTankType(self.tank_type.clone()))?;
        let material = material(catalog, &kind.material)?;
        let bad = |problem: String| CraftError::BadTank {
            tank_type: kind.id.clone(),
            problem,
        };
        let cells: Vec<_> = geometry
            .elements()
            .into_iter()
            .filter(|e| e.fill == Some(Fill::Cell) && !e.proxy)
            .collect();
        let cell = match &self.cell {
            Some(name) => cells
                .iter()
                .find(|e| &e.name == name)
                .ok_or_else(|| bad(format!("the part has no cell `{name}` to fill")))?,
            None => match cells.as_slice() {
                [only] => only,
                [] => return Err(bad("the part has no cell to fill".into())),
                _ => {
                    return Err(bad(
                        "the part has several cells: name the one it fills".into()
                    ));
                }
            },
        };
        let volume = cell.moments.measure;

        // Fill the usable volume in the given mass ratio.
        let total: f64 = self.contents.values().sum();
        if self.contents.is_empty() || total.is_nan() || total <= 0.0 {
            return Err(bad("it holds nothing".into()));
        }
        let mut contents = Vec::new();
        for (name, ratio) in &self.contents {
            let resource = catalog
                .resources
                .iter()
                .find(|r| &r.id == name)
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
                gas: resource.pressure > 0.0,
                column: 0.0,
            });
        }
        let utilization = kind.utilization.clamp(0.0, 1.0);
        let usable = volume * utilization;
        let mass = usable / contents.iter().map(|c| c.capacity / c.density).sum::<f64>();
        let bottom = settled(geometry, &cell.name, 0.0);
        for content in &mut contents {
            content.capacity *= mass;
            if !content.gas && !kind.cast {
                let own = content.capacity / content.density;
                content.column = settled(geometry, &cell.name, own) - bottom;
            }
        }
        let design_load = self.design_load.unwrap_or(kind.design_load);
        let design_pressure = kind.pressure + head(&contents, design_load * G0, |_| 1.0);

        let strength = material.yield_strength;
        let loads = wall_loads(geometry, &cell.name).map_err(&bad)?;
        let mut walls = Vec::new();
        for (element, load) in loads {
            let sized = load.thickness(kind.safety_factor * design_pressure, strength);
            let thickness = sized.max(kind.min_gauge);
            walls.push(SizedWall {
                element,
                material: material.id.clone(),
                thickness,
                areal_density: material.density * thickness * kind.stiffening.max(1.0)
                    + kind.insulation,
                strength,
                load,
            });
        }
        let max_axial_load = if kind.cast {
            None
        } else {
            axial_load(geometry, &walls, kind, material)
        };
        let mut sized = Sized {
            storage: Storage {
                cell: cell.name.clone(),
                tank_type: kind.id.clone(),
                material: material.id.clone(),
                volume,
                utilization,
                contents,
                pressure: kind.pressure,
                design_pressure,
                max_pressure: f64::INFINITY,
                max_axial_load,
                walls: walls.iter().map(|w| w.element.clone()).collect(),
            },
            walls,
        };
        sized.built(&BTreeMap::new());
        Ok(sized)
    }
}

/// The level, along the part's +Z, of `volume` settled in cell `name`.
fn settled(geometry: &Geometry, name: &str, volume: f64) -> f64 {
    geometry
        .settle(name, DVec3::Z, volume)
        .map_or(0.0, |s| s.level)
}

/// The walls around cell `name`, by element name, and how each carries its pressure.
fn wall_loads(geometry: &Geometry, name: &str) -> Result<Vec<(String, Load)>, String> {
    for sub in &geometry.subparts {
        match &sub.kind {
            Kind::Revolved(r) => {
                let Some(i) = r.regions().iter().position(|g| sub.named(&g.name) == name) else {
                    continue;
                };
                let mut loads: Vec<_> = revolved_loads(r, i)?
                    .into_iter()
                    .map(|(w, load)| (sub.named(&r.walls()[w].name), load))
                    .collect();
                // A sector's flat sides hold the pressure across the cell as plates.
                if !r.is_full() {
                    let boundary = &r.regions()[i].boundary;
                    let points = boundary.tessellate(1e-4 * boundary.size());
                    let (lo, hi) = extent(points.iter().map(|p| p.point));
                    let across = (hi - lo).min_element();
                    let plate = Load {
                        membrane: 0.0,
                        plate: STRIP * across * across,
                    };
                    loads.push((format!("{name}/sides"), plate));
                }
                return Ok(loads);
            }
            Kind::Lofted(l) if sub.named(&l.name) == name => {
                // Its skin's panels span the section's narrower way.
                let (lo, hi) = extent(l.section().iter().copied());
                let across = l
                    .stations()
                    .iter()
                    .map(|s| ((hi - lo) * s.scale).min_element())
                    .fold(0.0, f64::max);
                let plate = Load {
                    membrane: 0.0,
                    plate: STRIP * across * across,
                };
                return Ok(vec![(format!("{name}/skin"), plate)]);
            }
            _ => {}
        }
    }
    Err(format!("no cell `{name}`"))
}

fn extent(points: impl Iterator<Item = DVec2>) -> (DVec2, DVec2) {
    points.fold(
        (DVec2::splat(f64::INFINITY), DVec2::splat(f64::NEG_INFINITY)),
        |(lo, hi), p| (lo.min(p), hi.max(p)),
    )
}

/// The walls (by index) along region `i`'s boundary, and how each carries its pressure
/// there. A wall can run on past the region (a barrel parted by a common bulkhead), but
/// every part of the boundary off the axis must be some wall's.
fn revolved_loads(r: &Revolved, i: usize) -> Result<Vec<(usize, Load)>, String> {
    let region = &r.regions()[i];
    let boundary = &region.boundary;
    let size = boundary.size();
    let near = 1e-6 * size;
    let outline = Polyline::of(boundary, 1e-7 * size);
    let on = |p: DVec2| outline.distance(p) <= near;
    let balance = balance(boundary);
    let mut walls = Vec::new();
    for (w, wall) in r.walls().iter().enumerate() {
        let mut load = Load::default();
        let mut along = false;
        for segment in wall.curve.segments() {
            let inner: Vec<bool> = (0..8)
                .map(|k| on(segment.point((k as f64 + 0.5) / 8.0)))
                .collect();
            if !inner.iter().any(|&o| o) {
                continue;
            }
            along = true;
            if let Segment::Line { from, to } = *segment
                && (to.y - from.y).abs() <= 1e-9 * size
            {
                let across = (to.x - from.x).abs();
                load.plate = load.plate.max(DISC * across * across);
                continue;
            }
            // All along the boundary its ends count too; partly along, only what is on it.
            let whole = inner.iter().all(|&o| o);
            // Its very ends where the stress is defined there, just inside where not (a
            // dome's pole).
            let mut samples = vec![0.0, 1e-6];
            samples.extend((0..SAMPLES).map(|k| (k as f64 + 0.5) / SAMPLES as f64));
            samples.extend([1.0 - 1e-6, 1.0]);
            if !whole {
                samples = samples[2..samples.len() - 2].to_vec();
            }
            let stress = |t: f64| {
                (whole || on(segment.point(t)))
                    .then(|| membrane(segment, t, balance, size))
                    .flatten()
            };
            if let Some(n) = peak(stress, &samples) {
                load.membrane = load.membrane.max(n);
            }
        }
        if along {
            walls.push((w, load));
        }
    }
    let lines: Vec<Polyline> = walls
        .iter()
        .map(|&(w, _)| Polyline::of(&r.walls()[w].curve, 1e-7 * size))
        .collect();
    let inner = boundary
        .segments()
        .iter()
        .flat_map(|s| (0..8).map(move |k| s.point((k as f64 + 0.5) / 8.0)));
    if let Some(open) = inner
        .into_iter()
        .find(|&p| p.x > near && lines.iter().all(|l| l.distance(p) > near))
    {
        return Err(format!(
            "cell `{}` is open at {open}: walls must enclose it",
            region.name
        ));
    }
    Ok(walls)
}

/// The largest of `f` at `samples` (rising, within 0..1), sharpened by a golden-section
/// search between the best one's neighbours.
fn peak(f: impl Fn(f64) -> Option<f64>, samples: &[f64]) -> Option<f64> {
    let values: Vec<Option<f64>> = samples.iter().map(|&t| f(t)).collect();
    let (best, top) = values
        .iter()
        .enumerate()
        .filter_map(|(i, v)| v.map(|v| (i, v)))
        .max_by(|a, b| a.1.total_cmp(&b.1))?;
    let value = |t: f64| f(t).unwrap_or(f64::NEG_INFINITY);
    let ratio = (5f64.sqrt() - 1.0) / 2.0;
    let (mut a, mut b) = (
        samples[best.saturating_sub(1)],
        samples[(best + 1).min(samples.len() - 1)],
    );
    let (mut c, mut d) = (b - ratio * (b - a), a + ratio * (b - a));
    let (mut fc, mut fd) = (value(c), value(d));
    for _ in 0..48 {
        if fc > fd {
            (b, d, fd) = (d, c, fc);
            c = b - ratio * (b - a);
            fc = value(c);
        } else {
            (a, c, fc) = (c, d, fd);
            d = a + ratio * (b - a);
            fd = value(d);
        }
    }
    Some(top.max(fc).max(fd))
}

/// r_c², where the pressure on a cell's walls balances along the axis: the meridional
/// stress resultant at radius r is p (r² − r_c²) / (2 r sin φ). A cell that reaches the
/// axis balances there; one around it (a torus) at its crowns, where its wall runs level
/// and so can carry nothing along the axis. Without smooth crowns (a ring tank with flat
/// ends, whose plates carry the difference) it splits the difference.
fn balance(boundary: &Curve) -> f64 {
    let points: Vec<DVec2> = boundary
        .tessellate(1e-4 * boundary.size())
        .iter()
        .map(|p| p.point)
        .collect();
    let (lo, hi) = extent(points.iter().copied());
    if lo.x <= 1e-9 * boundary.size() {
        return 0.0;
    }
    let crowns = boundary.crowns();
    if crowns.is_empty() {
        (lo.x * lo.x + hi.x * hi.x) / 2.0
    } else {
        crowns.iter().map(|c| c.x * c.x).sum::<f64>() / crowns.len() as f64
    }
}

/// The larger of the meridional and hoop stress resultants per unit pressure at `t` along
/// `segment`, m. With the normal on the right of the way the curve runs, r its radius, n_r
/// the normal's share away from the axis and κ the curve's own curvature, axial balance
/// gives N_φ = (r² − r_c²) / (2 r n_r), and the pressure across the wall (Laplace)
/// N_φ κ + N_θ n_r / r = 1. Turning the curve round flips the signs of both, not their
/// sizes. `None` on the axis or where the wall runs level.
fn membrane(segment: &Segment, t: f64, balance: f64, size: f64) -> Option<f64> {
    let (p, d) = (segment.point(t), segment.derivative(t));
    let speed = d.length();
    let normal = d.y / speed;
    if !(p.x > 1e-9 * size && normal.abs() > 1e-9 && speed > 0.0) {
        return None;
    }
    let meridional = (p.x * p.x - balance) / (2.0 * p.x * normal);
    let hoop = p.x * (1.0 - meridional * segment.curvature(t)) / normal;
    Some(meridional.abs().max(hoop.abs()))
}

/// The axial compression the straight walls around the cell carry together, N: the
/// weakest of them, each buckling as a thin cylinder (or cone, by its half angle α) with
/// the SP-8007 knockdown or yielding, plus what the operating pressure takes off.
fn axial_load(
    geometry: &Geometry,
    walls: &[SizedWall],
    kind: &TankTypeDef,
    material: &MaterialDef,
) -> Option<f64> {
    let mut weakest: Option<f64> = None;
    for sub in &geometry.subparts {
        let Kind::Revolved(r) = &sub.kind else {
            continue;
        };
        for wall in r.walls() {
            let Some(sized) = walls.iter().find(|w| w.element == sub.named(&wall.name)) else {
                continue;
            };
            let t = sized.thickness;
            for segment in wall.curve.segments() {
                let Segment::Line { from, to } = *segment else {
                    continue;
                };
                let (rise, length) = ((to.y - from.y).abs(), (to - from).length());
                if rise <= 1e-9 * length {
                    continue;
                }
                let cos = rise / length;
                let (small, large) = (from.x.min(to.x), from.x.max(to.x));
                if small <= 0.0 {
                    continue;
                }
                let radius = large / cos;
                let knockdown = 1.0 - 0.901 * (1.0 - (-(radius / t).sqrt() / 16.0).exp());
                let buckling = 2.0
                    * PI
                    * knockdown
                    * CLASSICAL_BUCKLING
                    * kind.buckling_factor
                    * material.elastic_modulus
                    * t
                    * t
                    * cos
                    * cos;
                let yielding = 2.0 * PI * small * t * cos * material.yield_strength;
                let load = buckling.min(yielding) + PI * small * small * kind.pressure;
                weakest = Some(weakest.map_or(load, |w: f64| w.min(load)));
            }
        }
    }
    weakest
}

/// A curve as points close along it, for distances to it.
struct Polyline(Vec<DVec2>);

impl Polyline {
    fn of(curve: &Curve, tolerance: f64) -> Self {
        Self(
            curve
                .tessellate(tolerance)
                .iter()
                .map(|p| p.point)
                .collect(),
        )
    }

    fn distance(&self, p: DVec2) -> f64 {
        self.0
            .windows(2)
            .map(|w| {
                let (a, b) = (w[0], w[1]);
                let along = b - a;
                let t = if along.length_squared() > 0.0 {
                    ((p - a).dot(along) / along.length_squared()).clamp(0.0, 1.0)
                } else {
                    0.0
                };
                (a + along * t - p).length()
            })
            .fold(f64::INFINITY, f64::min)
    }
}

pub(crate) fn material<'a>(
    catalog: &'a PartCatalog,
    id: &str,
) -> Result<&'a MaterialDef, CraftError> {
    catalog
        .materials
        .iter()
        .find(|m| m.id == id)
        .ok_or_else(|| CraftError::UnknownMaterial(id.into()))
}

fn cannot_hold(kind: &TankTypeDef, resource: &ResourceDef) -> Option<String> {
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
