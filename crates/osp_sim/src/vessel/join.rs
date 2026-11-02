//! Two vessels in flight becoming one, and coming apart again.
//!
//! The parts are the propulsion's to join. Here the pair's centre of mass goes on as it was:
//! the new vessel is where it was and moves as it did, keeping all the linear and angular
//! momentum they had between them, and turned as this vessel was. If the two nodes were not quite
//! together, the other vessel is the one moved to them.

use glam::DVec3;

use super::{AttitudeTrack, Hold, Vessel, free_motion};
use crate::{
    attitude::{self, Attitude, MassProperties},
    orbit::StateVector,
    propulsion::{Port, Propulsion},
    system::SolarSystem,
    time::SimTime,
};

/// The momentum a vessel has about its own centre of mass, in inertial axes.
fn spin_momentum(mass: &MassProperties, attitude: &Attitude) -> DVec3 {
    attitude.orientation * (mass.inertia.tensor * attitude.angular_velocity)
}

impl Vessel {
    /// Whether `other` could join this vessel at these nodes. Only vessels in flight and in one
    /// piece can.
    pub(crate) fn can_join(
        &self,
        other: &Vessel,
        at: Port,
        to: Port,
        roll: f64,
    ) -> Result<(), String> {
        for vessel in [self, other] {
            if vessel.is_landed() {
                return Err(format!("{} is on the ground", vessel.name));
            }
            if vessel.destroyed {
                return Err(format!("{} has broken up", vessel.name));
            }
        }
        (self.propulsion)
            .join_frame(at, &other.propulsion, to, roll)
            .map(|_| ())
    }

    /// Make `other` part of this vessel, its node `to` joined to this vessel's `at`, turned
    /// `roll`. Nothing changes on an error.
    pub(crate) fn join(
        &mut self,
        mut other: Vessel,
        at: Port,
        to: Port,
        roll: f64,
        system: &SolarSystem,
        time: SimTime,
    ) -> Result<(), String> {
        self.can_join(&other, at, to, roll)?;
        let (mine, theirs) = (
            attitude::mass_properties(&self.propulsion),
            attitude::mass_properties(&other.propulsion),
        );
        let state = self.state_at(system, time);
        let their_state = system.transform(
            other.state_at(system, time),
            other.parent,
            self.parent,
            time,
        );
        let turned = self.attitude_state(system, time);
        let their_turn = other.attitude_state(system, time);

        // The pair's centre of mass, and its momentum about that.
        let total = mine.mass + theirs.mass;
        let position = (state.position * mine.mass + their_state.position * theirs.mass) / total;
        let velocity = (state.velocity * mine.mass + their_state.velocity * theirs.mass) / total;
        let orbital = |s: &StateVector, mass: f64| {
            (s.position - position).cross(s.velocity - velocity) * mass
        };
        let momentum = spin_momentum(&mine, &turned)
            + spin_momentum(&theirs, &their_turn)
            + orbital(&state, mine.mass)
            + orbital(&their_state, theirs.mass);

        let parts = std::mem::replace(&mut other.propulsion, Propulsion::nothing());
        self.propulsion.join(parts, at, to, roll)?;
        self.mass = attitude::mass_properties(&self.propulsion);
        self.motion = free_motion(
            system.body(self.parent),
            StateVector::new(position, velocity),
            time,
        );
        self.attitude = AttitudeTrack {
            epoch: time,
            state: Attitude {
                orientation: turned.orientation,
                angular_velocity: self.mass.inertia.tensor.inverse()
                    * (turned.orientation.inverse() * momentum),
            },
            inertia: self.mass.inertia,
            hold: Hold::Free,
        };
        self.flight.mass = self.mass.mass;
        self.gear.clear();
        self.structure.clear();
        self.joint_loads.clear();
        self.debris &= other.debris;
        self.propellant_settled &= other.propellant_settled;
        self.record(time, format!("Joined {}", other.name));
        self.propulsion.keep_vessel(other);
        Ok(())
    }

