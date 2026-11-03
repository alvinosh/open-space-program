//! Two vessels in flight becoming one, and coming apart again.
//!
//! The parts are the propulsion's to join. Here the pair's centre of mass goes on as it was:
//! the new vessel is where it was and moves as it did, keeping all the linear and angular
//! momentum they had between them, and turned as this vessel was. If the two nodes were not quite
//! together, the other vessel is the one moved to them.

use glam::{DQuat, DVec3};

use super::{AttitudeTrack, Hold, Vessel, free_motion};
use crate::{
    assembly::Pose,
    attitude::{self, Attitude, MassProperties},
    contact::Body,
    docking::{DockingPort, PortState},
    orbit::StateVector,
    parts::PartId,
    propulsion::{Port, Propulsion},
    system::SolarSystem,
    time::SimTime,
};

/// The momentum a vessel has about its own centre of mass, in inertial axes.
fn spin_momentum(mass: &MassProperties, attitude: &Attitude) -> DVec3 {
    attitude.orientation * (mass.inertia.tensor * attitude.angular_velocity)
}

impl Vessel {
    /// Where `port` of this vessel is and how it moves, in its parent's frame.
    pub fn port_state(&self, system: &SolarSystem, port: &DockingPort, time: SimTime) -> PortState {
        let attitude = self.attitude_state(system, time);
        PortState {
            at: self.point_state(system, port.position, time),
            out: attitude.orientation * port.direction,
            turning: attitude.angular_velocity_inertial(),
        }
    }

    /// Where `part` of this vessel is and how it moves, in its parent's frame: its docking port
    /// if it has one, or else its middle. `None` if it has left the vessel.
    pub fn part_state(
        &self,
        system: &SolarSystem,
        part: PartId,
        time: SimTime,
    ) -> Option<StateVector> {
        let port = (self.propulsion.docking_ports().iter()).find(|p| p.part == part);
        let point = match port {
            Some(port) => port.position,
            None => {
                let part = self.propulsion.parts().get(part)?;
                let middle = part.built.as_ref().map_or(DVec3::ZERO, |b| b.mass.center);
                part.position + part.rotation * middle
            }
        };
        Some(self.point_state(system, point, time))
    }

    /// Where a point of the assembly frame is and how it moves, in the parent's frame.
    fn point_state(&self, system: &SolarSystem, point: DVec3, time: SimTime) -> StateVector {
        let state = self.state_at(system, time);
        let attitude = self.attitude_state(system, time);
        let base = DVec3::Z * self.propulsion.parts().base();
        let arm = attitude.orientation * (point - base - self.mass.center_of_mass);
        let turning = attitude.angular_velocity_inertial();
        StateVector::new(state.position + arm, state.velocity + turning.cross(arm))
    }

    /// How the frame of `port` of this vessel is turned in space.
    pub(crate) fn port_turn(
        &self,
        system: &SolarSystem,
        port: &DockingPort,
        time: SimTime,
    ) -> DQuat {
        self.attitude_at(system, time) * port.frame
    }

    /// Put this vessel where its `port` is at `place`, facing `out` and no longer turning.
    pub(crate) fn put_port(
        &mut self,
        system: &SolarSystem,
        port: &DockingPort,
        place: StateVector,
        out: DVec3,
        time: SimTime,
    ) {
        let turned = self.attitude_at(system, time);
        let was = turned * port.direction;
        // Half a turn has no one way to go: about the vessel's own side.
        let turn = if was.dot(out) < -1.0 + 1e-9 {
            DQuat::from_axis_angle(turned * DVec3::X, std::f64::consts::PI)
        } else {
            DQuat::from_rotation_arc(was, out)
        };
        let orientation = (turn * turned).normalize();
        let base = DVec3::Z * self.propulsion.parts().base();
        let arm = orientation * (port.position - base - self.mass.center_of_mass);
        let center = StateVector::new(place.position - arm, place.velocity);
        self.motion = free_motion(system.body(self.parent), center, time);
        self.attitude = AttitudeTrack {
            epoch: time,
            state: Attitude {
                orientation,
                angular_velocity: DVec3::ZERO,
            },
            inertia: self.mass.inertia,
            hold: Hold::Free,
        };
    }

