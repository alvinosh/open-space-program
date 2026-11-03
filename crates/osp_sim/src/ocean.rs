//! The sea's waves, as a spectrum: how much energy each wavelength and direction has, for a wind.
//!
//! The JONSWAP spectrum (Hasselmann 1973), measured on the North Sea, says how much for a wind
//! blowing over so much open water; a cosine power spreads it about the wind's direction, tighter
//! near the spectrum's peak. Following Tessendorf (2001), each mode then gets a random amplitude
//! and phase, and the game turns it all back into a height field with an inverse FFT every frame,
//! each mode travelling at the deep-water speed for its length.
//!
//! No single tile can hold both the 500 m swell and the half-metre chop, so the waves are divided
//! among several cascades of different sizes, each given its own band of wavelengths. Their sizes
//! divide the renderer's detail period, so the tiles line up from one terrain chunk to the next.

use std::f64::consts::{PI, TAU};

use serde::{Deserialize, Serialize};

/// A sea as data.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OceanDef {
    /// Wind 10 m above the water, m/s.
    pub wind_speed: f64,
    /// Which way it blows, degrees from the first tile axis.
    pub wind_direction: f64,
    /// How much open water it has blown over, m.
    pub fetch: f64,
    /// Gravity at the surface, m/s².
    pub gravity: f64,
}

/// Samples along each side of a cascade (`N` in the shaders).
pub const OCEAN_SIZE: usize = 256;
/// The cascades' sizes, m (`CASCADES` in the shaders). They divide `planet::DETAIL_PERIOD`.
pub const CASCADES: [f64; 3] = [4096.0, 512.0, 64.0];

/// One mode's random part, reproducibly. Two normal deviates from a mixed hash of its place.
fn gaussian(i: usize, j: usize, cascade: usize) -> (f64, f64) {
    let mix = |mut h: u64| {
        h ^= h >> 29;
        h = h.wrapping_mul(0xBF58_476D_1CE4_E5B9);
        h ^= h >> 32;
        h = h.wrapping_mul(0x94D0_49BB_1331_11EB);
        h ^ (h >> 31)
    };
    let base = (i as u64).wrapping_mul(0x9E37_79B9_7F4A_7C15)
        ^ (j as u64).wrapping_mul(0xC2B2_AE3D_27D4_EB4F)
        ^ (cascade as u64 + 1).wrapping_mul(0x1656_67B1_9E37_79F9);
    let u = |h: u64| ((mix(h) >> 11) as f64 + 0.5) / (1u64 << 53) as f64;
    let (a, b) = (u(base), u(base ^ 0xDEAD_BEEF_CAFE_F00D));
    let r = (-2.0 * a.ln()).sqrt();
    (r * (TAU * b).cos(), r * (TAU * b).sin())
}

impl OceanDef {
    /// The JONSWAP spectrum of wave energy by angular frequency, m²s.
    pub fn jonswap(&self, omega: f64) -> f64 {
        if omega <= 0.0 {
            return 0.0;
        }
        let (u, f, g) = (self.wind_speed, self.fetch, self.gravity);
        let alpha = 0.076 * (u * u / (f * g)).powf(0.22);
        let peak = 22.0 * (g * g / (u * f)).powf(1.0 / 3.0);
        let sigma = if omega <= peak { 0.07 } else { 0.09 };
        let gamma = 3.3f64;
        let r = (-(omega - peak).powi(2) / (2.0 * sigma * sigma * peak * peak)).exp();
        alpha * g * g / omega.powi(5) * (-1.25 * (peak / omega).powi(4)).exp() * gamma.powf(r)
    }

    /// Angular frequency of the spectrum's peak, rad/s.
    pub fn peak(&self) -> f64 {
        let (u, f, g) = (self.wind_speed, self.fetch, self.gravity);
        22.0 * (g * g / (u * f)).powf(1.0 / 3.0)
    }

