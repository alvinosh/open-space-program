//! Aerodynamics after Ferram Aerospace Research (FAR), the realistic aerodynamics mod for
//! Kerbal Space Program.
//!
//! As in FAR, the vessel is cut into thin sections along its axis, and every section feels
//! the air it actually meets: the vessel's velocity through the air plus its own rotation.
//! Summing the sections' forces about the centre of mass gives the aerodynamic torque, so
//! static stability (where the centre of pressure sits) and the damping of any rotation come
//! out of the shape alone. FAR voxelizes arbitrary parts to find its sections; a stack of
//! round stages has an exact outline ([`Shape::new`]).
//!
//! Each section feels:
//!
//! - **Normal force** (Jorgensen, NASA TR R-474): slender-body potential flow where the
//!   cross-section grows along the flow (a nose lifts towards where it points, a boattail
//!   pushes the other way, a separated base does nothing), fading on steep slopes as
//!   Newtonian flow takes over (a flat face only pushes along its normal), plus viscous
//!   crossflow drag on its side, with Allen's factor for the body's fineness and a drag
//!   coefficient that rises through crossflow Mach 1.
//! - **Axial force**: skin friction (flat plate, Reynolds and Mach corrected), and pressure
//!   on the slopes and faces the flow meets: a stagnation fit subsonic; supersonic, the
//!   tangent-cone pressure of Rasmussen's hypersonic similarity solution, capped by the
//!   stagnation pressure behind a normal shock for blunt faces (modified Newtonian); and a
//!   transonic blend between. Where the outline turns away from the flow more sharply than
//!   the flow can follow (a base, a step down, the sides of a capsule behind its heat
//!   shield), the flow separates and the wake's base pressure pulls back on everything
//!   behind the corner that keeps narrowing, except on the base itself while engines fire
//!   (their plumes fill it).
//!
//! Fins are lifting surfaces at their aerodynamic centres: each fin's normal force at its
//! own angle of attack ([`FinSet::normal_coefficient`]), times Barrowman's body interference
//! factor, plus its own drag.
//!
//! Wings (and tailplanes and tail fins: [`WingDef`]) are worked out strip by strip along
//! their span, each strip at its own angle of attack (so a rolling wing damps its roll and
//! an aileron only lifts where it is): DATCOM's lift slope for the whole wing's aspect ratio
//! and sweep, linear up to the stall, Viterna and Corrigan's post-stall fit beyond it, and
//! induced drag for the aspect ratio on top of the profile drag (skin friction with a
//! thickness form factor, and wave drag past the drag-divergence Mach number). A wing sheds
//! downwash onto the surfaces behind it, which takes some of the bite out of a tailplane.
//! A control surface adds its flap effectiveness (thin-airfoil theory, less a fifth for the
//! hinge's real flow) times its deflection to its strips' angle of attack; which way it deflects for a rotation command
//! follows from where it is ([`deflection`]).
//!
//! Engine bells below the base sit in its wake and are left out.

use std::f64::consts::{FRAC_PI_2, PI, TAU};

use glam::DVec3;

use crate::{
    atmosphere::AtmosphereSample,
    data::{ControlSurfaceDef, TankShape, WingDef},
    gear,
    propulsion::Stage,
    structure::{FinSet, lift_slope},
};

/// Nose cone length over the diameter of the stage it tops (the renderer draws the same).
pub const NOSE_FINENESS: f64 = 1.3;
/// Points along a tangent-ogive nose.
const NOSE_STEPS: usize = 12;
/// A section is at most this share of the outline's length.
const MAX_SECTION: f64 = 1.0 / 64.0;
/// Turn of the outline away from the flow (rad, about 10°) at which the flow starts to
/// separate; it has separated completely at twice this.
const SEPARATION_TURN: f64 = 0.175;
/// Ratio of specific heats of the air.
const GAMMA: f64 = 1.4;
/// Subsonic pressure over a flat face meeting the flow, as a share of the stagnation
/// pressure.
const FLAT_FACE_PRESSURE: f64 = 0.8;
/// The transonic blend runs between these Mach numbers.
const TRANSONIC: (f64, f64) = (0.8, 1.2);
/// Reynolds number at which the boundary layer turns turbulent.
const TRANSITION_REYNOLDS: f64 = 5e5;
/// Strips along each wing panel.
const STRIPS: usize = 8;
/// Oswald's span efficiency for induced drag.
const OSWALD: f64 = 0.85;
/// Half-width of the blend from attached to separated flow around the stall, rad (2°).
const STALL_BLEND: f64 = 0.035;
/// A real hinged flap does about this share of what thin-airfoil theory says (the gap and
/// the boundary layer at the hinge).
const FLAP_VISCOUS_FACTOR: f64 = 0.8;
/// Allen's crossflow factor η (a finite cylinder's crossflow drag over an infinite one's)
/// against fineness (length over diameter).
const CROSSFLOW_FACTOR: [(f64, f64); 5] = [
    (2.0, 0.55),
    (5.0, 0.62),
    (10.0, 0.66),
    (20.0, 0.72),
    (40.0, 0.8),
];
/// Crossflow drag coefficient of a circular cylinder against crossflow Mach number
/// (Jorgensen, after Gowen and Perkins).
const CROSSFLOW_DRAG: [(f64, f64); 9] = [
    (0.4, 1.2),
    (0.7, 1.4),
    (0.9, 1.75),
    (1.0, 1.9),
    (1.2, 1.8),
    (1.6, 1.6),
    (2.0, 1.5),
    (3.0, 1.4),
    (5.0, 1.3),
];
/// Base pressure coefficient behind a blunt base (its magnitude) against Mach number, for
/// a turbulent boundary layer (Hoerner); beyond the table it is three quarters of a vacuum.
const BASE_PRESSURE: [(f64, f64); 7] = [
    (0.6, 0.12),
    (0.8, 0.15),
    (0.95, 0.22),
    (1.1, 0.32),
    (1.5, 0.28),
    (2.0, 0.21),
    (3.0, 0.12),
];

/// One slice of the outline: a frustum, or a flat ring where the radius steps.
#[derive(Debug, Clone, Copy, PartialEq)]
struct Section {
    /// Heights of its aft and forward ends above the base of the stack, m.
    aft: f64,
    fore: f64,
    /// Radii at those ends, m.
    aft_radius: f64,
    fore_radius: f64,
    /// The flat bottom of the stack, which engine plumes fill.
    base: bool,
    /// How far the flow has separated from it (0..1), flowing from the nose and from the
    /// base.
    separated: [f64; 2],
}

impl Section {
    fn center(&self) -> f64 {
        (self.aft + self.fore) / 2.0
    }

    /// How much the cross-section grows from the forward end to the aft end, m².
    fn taper(&self) -> f64 {
        PI * (self.aft_radius.powi(2) - self.fore_radius.powi(2))
    }

