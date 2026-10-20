//! Encke's method for integrating perturbed (thrusting, dragging) motion.
//!
//! Instead of integrating the full position (Cowell's method), we integrate only the small
//! deviation δ from a reference conic that is propagated analytically. Rounding error then
//! scales with |δ| rather than with the distance from the frame origin, so powered flight is
//! as precise 30 AU from the Sun as it is in low Earth orbit, and long, gentle burns (or
//! weak drag) don't accumulate gravity-integration error. When δ grows, the reference is
//! *rectified*: replaced by the osculating conic and δ reset to zero.

use glam::DVec3;

use crate::{
    orbit::{Orbit, StateVector},
    time::SimTime,
};

/// Rectify when |δr| exceeds this fraction of the reference radius.
pub const RECTIFY_RATIO: f64 = 1e-3;

#[derive(Debug, Clone, Copy)]
pub struct Encke {
    reference: Orbit,
    time: SimTime,
    dr: DVec3,
    dv: DVec3,
}

impl Encke {
    pub fn new(mu: f64, time: SimTime, state: StateVector) -> Self {
        Self {
            reference: Orbit::new(mu, time, state),
            time,
            dr: DVec3::ZERO,
            dv: DVec3::ZERO,
        }
    }

    pub fn time(&self) -> SimTime {
        self.time
    }

    pub fn state(&self) -> StateVector {
        self.reference.state_at(self.time) + StateVector::new(self.dr, self.dv)
    }

    /// The osculating conic (what the vessel would follow if forces stopped now).
    pub fn osculating(&self) -> Orbit {
        Orbit::new(self.reference.mu(), self.time, self.state())
    }

    /// One RK4 step from the current time to `end` under a non-gravitational acceleration
    /// `perturbation(r, v)`. Gravity of the frame body is handled by the reference conic.
    pub fn step(&mut self, end: SimTime, perturbation: impl Fn(DVec3, DVec3) -> DVec3) {
        let h = end.seconds_since(self.time);
        if h == 0.0 {
            return;
        }
        let mu = self.reference.mu();
        let base = self.time.seconds_since(self.reference.epoch());
        let reference_at = |s: f64| self.reference.state_at_offset(base + s);
        let (s0, s_half, s1) = (reference_at(0.0), reference_at(h / 2.0), reference_at(h));

        let derivative = |reference: StateVector, dr: DVec3, dv: DVec3| {
            let rho = reference.position;
            let r = rho + dr;
            let v = reference.velocity + dv;
            let rho2 = rho.length_squared();
            let q = dr.dot(2.0 * rho + dr) / rho2;
            let differential_gravity = mu / (rho2 * rho2.sqrt()) * (battin_f(q) * r - dr);
            (dv, differential_gravity + perturbation(r, v))
        };

        let (dr, dv) = (self.dr, self.dv);
        let k1 = derivative(s0, dr, dv);
        let k2 = derivative(s_half, dr + k1.0 * (h / 2.0), dv + k1.1 * (h / 2.0));
        let k3 = derivative(s_half, dr + k2.0 * (h / 2.0), dv + k2.1 * (h / 2.0));
        let k4 = derivative(s1, dr + k3.0 * h, dv + k3.1 * h);
        self.dr += (k1.0 + 2.0 * k2.0 + 2.0 * k3.0 + k4.0) * (h / 6.0);
        self.dv += (k1.1 + 2.0 * k2.1 + 2.0 * k3.1 + k4.1) * (h / 6.0);
        self.time = end;

        if self.dr.length() > RECTIFY_RATIO * s1.position.length() {
            self.reference = self.osculating();
            self.dr = DVec3::ZERO;
            self.dv = DVec3::ZERO;
        }
    }
}

/// f(q) = 1 − (1+q)^(−3/2), evaluated without cancellation for small q (Battin).
fn battin_f(q: f64) -> f64 {
    let s = (1.0 + q).powf(1.5);
    q * (3.0 + 3.0 * q + q * q) / (s * (1.0 + s))
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 3.986_004_418e14;

    /// Cowell (full-state) RK4 reference with tiny steps.
    fn cowell(
        state: StateVector,
        seconds: f64,
        dt: f64,
        accel: impl Fn(DVec3, DVec3) -> DVec3,
    ) -> StateVector {
        let f = |r: DVec3, v: DVec3| -MU * r / r.length().powi(3) + accel(r, v);
        let mut s = state;
        let steps = (seconds / dt).round() as usize;
        for _ in 0..steps {
            let (r, v) = (s.position, s.velocity);
            let k1 = (v, f(r, v));
            let k2 = (
                v + k1.1 * dt / 2.0,
                f(r + k1.0 * dt / 2.0, v + k1.1 * dt / 2.0),
            );
            let k3 = (
                v + k2.1 * dt / 2.0,
                f(r + k2.0 * dt / 2.0, v + k2.1 * dt / 2.0),
            );
            let k4 = (v + k3.1 * dt, f(r + k3.0 * dt, v + k3.1 * dt));
            s.position += (k1.0 + 2.0 * k2.0 + 2.0 * k3.0 + k4.0) * dt / 6.0;
            s.velocity += (k1.1 + 2.0 * k2.1 + 2.0 * k3.1 + k4.1) * dt / 6.0;
        }
        s
    }

    fn leo() -> StateVector {
        let r = 6.771e6;
        StateVector::new(
            DVec3::new(r, 0.0, 0.0),
            DVec3::new(0.0, (MU / r).sqrt(), 0.0),
        )
    }

    #[test]
    fn unperturbed_is_exactly_the_conic() {
        let start = SimTime::J2000;
        let mut encke = Encke::new(MU, start, leo());
        let mut t = start;
        for _ in 0..10_000 {
            t = t.add_ticks(20_000);
            encke.step(t, |_, _| DVec3::ZERO);
        }
        let exact = Orbit::new(MU, start, leo()).state_at(t);
        assert_eq!(encke.state().position, exact.position);
    }

    #[test]
    fn matches_cowell_under_thrust() {
        // 2 m/s² prograde for 10 minutes (an orbit-raising burn).
        let thrust = |_: DVec3, v: DVec3| v.normalize() * 2.0;
        let start = SimTime::J2000;
        let mut encke = Encke::new(MU, start, leo());
        let mut t = start;
        for _ in 0..(600 * 50) {
            t = t.add_ticks(20_000);
            encke.step(t, thrust);
        }
        let reference = cowell(leo(), 600.0, 0.002, thrust);
        let error = (encke.state().position - reference.position).length();
        assert!(error < 1e-3, "{error}");
    }

    #[test]
    fn battin_f_is_stable() {
        for q in [1e-14, 1e-8, 1e-3, 0.5, -0.5] {
            let naive = 1.0 - (1.0f64 + q).powf(-1.5);
            let rel = (battin_f(q) - naive).abs() / naive.abs();
            assert!(rel < if q.abs() < 1e-6 { 1e-2 } else { 1e-12 }, "q={q}");
        }
    }
}
