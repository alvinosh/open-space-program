//! The last metres of a docking: how one vessel's port stands towards another's, and whether
//! they capture.
//!
//! A port is a component on one of a part's nodes ([`crate::component::DockingPortDef`]). Two that
//! fit capture when they touch, near enough lined up and not coming too fast. That is soft capture:
//! a spring and damper between the ports then draws them together and lines them up over a few
//! seconds, and the vessels join there ([`crate::universe::Universe::join`]). Ports that do not
//! capture bump like anything else ([`crate::contact`]).

use glam::{DQuat, DVec3};
use serde::{Deserialize, Serialize};

use crate::{
    component::PortKind,
    orbit::StateVector,
    parts::{PartId, PartTree},
    vessel::VesselId,
};

/// How near the ports must be along their axis to touch, m, and how far past each other still
/// counts. A little before their surfaces meet: the probe and the drogue's funnel reach out.
const CONTACT: f64 = 0.15;
const PAST: f64 = 0.5;
/// How far off each other's axis they can capture, m: the drogue's funnel, as wide as its lip.
pub const MAX_OFF_AXIS: f64 = 0.4;
/// How far from facing each other they can capture, rad (15°).
pub const MAX_ANGLE: f64 = 15.0 * std::f64::consts::PI / 180.0;
/// How fast they can close, m/s, and at least how fast: ports that have only drifted together do
/// not latch, and ones pulling apart do not.
pub const MAX_CLOSING: f64 = 0.5;
const MIN_CLOSING: f64 = 1e-3;
/// How fast they can slide across each other, m/s.
pub const MAX_DRIFT: f64 = 0.3;
/// How fast they can be turning against each other, rad/s (3°/s).
pub const MAX_TURNING: f64 = 3.0 * std::f64::consts::PI / 180.0;
/// How much the diameters of two ports may differ and still fit, as a share.
const FIT: f64 = 0.05;
/// How fast soft capture draws the ports together and lines them up, 1/s: the spring's frequency,
/// critically damped.
pub(crate) const RETRACT: f64 = 1.2;
/// When the latches close: near enough together for them to reach and lined up, m and rad, or this
/// long after capture, s, whatever is left being taken up by them.
pub(crate) const LATCH_GAP: f64 = 0.05;
pub(crate) const LATCH_ANGLE: f64 = 1.0 * std::f64::consts::PI / 180.0;
pub(crate) const LATCH_TIME: f64 = 8.0;

/// A docking port on a vessel, in its assembly frame.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct DockingPort {
    pub part: PartId,
    pub node: String,
    pub kind: PortKind,
    /// m.
    pub diameter: f64,
    pub position: DVec3,
    /// Unit, out of the part.
    pub direction: DVec3,
    /// How its frame is turned: its +Z is `direction`.
    pub frame: DQuat,
    /// No joint is on its node.
    pub free: bool,
}

impl DockingPort {
    /// The ports on `tree`'s parts.
    pub(crate) fn all(tree: &PartTree) -> Vec<Self> {
        let mut out = Vec::new();
        for part in tree.parts() {
            let Some(built) = &part.built else { continue };
            for component in &built.components {
                let crate::component::Component::DockingPort(def) = component else {
                    continue;
                };
                let node = (built.nodes.iter())
                    .chain(&built.geometry.nodes)
                    .find(|n| n.name == def.node);
                let Some(node) = node else { continue };
                out.push(Self {
                    part: part.id,
                    node: node.name.clone(),
                    kind: def.kind,
                    diameter: node.size,
                    position: part.position + part.rotation * node.position,
                    direction: part.rotation * node.direction,
                    frame: part.rotation * crate::assembly::node_frame(node).rotation,
                    free: !part.holds_joint(tree, &node.name),
                });
            }
        }
        out
    }

    /// Whether it could capture `other`.
    pub fn fits(&self, other: &DockingPort) -> bool {
        self.free
            && other.free
            && self.kind.mates(other.kind)
            && (self.diameter - other.diameter).abs() <= FIT * self.diameter.max(other.diameter)
    }
}

/// Where a port is and how it moves, in the frame of a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PortState {
    pub at: StateVector,
    /// Unit, out of the part.
    pub out: DVec3,
    /// How the part it is on turns, rad/s, inertial axes.
    pub turning: DVec3,
}

/// How a port stands towards the one it is approaching. Everything is measured on that one's
/// axis, in the frame of a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Alignment {
    /// How far in front of the other port this one is, along its axis, m.
    pub gap: f64,
    /// From the other's axis to this port, across it, m.
    pub off_axis: DVec3,
    /// How fast the gap is shrinking, m/s.
    pub closing: f64,
    /// How fast this port slides across the other's axis, m/s.
    pub drift: DVec3,
    /// How far from facing the other port, rad.
    pub angle: f64,
    /// How fast the two are turning against each other, rad/s.
    pub turning: f64,
    /// The way the other port faces, unit: this one wants to face the other way.
    pub out: DVec3,
}

/// Why two ports would not capture, were they touching.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Problem {
    NotLinedUp,
    OffAxis,
    TooFast,
    NotClosing,
    Drifting,
    Turning,
}

impl Problem {
    /// What to tell the pilot.
    pub fn label(self) -> &'static str {
        match self {
            Problem::NotLinedUp => "Not lined up",
            Problem::OffAxis => "Off axis",
            Problem::TooFast => "Too fast",
            Problem::NotClosing => "Not closing",
            Problem::Drifting => "Drifting",
            Problem::Turning => "Turning",
        }
    }
}