    /// This vessel as a contact sees it.
    pub(crate) fn as_body(&self, system: &SolarSystem, time: SimTime) -> Body {
        let state = self.state_at(system, time);
        let attitude = self.attitude_state(system, time);
        Body {
            center: state.position,
            velocity: state.velocity,
            turned: attitude.orientation,
            spin: attitude.angular_velocity,
            mass: self.mass,
            fixed: self.is_landed() && !self.rolls_when_pushed(),
        }
    }

    /// Where its centre of mass is in its assembly frame.
    pub(crate) fn held(&self) -> DVec3 {
        DVec3::Z * self.propulsion.parts().base() + self.mass.center_of_mass
    }

    /// Where the assembly frame of this vessel is in its parent's frame, as `body` has it.
    pub(crate) fn frame_of(&self, body: &Body) -> Pose {
        Pose::new(body.center - body.turned * self.held(), body.turned)
    }

    /// Where its assembly frame is now, in its parent's frame.
    pub fn assembly_frame(&self, system: &SolarSystem, time: SimTime) -> Pose {
        self.frame_of(&self.as_body(system, time))
    }

    /// Go on from `body`, as something has made it.
    pub(crate) fn set_body(&mut self, system: &SolarSystem, body: &Body, time: SimTime) {
        if body.fixed {
            return;
        }
        self.motion = free_motion(
            system.body(self.parent),
            StateVector::new(body.center, body.velocity),
            time,
        );
        self.attitude = AttitudeTrack {
            epoch: time,
            state: Attitude {
                orientation: body.turned,
                angular_velocity: body.spin,
            },
            inertia: self.mass.inertia,
            hold: Hold::Free,
        };
    }

