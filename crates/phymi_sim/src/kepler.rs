//! Low-level two-body math in universal variables.
//!
//! The universal formulation treats elliptic, parabolic, hyperbolic and radial motion with a
//! single equation, so there are no singularities at e = 1 or h = 0. The universal Kepler
//! equation is monotonic in the universal anomaly χ (its derivative is the radius), which
//! lets the solver bracket the root and fall back to bisection: convergence is guaranteed
//! for any input, not just typical ones.

use std::f64::consts::{PI, TAU};

use glam::DVec3;

/// Stumpff functions C(z) and S(z).
///
/// Uses series near zero, and half-angle forms elsewhere to avoid the catastrophic
/// cancellation of `1 - cos` and `cosh - 1`.
pub fn stumpff(z: f64) -> (f64, f64) {
    if z.abs() < 1.0 {
        // C(z) = Σ (-z)^k / (2k+2)!,  S(z) = Σ (-z)^k / (2k+3)!
        let mut c = 0.0;
        let mut s = 0.0;
        let mut term_c = 0.5; // 1/2!
        let mut term_s = 1.0 / 6.0; // 1/3!
        for k in 0..20 {
            c += term_c;
            s += term_s;
            let k = k as f64;
            term_c *= -z / ((2.0 * k + 3.0) * (2.0 * k + 4.0));
            term_s *= -z / ((2.0 * k + 4.0) * (2.0 * k + 5.0));
            if term_c.abs() < 1e-18 && term_s.abs() < 1e-18 {
                break;
            }
        }
        (c, s)
    } else if z > 0.0 {
        let x = z.sqrt();
        let half = (x / 2.0).sin();
        (2.0 * half * half / z, (x - x.sin()) / (z * x))
    } else {
        let x = (-z).sqrt();
        let half = (x / 2.0).sinh();
        (2.0 * half * half / -z, (x.sinh() - x) / (-z * x))
    }
}

/// Reciprocal semi-major axis α = 1/a = 2/r − v²/μ. Positive for ellipses, zero for
/// parabolas, negative for hyperbolas; finite everywhere, unlike `a`.
pub fn reciprocal_semi_major_axis(mu: f64, r: DVec3, v: DVec3) -> f64 {
    2.0 / r.length() - v.length_squared() / mu
}

/// Orbital period for α > 0.
pub fn period(mu: f64, alpha: f64) -> Option<f64> {
    (alpha > 0.0).then(|| TAU / (mu.sqrt() * alpha * alpha.sqrt()))
}

/// Propagate a two-body state by `dt` seconds (may be negative). Valid for every conic.
///
/// `alpha` must be [`reciprocal_semi_major_axis`] of the input state (passed in so callers
/// can cache it together with the period).
pub fn propagate(
    mu: f64,
    r0: DVec3,
    v0: DVec3,
    alpha: f64,
    period: Option<f64>,
    dt: f64,
) -> (DVec3, DVec3) {
    let mut dt = dt;
    if let Some(period) = period {
        // Whole revolutions change nothing; dropping them keeps χ (and z ≤ 4π²) small.
        dt -= period * (dt / period).round();
    }
    if dt == 0.0 {
        return (r0, v0);
    }

    let sqrt_mu = mu.sqrt();
    let r0n = r0.length();
    let sigma0 = r0.dot(v0) / sqrt_mu;
    let chi = solve_universal_kepler(sqrt_mu, r0n, sigma0, alpha, dt);

    let z = alpha * chi * chi;
    let (c, s) = stumpff(z);
    let chi2 = chi * chi;
    let f = 1.0 - chi2 * c / r0n;
    let g = dt - chi2 * chi * s / sqrt_mu;
    let r = f * r0 + g * v0;
    let rn = r.length();
    let fdot = sqrt_mu * chi * (z * s - 1.0) / (r0n * rn);
    let gdot = 1.0 - chi2 * c / rn;
    (r, fdot * r0 + gdot * v0)
}

/// Residual and derivative of the universal Kepler equation:
/// F(χ) = σ0 χ² C + (1 − α r0) χ³ S + r0 χ − √μ Δt,  F'(χ) = r(χ) > 0.
fn universal_kepler(
    sqrt_mu: f64,
    r0: f64,
    sigma0: f64,
    alpha: f64,
    dt: f64,
    chi: f64,
) -> (f64, f64) {
    let chi2 = chi * chi;
    let z = alpha * chi2;
    let (c, s) = stumpff(z);
    let f = sigma0 * chi2 * c + (1.0 - alpha * r0) * chi2 * chi * s + r0 * chi - sqrt_mu * dt;
    let df = sigma0 * chi * (1.0 - z * s) + (1.0 - alpha * r0) * chi2 * c + r0;
    (f, df)
}