    /// Angle of the outline to the axis: 0 along a cylinder, π/2 across a flat ring.
    fn slope(&self) -> f64 {
        (self.fore_radius - self.aft_radius)
            .abs()
            .atan2(self.fore - self.aft)
    }

    /// Area seen side on (mean diameter times length), m².
    fn planform(&self) -> f64 {
        (self.aft_radius + self.fore_radius) * (self.fore - self.aft)
    }

    /// Surface area, m².
    fn wetted(&self) -> f64 {
        let slant = (self.fore - self.aft).hypot(self.fore_radius - self.aft_radius);
        PI * (self.aft_radius + self.fore_radius) * slant
    }
}

#[derive(Debug, Clone)]
struct Fins {
    set: FinSet,
    /// Height of their stage's base above the base of the stack, m.
    base: f64,
    /// Radius of the hull they stand on, m.
    hull_radius: f64,
}

/// One panel of a wing (a mirrored wing has two), in stack coordinates: X to the pilot's
/// right, Y to their feet, Z up the stack from its base.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Panel {
    /// Leading edge at the root and at the tip, m.
    pub root: DVec3,
    pub tip: DVec3,
    pub root_chord: f64,
    pub tip_chord: f64,
    /// The lifting side: angle of attack, incidence and a trailing edge deflected the other
    /// way all push the panel this way.
    pub normal: DVec3,
}

impl Panel {
    /// The panels of `wing` on a stage `hull_radius` wide whose base is `base` up the stack.
    pub fn of(wing: &WingDef, hull_radius: f64, base: f64) -> Vec<Panel> {
        let direction = |degrees: f64| {
            let (s, c) = degrees.to_radians().sin_cos();
            DVec3::new(c, s, 0.0)
        };
        let root = direction(wing.angle) * hull_radius + DVec3::Z * (base + wing.position);
        let along = direction(wing.angle - wing.dihedral);
        let panel = Panel {
            root,
            tip: root + along * wing.span - DVec3::Z * wing.sweep,
            root_chord: wing.root_chord,
            tip_chord: wing.tip_chord,
            normal: along.cross(DVec3::Z),
        };
        let mut panels = vec![panel];
        if wing.mirror {
            let flip = |v: DVec3| DVec3::new(-v.x, v.y, v.z);
            panels.push(Panel {
                root: flip(panel.root),
                tip: flip(panel.tip),
                normal: flip(panel.normal),
                ..panel
            });
        }
        panels
    }

    /// From the root to the tip, across the flow, m.
    pub fn span(&self) -> f64 {
        (self.tip - self.root).truncate().length()
    }

    pub fn area(&self) -> f64 {
        (self.root_chord + self.tip_chord) / 2.0 * self.span()
    }

    /// Leading edge and chord `eta` of the way out along the span.
    pub fn station(&self, eta: f64) -> (DVec3, f64) {
        (
            self.root.lerp(self.tip, eta),
            self.root_chord + (self.tip_chord - self.root_chord) * eta,
        )
    }

    /// Quarter-chord point of the mean aerodynamic chord: where its lift acts.
    pub fn aerodynamic_center(&self) -> DVec3 {
        let (root, tip) = (self.root_chord, self.tip_chord);
        let (leading_edge, chord) = self.station((root + 2.0 * tip) / (3.0 * (root + tip)));
        leading_edge - DVec3::Z * chord / 4.0
    }
}

/// How far a control surface on `panel` deflects for the rotation `command` (−1..1 about
/// each vessel axis), rad, positive with the trailing edge away from the lifting side: the
/// way that turns the vessel about the commanded axes it answers to, with the centre of mass
/// `center_of_mass` up the stack.
pub fn deflection(
    control: &ControlSurfaceDef,
    panel: &Panel,
    command: DVec3,
    center_of_mass: f64,
) -> f64 {
    let turn = (panel.aerodynamic_center() - DVec3::Z * center_of_mass).cross(panel.normal);
    let sum: f64 = control
        .axes
        .iter()
        .map(|axis| axis.index())
        .filter(|&k| turn[k].abs() > 1e-6 * turn.length())
        .map(|k| command[k] * turn[k].signum())
        .sum();
    control.deflection.to_radians() * sum.clamp(-1.0, 1.0)
}

/// A slice of a wing panel across its span.
#[derive(Debug, Clone, Copy)]
struct Strip {
    leading_edge: DVec3,
    chord: f64,
    area: f64,
    /// Part of it is the control surface.
    flap: bool,
}

/// A wing panel as the air sees it.
#[derive(Debug, Clone)]
struct Surface {
    panel: Panel,
    strips: Vec<Strip>,
    /// Aspect ratio of the panel with its mirror image (its twin, or its image in the hull).
    aspect: f64,
    /// Tangent of the mid-chord sweep.
    tan_sweep: f64,
    /// Rad.
    incidence: f64,
    /// Thickness over chord.
    thickness: f64,
    /// Lift coefficient at the stall.
    max_lift: f64,
    control: Option<ControlSurfaceDef>,
    /// Angle of attack a strip gains per radian of control deflection.
    effectiveness: f64,
    /// Surfaces ahead that wash this one down, and how much their lift lines up with its.
    downwash_from: Vec<(usize, f64)>,
}

impl Surface {
    fn new(wing: &WingDef, panel: Panel) -> Self {
        let span = panel.span();
        let area = panel.area();
        let chord_change = wing.tip_chord - wing.root_chord;
        let tan_quarter = (wing.sweep + chord_change / 4.0) / span;
        let section_max = (0.8 + 5.0 * wing.thickness).min(1.6);
        let effectiveness = wing.control.as_ref().map_or(0.0, |c| {
            let hinge = (2.0 * c.chord.clamp(0.0, 1.0) - 1.0).acos();
            FLAP_VISCOUS_FACTOR * (1.0 - (hinge - hinge.sin()) / PI)
        });
        let strips = (0..STRIPS)
            .map(|k| {
                let eta = (k as f64 + 0.5) / STRIPS as f64;
                let (leading_edge, chord) = panel.station(eta);
                Strip {
                    leading_edge,
                    chord,
                    area: chord * span / STRIPS as f64,
                    flap: wing
                        .control
                        .as_ref()
                        .is_some_and(|c| eta >= c.span[0] && eta <= c.span[1]),
                }
            })
            .collect();
        Self {
            panel,
            strips,
            aspect: 2.0 * span * span / area,
            tan_sweep: (wing.sweep + chord_change / 2.0) / span,
            incidence: wing.incidence.to_radians(),
            thickness: wing.thickness,
            max_lift: 0.9 * section_max * tan_quarter.atan().cos(),
            control: wing.control.clone(),
            effectiveness,
            downwash_from: Vec::new(),
        }
    }

