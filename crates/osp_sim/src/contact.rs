//! What happens when vessels touch: rigid bodies, solved by Rapier over the convex pieces of their
//! parts ([`crate::collision`]).
//!
//! Vessels are not Rapier's to move. They go on being conics and Encke states, integrated with their
//! thrust, their air and their gear by [`crate::vessel`], and only two near enough to touch ever come
//! here, so what is exact on rails stays exact (see T074). Each tick the two are put into a little
//! world of their own as they are, with the one being run into as its origin and at rest, so that the
//! numbers are small and an orbit's speed is nobody's concern. Rapier steps it once with no forces,
//! which looks ahead along how they are moving and stops what would meet. Whatever that changed is
//! all that is taken back: a vessel nothing touched comes out exactly as it went in.
//!
//! A world is kept from tick to tick while its two are near and built as they were, so contact
//! persists: something leaning on something else stays put, without being hit anew every tick. Three
//! vessels touching are two worlds, taken in turn.

use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    sync::Arc,
};

use glam::{DQuat, DVec3};
use rapier3d_f64::prelude::{MassProperties as ShapeMass, *};

use crate::{
    assembly::Pose,
    attitude::MassProperties,
    collision::{Collider, shape_pose},
    vessel::VesselId,
};

/// How much of the closing speed comes back: structure and latches take the rest.
const RESTITUTION: f64 = 0.25;
/// Metal on metal, about.
const FRICTION: f64 = 0.3;
/// Rounds of the solver in a step. More than a game usually needs: a tick is long, and a heavy
/// vessel may well meet a light one.
const SOLVER_ROUNDS: usize = 8;
/// How far apart surfaces begin to be looked at closely, m: how far they can come in a tick, between
/// these. Past that it is left to sweeping the faster one along its path, which needs no guess at
/// which way they will meet.
const LEAST_PREDICTION: f64 = 0.05;
const MOST_PREDICTION: f64 = 0.25;
/// The gap kept between surfaces, m: what rests on something else is not quite through it.
const SKIN: f64 = 0.005;
/// How fast surfaces already through each other are eased apart, m/s.
const EASE_MAX: f64 = 1.0;
/// Differences smaller than these are rounding, not contact: m, rad, m/s and rad/s.
const NOT_MOVED: f64 = 1e-7;
const NOT_TURNED: f64 = 1e-7;
const NOT_SPED: f64 = 1e-9;
/// Touching again this soon after they last touched is the same touch, s.
const SAME_TOUCH: f64 = 0.25;

/// A vessel as a contact sees it, in the frame of a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Body {
    pub center: DVec3,
    pub velocity: DVec3,
    pub turned: DQuat,
    /// In its own axes, rad/s.
    pub spin: DVec3,
    pub mass: MassProperties,
    /// It stands on the ground and cannot roll: nothing moves it.
    pub fixed: bool,
}

impl Body {
    fn turning(&self) -> DVec3 {
        self.turned * self.spin
    }

    /// Take `impulse` (N·s) at `point`.
    pub fn push(&mut self, impulse: DVec3, point: DVec3) {
        if self.fixed {
            return;
        }
        self.velocity += impulse / self.mass.mass;
        let about = self.turned.inverse() * (point - self.center).cross(impulse);
        self.spin += self.mass.inertia.tensor.inverse() * about;
    }

    /// Take a turning impulse (N·m·s, inertial axes).
    pub fn twist(&mut self, torque: DVec3) {
        if self.fixed {
            return;
        }
        let about = self.turned.inverse() * torque;
        self.spin += self.mass.inertia.tensor.inverse() * about;
    }
}

/// A vessel going into a world.
#[derive(Debug, Clone)]
pub(crate) struct Member {
    pub id: VesselId,
    pub body: Body,
    /// Where its centre of mass is in its assembly frame.
    pub held: DVec3,
    pub collider: Collider,
}

impl Member {
    /// Where its assembly frame is, relative to `origin`.
    fn frame(&self, origin: DVec3) -> Pose {
        Pose::new(
            self.body.center - origin - self.body.turned * self.held,
            self.body.turned,
        )
    }