    /// This vessel's parts and where they are, on the vessel that joined another and has just let
    /// go of it. Whatever it was before, it is a vessel again, not debris.
    pub(super) fn fly_again(mut self, parts: Vessel, time: SimTime) -> Vessel {
        self.record(time, parts.log[parts.log.len() - 1].message.clone());
        self.propulsion = parts.propulsion;
        self.parent = parts.parent;
        self.motion = parts.motion;
        self.attitude = parts.attitude;
        self.flight.mass = parts.mass.mass;
        self.mass = parts.mass;
        self.near_surface = parts.near_surface;
        self.debris = false;
        self.rotation_input = DVec3::ZERO;
        self.translation_input = DVec3::ZERO;
        self.gear.clear();
        self.wheels_down = false;
        self.structure.clear();
        self.joint_loads.clear();
        self.ullage_push = false;
        // What it held turned round with the vessel it was on.
        self.sas.release();
        self
    }
}

#[cfg(test)]
mod tests {
    use glam::DQuat;

    use super::*;
    use crate::{
        parts::PartId,
        rendezvous::Target,
        staging::Activation,
        system::tests::rss,
        universe::{PHYSICS_STEP, Universe},
        vessel::{
            Motion, VesselEventKind, VesselId,
            tests::{catalog, leo_vessel, lunar_upper},
        },
    };

    /// Mass, momentum and angular momentum about the origin of the parent, of `vessels`.
    fn totals(u: &Universe) -> (f64, DVec3, DVec3) {
        let (mut mass, mut momentum, mut angular) = (0.0, DVec3::ZERO, DVec3::ZERO);
        let t = u.time();
        for v in &u.vessels {
            let props = attitude::mass_properties(&v.propulsion);
            let state = v.state_at(&u.system, t);
            mass += props.mass;
            momentum += state.velocity * props.mass;
            angular += state.position.cross(state.velocity) * props.mass
                + spin_momentum(&props, &v.attitude_state(&u.system, t));
        }
        (mass, momentum, angular)
    }

    /// Where each part of `vessel` is, relative to its parent, by name.
    fn places(u: &Universe, vessel: &Vessel) -> Vec<(String, DVec3)> {
        let t = u.time();
        let base = vessel.base_at(&u.system, t);
        let turn = vessel.attitude_at(&u.system, t);
        let floor = DVec3::Z * vessel.propulsion.parts().base();
        (vessel.propulsion.parts().parts().iter())
            .map(|p| (p.name.clone(), base + turn * (p.position - floor)))
            .collect()
    }

    /// Two lunar upper stages in the same orbit, nose to nose where the joint will hold them,
    /// `b` tumbling and drifting: `(a's top, b's top)`.
    fn pair() -> (
        Universe,
        VesselId,
        VesselId,
        (PartId, String),
        (PartId, String),
    ) {
        let system = rss();
        let (a, t) = leo_vessel(&system);
        let def = lunar_upper();
        let start = crate::data::OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 28.6,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let mut b = Vessel::in_orbit(&def, &catalog(), &system, &start, t).unwrap();
        let top = |v: &Vessel| {
            let tree = v.propulsion.parts();
            let highest = (tree.free_nodes().into_iter())
                .filter(|(_, n)| n == "top")
                .max_by(|x, y| {
                    let z = |id: PartId| tree.get(id).unwrap().position.z;
                    z(x.0).total_cmp(&z(y.0))
                });
            highest.expect("a free top node")
        };
        let (at, to) = (top(&a), top(&b));

        // Put `b` exactly where the nodes meet.
        let frame = a
            .propulsion
            .join_frame(
                Port {
                    part: at.0,
                    node: &at.1,
                },
                &b.propulsion,
                Port {
                    part: to.0,
                    node: &to.1,
                },
                0.5,
            )
            .unwrap();
        let state = a.state_at(&system, t);
        let turn = a.attitude_at(&system, t);
        let world = |v: &Vessel, p: DVec3| {
            let floor = DVec3::Z * v.propulsion.parts().base();
            p - floor - v.mass.center_of_mass
        };
        let their_com = DVec3::Z * b.propulsion.parts().base() + b.mass.center_of_mass;
        let offset = turn * world(&a, frame.point(their_com));
        b.motion = Motion::Coasting(crate::orbit::Orbit::new(
            match &a.motion {
                Motion::Coasting(o) => o.mu(),
                _ => unreachable!(),
            },
            t,
            StateVector::new(state.position + offset, state.velocity),
        ));
        b.attitude.state.orientation = (turn * frame.rotation).normalize();
        let mut u = Universe::new(system, vec![a, b], t);
        let (ida, idb) = (u.vessels[0].id(), u.vessels[1].id());
        u.focus = Some(ida);
        // A little something to conserve.
        u.vessels[1].attitude.state.angular_velocity = DVec3::new(0.02, -0.01, 0.03);
        u.vessels[0].attitude.state.angular_velocity = DVec3::new(-0.01, 0.005, 0.0);
        let drift = u.vessels[1].state_at(&u.system, t);
        u.vessels[1].motion = Motion::Coasting(crate::orbit::Orbit::new(
            match &u.vessels[0].motion {
                Motion::Coasting(o) => o.mu(),
                _ => unreachable!(),
            },
            t,
            StateVector::new(drift.position, drift.velocity + DVec3::new(0.3, -0.1, 0.05)),
        ));
        (u, ida, idb, at, to)
    }