    /// Force and torque on it, and its mean lift coefficient. `downwash` turns the flow
    /// down onto it, rad.
    fn loads(
        &self,
        flow: &Flow,
        center_of_mass: f64,
        downwash: f64,
        add: &mut impl FnMut(DVec3, DVec3),
    ) -> f64 {
        let air = &flow.air;
        let rho = air.density;
        let speed = flow.velocity.length();
        let mach = speed / air.speed_of_sound;
        let slope = lift_slope(self.aspect, self.tan_sweep, mach);
        let mean_chord = self.panel.area() / self.panel.span();
        let reynolds = rho * speed * mean_chord / viscosity(air.temperature);
        let form = 1.0 + 2.0 * self.thickness + 60.0 * self.thickness.powi(4);
        let profile =
            2.0 * skin_friction(reynolds, mach) * form + airfoil_wave_drag(mach, self.thickness);
        let flap = match &self.control {
            Some(control) => {
                self.effectiveness * deflection(control, &self.panel, flow.controls, center_of_mass)
            }
            None => 0.0,
        };
        // Lift acts a quarter of the chord back subsonic, moving to half supersonic.
        let share = 0.25 + 0.25 * (mach - 1.0).clamp(0.0, 1.0);
        let n = self.panel.normal;
        let (mut lift_sum, mut area_sum) = (0.0, 0.0);
        for strip in &self.strips {
            let point = strip.leading_edge - DVec3::Z * (share * strip.chord + center_of_mass);
            let u = flow.velocity + flow.angular_velocity.cross(point);
            // The flow in the plane of the chord and the normal.
            let (across, along) = (u.dot(n), u.z);
            let speed = across.hypot(along);
            if speed < 1e-9 {
                continue;
            }
            let mut alpha = (-across).atan2(along) + self.incidence - downwash;
            if strip.flap {
                alpha += flap;
            }
            let alpha = alpha.sin().atan2(alpha.cos());
            let (cl, cd) = wing_coefficients(alpha, slope, self.max_lift, self.aspect, profile);
            let q = 0.5 * rho * speed * speed;
            let lift = (n * along - DVec3::Z * across) / speed;
            let drag = -(DVec3::Z * along + n * across) / speed;
            add(point, q * strip.area * (cl * lift + cd * drag));
            lift_sum += cl * strip.area;
            area_sum += strip.area;
        }
        if area_sum > 0.0 {
            lift_sum / area_sum
        } else {
            0.0
        }
    }
}

/// A vessel as the air sees it: its outline cut into sections, its fins and its wings.
#[derive(Debug, Clone, Default)]
pub struct Shape {
    sections: Vec<Section>,
    fins: Vec<Fins>,
    surfaces: Vec<Surface>,
    /// Landing gear hanging in the airflow: where (stack coordinates) and its drag area.
    gear: Vec<(DVec3, f64)>,
    /// Length and widest diameter of the outline, m.
    length: f64,
    diameter: f64,
}

/// How the vessel meets the air.
#[derive(Debug, Clone, Copy)]
pub struct Flow {
    /// The vessel's velocity through the air, vessel axes, m/s.
    pub velocity: DVec3,
    /// Its rotation relative to the air, vessel axes, rad/s.
    pub angular_velocity: DVec3,
    pub air: AtmosphereSample,
    /// Engines are firing out of the base.
    pub plume: bool,
    /// The rotation command (−1..1 about each vessel axis) the control surfaces follow.
    pub controls: DVec3,
}

/// The air's force on a vessel and its torque about the centre of mass, vessel axes.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Loads {
    /// N.
    pub force: DVec3,
    /// N·m.
    pub torque: DVec3,
}

impl Shape {
    /// The shape of a stack of stages, bottom first.
    pub fn new(stages: &[Stage]) -> Self {
        let mut fins = Vec::new();
        let mut surfaces = Vec::new();
        let mut base = 0.0;
        for stage in stages {
            if let Some(set) = &stage.fins {
                fins.push(Fins {
                    set: set.clone(),
                    base,
                    hull_radius: stage.diameter / 2.0,
                });
            }
            for wing in &stage.wings {
                for panel in Panel::of(wing, stage.diameter / 2.0, base) {
                    surfaces.push(Surface::new(wing, panel));
                }
            }
            base += stage.length;
        }
        let mut shape = Self::from_outline(&outline(stages), fins);
        shape.surfaces = with_downwash(surfaces);
        shape.gear = gear::legs(stages)
            .iter()
            .map(|leg| (leg.axle(leg.length / 2.0), leg.drag_area()))
            .collect();
        shape
    }

    /// Sections along `outline`, (height, radius) points from the bottom of the base up.
    fn from_outline(outline: &[(f64, f64)], fins: Vec<Fins>) -> Self {
        let length = outline.last().map_or(0.0, |p| p.0);
        let diameter = 2.0 * outline.iter().map(|p| p.1).fold(0.0, f64::max);
        let longest = (length * MAX_SECTION).max(1e-3);
        let mut sections = Vec::new();
        for (i, w) in outline.windows(2).enumerate() {
            let [(z0, r0), (z1, r1)] = [w[0], w[1]];
            if z1 == z0 && r1 == r0 {
                continue;
            }
            let pieces = ((z1 - z0) / longest).ceil().max(1.0) as usize;
            for k in 0..pieces {
                let (f0, f1) = (k as f64 / pieces as f64, (k + 1) as f64 / pieces as f64);
                sections.push(Section {
                    aft: z0 + (z1 - z0) * f0,
                    fore: z0 + (z1 - z0) * f1,
                    aft_radius: r0 + (r1 - r0) * f0,
                    fore_radius: r0 + (r1 - r0) * f1,
                    base: i == 0,
                    separated: [0.0; 2],
                });
            }
        }
        separate(&mut sections, 0, true);
        separate(&mut sections, 1, false);
        Self {
            sections,
            fins,
            surfaces: Vec::new(),
            gear: Vec::new(),
            length,
            diameter,
        }
    }

    /// Frontal area of the widest section, m²: what drag coefficients are quoted against.
    pub fn reference_area(&self) -> f64 {
        PI * (self.diameter / 2.0).powi(2)
    }

