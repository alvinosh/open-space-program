//! What happens when vessels touch: an impact solved over the points where their surfaces meet
//! ([`crate::collision`]).
//!
//! The solver is sequential impulses, on the rigid bodies the vessels already are: at each point,
//! an impulse along the normal that stops them closing (and gives back a share of how fast they
//! were) and friction across it, each taken equally and oppositely by the two, so that linear and
//! angular momentum go on exactly as they were. It works on the integrator the vessels already have
//! and only on physics ticks, so what is exact on rails stays exact (see T074).
//!
//! The universe finds the moment of first touch within the tick and puts the vessels back there
//! before this runs, so surfaces do not sink into each other.

use glam::{DQuat, DVec3};

use crate::{attitude::MassProperties, collision::Contact};

/// How much of the closing speed comes back: structure and latches take the rest.
const RESTITUTION: f64 = 0.25;
/// Closing slower than this just stops, m/s, so that a vessel leaning on another does not chatter.
const RESTING: f64 = 0.03;
/// Metal on metal, about.
const FRICTION: f64 = 0.3;
/// Rounds over the contact points: enough for them to agree.
const ROUNDS: usize = 16;
/// How fast surfaces already through each other are eased apart, per metre, and at most, m/s.
const EASE_RATE: f64 = 1.0;
const EASE_MAX: f64 = 0.1;

/// A vessel as a contact sees it, in the frame of a body.
#[derive(Debug, Clone, Copy, PartialEq)]
pub(crate) struct Body {
    pub center: DVec3,
    pub velocity: DVec3,
    pub turned: DQuat,
    /// In its own axes, rad/s.
    pub spin: DVec3,
    pub mass: MassProperties,
    /// It stands on the ground: nothing moves it.
    pub fixed: bool,
}

impl Body {
    fn turning(&self) -> DVec3 {
        self.turned * self.spin
    }

    pub fn velocity_at(&self, point: DVec3) -> DVec3 {
        self.velocity + self.turning().cross(point - self.center)
    }