    fn near(a: f64, b: f64, tolerance: f64) -> bool {
        (a - b).abs() <= tolerance * (1.0 + a.abs().max(b.abs()))
    }

    #[test]
    fn a_join_keeps_all_the_momentum_they_had() {
        let (mut u, a, b, at, to) = pair();
        let before = totals(&u);
        let places_a = places(&u, &u.vessels[0]);
        let places_b = places(&u, &u.vessels[1]);
        u.join(
            a,
            Port {
                part: at.0,
                node: &at.1,
            },
            b,
            Port {
                part: to.0,
                node: &to.1,
            },
            0.5,
        )
        .unwrap();
        assert_eq!(u.vessels.len(), 1);
        let after = totals(&u);
        assert!(near(before.0, after.0, 1e-12), "{} {}", before.0, after.0);
        assert!((before.1 - after.1).length() < 1e-9 * before.1.length());
        assert!(
            (before.2 - after.2).length() < 1e-9 * before.2.length(),
            "{} {}",
            before.2,
            after.2
        );
        // The parts stayed where they were, as the nodes were together.
        let now = places(&u, &u.vessels[0]);
        let count = places_a.len();
        for ((name, was), (_, is)) in places_a.iter().zip(&now[..count]) {
            assert!((*was - *is).length() < 1e-6, "{name}: {was} {is}");
        }
        for ((name, was), (_, is)) in places_b.iter().zip(&now[count..]) {
            assert!((*was - *is).length() < 1e-6, "{name}: {was} {is}");
        }
    }