    /// The air's force and torque on the vessel, its centre of mass `center_of_mass` above
    /// the base of the stack.
    pub fn loads(&self, flow: &Flow, center_of_mass: f64) -> Loads {
        let air = &flow.air;
        let mut loads = Loads::default();
        if air.density <= 0.0 || air.speed_of_sound <= 0.0 {
            return loads;
        }
        let rho = air.density;
        let mut add = |point: DVec3, force: DVec3| {
            loads.force += force;
            loads.torque += point.cross(force);
        };
        let mach = flow.velocity.length() / air.speed_of_sound;
        let reynolds = rho * flow.velocity.length() * self.length / viscosity(air.temperature);
        let friction = skin_friction(reynolds, mach);
        let eta = lookup(&CROSSFLOW_FACTOR, self.length / self.diameter.max(1e-9));

        for s in &self.sections {
            let point = DVec3::Z * (s.center() - center_of_mass);
            let u = flow.velocity + flow.angular_velocity.cross(point);
            let speed = u.length();
            if speed < 1e-9 {
                continue;
            }
            let local_mach = speed / air.speed_of_sound;
            let cross = DVec3::new(u.x, u.y, 0.0);
            let cross_speed = cross.length();
            let (cos, sin) = (u.z / speed, cross_speed / speed);
            // How much the section widens along the flow, m², and whether the flow has
            // left its surface.
            let forward = u.z >= 0.0;
            let growth = if forward { s.taper() } else { -s.taper() };
            let slope = s.slope();
            let separated = s.separated[usize::from(!forward)];

            // Normal force, along the crossflow: slender-body potential flow (with
            // Jorgensen's cos(α/2) taking it off broadside), then viscous crossflow drag.
            if cross_speed > 1e-9 {
                let q = 0.5 * rho * speed * speed;
                let lead = cos.abs().acos();
                let potential = q
                    * growth
                    * (2.0 * sin * cos).abs()
                    * (lead / 2.0).cos()
                    * slope.cos().powi(2)
                    * (1.0 - separated);
                let crossflow_mach = cross_speed / air.speed_of_sound;
                let eta = eta + (1.0 - eta) * ((crossflow_mach - 0.8) / 0.4).clamp(0.0, 1.0);
                let viscous = eta
                    * lookup(&CROSSFLOW_DRAG, crossflow_mach)
                    * s.planform()
                    * 0.5
                    * rho
                    * cross_speed
                    * cross_speed;
                add(point, -cross / cross_speed * (potential + viscous));
            }

            // Axial force, against the axial flow: friction on the skin, pressure on the
            // slopes and faces.
            let pressure = if growth > 0.0 {
                compression(local_mach, slope)
            } else if s.base && flow.plume {
                0.0
            } else {
                let base = base_pressure(local_mach);
                (1.0 - separated) * compression(local_mach, slope).min(base) + separated * base
            };
            let axial_q = 0.5 * rho * u.z * u.z;
            let axial = axial_q * (friction * s.wetted() + pressure * growth.abs());
            add(point, DVec3::Z * -u.z.signum() * axial);
        }

        for fins in &self.fins {
            let (set, d) = (&fins.set, &fins.set.def);
            let (out, height) = set.aerodynamic_center(mach);
            let interference = 1.0 + fins.hull_radius / (d.span + fins.hull_radius);
            let drag_area = set.drag_area() / d.count as f64;
            for k in 0..d.count {
                // Fin k stands out along `radial`; the air pushes it along `normal`.
                let (s, c) = (TAU * k as f64 / d.count as f64).sin_cos();
                let (radial, normal) = (DVec3::new(c, s, 0.0), DVec3::new(-s, c, 0.0));
                let point = radial * (fins.hull_radius + out)
                    + DVec3::Z * (fins.base + height - center_of_mass);
                let u = flow.velocity + flow.angular_velocity.cross(point);
                let speed = u.length();
                if speed < 1e-9 {
                    continue;
                }
                let across = u.dot(normal);
                let q = 0.5 * rho * (across * across + u.z * u.z);
                let coefficient =
                    set.normal_coefficient(speed / air.speed_of_sound, across.atan2(u.z));
                let lift = q * set.area * coefficient * interference;
                add(point, -normal * lift - 0.5 * rho * speed * drag_area * u);
            }
        }

        for &(at, drag_area) in &self.gear {
            let point = at - DVec3::Z * center_of_mass;
            let u = flow.velocity + flow.angular_velocity.cross(point);
            add(point, -0.5 * rho * u.length() * drag_area * u);
        }

        // Wings, front to back in effect: first each one's lift on its own, for the
        // downwash it sheds, then all of them in the downwash from those ahead.
        let lifts: Vec<f64> = if self.surfaces.iter().any(|s| !s.downwash_from.is_empty()) {
            self.surfaces
                .iter()
                .map(|s| s.loads(flow, center_of_mass, 0.0, &mut |_, _| {}))
                .collect()
        } else {
            vec![0.0; self.surfaces.len()]
        };
        for surface in &self.surfaces {
            let downwash: f64 = surface
                .downwash_from
                .iter()
                .map(|&(j, alignment)| 2.0 * lifts[j] / (PI * self.surfaces[j].aspect) * alignment)
                .sum();
            surface.loads(flow, center_of_mass, downwash, &mut add);
        }
        loads
    }

    /// The largest torque the control surfaces can give about each vessel axis in `flow`,
    /// N·m.
    pub fn control_authority(&self, flow: &Flow, center_of_mass: f64) -> DVec3 {
        let air = &flow.air;
        if air.density <= 0.0 || air.speed_of_sound <= 0.0 {
            return DVec3::ZERO;
        }
        let q = 0.5 * air.density * flow.velocity.length_squared();
        let mach = flow.velocity.length() / air.speed_of_sound;
        let mut authority = DVec3::ZERO;
        for surface in &self.surfaces {
            let Some(control) = &surface.control else {
                continue;
            };
            let slope = lift_slope(surface.aspect, surface.tan_sweep, mach);
            let per_area = q * slope * surface.effectiveness * control.deflection.to_radians();
            for strip in surface.strips.iter().filter(|s| s.flap) {
                let point = strip.leading_edge - DVec3::Z * center_of_mass;
                let turn = point.cross(surface.panel.normal).abs() * per_area * strip.area;
                for axis in &control.axes {
                    let k = axis.index();
                    authority[k] += turn[k];
                }
            }
        }
        authority
    }
}

/// Note, for each wing panel, the panels ahead whose downwash reaches it: their lift lines
/// up with its own, they are at least a chord ahead, and it is within their span.
fn with_downwash(mut surfaces: Vec<Surface>) -> Vec<Surface> {
    let panels: Vec<Panel> = surfaces.iter().map(|s| s.panel).collect();
    for (i, surface) in surfaces.iter_mut().enumerate() {
        let here = panels[i].aerodynamic_center();
        for (j, ahead) in panels.iter().enumerate() {
            let alignment = panels[i].normal.dot(ahead.normal);
            let along = (ahead.tip - ahead.root)
                .truncate()
                .extend(0.0)
                .normalize_or_zero();
            let reach = here.dot(along);
            let chord = ahead.area() / ahead.span();
            if i != j
                && alignment.abs() > 0.7
                && ahead.aerodynamic_center().z - here.z > chord
                && reach >= 0.0
                && reach <= ahead.tip.dot(along)
            {
                surface.downwash_from.push((j, alignment));
            }
        }
    }
    surfaces
}

