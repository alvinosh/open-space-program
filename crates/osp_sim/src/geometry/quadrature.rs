//! Adaptive Gauss–Legendre quadrature of vector-valued integrands.

/// The 8-point Gauss–Legendre rule on [−1, 1], by its positive nodes: exact for polynomials
/// up to degree 15.
const NODES: [f64; 4] = [
    0.183_434_642_495_649_8,
    0.525_532_409_916_329,
    0.796_666_477_413_626_7,
    0.960_289_856_497_536_3,
];
const WEIGHTS: [f64; 4] = [
    0.362_683_783_378_362,
    0.313_706_645_877_887_3,
    0.222_381_034_453_374_5,
    0.101_228_536_290_376_3,
];

/// Relative accuracy asked of each component.
const ACCURACY: f64 = 1e-13;
/// Differences below this share of the largest component's magnitude are rounding, and
/// never call for more halving: a component that cancels to nearly nothing (∫x over
/// something symmetric) is only known to the rounding of the terms that cancelled.
const ROUNDING: f64 = 1e-15;
/// Deepest an interval is halved: 65536 pieces, for an integrand that never settles.
const MAX_DEPTH: u32 = 16;

fn add<const N: usize>(a: [f64; N], b: [f64; N]) -> [f64; N] {
    std::array::from_fn(|i| a[i] + b[i])
}

fn rule<const N: usize>(f: &impl Fn(f64) -> [f64; N], a: f64, b: f64) -> [f64; N] {
    let (mid, half) = ((a + b) / 2.0, (b - a) / 2.0);
    let mut sum = [0.0; N];
    for (x, w) in NODES.iter().zip(WEIGHTS) {
        for t in [mid - half * x, mid + half * x] {
            let v = f(t);
            for i in 0..N {
                sum[i] += w * v[i];
            }
        }
    }
    sum.map(|s| s * half)
}

/// ∫ f from `a` to `b`, each component to about 1e-13 of the integral of its magnitude:
/// halving the interval where the rule and its two halves disagree. The integrand must be
/// smooth to well below that, so not itself an adaptive integral (use [`fixed`] inside).
pub fn integrate<const N: usize>(f: impl Fn(f64) -> [f64; N], a: f64, b: f64) -> [f64; N] {
    if a == b {
        return [0.0; N];
    }
    let magnitude = rule(&|t| f(t).map(f64::abs), a, b);
    let tolerance = magnitude.map(|m| ACCURACY * m);
    let largest = magnitude.iter().fold(0.0f64, |m, x| m.max(*x));
    let floor = [ROUNDING * largest + f64::MIN_POSITIVE; N];
    let whole = rule(&f, a, b);
    refine(&f, a, b, whole, tolerance, &floor, MAX_DEPTH)
}

fn refine<const N: usize>(
    f: &impl Fn(f64) -> [f64; N],
    a: f64,
    b: f64,
    whole: [f64; N],
    tolerance: [f64; N],
    floor: &[f64; N],
    depth: u32,
) -> [f64; N] {
    let mid = (a + b) / 2.0;
    let (left, right) = (rule(f, a, mid), rule(f, mid, b));
    let halves = add(left, right);
    let settled = (0..N).all(|i| (halves[i] - whole[i]).abs() <= tolerance[i].max(floor[i]));
    if depth == 0 || settled {
        return halves;
    }
    let half = tolerance.map(|t| t / 2.0);
    add(
        refine(f, a, mid, left, half, floor, depth - 1),
        refine(f, mid, b, right, half, floor, depth - 1),
    )
}

/// ∫ f from `a` to `b` by the rule on `pieces` equal pieces: no adaptation, so the result
/// changes smoothly with anything the integrand depends on.
pub fn fixed<const N: usize>(
    f: impl Fn(f64) -> [f64; N],
    a: f64,
    b: f64,
    pieces: usize,
) -> [f64; N] {
    let width = (b - a) / pieces as f64;
    (0..pieces).fold([0.0; N], |sum, k| {
        let start = a + width * k as f64;
        add(sum, rule(&f, start, start + width))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn integrates_smooth_and_awkward_functions() {
        let [sine, square] = integrate(|t| [t.sin(), t * t], 0.0, std::f64::consts::PI);
        assert!((sine - 2.0).abs() < 1e-14);
        assert!((square - std::f64::consts::PI.powi(3) / 3.0).abs() < 1e-12);
        // A square-root singularity at the end still converges.
        let [root] = integrate(|t: f64| [(1.0 - t).max(0.0).sqrt()], 0.0, 1.0);
        assert!((root - 2.0 / 3.0).abs() < 1e-10, "{root}");
    }
}
