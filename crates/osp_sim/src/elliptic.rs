//! Elliptic integrals and Jacobi's elliptic functions, for torque-free rotation
//! ([`crate::attitude::coast`]).
//!
//! The integrals come from Carlson's symmetric forms by duplication, the functions from the
//! arithmetic–geometric mean; both are exact to rounding. The parameter `m` is the square
//! of the modulus, 0 ≤ m < 1.

use std::f64::consts::PI;

/// F(φ|m) = ∫₀^φ dθ / √(1 − m sin²θ) and Π(n; φ|m), the same with 1 / (1 − n sin²θ) in the
/// integrand, for any φ and n < 1.
pub fn incomplete(phi: f64, m: f64, n: f64) -> (f64, f64) {
    // Each half turn adds the complete integrals.
    let turns = (phi / PI).round();
    let (s, c) = (phi - turns * PI).sin_cos();
    let q = 1.0 - m * s * s;
    let f = s * rf(c * c, q, 1.0);
    let pi = f + n / 3.0 * s.powi(3) * rj(c * c, q, 1.0, 1.0 - n * s * s);
    let (k, whole) = complete(m, n);
    (f + 2.0 * turns * k, pi + 2.0 * turns * whole)
}

/// K(m) = F(π/2|m) and Π(n|m) = Π(n; π/2|m).
pub fn complete(m: f64, n: f64) -> (f64, f64) {
    let k = rf(0.0, 1.0 - m, 1.0);
    (k, k + n / 3.0 * rj(0.0, 1.0 - m, 1.0, 1.0 - n))
}

/// The amplitude am(u|m), the angle φ with F(φ|m) = u, and sn = sin φ, cn = cos φ and
/// dn = √(1 − m sn²).
pub fn jacobi(u: f64, m: f64) -> (f64, f64, f64, f64) {
    let (mut a, mut b) = (1.0f64, (1.0 - m).sqrt());
    let mut ratios = Vec::with_capacity(8);
    let mut c = m.sqrt();
    while c > f64::EPSILON * a && ratios.len() < 32 {
        (a, b, c) = ((a + b) / 2.0, (a * b).sqrt(), (a - b) / 2.0);
        ratios.push(c / a);
    }
    let mut phi = 2f64.powi(ratios.len() as i32) * a * u;
    for ratio in ratios.iter().rev() {
        phi = (phi + (ratio * phi.sin()).asin()) / 2.0;
    }
    let (sn, cn) = phi.sin_cos();
    (phi, sn, cn, (1.0 - m * sn * sn).sqrt())
}

/// Carlson's R_F(x, y, z), at most one argument zero.
fn rf(x: f64, y: f64, z: f64) -> f64 {
    let (mut x, mut y, mut z) = (x, y, z);
    loop {
        let (sx, sy, sz) = (x.sqrt(), y.sqrt(), z.sqrt());
        let lambda = sx * (sy + sz) + sy * sz;
        x = 0.25 * (x + lambda);
        y = 0.25 * (y + lambda);
        z = 0.25 * (z + lambda);
        let mean = (x + y + z) / 3.0;
        let (dx, dy, dz) = (1.0 - x / mean, 1.0 - y / mean, 1.0 - z / mean);
        if dx.abs().max(dy.abs()).max(dz.abs()) <= 0.0025 {
            let (e2, e3) = (dx * dy - dz * dz, dx * dy * dz);
            return (1.0 + (e2 / 24.0 - 0.1 - 3.0 / 44.0 * e3) * e2 + e3 / 14.0) / mean.sqrt();
        }
    }
}

/// Carlson's R_C(x, y), y > 0.
fn rc(x: f64, y: f64) -> f64 {
    let (mut x, mut y) = (x, y);
    loop {
        let lambda = 2.0 * x.sqrt() * y.sqrt() + y;
        x = 0.25 * (x + lambda);
        y = 0.25 * (y + lambda);
        let mean = (x + y + y) / 3.0;
        let s = (y - mean) / mean;
        if s.abs() <= 0.0012 {
            let series = 1.0 + s * s * (0.3 + s * (1.0 / 7.0 + s * (0.375 + s * 9.0 / 22.0)));
            return series / mean.sqrt();
        }
    }
}