/// Lift and drag coefficients of a wing section at angle of attack `alpha` (rad, −π..π)
/// with lift slope `slope`: linear up to the stall at `max_lift` with induced drag for
/// `aspect` on top of `profile` drag; past the stall, Viterna and Corrigan's lift (from the
/// stall down to a flat plate's broadside), with the separated flow pushing square to the
/// chord (drag = lift · tan α); and a flat plate flying backwards.
fn wing_coefficients(
    alpha: f64,
    slope: f64,
    max_lift: f64,
    aspect: f64,
    profile: f64,
) -> (f64, f64) {
    // Drag of a flat plate of this aspect ratio broadside to the flow.
    let plate = 1.11 + 0.018 * aspect.min(50.0);
    let induced = |cl: f64| cl * cl / (PI * OSWALD * aspect);
    let (sign, a) = (alpha.signum(), alpha.abs());
    if a > FRAC_PI_2 {
        return (
            plate * alpha.sin() * alpha.cos(),
            profile + plate * alpha.sin().powi(2),
        );
    }
    let stall = (max_lift / slope).min(FRAC_PI_2 - 2.0 * STALL_BLEND);
    let attached = (slope * a, profile + induced(slope * a));
    if a <= stall - STALL_BLEND {
        return (sign * attached.0, attached.1);
    }
    let (ss, cs) = stall.sin_cos();
    let a2 = (max_lift - plate * ss * cs) * ss / (cs * cs);
    let (s, c) = a.sin_cos();
    let separated = (
        plate * s * c + a2 * c * c / s,
        profile + plate * s * s + a2 * c,
    );
    let t = ((a - stall + STALL_BLEND) / (2.0 * STALL_BLEND)).clamp(0.0, 1.0);
    let t = t * t * (3.0 - 2.0 * t);
    (
        sign * (attached.0 * (1.0 - t) + separated.0 * t),
        attached.1 * (1.0 - t) + separated.1 * t,
    )
}

/// Wave drag coefficient of an airfoil of thickness `thickness` over chord: none below its
/// drag-divergence Mach number, a biconvex section's supersonic value (Ackeret) above, and
/// a rise between.
fn airfoil_wave_drag(mach: f64, thickness: f64) -> f64 {
    let divergence = 0.87 - thickness;
    let supersonic = |m: f64| 16.0 / 3.0 * thickness * thickness / (m * m - 1.0).sqrt();
    if mach <= divergence {
        0.0
    } else if mach >= 1.2 {
        supersonic(mach)
    } else {
        let t = (mach - divergence) / (1.2 - divergence);
        t * t * (3.0 - 2.0 * t) * supersonic(1.2)
    }
}

/// Mark where the flow has left the surface (into `separated[side]`), walking `sections`
/// downstream: from the nose if `from_nose`, else from the base. Where the outline turns
/// away from the flow by more than [`SEPARATION_TURN`] it separates, and it stays separated
/// over everything behind that keeps narrowing.
fn separate(sections: &mut [Section], side: usize, from_nose: bool) {
    // Slope of the previous section along the flow (positive widening), and its separation.
    let (mut previous, mut separated) = (0.0, 0.0);
    let mut walk = |s: &mut Section| {
        let growth = if from_nose { s.taper() } else { -s.taper() };
        let slope = if growth < 0.0 { -s.slope() } else { s.slope() };
        separated = if growth < 0.0 {
            let turn = previous - slope;
            ((turn - SEPARATION_TURN) / SEPARATION_TURN)
                .clamp(0.0, 1.0)
                .max(separated)
        } else {
            0.0
        };
        s.separated[side] = separated;
        previous = slope;
    };
    if from_nose {
        sections.iter_mut().rev().for_each(&mut walk);
    } else {
        sections.iter_mut().for_each(&mut walk);
    }
}

/// The outline of a stack, (height above its base, radius) from the middle of the base up
/// to the tip, as the renderer draws it: each stage a cylinder over its tanks (or its whole
/// fairing), tapering above them to the stage above; a pod a frustum on top of its stage; a
/// jettisoned fairing's payload a narrow adapter and bus; and a tangent-ogive nose on top
/// unless a pod is there. A step in radius is two points at the same height.
pub fn outline(stages: &[Stage]) -> Vec<(f64, f64)> {
    let mut points = vec![(0.0, 0.0)];
    let mut base = 0.0;
    for (i, stage) in stages.iter().enumerate() {
        let radius = stage.diameter / 2.0;
        let above = stages.get(i + 1);
        let hull = stage.length - stage.pod.as_ref().map_or(0.0, |p| p.height);
        if stage.fairing.as_ref().is_some_and(|f| !f.attached) {
            // An adapter cone up to a bus about half as wide.
            let (adapter, bus) = (0.9, 0.5 * radius);
            let top = base + adapter + (hull - adapter).clamp(1.0, 4.5);
            points.extend([
                (base, radius),
                (base + 0.12, radius),
                (base + adapter, bus),
                (top, bus),
            ]);
            base += stage.length;
            continue;
        }
        let fairing = match &stage.fairing {
            Some(_) => true,
            None => {
                above.is_none()
                    && stage.pod.is_none()
                    && stage.engines.is_empty()
                    && stage.tanks.is_empty()
            }
        };
        let tanks = if fairing {
            hull
        } else {
            tanks_top(stage).min(hull)
        };
        points.push((base, radius));
        if tanks > 0.0 {
            points.push((base + tanks, radius));
        }
        if tanks < hull {
            points.push((base + hull, above.map_or(radius, |s| s.diameter / 2.0)));
        }
        if let Some(pod) = &stage.pod {
            points.push((base + hull, pod.diameter / 2.0));
            points.push((base + stage.length, pod.top_diameter() / 2.0));
        } else if above.is_none() {
            points.extend(nose(radius, base + hull).skip(1));
        }
        base += stage.length;
    }
    if let Some(&(top, r)) = points.last()
        && r > 0.0
    {
        points.push((top, 0.0));
    }
    points.dedup();
    points
}

/// Tangent-ogive nose of `radius` from height `base`, `NOSE_FINENESS` diameters long.
fn nose(radius: f64, base: f64) -> impl Iterator<Item = (f64, f64)> {
    let length = NOSE_FINENESS * 2.0 * radius;
    let rho = (radius * radius + length * length) / (2.0 * radius);
    (0..=NOSE_STEPS).map(move |i| {
        let x = length * i as f64 / NOSE_STEPS as f64;
        let r = (rho * rho - x * x).max(0.0).sqrt() + radius - rho;
        (base + x, if i == NOSE_STEPS { 0.0 } else { r.max(0.0) })
    })
}

/// Height of the top of a stage's stacked cylindrical tanks above its base, m.
fn tanks_top(stage: &Stage) -> f64 {
    stage
        .tanks
        .iter()
        .filter(|t| t.shape == TankShape::Cylinder)
        .map(|t| t.base + t.length)
        .fold(0.0, f64::max)
}