    #[test]
    fn letting_go_keeps_the_momentum_and_gives_back_the_vessel() {
        let (mut u, a, b, at, to) = pair();
        // What only the vessel knows comes back with it.
        u.vessels[1].name = "Eagle".into();
        u.vessels[1].set_rcs(true);
        u.vessels[1].target = Some(Target::Vessel(a));
        u.vessels[0].target = Some(Target::Vessel(b));
        let name_a = u.vessels[0].name.clone();
        let mass_a = attitude::mass_properties(&u.vessels[0].propulsion);
        let mass_b = attitude::mass_properties(&u.vessels[1].propulsion);
        u.focus = Some(b);
        u.join(
            a,
            Port {
                part: at.0,
                node: &at.1,
            },
            b,
            Port {
                part: to.0,
                node: &to.1,
            },
            0.5,
        )
        .unwrap();
        // Whoever flew it flies the two; whoever targeted it does not.
        assert_eq!(u.focus, Some(a));
        assert_eq!(u.vessels[0].target, None);
        let port = u.vessels[0].propulsion.joined_parts().next().unwrap();
        let before = totals(&u);
        let places_all = places(&u, &u.vessels[0]);
        assert!(u.perform(a, Activation::decoupler(port)));
        assert_eq!(u.vessels.len(), 2);
        let after = totals(&u);
        assert!(near(before.0, after.0, 1e-12));
        assert!((before.1 - after.1).length() < 1e-9 * before.1.length());

        let (kept, left) = (&u.vessels[0], &u.vessels[1]);
        assert_eq!(kept.name, name_a);
        assert_eq!(left.name, "Eagle");
        assert!(left.rcs() && !left.is_debris());
        assert_ne!(left.id(), kept.id());
        let log = &left.log[left.log.len() - 1].message;
        assert!(log.contains("Separated"), "{log}");
        for (x, y) in [(&kept.mass, &mass_a), (&left.mass, &mass_b)] {
            assert!(near(x.mass, y.mass, 1e-12));
            assert!((x.center_of_mass - y.center_of_mass).length() < 1e-8);
            let d = x.inertia.tensor - y.inertia.tensor;
            let size = y.inertia.tensor.x_axis.length();
            assert!(
                d.x_axis.length() + d.y_axis.length() + d.z_axis.length() < 1e-7 * size,
                "{:?} {:?}",
                x.inertia,
                y.inertia
            );
        }
        // It is pointing as it did before it joined, in its own frame: its nose +Z again.
        let t = u.time();
        let nose = left.attitude_at(&u.system, t) * DVec3::Z;
        let theirs = kept.attitude_at(&u.system, t) * DVec3::Z;
        assert!(nose.dot(theirs) < -0.999, "{nose} {theirs}");
        // And it is where it was, near enough, its parts pushed apart by nothing but this tick.
        let now_a = places(&u, kept);
        let now_b = places(&u, left);
        for ((name, was), (_, is)) in places_all.iter().zip(now_a.iter().chain(&now_b)) {
            assert!((*was - *is).length() < 1e-6, "{name}: {was} {is}");
        }
        // They go apart along the line of the nodes.
        let relative = left.state_at(&u.system, t).velocity - kept.state_at(&u.system, t).velocity;
        assert!(relative.length() > 0.1);
        // It is flown as it was: its own stage key.
        assert_eq!(left.propulsion.joined_parts().count(), 0);
    }

    #[test]
    fn the_stack_flies_on_the_engine_of_the_vessel_it_is_flown_from() {
        let (mut u, a, b, at, to) = pair();
        u.join(
            a,
            Port {
                part: at.0,
                node: &at.1,
            },
            b,
            Port {
                part: to.0,
                node: &to.1,
            },
            0.5,
        )
        .unwrap();
        let v = &mut u.vessels[0];
        let tree_mass = v.propulsion.mass();
        // The burn executor's way of lighting up: only this vessel's engines.
        v.propulsion.arm_bottom();
        let lit: Vec<_> = (v.propulsion.engines.iter())
            .filter(|e| v.propulsion.engine_armed(e.part))
            .map(|e| e.direction)
            .collect();
        assert!(!lit.is_empty());
        assert!(lit.iter().all(|d| d.z > 0.99), "{lit:?}");
        // Its RCS settle the propellant, on the two together.
        v.set_rcs(true);
        v.translation_input = DVec3::Z;
        for _ in 0..(10.0 / PHYSICS_STEP.as_seconds()) as usize {
            u.step_physics();
        }
        let v = &mut u.vessels[0];
        v.translation_input = DVec3::ZERO;
        v.set_throttle(1.0);
        let nose = |u: &Universe| u.vessels[0].attitude_at(&u.system, u.time()) * DVec3::Z;
        let start = u.vessels[0].state_at(&u.system, u.time());
        let along = nose(&u);
        let mut destroyed = false;
        for _ in 0..(60.0 / PHYSICS_STEP.as_seconds()) as usize {
            let events = u.step_physics();
            destroyed |= events
                .iter()
                .any(|e| matches!(e.event.kind, VesselEventKind::Destroyed));
        }
        assert!(!destroyed);
        let v = &u.vessels[0];
        let log: Vec<_> = v.log.iter().map(|e| e.message.as_str()).collect();
        assert!(v.flight.thrust > 0.0, "{log:?}");
        assert!(v.propulsion.mass() < tree_mass);
        let gained = v.state_at(&u.system, u.time()).velocity - start.velocity;
        assert!(gained.dot(along) > 10.0, "{gained}");
    }