    /// It has been hit too hard to survive.
    pub(crate) fn break_up(&mut self, time: SimTime, why: String) {
        if self.destroyed {
            return;
        }
        self.destroyed = true;
        self.propulsion.shut_down();
        self.rcs = false;
        self.sas.set_enabled(false);
        self.record(time, why);
    }

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
        self.record(time, format!("Docked with {}", other.name));
        self.propulsion.keep_vessel(other);
        Ok(())
    }

    /// This vessel's parts and where they are, on the vessel that joined another and has just let
    /// go of it, `from`. Whatever it was before, it is a vessel again, not debris.
    pub(super) fn fly_again(mut self, parts: Vessel, from: &str, time: SimTime) -> Vessel {
        self.record(time, format!("Undocked from {from}"));
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
        assert!(u.perform(a, Activation::undock(port)));
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
        assert!(log.contains("Undocked"), "{log}");
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
        let released = p.perform(Activation::undock(port), false).unwrap();
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

    /// A small craft with one port on its nose.
    fn with_port(name: &str, kind: &str, diameter: f64) -> crate::data::CraftDef {
        crate::test_data::records(&format!(
            "{{ id: '{name}', name: '{name}', parts: [{{ id: 'body', \
             shape: {{ shell: {{ bottom_diameter: {diameter}, sections: [{{ length: 3 }}], \
             cap_top: true, cap_bottom: true }} }}, structure: {{ mass: 1000 }}, \
             components: [{{ docking_port: {{ node: 'top', kind: '{kind}', impulse: 500 }} }}] }}] }}"
        ))
        .remove(0)
    }

    /// `a` and `b` in orbit together, `a` flown, targeting `b`, and `gap` m off its port.
    fn ports(
        a: &crate::data::CraftDef,
        b: &crate::data::CraftDef,
        gap: f64,
        closing: f64,
    ) -> Universe {
        let system = rss();
        let start = crate::data::OrbitStartDef {
            body: "Earth".into(),
            periapsis_altitude: 300e3,
            apoapsis_altitude: 300e3,
            inclination: 0.0,
            longitude_of_ascending_node: 0.0,
            plane: None,
            argument_of_periapsis: 0.0,
            mean_anomaly: 0.0,
            spin: [0.0; 3],
        };
        let t = SimTime::J2000;
        let spawn = |def: &crate::data::CraftDef| {
            Vessel::in_orbit(def, &catalog(), &system, &start, t).unwrap()
        };
        let (first, second) = (spawn(a), spawn(b));
        let mut u = Universe::new(system, vec![first, second], t);
        u.focus = Some(u.vessels[0].id());
        u.vessels[0].target = Some(Target::Vessel(u.vessels[1].id()));
        assert!(u.line_up_ports(0, gap, closing));
        u
    }

    fn run_for(u: &mut Universe, seconds: f64) -> Vec<VesselEventKind> {
        let mut seen = Vec::new();
        for _ in 0..(seconds / PHYSICS_STEP.as_seconds()) as usize {
            seen.extend(u.step_physics().into_iter().map(|e| e.event.kind));
        }
        seen
    }

    #[test]
    fn ports_that_touch_slowly_and_lined_up_capture() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 0.5, 0.2);
        let seen = u.port_approach(0, u.time()).unwrap();
        assert!(seen.alignment.angle < 1e-6, "{}", seen.alignment.angle);
        assert!((seen.alignment.gap - 0.5).abs() < 1e-6);
        assert!((seen.alignment.closing - 0.2).abs() < 1e-6);
        let (id, mass) = (u.vessels[0].id(), a_mass(&u));
        let events = run_for(&mut u, 5.0);
        assert_eq!(u.vessels.len(), 1);
        assert_eq!(
            events
                .iter()
                .filter(|e| matches!(e, VesselEventKind::Docked))
                .count(),
            1
        );
        // The one being flown is the one that stays.
        assert_eq!(u.vessels[0].id(), id);
        assert_eq!(u.focus, Some(id));
        assert!((a_mass(&u) - mass).abs() < 1e-6);
        assert!(
            u.vessels[0]
                .log
                .iter()
                .any(|e| e.message == "Docked with b")
        );
        assert_eq!(
            u.vessels[0]
                .propulsion
                .joined_vessels()
                .next()
                .unwrap()
                .name,
            "b"
        );
        let free = |u: &Universe| {
            (u.vessels[0].propulsion.docking_ports().iter())
                .filter(|p| p.free)
                .count()
        };
        assert_eq!(free(&u), 0);
    }

    /// Caught at whatever roll it came in at: nothing turns about the line they meet along.
    #[test]
    fn a_vessel_is_caught_at_the_roll_it_comes_in_at() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 0.5, 0.2);
        let t = u.time();
        // Turned about the line of the ports.
        let out = u.port_approach(0, t).unwrap().alignment.out;
        let turned = u.vessels[0].attitude_at(&u.system, t);
        u.vessels[0].attitude.state.orientation =
            (DQuat::from_axis_angle(out, 1.1) * turned).normalize();
        let id = u.vessels[0].id();
        let was = u.vessels[1].attitude_at(&u.system, t);
        run_for(&mut u, 5.0);
        assert_eq!(u.vessels.len(), 1);
        let port = u.vessels[0].propulsion.joined_parts().next().unwrap();
        assert!(u.perform(id, Activation::undock(port)));
        let now = u.vessels[1].attitude_at(&u.system, u.time());
        assert!(was.angle_between(now) < 1e-6, "{}", was.angle_between(now));
    }

    /// A sloppy approach still catches: soft capture draws the ports together and lines them up,
    /// and what it took is equal and opposite.
    #[test]
    fn soft_capture_takes_up_what_the_pilot_left() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 0.3, 0.2);
        let t = u.time();
        let seen = u.port_approach(0, t).unwrap().alignment;
        let side = seen.out.any_orthonormal_vector();
        let body = u.vessels[0].as_body(&u.system, t);
        let turned = DQuat::from_axis_angle(side, 8f64.to_radians()) * body.turned;
        let moved = crate::contact::Body {
            center: body.center + seen.out.cross(side) * 0.1,
            turned,
            ..body
        };
        u.vessels[0].set_body(&u.system, &moved, t);
        let seen = u.port_approach(0, t).unwrap().alignment;
        assert!(seen.problem().is_none(), "{seen:?}");
        assert!(seen.off_axis.length() > 0.1 && seen.angle > 5f64.to_radians());
        let before = totals(&u);
        // It catches, and is held in soft capture a while, before they are one.
        run_for(&mut u, 2.0);
        assert_eq!(u.vessels.len(), 2);
        assert!(u.in_soft_capture(u.vessels[0].id()));
        assert!(!u.can_use_rails());
        // Drawn together, without one going through the other.
        let closer = u.port_approach(0, u.time()).unwrap().alignment;
        assert!(
            closer.off_axis.length() < seen.off_axis.length(),
            "{closer:?}"
        );
        assert!(closer.angle < seen.angle, "{closer:?}");
        run_for(&mut u, 10.0);
        assert_eq!(u.vessels.len(), 1);
        let after = totals(&u);
        assert!((before.0 - after.0).abs() < 1e-9);
        let turning = u.vessels[0]
            .attitude_state(&u.system, u.time())
            .angular_velocity;
        assert!(turning.length() < 0.05, "{turning}");
    }

    /// However it comes in, what is flown never ends a tick sunk into what it hit.
    #[test]
    fn a_vessel_stops_at_the_surface_of_what_it_hits() {
        let (a, b) = (with_port("a", "probe", 2.0), with_port("b", "probe", 2.0));
        for closing in [0.2, 1.0, 5.0] {
            let system = rss();
            let (first, t) = leo_vessel(&system);
            let _ = first;
            let start = crate::data::OrbitStartDef {
                body: "Earth".into(),
                periapsis_altitude: 300e3,
                apoapsis_altitude: 300e3,
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: 0.0,
                spin: [0.0; 3],
            };
            let spawn = |def: &crate::data::CraftDef| {
                Vessel::in_orbit(def, &catalog(), &system, &start, t).unwrap()
            };
            let (p, q) = (spawn(&a), spawn(&b));
            let mut u = Universe::new(system, vec![p, q], t);
            u.focus = Some(u.vessels[0].id());
            // Nose to nose, 12 m apart on the line they meet along, closing.
            let there = u.vessels[1].as_body(&u.system, t);
            let out = there.turned * DVec3::Z;
            let port = u.vessels[0].propulsion.docking_ports()[0].clone();
            let far = u.vessels[1].propulsion.docking_ports()[0].clone();
            let place = u.vessels[1].port_state(&u.system, &far, t).at;
            u.vessels[0].put_port(
                &u.system,
                &port,
                StateVector::new(place.position + out * 12.0, place.velocity - out * closing),
                -out,
                t,
            );
            let (mut bumped, mut steps) = (false, 0);
            while !bumped && steps < 5_000 {
                bumped |= u
                    .step_physics()
                    .iter()
                    .any(|e| matches!(e.event.kind, VesselEventKind::Bumped { .. }));
                steps += 1;
                let now = u.time();
                let (x, y) = (&u.vessels[0], &u.vessels[1]);
                let (hull_x, hull_y) = (x.propulsion.parts().clone(), y.propulsion.parts().clone());
                let (mx, my) = (u.colliders.of(&hull_x), u.colliders.of(&hull_y));
                let (fx, fy) = (
                    u.vessels[0].assembly_frame(&u.system, now),
                    u.vessels[1].assembly_frame(&u.system, now),
                );
                // Solved over a tick, a hard hit may go in by a little.
                let eased = Pose::new(fx.position + out * 0.02, fx.rotation);
                assert!(!mx.touches(eased, &my, fy), "through it at {closing} m/s");
            }
            assert!(bumped, "{closing} m/s");
            // They have parted, and neither broke.
            run_for(&mut u, 1.0);
            let now = u.time();
            let parting = u.vessels[0].state_at(&u.system, now).velocity
                - u.vessels[1].state_at(&u.system, now).velocity;
            assert!(parting.dot(out) > 0.0, "{parting}");
            assert!(!u.vessels[0].is_destroyed() && !u.vessels[1].is_destroyed());
        }
    }

    /// Hit hard enough, both go.
    #[test]
    fn a_hard_enough_hit_breaks_them_up() {
        let (a, b) = (with_port("a", "probe", 2.0), with_port("b", "probe", 2.0));
        let mut u = {
            let system = rss();
            let start = crate::data::OrbitStartDef {
                body: "Earth".into(),
                periapsis_altitude: 300e3,
                apoapsis_altitude: 300e3,
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: 0.0,
                spin: [0.0; 3],
            };
            let t = SimTime::J2000;
            let p = Vessel::in_orbit(&a, &catalog(), &system, &start, t).unwrap();
            let q = Vessel::in_orbit(&b, &catalog(), &system, &start, t).unwrap();
            Universe::new(system, vec![p, q], t)
        };
        u.focus = Some(u.vessels[0].id());
        let t = u.time();
        let there = u.vessels[1].as_body(&u.system, t);
        let out = there.turned * DVec3::Z;
        let port = u.vessels[0].propulsion.docking_ports()[0].clone();
        let far = u.vessels[1].propulsion.docking_ports()[0].clone();
        let place = u.vessels[1].port_state(&u.system, &far, t).at;
        u.vessels[0].put_port(
            &u.system,
            &port,
            StateVector::new(place.position + out * 40.0, place.velocity - out * 30.0),
            -out,
            t,
        );
        let events = run_for(&mut u, 3.0);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, VesselEventKind::Destroyed))
        );
        assert!(u.vessels[0].is_destroyed() && u.vessels[1].is_destroyed());
    }

    /// All the vessels' mass.
    fn a_mass(u: &Universe) -> f64 {
        u.vessels.iter().map(|v| v.propulsion.mass()).sum()
    }

    /// Too fast to catch, they bump: nothing goes through anything, and the momentum they had is
    /// still theirs.
    #[test]
    fn ports_coming_in_too_fast_bump_and_part() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 10.0, 0.8);
        let before = totals(&u);
        let events = run_for(&mut u, 16.0);
        assert_eq!(u.vessels.len(), 2);
        let bumps = events
            .iter()
            .filter(|e| matches!(e, VesselEventKind::Bumped { .. }))
            .count();
        assert!(bumps >= 1, "{events:?}");
        let seen = u.port_approach(0, u.time()).unwrap().alignment;
        assert!(seen.gap > 0.0 && seen.closing < 0.0, "{seen:?}");
        let log = &u.vessels[0].log;
        let said = log
            .iter()
            .find(|e| e.message.starts_with("Bumped"))
            .unwrap();
        assert!(said.message.contains("Too fast"), "{}", said.message);
        // Momentum, over this long falling round the Earth: no more than a coast would do.
        let after = totals(&u);
        assert!((before.0 - after.0).abs() < 1e-9);
        let mut coast = ports(&a, &b, 200.0, 0.0);
        let start = totals(&coast);
        run_for(&mut coast, 16.0);
        let moved = (totals(&coast).1 - start.1).length();
        assert!(
            ((after.1 - before.1).length() - moved).abs() < 0.05 * moved,
            "{moved}"
        );
    }

    /// Missing the port by more than it can take, the hulls still meet.
    #[test]
    fn vessels_that_miss_the_port_hit_each_other() {
        let (a, b) = (with_port("a", "probe", 2.0), with_port("b", "drogue", 2.0));
        let mut u = ports(&a, &b, 10.0, 0.2);
        let t = u.time();
        let state = u.vessels[0].state_at(&u.system, t);
        let out = u.port_approach(0, t).unwrap().alignment.out;
        let side = out.any_orthonormal_vector();
        let mu = match &u.vessels[0].motion {
            Motion::Coasting(o) => o.mu(),
            _ => unreachable!(),
        };
        u.vessels[0].motion = Motion::Coasting(crate::orbit::Orbit::new(
            mu,
            t,
            StateVector::new(state.position + side * 1.0, state.velocity),
        ));
        let events = run_for(&mut u, 60.0);
        assert_eq!(u.vessels.len(), 2);
        assert!(
            events
                .iter()
                .any(|e| matches!(e, VesselEventKind::Bumped { .. })),
            "{events:?}"
        );
        let seen = u.port_approach(0, u.time()).unwrap().alignment;
        assert!(seen.gap > 0.0, "{seen:?}");
    }

    /// What has just let go is touching what it left, and may drift back on its last thrust: that
    /// is not a bump until they have been properly apart.
    #[test]
    fn what_has_just_let_go_does_not_bump_what_it_left() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 0.5, 0.2);
        let id = u.vessels[0].id();
        run_for(&mut u, 5.0);
        let port = u.vessels[0].propulsion.joined_parts().next().unwrap();
        assert!(u.perform(id, Activation::undock(port)));
        // Pushed straight back together as they part, without a port to catch.
        let t = u.time();
        let state = u.vessels[1].state_at(&u.system, t);
        let toward = (u.vessels[0].state_at(&u.system, t).position - state.position).normalize();
        let mu = match &u.vessels[1].motion {
            Motion::Coasting(o) => o.mu(),
            _ => unreachable!(),
        };
        u.vessels[1].motion = Motion::Coasting(crate::orbit::Orbit::new(
            mu,
            t,
            StateVector::new(state.position, state.velocity + toward * 2.0),
        ));
        u.vessels[0].target = None;
        let events = run_for(&mut u, 1.0);
        assert!(
            !events
                .iter()
                .any(|e| matches!(e, VesselEventKind::Bumped { .. })),
            "{events:?}"
        );
    }

    /// Time warp does not jump over the meeting: it stops short, and physics takes it from there.
    #[test]
    fn warp_stops_short_of_a_vessel_it_is_closing_on() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 1_000.0, 2.0);
        assert!(u.can_use_rails());
        let start = u.time();
        let outcome = u.advance_rails(start.add_seconds(3_600.0));
        assert!(outcome.events.is_empty());
        let flew = u.time().seconds_since(start);
        assert!(flew < 500.0, "it flew {flew} s on rails");
        assert!(!u.can_use_rails());
        // Slowing down in time is the pilot's to do: here it is slow enough to catch.
        assert!(u.line_up_ports(0, 0.5, 0.2));
        run_for(&mut u, 5.0);
        assert_eq!(u.vessels.len(), 1);
        // Rails again once there is nothing to meet.
        assert!(u.can_use_rails());
    }

    #[test]
    fn ports_that_do_not_fit_do_not_capture() {
        for (x, y) in [
            (with_port("a", "probe", 0.8), with_port("b", "probe", 0.8)),
            (with_port("a", "drogue", 0.8), with_port("b", "drogue", 0.8)),
            (
                with_port("a", "androgynous", 0.8),
                with_port("b", "probe", 0.8),
            ),
            (with_port("a", "probe", 0.8), with_port("b", "drogue", 1.5)),
        ] {
            let system = rss();
            let (first, t) = leo_vessel(&system);
            let _ = first;
            let start = crate::data::OrbitStartDef {
                body: "Earth".into(),
                periapsis_altitude: 300e3,
                apoapsis_altitude: 300e3,
                inclination: 0.0,
                longitude_of_ascending_node: 0.0,
                plane: None,
                argument_of_periapsis: 0.0,
                mean_anomaly: 0.0,
                spin: [0.0; 3],
            };
            let spawn = |def: &crate::data::CraftDef| {
                Vessel::in_orbit(def, &catalog(), &system, &start, t).unwrap()
            };
            let (p, q) = (spawn(&x), spawn(&y));
            let mut u = Universe::new(system, vec![p, q], t);
            u.vessels[0].target = Some(Target::Vessel(u.vessels[1].id()));
            // No way in, so nothing lines up, and they share a place without catching.
            assert!(u.port_approach(0, t).is_none());
            assert!(!u.line_up_ports(0, 0.05, 0.1));
            run_for(&mut u, 2.0);
            assert_eq!(u.vessels.len(), 2);
        }
    }

    #[test]
    fn undocked_vessels_do_not_catch_again_until_brought_back() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 0.5, 0.2);
        let id = u.vessels[0].id();
        run_for(&mut u, 5.0);
        let port = u.vessels[0].propulsion.joined_parts().next().unwrap();
        // Offered from either side of the join.
        let action = Activation::undock(port);
        let near = u.vessels[0].propulsion.parts().parent(port).unwrap().id;
        assert!(u.vessels[0].part_actions(port).contains(&action));
        assert!(u.vessels[0].part_actions(near).contains(&action));
        let before = totals(&u);
        assert!(u.perform(id, action));
        let after = totals(&u);
        assert!((before.1 - after.1).length() < 1e-9 * before.1.length());
        assert_eq!(u.vessels.len(), 2);
        // Their springs push them apart: 500 N s from each port.
        let t = u.time();
        let (p, q) = (&u.vessels[0], &u.vessels[1]);
        let opening =
            (p.state_at(&u.system, t).velocity - q.state_at(&u.system, t).velocity).length();
        let spring = 500.0 * 2.0 * (1.0 / p.propulsion.mass() + 1.0 / q.propulsion.mass());
        assert!((opening - spring).abs() < 1e-6, "{opening} {spring}");
        run_for(&mut u, 10.0);
        assert_eq!(u.vessels.len(), 2);
        // The target is gone with the id it had, so it is chosen again, and they meet again.
        u.vessels[0].target = Some(Target::Vessel(u.vessels[1].id()));
        assert!(u.line_up_ports(0, 0.5, 0.2));
        run_for(&mut u, 5.0);
        assert_eq!(u.vessels.len(), 1);
    }

    #[test]
    fn the_vessel_flown_is_the_one_that_stays_whichever_came_to_which() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 0.5, 0.2);
        let other = u.vessels[1].id();
        u.focus = Some(other);
        run_for(&mut u, 5.0);
        assert_eq!(u.vessels.len(), 1);
        assert_eq!(u.vessels[0].id(), other);
        assert_eq!(u.vessels[0].name, "b");
    }

    /// A body with a port on each side.
    fn with_two_ports() -> crate::data::CraftDef {
        crate::test_data::records(
            "{ id: 'two', name: 'two', parts: [\
             { id: 'body', shape: { shell: { bottom_diameter: 2, sections: [{ length: 3 }], \
             cap_top: true, cap_bottom: true } }, structure: { mass: 1000 } }, \
             { id: 'up', on: 'body.top', \
             shape: { shell: { bottom_diameter: 0.8, sections: [{ length: 1 }], \
             cap_top: true, cap_bottom: true } }, structure: { mass: 50 }, \
             components: [{ docking_port: { node: 'top', kind: 'drogue' } }] }, \
             { id: 'down', on: 'body.bottom', by: 'top', \
             shape: { shell: { bottom_diameter: 0.8, sections: [{ length: 1 }], \
             cap_top: true, cap_bottom: true } }, structure: { mass: 50 }, \
             components: [{ docking_port: { node: 'bottom', kind: 'drogue' } }] }] }",
        )
        .remove(0)
    }

    #[test]
    fn a_targeted_port_is_the_one_to_dock_with_and_to_steer_for() {
        let (a, b) = (with_port("a", "probe", 0.8), with_two_ports());
        let mut u = ports(&a, &b, 50.0, 0.0);
        let t = u.time();
        let other = u.vessels[1].id();
        let theirs = u.vessels[1].propulsion.docking_ports().to_vec();
        assert_eq!(theirs.len(), 2);
        // Left to itself it takes the nearest, so the other is one only a target can choose.
        let nearest = u.port_approach(0, t).unwrap().target_port.part;
        let far = theirs.iter().find(|p| p.part != nearest).unwrap();
        u.vessels[0].target = Some(Target::Part(other, far.part));
        let approach = u.port_approach(0, t).unwrap();
        assert_eq!(approach.target_port.part, far.part);
        assert_eq!(approach.target, other);

        // Steering for it means for the port, not for the vessel's middle.
        let port = u.vessels[1].port_state(&u.system, far, t).at;
        let aimed = u.target_state(0, t).unwrap();
        assert!((aimed.position - port.position).length() < 1e-6);
        let middle = u.vessels[1].state_at(&u.system, t).position;
        assert!((aimed.position - middle).length() > 0.5);
    }

    #[test]
    fn any_part_can_be_a_target_and_it_goes_when_the_part_or_the_vessel_does() {
        let (a, b) = (with_port("a", "probe", 0.8), with_two_ports());
        let mut u = ports(&a, &b, 50.0, 0.0);
        let t = u.time();
        let other = u.vessels[1].id();
        let body = u.vessels[1].propulsion.parts().parts()[0].id;
        assert!(
            u.vessels[1]
                .propulsion
                .docking_ports()
                .iter()
                .all(|p| p.part != body)
        );

        // Not a port: the middle of the part, and any port will still do to dock.
        u.vessels[0].target = Some(Target::Part(other, body));
        let aimed = u.target_state(0, t).unwrap();
        let expected = u.vessels[1].part_state(&u.system, body, t).unwrap();
        assert!((aimed.position - expected.position).length() < 1e-6);
        assert!(u.port_approach(0, t).is_some());

        // A part no longer on it: the vessel.
        u.vessels[0].target = Some(Target::Part(other, PartId(9999)));
        let aimed = u.target_state(0, t).unwrap();
        let vessel = u.vessels[1].state_at(&u.system, t);
        assert!((aimed.position - vessel.position).length() < 1e-6);

        u.remove_vessel(other);
        assert_eq!(u.vessels[0].target, None);
    }

    #[test]
    fn sas_knows_the_way_to_face_the_target_port() {
        let (a, b) = (with_port("a", "probe", 0.8), with_port("b", "drogue", 0.8));
        let mut u = ports(&a, &b, 50.0, 0.0);
        let t = u.time();
        let out = u.port_approach(0, t).unwrap().alignment.out;
        let (system, vessel) = (&u.system, &mut u.vessels[0]);
        vessel.set_sas_mode(system, t, crate::attitude::SasMode::TargetPort);
        u.step_physics();
        let wanted = u.vessels[0].target_port.unwrap();
        assert!((wanted + out).length() < 1e-3, "{wanted} {out}");
    }

    /// Flown from a port that looks another way, "forward" is out of that port.
    #[test]
    fn flying_from_a_port_turns_the_axes_with_it() {
        let craft: crate::data::CraftDef = crate::test_data::records(
            "{ id: 'side', name: 'side', parts: [{ id: 'body', \
             shape: { shell: { bottom_diameter: 2, sections: [{ length: 3 }], cap_top: true, \
             cap_bottom: true } }, structure: { mass: 1000 }, \
             nodes: [{ name: 'side', position: [1, 0, 1.5], direction: [1, 0, 0], size: 0.8 }], \
             components: [{ docking_port: { node: 'side' } }] }] }",
        )
        .remove(0);
        let mut p = Propulsion::from_craft(&craft, &catalog()).unwrap();
        assert_eq!(p.control_rotation(), DQuat::IDENTITY);
        let port = p.docking_ports()[0].clone();
        let control = Activation::new(port.part, crate::staging::Action::CONTROL);
        assert!(p.part_actions(port.part, false).contains(&control));
        p.perform(control, false).unwrap();
        let forward = p.control_rotation() * DVec3::Z;
        assert!((forward - DVec3::X).length() < 1e-12, "{forward}");
    }

    #[test]
    fn a_port_is_sized_from_what_it_carries() {
        let def: crate::sizing::ProceduralDef = crate::test_data::records(
            "{ docking_port: { diameter: 0.8, load: 100000, kind: 'drogue', impulse: 100 } }",
        )
        .remove(0);
        let design = def.size(&catalog()).unwrap();
        let [crate::component::ComponentDef::DockingPort(port)] = design.components.as_slice()
        else {
            panic!("{:?}", design.components);
        };
        assert_eq!(port.kind, crate::component::PortKind::Drogue);
        assert_eq!(port.node, "top");
        let mass = design.structure.mass.unwrap();
        assert!((10.0..100.0).contains(&mass), "{mass}");
        // A bigger one for a bigger load weighs more.
        let big: crate::sizing::ProceduralDef =
            crate::test_data::records("{ docking_port: { diameter: 3, load: 2000000 } }").remove(0);
        assert!(big.size(&catalog()).unwrap().structure.mass.unwrap() > 10.0 * mass);
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