/// Pressure coefficient on a surface at `slope` to the axis that the flow meets.
fn compression(mach: f64, slope: f64) -> f64 {
    let (low, high) = TRANSONIC;
    if mach <= low {
        subsonic_compression(mach, slope)
    } else if mach >= high {
        supersonic_compression(mach, slope)
    } else {
        let t = (mach - low) / (high - low);
        let t = t * t * (3.0 - 2.0 * t);
        subsonic_compression(low, slope) * (1.0 - t) + supersonic_compression(high, slope) * t
    }
}

/// Little on gentle slopes, most of the stagnation pressure on a flat face.
fn subsonic_compression(mach: f64, slope: f64) -> f64 {
    FLAT_FACE_PRESSURE * slope.sin().powi(3) * stagnation(mach)
}

/// The tangent cone (Rasmussen: Cp / sin²δ = 1 + ((γ+1)K² + 2) / ((γ−1)K² + 2) ·
/// ln((γ+1)/2 + 1/K²), K = M sin δ), which tends to Newtonian with the cone's centrifugal
/// correction at high Mach; where the shock stands off a blunt face, the stagnation
/// pressure caps it.
fn supersonic_compression(mach: f64, slope: f64) -> f64 {
    let sin = slope.sin();
    if sin < 1e-6 {
        return 0.0;
    }
    let k2 = (mach * sin).powi(2);
    let ratio = ((GAMMA + 1.0) * k2 + 2.0) / ((GAMMA - 1.0) * k2 + 2.0);
    let cone = sin * sin * (1.0 + ratio * ((GAMMA + 1.0) / 2.0 + 1.0 / k2).ln());
    cone.min(stagnation(mach) * sin)
}

/// Pressure coefficient where the flow stops: isentropic subsonic, behind a normal shock
/// supersonic (Rayleigh's pitot formula); 1.84 at hypersonic speeds.
fn stagnation(mach: f64) -> f64 {
    let m2 = mach * mach;
    if m2 < 1e-9 {
        return 1.0;
    }
    let power = GAMMA / (GAMMA - 1.0);
    let ratio = if mach <= 1.0 {
        (1.0 + (GAMMA - 1.0) / 2.0 * m2).powf(power)
    } else {
        ((GAMMA + 1.0).powi(2) * m2 / (4.0 * GAMMA * m2 - 2.0 * (GAMMA - 1.0))).powf(power)
            * (1.0 - GAMMA + 2.0 * GAMMA * m2)
            / (GAMMA + 1.0)
    };
    (ratio - 1.0) / (GAMMA / 2.0 * m2)
}

/// Magnitude of the pressure coefficient in a separated wake.
fn base_pressure(mach: f64) -> f64 {
    if mach > 3.0 {
        0.75 * 2.0 / (GAMMA * mach * mach)
    } else {
        lookup(&BASE_PRESSURE, mach)
    }
}

/// Mean skin-friction coefficient of a flat plate at Reynolds number `reynolds` (laminar
/// below transition, then Prandtl–Schlichting with the transition correction), reduced for
/// compressibility.
fn skin_friction(reynolds: f64, mach: f64) -> f64 {
    let re = reynolds.max(1.0);
    let incompressible = if re < TRANSITION_REYNOLDS {
        1.328 / re.sqrt()
    } else {
        0.455 / re.log10().powf(2.58) - 1700.0 / re
    };
    incompressible / (1.0 + 0.144 * mach * mach).powf(0.65)
}

/// Dynamic viscosity of air at `temperature` K (Sutherland), Pa·s.
fn viscosity(temperature: f64) -> f64 {
    let t = temperature.max(1.0);
    1.458e-6 * t.powf(1.5) / (t + 110.4)
}