    fn mass_properties(&self) -> ShapeMass {
        let mass = &self.body.mass;
        ShapeMass::with_inertia_matrix(self.held, mass.mass, mass.inertia.tensor)
    }

    /// How it is built: the same only if its parts have not changed.
    fn build(&self) -> Build {
        (self.collider.parts.iter())
            .map(|part| (Arc::as_ptr(&part.mesh) as usize, part.pose))
            .collect()
    }
}

/// Each part's mesh, told apart by where it is kept, and where it is on the vessel.
type Build = Vec<(usize, Pose)>;

/// What a tick of contact did.
#[derive(Debug, Default)]
pub(crate) struct Outcome {
    /// The vessels it changed, as they are to go on.
    pub moved: Vec<(VesselId, Body)>,
    /// If the two have just met, not if they are still touching from before: how fast they were
    /// closing where they closed fastest, m/s.
    pub met: Option<f64>,
}

/// The little worlds of the pairs of vessels near enough to touch.
#[derive(Default)]
pub struct Contacts {
    worlds: BTreeMap<(VesselId, VesselId), World>,
    /// When each pair last touched, s from any fixed moment.
    touching: BTreeMap<(VesselId, VesselId), f64>,
}

impl fmt::Debug for Contacts {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Contacts")
            .field("worlds", &self.worlds.len())
            .finish()
    }
}

impl Contacts {
    /// Forget everything: what comes next has nothing to do with what was here.
    pub fn clear(&mut self) {
        self.worlds.clear();
        self.touching.clear();
    }

    /// Step `a` and `b` over `dt`. `now` says when, s from any fixed moment.
    pub(crate) fn step(&mut self, a: Member, b: Member, now: f64, dt: f64) -> Outcome {
        let members = if a.id < b.id { [a, b] } else { [b, a] };
        let pair = (members[0].id, members[1].id);
        let builds = [members[0].build(), members[1].build()];
        let world = self.worlds.entry(pair).or_default();
        if world.builds != builds {
            *world = World::new(&members, builds);
        }
        let (moved, pressing) = world.step(&members, dt);
        let met = pressing.filter(|_| {
            let last = self.touching.insert(pair, now);
            last.is_none_or(|last| now - last > SAME_TOUCH)
        });
        Outcome { moved, met }
    }

    /// Let go of every pair but `near`.
    pub(crate) fn keep_only(&mut self, near: &BTreeSet<(VesselId, VesselId)>) {
        self.worlds.retain(|pair, _| near.contains(pair));
        self.touching.retain(|pair, _| near.contains(pair));
    }
}

/// Two vessels in Rapier's hands.
struct World {
    /// How each was built when this was made.
    builds: [Build; 2],
    handles: [RigidBodyHandle; 2],
    pipeline: PhysicsPipeline,
    parameters: IntegrationParameters,
    islands: IslandManager,
    broad_phase: BroadPhaseBvh,
    narrow_phase: NarrowPhase,
    bodies: RigidBodySet,
    colliders: ColliderSet,
    impulse_joints: ImpulseJointSet,
    multibody_joints: MultibodyJointSet,
    soft_bodies: SoftBodySet,
    ccd: CCDSolver,
}

impl Default for World {
    fn default() -> Self {
        Self {
            builds: Default::default(),
            handles: [RigidBodyHandle::invalid(); 2],
            pipeline: PhysicsPipeline::new(),
            parameters: IntegrationParameters {
                num_solver_iterations: SOLVER_ROUNDS,
                normalized_prediction_distance: LEAST_PREDICTION,
                normalized_max_corrective_velocity: EASE_MAX,
                // Two vessels may meet at any speed at all.
                normalized_max_linear_velocity: f64::MAX,
                // Both are put where they are each tick: contact is worked out afresh from there.
                contact_recycling: false,
                // Not the quick approximation: two bodies are nothing to solve properly.
                friction_model: FrictionModel::Coulomb,
                ..IntegrationParameters::default()
            },
            islands: IslandManager::new(),
            broad_phase: BroadPhaseBvh::new(),
            narrow_phase: NarrowPhase::new(),
            bodies: RigidBodySet::new(),
            colliders: ColliderSet::new(),
            impulse_joints: ImpulseJointSet::new(),
            multibody_joints: MultibodyJointSet::new(),
            soft_bodies: SoftBodySet::new(),
            ccd: CCDSolver::new(),
        }
    }
}