    /// Nothing stops a solid, not even a join; anything else goes out until it is let go.
    #[test]
    fn a_join_puts_out_every_engine_of_the_other_vessel_but_a_solid() {
        let (mut p, q) = (
            Propulsion::from_craft(&lunar_upper(), &catalog()).unwrap(),
            Propulsion::from_craft(&crate::vessel::tests::pathfinder(), &catalog()).unwrap(),
        );
        let mut other = q.clone();
        // Lit, the booster solid or not: which it is decides.
        other.activate_next_sequence().unwrap();
        let armed: Vec<_> = (other.engines.iter())
            .filter(|e| other.engine_armed(e.part))
            .map(|e| (e.part, e.is_solid()))
            .collect();
        assert!(!armed.is_empty());
        let at = p.parts().free_nodes()[0].clone();
        let to = other.parts().free_nodes()[0].clone();
        let port = p
            .join(
                other,
                Port {
                    part: at.0,
                    node: &at.1,
                },
                Port {
                    part: to.0,
                    node: &to.1,
                },
                0.0,
            )
            .unwrap();
        let shift = port.0 - to.0.0;
        for &(part, solid) in &armed {
            assert_eq!(p.engine_armed(PartId(part.0 + shift)), solid);
        }
        let released = p.perform(Activation::decoupler(port), false).unwrap();
        let back = Propulsion::come_home(released.released.into_iter().next().unwrap());
        for &(part, _) in &armed {
            assert!(back.engine_armed(part));
        }
    }

    /// A little to one side, the vessel that joined is the one that moves to the node, and the
    /// momentum does not mind.
    #[test]
    fn nodes_a_little_apart_come_together() {
        let (mut u, a, b, at, to) = pair();
        let t = u.time();
        let state = u.vessels[1].state_at(&u.system, t);
        let mu = match &u.vessels[1].motion {
            Motion::Coasting(o) => o.mu(),
            _ => unreachable!(),
        };
        let off = StateVector::new(state.position + DVec3::new(0.2, 0.1, -0.1), state.velocity);
        u.vessels[1].motion = Motion::Coasting(crate::orbit::Orbit::new(mu, t, off));
        let turned = u.vessels[1].attitude.state.orientation;
        u.vessels[1].attitude.state.orientation =
            (turned * DQuat::from_rotation_x(0.03)).normalize();
        let before = totals(&u);
        u.join(
            a,
            Port {
                part: at.0,
                node: &at.1,
            },
            b,
            Port {
                part: to.0,
                node: &to.1,
            },
            0.5,
        )
        .unwrap();
        let after = totals(&u);
        assert!((before.1 - after.1).length() < 1e-9 * before.1.length());
        assert!((before.2 - after.2).length() < 1e-9 * before.2.length());
    }

    #[test]
    fn only_vessels_in_flight_and_in_one_piece_join() {
        let (mut u, a, b, at, to) = pair();
        let (at, to) = (
            Port {
                part: at.0,
                node: &at.1,
            },
            Port {
                part: to.0,
                node: &to.1,
            },
        );
        assert!(u.join(a, at, a, to, 0.0).is_err());
        u.vessels[1].destroyed = true;
        assert!(u.join(a, at, b, to, 0.0).is_err());
        assert_eq!(u.vessels.len(), 2);
        u.vessels[1].destroyed = false;
        let (at_ground, _) = crate::vessel::tests::pad_vessel(&u.system);
        let ground = u.add_vessel(at_ground);
        let wrong = u.vessel(ground).unwrap().propulsion.parts().free_nodes()[0].clone();
        assert!(
            (u.join(
                a,
                at,
                ground,
                Port {
                    part: wrong.0,
                    node: &wrong.1
                },
                0.0,
            ))
            .unwrap_err()
            .contains("on the ground")
        );
        // No such part, no such node.
        assert!(
            u.join(
                a,
                Port {
                    part: PartId(100_000),
                    node: "top"
                },
                b,
                to,
                0.0
            )
            .is_err()
        );
        assert!(
            u.join(
                a,
                Port {
                    part: at.part,
                    node: "nowhere"
                },
                b,
                to,
                0.0
            )
            .is_err()
        );
        assert_eq!(u.vessels.len(), 3);
    }
}