/// Safeguarded Newton iteration on a bracket. F is strictly increasing in χ, so the root is
/// unique and bracketing always succeeds.
fn solve_universal_kepler(sqrt_mu: f64, r0: f64, sigma0: f64, alpha: f64, dt: f64) -> f64 {
    let eval = |chi| universal_kepler(sqrt_mu, r0, sigma0, alpha, dt, chi);

    let guess = if alpha > 0.0 {
        sqrt_mu * dt * alpha
    } else {
        sqrt_mu * dt / r0
    };

    // Bracket the root. For ellipses (|Δt| ≤ P/2 after reduction) |ΔE| < π + 2, so
    // |χ| < 2π/√α. Otherwise expand geometrically from the guess.
    let (mut lo, mut hi) = if alpha > 0.0 {
        let bound = TAU / alpha.sqrt();
        (-bound, bound)
    } else {
        let mut width = guess.abs().max(1e-3 * r0.sqrt()).max(1e-9);
        let (mut lo, mut hi) = (guess - width, guess + width);
        while eval(lo).0 > 0.0 {
            width *= 2.0;
            lo = guess - width;
        }
        while eval(hi).0 < 0.0 {
            width *= 2.0;
            hi = guess + width;
        }
        (lo, hi)
    };

    let mut chi = guess.clamp(lo, hi);
    for _ in 0..200 {
        let (f, df) = eval(chi);
        if f == 0.0 {
            return chi;
        }
        if f < 0.0 {
            lo = chi;
        } else {
            hi = chi;
        }
        let newton = chi - f / df;
        let next = if newton > lo && newton < hi {
            newton
        } else {
            0.5 * (lo + hi)
        };
        let converged = (next - chi).abs() <= 1e-15 * chi.abs().max(1e-300)
            || hi - lo <= 1e-15 * chi.abs().max(1e-300);
        chi = next;
        if converged {
            break;
        }
    }
    chi
}

/// Universal anomaly χ measured from periapsis at true anomaly `nu`.
///
/// Written in terms of the periapsis radius and q = (1−e)/(1+e) so it is numerically
/// stable for every eccentricity, including e → 1 where `a` blows up:
/// χ = 2 √(rp/(1+e)) · w · A(q w²), with w = tan(ν/2) and A(y) = atan(√y)/√y.
pub fn universal_anomaly_from_periapsis(rp: f64, e: f64, nu: f64) -> f64 {
    let w = (nu / 2.0).tan();
    let q = (1.0 - e) / (1.0 + e);
    2.0 * (rp / (1.0 + e)).sqrt() * w * atan_ratio(q * w * w)
}

/// atan(√y)/√y, continued analytically to atanh for y < 0, with a series near zero.
fn atan_ratio(y: f64) -> f64 {
    if y.abs() < 1e-3 {
        1.0 - y / 3.0 + y * y / 5.0 - y * y * y / 7.0 + y * y * y * y / 9.0
    } else if y > 0.0 {
        let s = y.sqrt();
        s.atan() / s
    } else {
        let s = (-y).sqrt();
        s.atanh() / s
    }
}

/// Time from periapsis passage to true anomaly `nu` (negative before periapsis).
/// Universal form √μ t = e χ³ S(αχ²) + rp χ, valid for all conics.
pub fn time_from_periapsis(mu: f64, rp: f64, e: f64, nu: f64) -> f64 {
    let chi = universal_anomaly_from_periapsis(rp, e, nu);
    let alpha = (1.0 - e) / rp;
    let (_, s) = stumpff(alpha * chi * chi);
    (e * chi * chi * chi * s + rp * chi) / mu.sqrt()
}