impl World {
    fn new(members: &[Member; 2], builds: [Build; 2]) -> Self {
        let mut world = Self {
            builds,
            ..Self::default()
        };
        for (k, member) in members.iter().enumerate() {
            let handle = world.bodies.insert(
                RigidBodyBuilder::dynamic()
                    .can_sleep(false)
                    .additional_mass_properties(member.mass_properties()),
            );
            for part in &member.collider.parts {
                let Some(shape) = part.mesh.shape() else {
                    continue;
                };
                let collider = ColliderBuilder::new(shape.clone())
                    .position(shape_pose(part.pose))
                    // Its mass is the vessel's, not worked out from these.
                    .density(0.0)
                    .friction(FRICTION)
                    .restitution(RESTITUTION)
                    .contact_skin(SKIN)
                    .user_data(k as u128);
                world
                    .colliders
                    .insert_with_parent(collider, handle, &mut world.bodies);
            }
            world.handles[k] = handle;
        }
        world
    }

    /// One tick, from how `members` are now. The vessels it changed, and, if the two are pressing on
    /// each other, how fast they were closing.
    fn step(&mut self, members: &[Member; 2], dt: f64) -> (Vec<(VesselId, Body)>, Option<f64>) {
        // The one that cannot be moved is what is run into, or else the first. Rapier looks ahead
        // from the other, the only one it sees moving, so nothing is too fast to catch.
        let target = usize::from(members[1].body.fixed);
        let (origin, at_rest) = (members[target].body.center, members[target].body.velocity);
        for (k, member) in members.iter().enumerate() {
            let body = &mut self.bodies[self.handles[k]];
            let kind = if member.body.fixed {
                RigidBodyType::Fixed
            } else {
                RigidBodyType::Dynamic
            };
            if body.body_type() != kind {
                body.set_body_type(kind, true);
            }
            body.enable_ccd(k != target);
            let mass = member.mass_properties();
            if body.mass_properties().local_mprops != mass {
                body.set_additional_mass_properties(mass, true);
            }
            body.set_position(shape_pose(member.frame(origin)), true);
            if !member.body.fixed {
                body.set_linvel(member.body.velocity - at_rest, true);
                body.set_angvel(member.body.turning(), true);
            }
        }
        let [first, second] = [&members[0].body, &members[1].body];
        let closing = (first.velocity - second.velocity).length()
            + (first.turning().length() + second.turning().length())
                * first.center.distance(second.center);
        self.parameters.dt = dt;
        self.parameters.normalized_prediction_distance =
            (closing * dt).clamp(LEAST_PREDICTION, MOST_PREDICTION);
        self.pipeline.step(
            DVec3::ZERO,
            &self.parameters,
            &mut self.islands,
            &mut self.broad_phase,
            &mut self.narrow_phase,
            &mut self.bodies,
            &mut self.colliders,
            &mut self.impulse_joints,
            &mut self.multibody_joints,
            &mut self.soft_bodies,
            &mut self.ccd,
            &(),
            &(),
        );
        let pressing = self.closing(members, origin, at_rest);
        let moved = members
            .iter()
            .zip(&self.handles)
            .filter_map(|(member, &handle)| {
                let body = self.after(member, handle, origin, at_rest, dt)?;
                Some((member.id, body))
            })
            .collect();
        (moved, pressing)
    }

    /// If the solver pushed the two apart, how fast they were closing before it did, where they
    /// were closing fastest.
    fn closing(&self, members: &[Member; 2], origin: DVec3, at_rest: DVec3) -> Option<f64> {
        let mut fastest = None;
        for pair in self.narrow_phase.contact_pairs() {
            let (Some(first), Some(second)) = (
                self.colliders.get(pair.collider1),
                self.colliders.get(pair.collider2),
            ) else {
                continue;
            };
            // The first member's velocity less the second's, at `point`.
            let moving = |k: u128, point: DVec3| {
                let body = &members[k as usize].body;
                body.velocity - at_rest + body.turning().cross(point - (body.center - origin))
            };
            let PairContacts::Rigid(contacts) = &pair.contacts else {
                continue;
            };
            // What the solver worked on: a part's convex pieces may have been merged.
            for manifold in contacts.solver_manifolds() {
                for point in &manifold.points {
                    if point.data.impulse <= 0.0 {
                        continue;
                    }
                    let local = (manifold.subshape_pos1())
                        .map_or(point.local_p1, |pose| pose * point.local_p1);
                    let at = first.position() * local;
                    let closing = (moving(first.user_data, at) - moving(second.user_data, at))
                        .dot(manifold.data.normal);
                    fastest = Some(closing.max(fastest.unwrap_or(0.0)));
                }
            }
        }
        fastest
    }