/// Piecewise-linear lookup in `(x, y)` points sorted by x, flat beyond the ends.
fn lookup(table: &[(f64, f64)], x: f64) -> f64 {
    let (first, last) = (table[0], table[table.len() - 1]);
    if x <= first.0 {
        return first.1;
    }
    if x >= last.0 {
        return last.1;
    }
    let i = table.partition_point(|p| p.0 <= x);
    let ((x0, y0), (x1, y1)) = (table[i - 1], table[i]);
    y0 + (y1 - y0) * (x - x0) / (x1 - x0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        atmosphere::{Atmosphere, AtmosphereModel},
        data::CraftDef,
        propulsion::Propulsion,
        test_data::records,
        vessel::tests::catalog,
    };

    fn air(altitude: f64) -> AtmosphereSample {
        Atmosphere {
            model: AtmosphereModel::Us1976,
            height: 140_000.0,
        }
        .sample(altitude)
    }

    /// Flying at `mach` with the nose `angle` off the airflow (pitched up about +X), not
    /// turning.
    fn flow(air: AtmosphereSample, mach: f64, angle: f64) -> Flow {
        let speed = mach * air.speed_of_sound;
        Flow {
            velocity: DVec3::new(0.0, angle.sin(), angle.cos()) * speed,
            angular_velocity: DVec3::ZERO,
            air,
            plume: false,
            controls: DVec3::ZERO,
        }
    }

    /// Drag coefficient (against the flow, on the widest section) and the force.
    fn drag_coefficient(shape: &Shape, flow: &Flow) -> f64 {
        let loads = shape.loads(flow, shape.length / 2.0);
        let q = 0.5 * flow.air.density * flow.velocity.length_squared();
        -loads.force.dot(flow.velocity.normalize()) / (q * shape.reference_area())
    }

    /// A 1 m body: a tangent ogive `nose` calibres long on a cylinder, `length` calibres
    /// in all, with a flat base.
    fn ogive_cylinder(nose_calibres: f64, length: f64) -> Shape {
        let (r, l) = (0.5, nose_calibres);
        let rho = (r * r + l * l) / (2.0 * r);
        let mut points = vec![(0.0, 0.0), (0.0, r), (length - l, r)];
        for i in 1..=24 {
            let x = l * i as f64 / 24.0;
            let radius = ((rho * rho - x * x).max(0.0).sqrt() + r - rho).max(0.0);
            points.push((length - l + x, if i == 24 { 0.0 } else { radius }));
        }
        Shape::from_outline(&points, Vec::new())
    }

    fn sounder() -> Propulsion {
        let craft: CraftDef =
            records(include_str!("../../../assets/core/craft/sounder.json5")).remove(0);
        Propulsion::from_craft(&craft, &catalog()).unwrap()
    }

    #[test]
    fn outline_follows_the_stack() {
        let p = sounder();
        let outline = outline(&p.stages);
        // Base, booster barrel to its tank top, interstage taper, capsule, closed top.
        assert_eq!(
            outline,
            vec![
                (0.0, 0.0),
                (0.0, 0.9),
                (10.0, 0.9),
                (12.0, 0.95),
                (14.9, 0.73 / 2.0),
                (14.9, 0.0)
            ]
        );
        let shape = Shape::new(&p.stages);
        assert!((shape.length - 14.9).abs() < 1e-9);
        assert!(shape.sections.iter().all(|s| s.fore >= s.aft));
        assert!(shape.sections[0].base && !shape.sections[1].base);
        assert_eq!(shape.fins.len(), 1);
    }

    #[test]
    fn ogive_cylinder_drag_follows_the_mach_number() {
        // A 3-calibre tangent ogive on a 10-calibre body, at sea level, power off: about
        // 0.3 subsonic, rising through Mach 1, falling away supersonic.
        let shape = ogive_cylinder(3.0, 10.0);
        let cd = |mach: f64| drag_coefficient(&shape, &flow(air(0.0), mach, 0.0));
        let (sub, peak, two, four) = (cd(0.5), cd(1.2), cd(2.0), cd(4.0));
        assert!((0.2..0.4).contains(&sub), "{sub}");
        assert!((0.45..0.8).contains(&peak), "{peak}");
        assert!((0.3..0.55).contains(&two), "{two}");
        assert!((0.15..0.4).contains(&four), "{four}");
        assert!(
            peak > two && two > four && peak > sub,
            "{sub} {peak} {two} {four}"
        );
        // The plume fills the base and takes most of the subsonic drag away.
        let mut powered = flow(air(0.0), 0.5, 0.0);
        powered.plume = true;
        assert!(drag_coefficient(&shape, &powered) < 0.6 * sub);
    }

    #[test]
    fn slender_bodies_lift_from_the_nose() {
        // Slender-body theory: C_N ≈ 2α on the base area, acting on the nose, well ahead
        // of the middle, so a bare body is unstable.
        let shape = ogive_cylinder(3.0, 10.0);
        let alpha = 2f64.to_radians();
        let f = flow(air(0.0), 0.5, alpha);
        let loads = shape.loads(&f, 5.0);
        let q = 0.5 * f.air.density * f.velocity.length_squared();
        // The nose is pitched up (the air comes from below, +Y): it lifts towards −Y.
        let normal = -loads.force.y / (q * shape.reference_area());
        let slope = normal / alpha;
        assert!((2.0..2.6).contains(&slope), "{slope}");
        // It acts well ahead of the middle, pitching the nose further up (about +X).
        let center_of_pressure = 5.0 - loads.torque.x / loads.force.y;
        assert!(center_of_pressure > 7.0, "{center_of_pressure}");
        assert!(loads.torque.x > 0.0);
    }

    #[test]
    fn fins_put_the_centre_of_pressure_behind_the_centre_of_mass() {
        let mut p = sounder();
        let com = crate::attitude::mass_properties(&p).center_of_mass;
        let f = flow(air(5_000.0), 0.8, 4f64.to_radians());
        // With fins, the torque pitches the nose back down into the wind (about −X).
        let stable = Shape::new(&p.stages).loads(&f, com);
        assert!(stable.torque.x < 0.0, "{:?}", stable.torque);
        // Without, it pitches further up.
        p.stages[0].fins = None;
        let bare = Shape::new(&p.stages).loads(&f, com);
        assert!(bare.torque.x > 0.0, "{:?}", bare.torque);
        // Both push the same way, sideways out of the wind.
        assert!(stable.force.y < bare.force.y && bare.force.y < 0.0);
    }

    #[test]
    fn fins_hold_their_own_whichever_way_the_vessel_is_rolled() {
        let p = sounder();
        let shape = Shape::new(&p.stages);
        let com = crate::attitude::mass_properties(&p).center_of_mass;
        let f = flow(air(5_000.0), 0.8, 4f64.to_radians());
        let pitch = shape.loads(&f, com);
        // The same angle in yaw (air from the left, nose to the right of the flow).
        let mut yawed = f;
        yawed.velocity = DVec3::new(f.velocity.y, 0.0, f.velocity.z);
        let yaw = shape.loads(&yawed, com);
        assert!((yaw.force.x - pitch.force.y).abs() < 1e-6 * pitch.force.length());
        assert!((yaw.torque.y + pitch.torque.x).abs() < 1e-6 * pitch.torque.length());
        assert!(yaw.torque.x.abs() < 1e-6 * pitch.torque.length());
    }

    #[test]
    fn rotation_is_damped() {
        let p = sounder();
        let shape = Shape::new(&p.stages);
        let com = crate::attitude::mass_properties(&p).center_of_mass;
        let mut f = flow(air(5_000.0), 0.8, 0.0);
        f.angular_velocity = DVec3::new(0.2, -0.1, 0.0);
        let loads = shape.loads(&f, com);
        assert!(
            loads.torque.dot(f.angular_velocity) < 0.0,
            "{:?}",
            loads.torque
        );
        // Only the fins resist a roll (the model leaves out skin friction around the hull).
        f.angular_velocity = DVec3::new(0.0, 0.0, 1.0);
        assert!(shape.loads(&f, com).torque.z < 0.0);
    }

    #[test]
    fn a_capsule_falls_heat_shield_first() {
        // The C-1 capsule alone: a flat heat shield, tapering above, its centre of mass
        // low.
        let shape = Shape::from_outline(
            &[(0.0, 0.0), (0.0, 0.95), (2.9, 0.365), (2.9, 0.0)],
            Vec::new(),
        );
        let com = 1.0;
        // Hypersonic, heat shield first: about Newtonian, 1.84 on the shield.
        let f = flow(air(60_000.0), 15.0, PI);
        let cd = drag_coefficient(&shape, &f);
        assert!((1.6..2.0).contains(&cd), "{cd}");
        // Ten degrees off (falling along −Z, drifting towards +Y), the air swings the heat
        // shield back into the flow: −Z turns towards +Y, a rotation about +X.
        let tilted = flow(air(60_000.0), 15.0, PI - 10f64.to_radians());
        let loads = shape.loads(&tilted, com);
        assert!(loads.torque.x > 0.0, "{:?}", loads.torque);
        // Subsonic, a blunt body: around one.
        let cd = drag_coefficient(&shape, &flow(air(3_000.0), 0.4, PI));
        assert!((0.8..1.4).contains(&cd), "{cd}");
    }

    /// A straight wing of `span` each side and `chord`, alone in the air.
    fn plain_wing(span: f64, chord: f64) -> (WingDef, Shape) {
        let wing = WingDef {
            name: "Wing".into(),
            position: 0.0,
            angle: 0.0,
            root_chord: chord,
            tip_chord: chord,
            span,
            sweep: 0.0,
            dihedral: 0.0,
            incidence: 0.0,
            thickness: 0.12,
            mass: 0.0,
            mirror: true,
            control: None,
        };
        let surfaces = Panel::of(&wing, 0.0, 0.0)
            .into_iter()
            .map(|p| Surface::new(&wing, p))
            .collect();
        let shape = Shape {
            surfaces: with_downwash(surfaces),
            ..Default::default()
        };
        (wing, shape)
    }

    /// Lift and drag coefficients of `shape` on `area` at `degrees` of angle of attack,
    /// slow, at sea level, with the centre of mass a quarter chord back.
    fn polar(shape: &Shape, area: f64, degrees: f64) -> (f64, f64) {
        // Wings lift towards −Y, so the nose is pitched up with the air from +Y.
        let f = flow(air(0.0), 0.15, degrees.to_radians());
        let loads = shape.loads(&f, -0.25);
        let q = 0.5 * f.air.density * f.velocity.length_squared();
        let along = f.velocity.normalize();
        let drag = -loads.force.dot(along);
        let lift = (loads.force + along * drag).length() * -loads.force.y.signum();
        (lift / (q * area), drag / (q * area))
    }

    #[test]
    fn a_wing_lifts_stalls_and_drags_like_a_wing() {
        // Aspect ratio 8: DATCOM gives a lift slope of about 4.98 per radian at low speed.
        let (_, shape) = plain_wing(4.0, 1.0);
        let (cl4, cd4) = polar(&shape, 8.0, 4.0);
        let slope = cl4 / 4f64.to_radians();
        assert!((slope - 4.98).abs() < 0.1, "{slope}");
        // Induced drag CL²/(π e A) on top of the profile drag at zero lift.
        let (cl0, cd0) = polar(&shape, 8.0, 0.0);
        assert!(
            cl0.abs() < 1e-9 && (0.005..0.015).contains(&cd0),
            "{cl0} {cd0}"
        );
        let induced = cd4 - cd0;
        let expected = cl4 * cl4 / (PI * OSWALD * 8.0);
        assert!(
            (induced - expected).abs() < 0.1 * expected,
            "{induced} {expected}"
        );
        // It stalls a little past 14° (C_L max about 1.26), and loses lift beyond.
        let (cl14, _) = polar(&shape, 8.0, 14.0);
        let (cl25, cd25) = polar(&shape, 8.0, 25.0);
        assert!((1.15..1.3).contains(&cl14), "{cl14}");
        assert!(cl25 < cl14 && cd25 > 0.3, "{cl25} {cd25}");
        // Symmetric: negative angles lift the other way.
        let (cl_neg, _) = polar(&shape, 8.0, -4.0);
        assert!((cl_neg + cl4).abs() < 1e-9);
    }

    #[test]
    fn a_rolling_wing_damps_its_roll() {
        let (_, shape) = plain_wing(4.0, 1.0);
        let mut f = flow(air(0.0), 0.15, 3f64.to_radians());
        f.angular_velocity = DVec3::new(0.0, 0.0, 0.5);
        let loads = shape.loads(&f, 0.0);
        assert!(loads.torque.z < 0.0, "{:?}", loads.torque);
    }

    fn glider() -> (Propulsion, f64) {
        let p = Propulsion::from_craft(&crate::vessel::tests::glider(), &catalog()).unwrap();
        let com = crate::attitude::mass_properties(&p).center_of_mass;
        (p, com)
    }

    #[test]
    fn control_surfaces_turn_the_vessel_the_commanded_way() {
        let (p, com) = glider();
        let shape = Shape::new(&p.stages);
        let mut f = flow(air(1_000.0), 0.11, 4f64.to_radians());
        let trimmed = shape.loads(&f, com);
        for (command, axis) in [
            (DVec3::X, 0), // Pitch up: elevator.
            (DVec3::Y, 1), // Yaw right: rudder.
            (DVec3::Z, 2), // Roll right: ailerons.
        ] {
            f.controls = command;
            let turn = shape.loads(&f, com).torque - trimmed.torque;
            assert!(turn[axis] > 0.0, "{command}: {turn}");
            // Mostly about that axis.
            assert!(turn[axis] > 0.5 * turn.length(), "{command}: {turn}");
            // And the authority SAS plans with is about what it gets.
            let authority = shape.control_authority(&f, com)[axis];
            assert!(
                (0.5..2.0).contains(&(turn[axis] / authority)),
                "{command}: {} of {authority}",
                turn[axis]
            );
            f.controls = -command;
            assert!((shape.loads(&f, com).torque - trimmed.torque)[axis] < 0.0);
        }
        // Wings level, stick centred: nothing to roll or yaw it.
        assert!(trimmed.torque.y.abs() + trimmed.torque.z.abs() < 1e-6 * trimmed.force.length());
    }

    #[test]
    fn the_wing_washes_the_tailplane_down() {
        let (p, _) = glider();
        let shape = Shape::new(&p.stages);
        // Panels: tailplane right and left, fin, wing right and left.
        let names: Vec<usize> = shape
            .surfaces
            .iter()
            .map(|s| s.downwash_from.len())
            .collect();
        assert_eq!(names, vec![1, 1, 0, 0, 0]);
        // Each half of the tailplane sits behind its own half of the wing.
        assert_eq!(shape.surfaces[0].downwash_from[0].0, 3);
        assert_eq!(shape.surfaces[1].downwash_from[0].0, 4);
    }

    #[test]
    fn a_flat_wing_obeys_the_perpendicular_axis_theorem() {
        // Mostly a flat plate in the X–Z plane: yaw ≈ pitch + roll.
        let (p, _) = glider();
        let i = crate::attitude::mass_properties(&p).inertia.diagonal();
        // Pitch, yaw and roll.
        assert!(i.y > i.z && i.z > i.x, "{i}");
        assert!((i.y - (i.x + i.z)).abs() < 0.05 * i.y, "{i}");
    }

    #[test]
    fn stagnation_pressure_is_continuous_and_newtonian_at_high_speed() {
        assert!((stagnation(1.0 - 1e-9) - stagnation(1.0 + 1e-9)).abs() < 1e-6);
        assert!((stagnation(0.0) - 1.0).abs() < 1e-9);
        assert!((stagnation(20.0) - 1.837).abs() < 0.01);
        // A 10° cone at Mach 2: Taylor–Maccoll gives about 0.1.
        let cone = supersonic_compression(2.0, 10f64.to_radians());
        assert!((0.09..0.12).contains(&cone), "{cone}");
    }

    #[test]
    fn lookup_interpolates_and_clamps() {
        let table = [(0.0, 1.0), (1.0, 3.0), (2.0, 2.0)];
        assert_eq!(lookup(&table, -1.0), 1.0);
        assert_eq!(lookup(&table, 0.5), 2.0);
        assert_eq!(lookup(&table, 1.5), 2.5);
        assert_eq!(lookup(&table, 9.0), 2.0);
    }
}