    /// How much of a wave's energy goes at an angle `theta` from the wind: most waves run with it,
    /// the longest in a narrow fan and the short ones more widely. A cos^(2s)(θ/2) with a
    /// normalising constant found by integrating it (Longuet-Higgins 1963).
    fn spreading(&self, omega: f64, theta: f64) -> f64 {
        let ratio = omega / self.peak();
        let s = if ratio <= 1.0 {
            6.97 * ratio.powf(4.06)
        } else {
            9.77 * ratio.powf(-2.33)
        }
        .max(0.5);
        // The integral over the circle has a closed form, a ratio of gamma functions.
        let normal = (2.0 * s).exp2() * gamma(s + 1.0).powi(2) / (2.0 * PI * gamma(2.0 * s + 1.0));
        normal * (theta * 0.5).cos().abs().powf(2.0 * s)
    }

    /// Wave energy per unit area of wave-number space, m⁴, at `(kx, kz)`.
    fn spectrum(&self, kx: f64, kz: f64) -> f64 {
        let k = (kx * kx + kz * kz).sqrt();
        if k <= 0.0 {
            return 0.0;
        }
        let g = self.gravity;
        let omega = (g * k).sqrt();
        // From frequency to wave number, then per unit angle and wave number.
        let wave_number = self.jonswap(omega) * g / (2.0 * omega);
        let wind = self.wind_direction.to_radians();
        let theta = (kz.atan2(kx) - wind).rem_euclid(TAU);
        wave_number * self.spreading(omega, theta) / k
    }

    /// The wave numbers a cascade takes care of, rad/m: from its longest wave to where the next,
    /// smaller cascade takes over.
    pub fn band(cascade: usize) -> (f64, f64) {
        let size = CASCADES[cascade];
        let lowest = if cascade == 0 {
            0.0
        } else {
            TAU / CASCADES[cascade - 1] * 6.0
        };
        let highest = if cascade + 1 < CASCADES.len() {
            TAU / size * 6.0
        } else {
            TAU / size * (OCEAN_SIZE / 2) as f64
        };
        (lowest, highest)
    }

    /// A cascade's modes at time zero, row by row: a mode's amplitude, and the conjugate of the
    /// amplitude of its opposite, which together give a real height field at any time.
    pub fn amplitudes(&self, cascade: usize) -> Vec<[f32; 4]> {
        let n = OCEAN_SIZE;
        let size = CASCADES[cascade];
        let step = TAU / size;
        let (low, high) = Self::band(cascade);
        let wave_number = |i: usize| {
            let i = i as i64;
            let half = n as i64 / 2;
            (if i < half { i } else { i - n as i64 }) as f64 * step
        };
        let mode = |i: usize, j: usize| {
            let (kx, kz) = (wave_number(i), wave_number(j));
            let k = (kx * kx + kz * kz).sqrt();
            if k <= low || k > high {
                return (0.0, 0.0);
            }
            let (a, b) = gaussian(i, j, cascade);
            // E|h|² is half the energy of the mode: the opposite mode brings the other half.
            let amplitude = (self.spectrum(kx, kz) * step * step / 4.0).sqrt();
            (a * amplitude, b * amplitude)
        };
        let mut out = Vec::with_capacity(n * n);
        for j in 0..n {
            for i in 0..n {
                let (re, im) = mode(i, j);
                let (ore, oim) = mode((n - i) % n, (n - j) % n);
                out.push([re as f32, im as f32, ore as f32, -oim as f32]);
            }
        }
        out
    }

    /// The variance of the height, m², and of the slope, by adding up a cascade's modes.
    pub fn cascade_variance(&self, cascade: usize) -> (f64, f64) {
        let n = OCEAN_SIZE;
        let step = TAU / CASCADES[cascade];
        let amplitudes = self.amplitudes(cascade);
        let (mut height, mut slope) = (0.0, 0.0);
        for j in 0..n {
            for i in 0..n {
                let [re, im, ore, oim] = amplitudes[j * n + i].map(f64::from);
                let wave = |x: usize| {
                    let x = x as i64;
                    (if x < n as i64 / 2 { x } else { x - n as i64 }) as f64 * step
                };
                let k2 = wave(i).powi(2) + wave(j).powi(2);
                // In the height field, a mode's mean square.
                let power = re * re + im * im + ore * ore + oim * oim;
                height += power;
                slope += power * k2;
            }
        }
        (height, slope)
    }