    /// `member` after the step, if that changed it. The step has already moved it on along its new
    /// velocity, which the vessel will do for itself over the next tick: what it takes from here is
    /// the new velocity and whatever moved it besides that, surfaces being eased apart or held short
    /// of meeting.
    fn after(
        &self,
        member: &Member,
        handle: RigidBodyHandle,
        origin: DVec3,
        at_rest: DVec3,
        dt: f64,
    ) -> Option<Body> {
        let was = &member.body;
        if was.fixed {
            return None;
        }
        let now = &self.bodies[handle];
        let (velocity, turning) = (now.linvel(), now.angvel());
        let extra = now.center_of_mass() - (was.center - origin + velocity * dt);
        let would_be = (DQuat::from_scaled_axis(turning * dt) * was.turned).normalize();
        let extra_turn = (now.position().rotation * would_be.inverse()).normalize();
        let (axis, angle) = extra_turn.to_axis_angle();
        // Whatever went wrong in there, a vessel is never left with no place to be.
        if !(extra.is_finite() && velocity.is_finite() && turning.is_finite() && angle.is_finite())
        {
            return None;
        }
        let same = extra.length() < NOT_MOVED
            && angle.abs() < NOT_TURNED
            && (velocity - (was.velocity - at_rest)).length() < NOT_SPED
            && (turning - was.turning()).length() < NOT_SPED;
        if same {
            return None;
        }
        let turned = (DQuat::from_axis_angle(axis, angle) * was.turned).normalize();
        Some(Body {
            center: was.center + extra,
            velocity: velocity + at_rest,
            turned,
            spin: turned.inverse() * turning,
            ..*was
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        attitude::Inertia,
        collision::{Bounds, Mesh, PlacedMesh},
    };

    const DT: f64 = 0.02;

    /// A closed box, centred on the origin.
    fn cuboid(half: DVec3) -> Collider {
        let corners = (0..8)
            .map(|k| {
                DVec3::new(
                    if k & 1 == 0 { -half.x } else { half.x },
                    if k & 2 == 0 { -half.y } else { half.y },
                    if k & 4 == 0 { -half.z } else { half.z },
                )
            })
            .collect();
        let mesh = Mesh::new(
            corners,
            vec![
                [0, 2, 1],
                [1, 2, 3],
                [4, 5, 6],
                [5, 7, 6],
                [0, 1, 4],
                [1, 5, 4],
                [2, 6, 3],
                [3, 6, 7],
                [0, 4, 2],
                [2, 4, 6],
                [1, 3, 5],
                [3, 7, 5],
            ],
        );
        let bounds: Bounds = mesh.bounds().unwrap();
        Collider {
            parts: vec![PlacedMesh {
                mesh: Arc::new(mesh),
                pose: Pose::IDENTITY,
                bounds,
            }],
        }
    }

    fn member(id: u64, center: DVec3, velocity: DVec3, mass: f64, half: DVec3) -> Member {
        Member {
            id: VesselId(id),
            body: Body {
                center,
                velocity,
                turned: DQuat::IDENTITY,
                spin: DVec3::ZERO,
                mass: MassProperties {
                    mass,
                    center_of_mass: DVec3::ZERO,
                    inertia: Inertia::about_axes(mass, mass, mass),
                },
                fixed: false,
            },
            held: DVec3::ZERO,
            collider: cuboid(half),
        }
    }

    /// What vessels do between contacts: go on as they are moving.
    fn coast(member: &mut Member, dt: f64) {
        let body = &mut member.body;
        body.center += body.velocity * dt;
        body.turned = (DQuat::from_scaled_axis(body.turning() * dt) * body.turned).normalize();
    }

    /// Run them for `seconds`, the way the universe does, and how fast they met each time they did.
    fn run(members: &mut [Member; 2], seconds: f64) -> Vec<f64> {
        let mut contacts = Contacts::default();
        let mut touches = Vec::new();
        let ticks = (seconds / DT).round() as usize;
        for tick in 0..ticks {
            let met = tick_of(&mut contacts, members, tick);
            touches.extend(met);
        }
        touches
    }

    fn tick_of(contacts: &mut Contacts, members: &mut [Member; 2], tick: usize) -> Option<f64> {
        for member in members.iter_mut() {
            coast(member, DT);
        }
        let outcome = contacts.step(members[0].clone(), members[1].clone(), tick as f64 * DT, DT);
        for (id, body) in outcome.moved {
            members.iter_mut().find(|m| m.id == id).unwrap().body = body;
        }
        outcome.met
    }

    /// Linear and angular momentum about the origin.
    fn momentum(members: &[Member]) -> (DVec3, DVec3) {
        let (mut linear, mut angular) = (DVec3::ZERO, DVec3::ZERO);
        for m in members {
            let b = &m.body;
            linear += b.velocity * b.mass.mass;
            angular += b.center.cross(b.velocity) * b.mass.mass
                + b.turned * (b.mass.inertia.tensor * b.spin);
        }
        (linear, angular)
    }

    #[test]
    fn a_head_on_hit_gives_back_a_share_of_the_closing_speed() {
        let half = DVec3::splat(1.0);
        let mut both = [
            member(1, DVec3::Z * 3.0, DVec3::NEG_Z * 2.0, 1000.0, half),
            member(2, DVec3::ZERO, DVec3::ZERO, 3000.0, half),
        ];
        let touches = run(&mut both, 1.0);
        assert_eq!(touches.len(), 1, "{touches:?}");
        assert!((touches[0] - 2.0).abs() < 1e-6, "{touches:?}");
        let parting = (both[0].body.velocity - both[1].body.velocity).z;
        assert!((parting - RESTITUTION * 2.0).abs() < 0.1, "{parting}");
        // The heavier one takes what the lighter one lost.
        assert!(
            both[1].body.velocity.z < -0.5,
            "{:?}",
            both[1].body.velocity
        );
    }

    #[test]
    fn nothing_happens_to_what_never_touches() {
        let half = DVec3::splat(1.0);
        let mut both = [
            member(1, DVec3::Z * 3.0, DVec3::Z * 3.0, 1000.0, half),
            member(2, DVec3::ZERO, DVec3::new(0.1, -0.2, 0.0), 3000.0, half),
        ];
        both[0].body.spin = DVec3::new(0.3, -0.2, 0.1);
        let velocities = (both[0].body.velocity, both[1].body.velocity);
        let spin = both[0].body.spin;
        assert!(run(&mut both, 0.5).is_empty());
        assert_eq!((both[0].body.velocity, both[1].body.velocity), velocities);
        assert_eq!(both[0].body.spin, spin);
    }

    #[test]
    fn momentum_goes_on_whatever_the_contact() {
        let half = DVec3::new(1.0, 2.0, 1.0);
        let mut both = [
            member(
                1,
                DVec3::new(0.7, -0.3, 2.6),
                DVec3::new(0.2, 0.7, -1.3),
                800.0,
                half,
            ),
            member(2, DVec3::ZERO, DVec3::new(0.1, -0.2, 0.4), 2500.0, half),
        ];
        both[0].body.spin = DVec3::new(0.1, -0.2, 0.05);
        both[1].body.spin = DVec3::new(-0.03, 0.02, 0.2);
        let before = momentum(&both);
        assert!(!run(&mut both, 0.5).is_empty());
        let after = momentum(&both);
        assert!((before.0 - after.0).length() < 1e-6, "{before:?} {after:?}");
        // Easing surfaces apart moves them, so this is close, not exact.
        assert!(
            (before.1 - after.1).length() < 0.01 * before.1.length(),
            "{before:?} {after:?}"
        );
    }

    /// Hit off to one side, it starts to turn; hit through its centre, it does not.
    #[test]
    fn where_it_is_hit_decides_whether_it_turns() {
        let spin_after = |x: f64| {
            let mut both = [
                member(
                    1,
                    DVec3::new(x, 0.0, 2.2),
                    DVec3::NEG_Z,
                    1000.0,
                    DVec3::splat(1.0),
                ),
                member(2, DVec3::ZERO, DVec3::ZERO, 1000.0, DVec3::splat(1.0)),
            ];
            run(&mut both, 0.5);
            (both[0].body.spin.length(), both[1].body.spin.length())
        };
        let (straight, _) = spin_after(0.0);
        let (a, b) = spin_after(1.5);
        // Dead centre is not quite no spin at all, the solver's contact points not being perfectly
        // even, but it is nothing to what being hit off to one side does.
        assert!(
            a > 10.0 * straight && b > 10.0 * straight,
            "{straight} {a} {b}"
        );
    }

    /// Pressed together, they stay together, without being hit anew over and over.
    #[test]
    fn leaning_on_it_is_one_touch_and_no_bouncing() {
        let half = DVec3::splat(1.0);
        let mut both = [
            member(1, DVec3::Z * 2.05, DVec3::NEG_Z * 0.01, 1000.0, half),
            member(2, DVec3::ZERO, DVec3::ZERO, 1000.0, half),
        ];
        let mut contacts = Contacts::default();
        let mut touches = 0;
        for tick in 0..300 {
            // Something keeps pushing the first down on the second.
            both[0].body.velocity.z -= 0.01;
            touches += tick_of(&mut contacts, &mut both, tick).iter().count();
        }
        assert_eq!(touches, 1);
        let apart = both[0].body.center.z - both[1].body.center.z;
        assert!((apart - 2.0).abs() < 0.05, "{apart}");
        let closing = (both[1].body.velocity - both[0].body.velocity).z;
        assert!(closing.abs() < 0.05, "{closing}");
    }

    #[test]
    fn what_cannot_be_moved_stays_where_it_is() {
        let half = DVec3::splat(1.0);
        let mut both = [
            member(1, DVec3::Z * 3.0, DVec3::NEG_Z * 2.0, 1000.0, half),
            member(2, DVec3::ZERO, DVec3::ZERO, 1000.0, half),
        ];
        both[1].body.fixed = true;
        run(&mut both, 1.0);
        assert_eq!(both[1].body.velocity, DVec3::ZERO);
        assert_eq!(both[1].body.center, DVec3::ZERO);
        assert!(
            (both[0].body.velocity.z - RESTITUTION * 2.0).abs() < 0.1,
            "{:?}",
            both[0].body
        );
    }

    /// Sliding along a wall it slows, and pressing harder slows it more.
    #[test]
    fn friction_takes_up_what_pressing_allows() {
        let slide = |into: f64| {
            let half = DVec3::new(1.0, 1.0, 1.0);
            let mut both = [
                member(1, DVec3::Z * 2.2, DVec3::new(2.0, 0.0, -into), 1000.0, half),
                member(
                    2,
                    DVec3::ZERO,
                    DVec3::ZERO,
                    1000.0,
                    DVec3::new(50.0, 50.0, 1.0),
                ),
            ];
            both[1].body.fixed = true;
            run(&mut both, 1.0);
            both[0].body.velocity.x
        };
        assert!(slide(0.5) < 2.0);
        assert!(slide(1.0) < slide(0.5));
    }

    /// A hit far too fast to see between ticks is still a hit.
    #[test]
    fn nothing_goes_through_at_any_speed() {
        let mut both = [
            member(
                1,
                DVec3::Z * 30.0,
                DVec3::NEG_Z * 800.0,
                1000.0,
                DVec3::splat(0.5),
            ),
            member(
                2,
                DVec3::ZERO,
                DVec3::ZERO,
                1000.0,
                DVec3::new(5.0, 5.0, 0.05),
            ),
        ];
        let touches = run(&mut both, 0.2);
        assert!(!touches.is_empty());
        assert!(
            both[0].body.center.z > both[1].body.center.z,
            "{:?}",
            both[0].body
        );
        assert!(
            both[0].body.velocity.z > both[1].body.velocity.z,
            "{:?}",
            both[0].body
        );
    }
}