/// Wrap an angle to (−π, π].
pub fn wrap_pi(angle: f64) -> f64 {
    let a = (angle + PI).rem_euclid(TAU) - PI;
    if a == -PI { PI } else { a }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MU: f64 = 3.986_004_418e14;

    fn energy(r: DVec3, v: DVec3) -> f64 {
        v.length_squared() / 2.0 - MU / r.length()
    }

    fn prop(r: DVec3, v: DVec3, dt: f64) -> (DVec3, DVec3) {
        let alpha = reciprocal_semi_major_axis(MU, r, v);
        propagate(MU, r, v, alpha, period(MU, alpha), dt)
    }

    /// A spread of hard cases: circular, eccentric, near-parabolic on both sides,
    /// exactly parabolic, hyperbolic, and purely radial (h = 0). The last field caps |dt|:
    /// a bound radial orbit hits r = 0 (a collision singularity) after about 1200 s.
    fn cases() -> Vec<(&'static str, DVec3, DVec3, f64)> {
        let r = 7.0e6;
        let vc = (MU / r).sqrt();
        let ve = (2.0 * MU / r).sqrt();
        let x = DVec3::new(r, 0.0, 0.0);
        let inf = f64::INFINITY;
        vec![
            ("circular", x, DVec3::new(0.0, vc, 0.0), inf),
            (
                "inclined",
                DVec3::new(r, 1e5, -2e5),
                DVec3::new(-300.0, 6_000.0, 4_000.0),
                inf,
            ),
            ("e=0.99", x, DVec3::new(0.0, ve * 0.997_5, 0.0), inf),
            (
                "near-parabolic-",
                x,
                DVec3::new(0.0, ve * (1.0 - 1e-12), 0.0),
                inf,
            ),
            ("parabolic", x, DVec3::new(0.0, ve, 0.0), inf),
            (
                "near-parabolic+",
                x,
                DVec3::new(0.0, ve * (1.0 + 1e-12), 0.0),
                inf,
            ),
            ("hyperbolic", x, DVec3::new(500.0, ve * 1.8, 100.0), inf),
            ("radial-bound", x, DVec3::new(3_000.0, 0.0, 0.0), 600.0),
            ("radial-escape", x, DVec3::new(ve * 1.5, 0.0, 0.0), inf),
        ]
    }

    #[test]
    fn stumpff_is_continuous_across_branches() {
        for z in [1.0f64, -1.0] {
            let below = stumpff(z * (1.0 - 1e-12));
            let above = stumpff(z * (1.0 + 1e-12));
            // The functions genuinely change by ~|z|·2e-12/24 across this gap.
            assert!((below.0 - above.0).abs() < 1e-12 && (below.1 - above.1).abs() < 1e-12);
        }
        let (c, s) = stumpff(0.0);
        assert_eq!((c, s), (0.5, 1.0 / 6.0));
    }

    #[test]
    fn forward_then_back_returns_to_start() {
        for (name, r0, v0, max_dt) in cases() {
            for dt in [1.0, 600.0, 5_000.0, 86_400.0]
                .into_iter()
                .filter(|dt| *dt <= max_dt)
            {
                let (r1, v1) = prop(r0, v0, dt);
                let (r2, v2) = prop(r1, v1, -dt);
                let dr = (r2 - r0).length() / r0.length();
                let dv = (v2 - v0).length() / v0.length();
                assert!(
                    dr < 1e-9 && dv < 1e-9,
                    "{name} dt={dt}: dr={dr:e} dv={dv:e}"
                );
            }
        }
    }

    #[test]
    fn composition_matches_single_step() {
        for (name, r0, v0, max_dt) in cases() {
            let (a, b) = if max_dt < 7_000.0 {
                (200.0, 300.0)
            } else {
                (3_000.0, 4_000.0)
            };
            let (ra, va) = prop(r0, v0, a);
            let (r_two, v_two) = prop(ra, va, b);
            let (r_one, v_one) = prop(r0, v0, a + b);
            let dr = (r_two - r_one).length() / r_one.length();
            let dv = (v_two - v_one).length() / v_one.length().max(1.0);
            assert!(dr < 1e-9 && dv < 1e-9, "{name}: dr={dr:e} dv={dv:e}");
        }
    }

    #[test]
    fn conserves_energy_and_angular_momentum() {
        for (name, r0, v0, max_dt) in cases() {
            let (r1, v1) = prop(r0, v0, max_dt.min(12_345.0));
            let de = (energy(r1, v1) - energy(r0, v0)).abs() / (MU / r0.length());
            let dh = (r1.cross(v1) - r0.cross(v0)).length() / (r0.length() * v0.length());
            assert!(de < 1e-10 && dh < 1e-10, "{name}: de={de:e} dh={dh:e}");
        }
    }

    #[test]
    fn whole_revolutions_are_exact_over_a_century() {
        let r0 = DVec3::new(6.771e6, 0.0, 0.0);
        let v0 = DVec3::new(0.0, 7_600.0, 1_200.0);
        let alpha = reciprocal_semi_major_axis(MU, r0, v0);
        let p = period(MU, alpha).unwrap();
        let revolutions = (100.0 * 365.25 * 86_400.0 / p).round();
        let (r1, _) = propagate(MU, r0, v0, alpha, Some(p), revolutions * p);
        assert!((r1 - r0).length() < 1e-3, "{}", (r1 - r0).length());
    }

    #[test]
    fn time_from_periapsis_matches_propagation() {
        let rp = 6.8e6;
        for e in [0.0, 0.1, 0.7, 0.999_999, 1.0, 1.000_001, 2.5] {
            let vp = (MU * (1.0 + e) / rp).sqrt();
            let (r0, v0) = (DVec3::new(rp, 0.0, 0.0), DVec3::new(0.0, vp, 0.0));
            for nu in [-2.0f64, -0.5, 0.3, 1.0, 2.0] {
                if e >= 1.0 && nu.abs() >= (-1.0 / e).acos() {
                    continue;
                }
                let t = time_from_periapsis(MU, rp, e, nu);
                let (r, _) = prop(r0, v0, t);
                let got = r.y.atan2(r.x);
                assert!((got - nu).abs() < 1e-9, "e={e} nu={nu}: got {got}");
            }
        }
    }
}