/// Carlson's R_J(x, y, z, p), p > 0, at most one of x, y, z zero.
fn rj(x: f64, y: f64, z: f64, p: f64) -> f64 {
    const C1: f64 = 3.0 / 14.0;
    const C2: f64 = 1.0 / 3.0;
    const C3: f64 = 3.0 / 22.0;
    const C4: f64 = 3.0 / 26.0;
    let (mut x, mut y, mut z, mut p) = (x, y, z, p);
    let (mut sum, mut factor) = (0.0, 1.0);
    loop {
        let (sx, sy, sz) = (x.sqrt(), y.sqrt(), z.sqrt());
        let lambda = sx * (sy + sz) + sy * sz;
        let alpha = (p * (sx + sy + sz) + sx * sy * sz).powi(2);
        let beta = p * (p + lambda).powi(2);
        sum += factor * rc(alpha, beta);
        factor *= 0.25;
        x = 0.25 * (x + lambda);
        y = 0.25 * (y + lambda);
        z = 0.25 * (z + lambda);
        p = 0.25 * (p + lambda);
        let mean = 0.2 * (x + y + z + p + p);
        let (dx, dy, dz, dp) = (
            1.0 - x / mean,
            1.0 - y / mean,
            1.0 - z / mean,
            1.0 - p / mean,
        );
        if dx.abs().max(dy.abs()).max(dz.abs()).max(dp.abs()) <= 0.0015 {
            let ea = dx * (dy + dz) + dy * dz;
            let eb = dx * dy * dz;
            let ec = dp * dp;
            let ed = ea - 3.0 * ec;
            let ee = eb + 2.0 * dp * (ea - ec);
            let series = 1.0
                + ed * (-C1 + 0.75 * C3 * ed - 1.5 * C4 * ee)
                + eb * (0.5 * C2 + dp * (-2.0 * C3 + dp * C4))
                + dp * ea * (C2 - dp * C3)
                - C2 * dp * ec;
            return 3.0 * sum + factor * series / (mean * mean.sqrt());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// ∫₀^φ f(θ) dθ by Simpson's rule, fine enough to be exact to about 1e-14 here.
    fn simpson(phi: f64, f: impl Fn(f64) -> f64) -> f64 {
        let n = 20_000;
        let h = phi / n as f64;
        let inner: f64 = (1..n)
            .map(|i| f(i as f64 * h) * if i % 2 == 1 { 4.0 } else { 2.0 })
            .sum();
        (f(0.0) + inner + f(phi)) * h / 3.0
    }

    #[test]
    fn integrals_match_their_definitions() {
        for (m, n) in [(0.0, -0.4), (0.3, -2.5), (0.8, 0.5), (0.97, -0.05)] {
            for phi in [0.3, 1.2, 2.9, -4.0, 9.5] {
                let (f, pi) = incomplete(phi, m, n);
                let root = |t: f64| (1.0 - m * t.sin().powi(2)).sqrt();
                let f_ref = simpson(phi, |t| 1.0 / root(t));
                let pi_ref = simpson(phi, |t| 1.0 / ((1.0 - n * t.sin().powi(2)) * root(t)));
                assert!((f - f_ref).abs() < 1e-12, "F({phi}|{m}): {f} vs {f_ref}");
                assert!(
                    (pi - pi_ref).abs() < 1e-12,
                    "Π({n}; {phi}|{m}): {pi} vs {pi_ref}"
                );
            }
        }
        assert!((complete(0.0, 0.0).0 - PI / 2.0).abs() < 1e-15);
        assert!((complete(0.5, 0.0).0 - 1.854_074_677_301_372).abs() < 1e-14);
    }

    #[test]
    fn jacobi_functions_invert_the_first_integral() {
        for m in [0.0, 0.2, 0.9, 0.999] {
            for phi in [0.0, 0.7, -1.4, 3.3, 20.0] {
                let (u, _) = incomplete(phi, m, 0.0);
                let (am, sn, cn, dn) = jacobi(u, m);
                assert!((am - phi).abs() < 1e-12, "am(F({phi}|{m})) = {am}");
                assert!((sn - phi.sin()).abs() < 1e-12 && (cn - phi.cos()).abs() < 1e-12);
                assert!((dn * dn + m * sn * sn - 1.0).abs() < 1e-14);
            }
        }
    }
}