impl Alignment {
    pub fn of(own: &PortState, other: &PortState) -> Self {
        let out = other.out;
        let apart = own.at.position - other.at.position;
        let relative = own.at.velocity - other.at.velocity;
        let gap = apart.dot(out);
        let along = relative.dot(out);
        Self {
            gap,
            off_axis: apart - out * gap,
            closing: -along,
            drift: relative - out * along,
            angle: own.out.angle_between(-out),
            turning: (own.turning - other.turning).length(),
            out,
        }
    }

    /// The first thing that would stop them capturing, however far apart they are now.
    pub fn problem(&self) -> Option<Problem> {
        if self.angle > MAX_ANGLE {
            Some(Problem::NotLinedUp)
        } else if self.off_axis.length() > MAX_OFF_AXIS {
            Some(Problem::OffAxis)
        } else if self.closing > MAX_CLOSING {
            Some(Problem::TooFast)
        } else if self.closing < MIN_CLOSING {
            Some(Problem::NotClosing)
        } else if self.drift.length() > MAX_DRIFT {
            Some(Problem::Drifting)
        } else if self.turning > MAX_TURNING {
            Some(Problem::Turning)
        } else {
            None
        }
    }

    pub fn touching(&self) -> bool {
        (-PAST..=CONTACT).contains(&self.gap)
    }

    /// Whether they capture now.
    pub fn captures(&self) -> bool {
        self.touching() && self.problem().is_none()
    }

    /// Between the ports, m.
    pub fn distance(&self) -> f64 {
        (self.gap * self.gap + self.off_axis.length_squared()).sqrt()
    }
}

/// A vessel's way in to the vessel it has targeted: the ports that would meet.
#[derive(Debug, Clone, PartialEq)]
pub struct PortApproach {
    pub port: DockingPort,
    pub target: VesselId,
    pub target_port: DockingPort,
    pub alignment: Alignment,
    /// Where each is and how it moves, in the frame of the vessel's parent.
    pub near: PortState,
    pub far: PortState,
}

/// The roll that brings the ports together as the vessels now stand (see
/// [`crate::assembly::meet`]): `own` and `other` are how the ports' frames are turned in space.
pub(crate) fn roll_between(own: DQuat, other: DQuat) -> f64 {
    let turn = own.inverse() * other * DQuat::from_rotation_x(std::f64::consts::PI);
    let angle = 2.0 * turn.z.atan2(turn.w);
    (angle + std::f64::consts::PI).rem_euclid(std::f64::consts::TAU) - std::f64::consts::PI
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(z: f64, speed: f64) -> PortState {
        PortState {
            at: StateVector::new(DVec3::Z * z, DVec3::Z * speed),
            out: DVec3::NEG_Z,
            turning: DVec3::ZERO,
        }
    }

    fn target() -> PortState {
        PortState {
            at: StateVector::default(),
            out: DVec3::Z,
            turning: DVec3::ZERO,
        }
    }

    #[test]
    fn ports_capture_touching_lined_up_and_closing_slowly() {
        let good = at(0.05, -0.2);
        assert!(Alignment::of(&good, &target()).captures());
        // Not there yet, or gone past.
        assert!(!Alignment::of(&at(0.6, -0.2), &target()).captures());
        assert!(!Alignment::of(&at(-1.0, -0.2), &target()).captures());
        // Each thing that stops it says so.
        let problem = |own: PortState| Alignment::of(&own, &target()).problem();
        assert_eq!(problem(at(0.05, -0.8)), Some(Problem::TooFast));
        assert_eq!(problem(at(0.05, 0.1)), Some(Problem::NotClosing));
        assert_eq!(problem(at(0.05, 0.0)), Some(Problem::NotClosing));
        let mut side = good;
        side.at.position.x = 0.5;
        assert_eq!(problem(side), Some(Problem::OffAxis));
        let mut slide = good;
        slide.at.velocity.y = 0.4;
        assert_eq!(problem(slide), Some(Problem::Drifting));
        let mut crooked = good;
        crooked.out = (DVec3::NEG_Z + DVec3::X * 0.4).normalize();
        assert_eq!(problem(crooked), Some(Problem::NotLinedUp));
        let mut spinning = good;
        spinning.turning = DVec3::X * 0.2;
        assert_eq!(problem(spinning), Some(Problem::Turning));
    }

    #[test]
    fn the_kinds_mate_as_the_real_ones_do() {
        use PortKind::{Androgynous, Berthing, Drogue, Probe};
        assert!(Probe.mates(Drogue) && Drogue.mates(Probe));
        assert!(Androgynous.mates(Androgynous) && Berthing.mates(Berthing));
        assert!(!Probe.mates(Probe) && !Drogue.mates(Drogue));
        assert!(!Androgynous.mates(Probe) && !Berthing.mates(Androgynous));
    }

    #[test]
    fn the_roll_is_how_far_round_the_line_they_meet_along_they_are() {
        use crate::assembly::{Pose, meet};
        let frame = DQuat::from_rotation_y(0.7) * DQuat::from_rotation_x(0.3);
        for roll in [0.0, 0.5, -1.2, 3.0] {
            let near = Pose::new(DVec3::X, frame);
            let far = Pose::new(DVec3::Y, DQuat::from_rotation_z(0.4));
            // What `meet` does to the far part's frame, the roll gets back.
            let met = meet(near, far, roll) * far;
            let back = roll_between(near.rotation, met.rotation);
            assert!((back - roll).abs() < 1e-9, "{roll} {back}");
        }
        // A small tilt on top changes it little.
        let facing = DQuat::from_rotation_z(0.5) * DQuat::from_rotation_x(std::f64::consts::PI);
        let tilted = roll_between(DQuat::IDENTITY, facing * DQuat::from_rotation_x(0.01));
        assert!((tilted - 0.5).abs() < 1e-3, "{tilted}");
    }
}