    /// Significant wave height, m: four standard deviations of the surface.
    pub fn significant_height(&self) -> f64 {
        let variance: f64 = (0..CASCADES.len())
            .map(|c| self.cascade_variance(c).0)
            .sum();
        4.0 * variance.sqrt()
    }

    /// Mean square slope of all the cascades.
    pub fn mean_square_slope(&self) -> f64 {
        (0..CASCADES.len())
            .map(|c| self.cascade_variance(c).1)
            .sum()
    }
}

/// An inverse DFT of `x` (a power of two long), without normalising: the sum over modes of
/// `x[k] exp(2 pi i k n / N)`. Stage by stage as the shader does it (`shaders/ocean_fft.wesl`), each
/// output reading the two inputs that make it. Here so its indices can be tested.
pub fn inverse_fft(x: &[[f64; 2]]) -> Vec<[f64; 2]> {
    let n = x.len();
    let mut from = x.to_vec();
    let mut to = vec![[0.0; 2]; n];
    let mut p = 1usize;
    while p < n {
        for (output, out) in to.iter_mut().enumerate() {
            let k = output & (p - 1);
            let first = k + ((output & !(2 * p - 1)) >> 1);
            let [ur, ui] = from[first];
            let [vr, vi] = from[first + n / 2];
            let angle = PI * k as f64 / p as f64;
            let (c, s) = (angle.cos(), angle.sin());
            let (tr, ti) = (vr * c - vi * s, vr * s + vi * c);
            *out = if output & p != 0 {
                [ur - tr, ui - ti]
            } else {
                [ur + tr, ui + ti]
            };
        }
        std::mem::swap(&mut from, &mut to);
        p *= 2;
    }
    from
}