    /// How much a unit impulse along `direction` at `point` moves that point along `direction`.
    fn give(&self, point: DVec3, direction: DVec3) -> f64 {
        if self.fixed {
            return 0.0;
        }
        let arm = point - self.center;
        let about = self.turned.inverse() * arm.cross(direction);
        let turn = self.turned * (self.mass.inertia.tensor.inverse() * about);
        1.0 / self.mass.mass + turn.cross(arm).dot(direction)
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

/// Resolve `contacts` between `a` and `b`, each normal pushing `a` away from `b`. How fast they
/// were closing where they closed fastest, m/s; 0 if they were not.
pub(crate) fn solve(a: &mut Body, b: &mut Body, contacts: &[Contact]) -> f64 {
    let mut fastest: f64 = 0.0;
    // What each point wants: how fast it should be parting along its normal.
    let wanted: Vec<f64> = contacts
        .iter()
        .map(|c| {
            let closing = -(a.velocity_at(c.point) - b.velocity_at(c.point)).dot(c.normal);
            fastest = fastest.max(closing);
            let rebound = if closing > RESTING {
                RESTITUTION * closing
            } else {
                0.0
            };
            rebound.max((EASE_RATE * c.depth).min(EASE_MAX))
        })
        .collect();
    // What has been given along each normal and across it so far.
    let mut along = vec![0.0; contacts.len()];
    let mut across = vec![DVec3::ZERO; contacts.len()];
    for _ in 0..ROUNDS {
        for (k, c) in contacts.iter().enumerate() {
            let parting = (a.velocity_at(c.point) - b.velocity_at(c.point)).dot(c.normal);
            let give = a.give(c.point, c.normal) + b.give(c.point, c.normal);
            if give <= 0.0 {
                continue;
            }
            // Never pulling: what has been given so far can only go down to nothing.
            let more = (along[k] + (wanted[k] - parting) / give).max(0.0);
            let impulse = c.normal * (more - along[k]);
            along[k] = more;
            a.push(impulse, c.point);
            b.push(-impulse, c.point);
            // Friction, up to what pressing them together this hard allows.
            let moving = a.velocity_at(c.point) - b.velocity_at(c.point);
            let sliding = moving - c.normal * moving.dot(c.normal);
            let speed = sliding.length();
            if speed < 1e-9 {
                continue;
            }
            let way = sliding / speed;
            let give = a.give(c.point, way) + b.give(c.point, way);
            if give <= 0.0 {
                continue;
            }
            let wanted_rub = across[k] - way * (speed / give);
            let rub = wanted_rub.clamp_length_max(FRICTION * more);
            let change = rub - across[k];
            across[k] = rub;
            a.push(change, c.point);
            b.push(-change, c.point);
        }
    }
    fastest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::attitude::Inertia;

    fn body(center: DVec3, velocity: DVec3, mass: f64) -> Body {
        Body {
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
        }
    }

    fn at(point: DVec3, normal: DVec3) -> Contact {
        Contact {
            point,
            normal,
            depth: 0.0,
        }
    }

    /// Linear and angular momentum about the origin.
    fn momentum(bodies: &[&Body]) -> (DVec3, DVec3) {
        let (mut linear, mut angular) = (DVec3::ZERO, DVec3::ZERO);
        for b in bodies {
            linear += b.velocity * b.mass.mass;
            angular += b.center.cross(b.velocity) * b.mass.mass
                + b.turned * (b.mass.inertia.tensor * b.spin);
        }
        (linear, angular)
    }

    #[test]
    fn a_head_on_bump_gives_back_a_share_of_the_closing_speed() {
        let (mut a, mut b) = (
            body(DVec3::Z * 5.0, DVec3::NEG_Z, 1000.0),
            body(DVec3::ZERO, DVec3::ZERO, 3000.0),
        );
        let closing = solve(&mut a, &mut b, &[at(DVec3::Z * 2.5, DVec3::Z)]);
        assert!((closing - 1.0).abs() < 1e-9);
        let parting = (a.velocity - b.velocity).z;
        assert!((parting - RESTITUTION).abs() < 1e-9, "{parting}");
    }

    #[test]
    fn nothing_happens_to_what_is_already_parting() {
        let (mut a, mut b) = (
            body(DVec3::Z * 5.0, DVec3::Z, 1000.0),
            body(DVec3::ZERO, DVec3::ZERO, 3000.0),
        );
        let before = (a, b);
        assert_eq!(solve(&mut a, &mut b, &[at(DVec3::Z * 2.5, DVec3::Z)]), 0.0);
        assert_eq!((a, b), before);
    }

    #[test]
    fn momentum_goes_on_whatever_the_contact() {
        let (mut a, mut b) = (
            body(
                DVec3::new(1.0, -2.0, 5.0),
                DVec3::new(0.2, 0.7, -1.3),
                800.0,
            ),
            body(
                DVec3::new(-1.0, 0.5, 0.0),
                DVec3::new(0.1, -0.2, 0.4),
                2500.0,
            ),
        );
        a.spin = DVec3::new(0.1, -0.2, 0.05);
        b.spin = DVec3::new(-0.03, 0.02, 0.2);
        let before = momentum(&[&a, &b]);
        let contacts = [
            at(DVec3::new(0.3, 0.1, 2.4), DVec3::Z),
            at(
                DVec3::new(-0.3, 0.2, 2.5),
                DVec3::new(0.1, 0.0, 1.0).normalize(),
            ),
            at(
                DVec3::new(0.0, -0.4, 2.6),
                DVec3::new(0.0, 0.1, 1.0).normalize(),
            ),
        ];
        assert!(solve(&mut a, &mut b, &contacts) > 0.0);
        let after = momentum(&[&a, &b]);
        assert!((before.0 - after.0).length() < 1e-9);
        assert!((before.1 - after.1).length() < 1e-8);
    }

    /// Hit off to one side, it starts to turn; hit through its centre, it does not.
    #[test]
    fn where_it_is_hit_decides_whether_it_turns() {
        let run = |x: f64| {
            let (mut a, mut b) = (
                body(DVec3::Z * 5.0, DVec3::NEG_Z, 1000.0),
                body(DVec3::ZERO, DVec3::ZERO, 1000.0),
            );
            solve(&mut a, &mut b, &[at(DVec3::new(x, 0.0, 2.5), DVec3::Z)]);
            (a.spin, b.spin)
        };
        assert_eq!(run(0.0).0, DVec3::ZERO);
        let (spin_a, spin_b) = run(1.0);
        assert!(spin_a.y.abs() > 1e-3 && spin_b.y.abs() > 1e-3);
    }

    #[test]
    fn leaning_on_it_stops_without_bouncing() {
        let (mut a, mut b) = (
            body(DVec3::Z * 5.0, DVec3::NEG_Z * 0.01, 1000.0),
            body(DVec3::ZERO, DVec3::ZERO, 1000.0),
        );
        solve(&mut a, &mut b, &[at(DVec3::Z * 2.5, DVec3::Z)]);
        assert!((a.velocity - b.velocity).length() < 1e-9);
    }

    #[test]
    fn what_stands_on_the_ground_stays_where_it_is() {
        let (mut a, mut b) = (
            body(DVec3::Z * 5.0, DVec3::NEG_Z, 1000.0),
            body(DVec3::ZERO, DVec3::ZERO, 1000.0),
        );
        b.fixed = true;
        solve(&mut a, &mut b, &[at(DVec3::Z * 2.5, DVec3::Z)]);
        assert_eq!(b.velocity, DVec3::ZERO);
        assert!((a.velocity.z - RESTITUTION).abs() < 1e-9);
    }

    /// Sliding along a wall it slows, and pressing harder slows it more, up to a stop across it.
    #[test]
    fn friction_takes_up_what_pressing_allows() {
        let slide = |into: f64| {
            let (mut a, mut b) = (
                body(DVec3::Z * 5.0, DVec3::new(2.0, 0.0, -into), 1000.0),
                body(DVec3::ZERO, DVec3::ZERO, 1000.0),
            );
            b.fixed = true;
            solve(&mut a, &mut b, &[at(DVec3::Z * 2.5, DVec3::Z)]);
            a.velocity.x
        };
        assert!(slide(0.5) < 2.0);
        assert!(slide(1.0) < slide(0.5));
    }
}