/// The gamma function, from Lanczos's approximation.
fn gamma(x: f64) -> f64 {
    const G: f64 = 7.0;
    const C: [f64; 9] = [
        0.999_999_999_999_809_9,
        676.520_368_121_885_1,
        -1_259.139_216_722_402_8,
        771.323_428_777_653_1,
        -176.615_029_162_140_6,
        12.507_343_278_686_905,
        -0.138_571_095_265_720_12,
        9.984_369_578_019_572e-6,
        1.505_632_735_149_311_6e-7,
    ];
    if x < 0.5 {
        return PI / ((PI * x).sin() * gamma(1.0 - x));
    }
    let x = x - 1.0;
    let mut a = C[0];
    let t = x + G + 0.5;
    for (i, c) in C.iter().enumerate().skip(1) {
        a += c / (x + i as f64);
    }
    (TAU).sqrt() * t.powf(x + 0.5) * (-t).exp() * a
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sea(wind: f64) -> OceanDef {
        OceanDef {
            wind_speed: wind,
            wind_direction: 40.0,
            fetch: 300_000.0,
            gravity: 9.81,
        }
    }

    #[test]
    fn the_stages_add_up_to_the_transform_they_stand_for() {
        let n = 64;
        let input: Vec<[f64; 2]> = (0..n)
            .map(|i| {
                let (a, b) = gaussian(i, 3, 0);
                [a, b]
            })
            .collect();
        let fast = inverse_fft(&input);
        for (out, value) in fast.iter().enumerate() {
            let (mut re, mut im) = (0.0, 0.0);
            for (k, [a, b]) in input.iter().enumerate() {
                let angle = TAU * (k * out) as f64 / n as f64;
                let (c, s) = (angle.cos(), angle.sin());
                re += a * c - b * s;
                im += a * s + b * c;
            }
            assert!((value[0] - re).abs() < 1e-9, "{out}: {} vs {re}", value[0]);
            assert!((value[1] - im).abs() < 1e-9, "{out}: {} vs {im}", value[1]);
        }
    }

    #[test]
    fn a_cascade_has_the_variance_its_modes_say_it_has() {
        // Through the transform itself, on one row of modes: Parseval.
        let n = OCEAN_SIZE;
        let row: Vec<[f64; 2]> = (0..n)
            .map(|i| {
                let (a, b) = gaussian(i, 9, 1);
                [a * 0.01, b * 0.01]
            })
            .collect();
        let field = inverse_fft(&row);
        let modes: f64 = row.iter().map(|[a, b]| a * a + b * b).sum();
        let samples: f64 = field.iter().map(|[a, b]| a * a + b * b).sum::<f64>() / n as f64;
        assert!(
            (samples - modes).abs() < 1e-9 * modes.max(1.0),
            "{samples} vs {modes}"
        );
    }

    #[test]
    fn gamma_is_gamma() {
        assert!((gamma(1.0) - 1.0).abs() < 1e-10);
        assert!((gamma(5.0) - 24.0).abs() < 1e-8);
        assert!((gamma(0.5) - PI.sqrt()).abs() < 1e-9);
    }

    #[test]
    fn spreading_adds_up_to_one_around_the_circle() {
        let def = sea(8.0);
        for ratio in [0.6, 1.0, 1.5, 3.0, 10.0] {
            let omega = def.peak() * ratio;
            let steps = 20_000;
            let sum: f64 = (0..steps)
                .map(|i| def.spreading(omega, TAU * (i as f64 + 0.5) / steps as f64))
                .sum::<f64>()
                * TAU
                / steps as f64;
            assert!((sum - 1.0).abs() < 0.01, "ratio {ratio}: {sum}");
        }
    }

    #[test]
    fn a_stronger_wind_makes_higher_waves_in_about_the_measured_proportion() {
        // The usual fit to measured seas, 0.0163 sqrt(F[km]) U. Integrating the spectrum itself
        // comes out about 1.4 times that: the two are fits to the same scattered data.
        for wind in [5.0, 8.0, 12.0, 18.0] {
            let def = sea(wind);
            let expected = 0.0163 * (def.fetch / 1000.0).sqrt() * wind;
            let height = def.significant_height();
            assert!(
                height > 0.7 * expected && height < 1.7 * expected,
                "wind {wind}: {height} m vs about {expected} m"
            );
        }
        assert!(sea(12.0).significant_height() > sea(6.0).significant_height());
    }

    #[test]
    fn the_slope_is_about_what_sun_glint_measures() {
        // Cox and Munk (1954), from photographs of the Sun's glint: 0.003 + 0.00512 U, all waves.
        // Ripples too small for the finest cascade make up the rest: the renderer adds them as
        // roughness.
        for wind in [5.0, 8.0, 12.0] {
            let measured = 0.003 + 0.00512 * wind;
            let mss = sea(wind).mean_square_slope();
            assert!(
                mss > 0.25 * measured && mss < 1.1 * measured,
                "wind {wind}: {mss} vs {measured}"
            );
        }
    }

    #[test]
    fn the_modes_make_a_real_height_field() {
        // A mode's stored opposite is the conjugate of the mode at -k, whichever cell it is read
        // from: the amplitudes at k and -k mirror each other.
        let n = OCEAN_SIZE;
        let def = sea(8.0);
        let a = def.amplitudes(1);
        for (i, j) in [(10, 20), (3, 250), (100, 5)] {
            let here = a[j * n + i];
            let there = a[(n - j) % n * n + (n - i) % n];
            assert_eq!(here[0], there[2]);
            assert_eq!(here[1], -there[3]);
        }
    }

    #[test]
    fn the_cascades_cover_their_bands_without_overlap() {
        for c in 1..CASCADES.len() {
            assert_eq!(OceanDef::band(c - 1).1, OceanDef::band(c).0);
            assert!(OceanDef::band(c).1 > OceanDef::band(c).0);
        }
    }

    #[test]
    fn waves_run_with_the_wind() {
        let def = sea(8.0);
        let (a, b) = {
            let k = 0.1;
            let wind = def.wind_direction.to_radians();
            (
                def.spectrum(k * wind.cos(), k * wind.sin()),
                def.spectrum(-k * wind.cos(), -k * wind.sin()),
            )
        };
        assert!(a > 50.0 * b, "{a} vs {b}");
    }
}
