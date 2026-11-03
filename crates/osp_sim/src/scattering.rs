//! How a body's air scatters sunlight: the sky, the haze, sunsets, and what light reaches a
//! point. This is optics, not flight physics; the air's density for drag is in
//! [`crate::atmosphere`].
//!
//! The air is exponential layers of molecules (Rayleigh) and aerosols (Mie) plus a band of
//! ozone, described per body in its data ([`ScatteringDef`]), so Earth's blue sky and Mars's
//! butterscotch one with its blue sunset come from the same code. Three tables, built once
//! per body, stand in for integrals too slow to do per pixel:
//!
//! - **Transmittance**: sunlight surviving from a point to the top of the air, by height and
//!   sun angle, in Bruneton and Neyret's parametrisation (2008), which packs texels where it
//!   changes fastest, near the horizon.
//! - **Multiple scattering**: light that scattered more than once, as Hillaire (2020) does it.
//!   At a point, the second order arriving from every direction is integrated; assuming each
//!   further order comes equally from all sides, the orders are a geometric series with a
//!   closed form. It is what makes the sky bright enough, and what lights it after sunset.
//! - **Sky irradiance**: the light the whole sky puts on level ground (Bruneton's E), for the
//!   terrain and the ground seen by the vessel's light probe.
//!
//! [`Air::scatter`] is the CPU's copy of the shaders' ray march (`shaders/atmosphere.wesl` in
//! the game; keep the two in sync). It is here to check that march against a fine integral,
//! and to say what the sky looks like in a test.
//!
//! All light is per unit of sunlight above the air: multiply by the sun's illuminance.

use std::f64::consts::PI;

use glam::DVec3;
use serde::{Deserialize, Serialize};

/// A body's air as far as light is concerned. Coefficients are per metre at the surface, for
/// red, green and blue.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ScatteringDef {
    /// Top of the air above the mean radius, m: scattering stops there.
    pub height: f64,
    /// Scattering by molecules, which scatter blue most and in nearly every direction.
    pub rayleigh: [f64; 3],
    pub rayleigh_height: f64,
    /// Scattering by aerosols (haze, dust).
    pub mie: [f64; 3],
    /// Absorption by aerosols: what colours dust.
    pub mie_absorption: [f64; 3],
    pub mie_height: f64,
    /// How strongly aerosols scatter forward, -1 to 1, by colour: particles a little larger
    /// than the wavelength throw blue into a narrower cone than red.
    pub mie_g: [f64; 3],
    /// Absorption at the peak of the ozone band (none by default).
    #[serde(default)]
    pub ozone: [f64; 3],
    /// Height of that peak, m.
    #[serde(default)]
    pub ozone_height: f64,
    /// The band tapers to nothing this far above and below its peak, m.
    #[serde(default = "default_ozone_width")]
    pub ozone_width: f64,
    /// Share of light the ground reflects back into the air.
    #[serde(default)]
    pub ground_albedo: [f64; 3],
    /// The colour its air burns when a vessel comes in fast, if not the orange of air.
    #[serde(default)]
    pub plasma: Option<[f64; 3]>,
}

fn default_ozone_width() -> f64 {
    1.0
}

/// Sun rays grazing deeper than this below the mean surface are in the planet's shadow (the
/// shadow's soft edge spans `SHADOW_DEPTH` to `SHADOW_EDGE`, which stands in for the sun's disc
/// and refraction, and keeps ground-level points lit while the sun is above their horizon). The
/// transmittance table starts there (`LUT_DEPTH` in the shader).
pub const SHADOW_DEPTH: f64 = 6000.0;
pub const SHADOW_EDGE: f64 = 1000.0;
/// Transmittance table size: sun angle across, height down (`TRANSMITTANCE_SIZE` in the shader).
pub const TRANSMITTANCE_SIZE: (usize, usize) = (256, 64);
/// Size of the multiple scattering and sky irradiance tables: sun angle across, height down
/// (`SKY_TABLE_SIZE` in the shader).
pub const SKY_TABLE_SIZE: (usize, usize) = (32, 32);
/// Integration steps along each ray of the transmittance table.
const TRANSMITTANCE_STEPS: usize = 128;
/// Directions averaged at each point of the sky tables, and the steps along each.
const SKY_TABLE_DIRECTIONS: usize = 64;
const SKY_TABLE_STEPS: usize = 24;
/// Scattered orders can't add up past this: the series diverges as air stops absorbing.
const MAX_REPEAT: f64 = 0.98;

/// One colour value per channel.
pub type Rgb = [f64; 3];

fn zip(a: Rgb, b: Rgb, f: impl Fn(f64, f64) -> f64) -> Rgb {
    [f(a[0], b[0]), f(a[1], b[1]), f(a[2], b[2])]
}

fn scale(a: Rgb, s: f64) -> Rgb {
    a.map(|x| x * s)
}

fn add(a: Rgb, b: Rgb) -> Rgb {
    zip(a, b, |x, y| x + y)
}

/// The integral over a step of a source `s` seen through extinction `e` over distance `ds`, or
/// `ds * s` as `e` goes to zero. Exact for a source constant over the step, so thick air stays
/// stable however long the step is (Hillaire 2015).
fn step_integral(s: f64, e: f64, ds: f64) -> f64 {
    if e * ds < 1e-6 {
        s * ds
    } else {
        s * (1.0 - (-e * ds).exp()) / e
    }
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Distances along a ray to where it enters and leaves a sphere centred on the origin, if it
/// meets it. Written to stay accurate for points near the sphere.
pub fn sphere_hits(origin: DVec3, dir: DVec3, radius: f64) -> Option<(f64, f64)> {
    let b = origin.dot(dir);
    let r = origin.length();
    let disc = b * b - (r - radius) * (r + radius);
    if disc < 0.0 {
        return None;
    }
    let s = disc.sqrt();
    Some((-b - s, -b + s))
}

/// Rayleigh's phase function.
fn phase_rayleigh(mu: f64) -> f64 {
    3.0 / (16.0 * PI) * (1.0 + mu * mu)
}

/// The Cornette-Shanks phase function, for aerosols.
fn phase_mie(mu: f64, g: f64) -> f64 {
    let g2 = g * g;
    3.0 / (8.0 * PI) * ((1.0 - g2) * (1.0 + mu * mu))
        / ((2.0 + g2) * (1.0 + g2 - 2.0 * g * mu).max(1e-9).powf(1.5))
}

/// Scattering seen along a ray.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Ray {
    /// Light scattered towards the viewer, per unit of sunlight above the air.
    pub inscatter: Rgb,
    /// Fraction of light from beyond the segment that gets through.
    pub transmittance: Rgb,
}

/// A body's air with its tables.
#[derive(Debug, Clone)]
pub struct Air {
    pub def: ScatteringDef,
    pub radius: f64,
    /// Row-major optical depths, `TRANSMITTANCE_SIZE.0` per row.
    transmittance: Vec<[f32; 3]>,
    /// Row-major light from orders above the first, per unit of scattering.
    multiple: Vec<[f32; 3]>,
    /// Row-major irradiance of level ground from the sky.
    irradiance: Vec<[f32; 3]>,
}

impl Air {
    pub fn new(def: &ScatteringDef, radius: f64) -> Self {
        let mut air = Self {
            def: def.clone(),
            radius,
            transmittance: Vec::new(),
            multiple: Vec::new(),
            irradiance: Vec::new(),
        };
        air.transmittance = air.build_transmittance();
        air.multiple = air.build_multiple();
        air.irradiance = air.build_irradiance();
        air
    }

    /// Radius of the top of the air.
    pub fn top(&self) -> f64 {
        self.radius + self.def.height
    }

    /// Relative density of molecules, aerosols and ozone at a height above the surface.
    fn densities(&self, height: f64) -> [f64; 3] {
        let h = height.max(0.0);
        let d = &self.def;
        let ozone = (1.0 - (h - d.ozone_height).abs() / d.ozone_width).max(0.0);
        [
            (-h / d.rayleigh_height).exp(),
            (-h / d.mie_height).exp(),
            ozone,
        ]
    }

    fn extinction(&self, density: [f64; 3]) -> Rgb {
        let d = &self.def;
        std::array::from_fn(|c| {
            d.rayleigh[c] * density[0]
                + (d.mie[c] + d.mie_absorption[c]) * density[1]
                + d.ozone[c] * density[2]
        })
    }

    fn scattering(&self, density: [f64; 3]) -> Rgb {
        let d = &self.def;
        std::array::from_fn(|c| d.rayleigh[c] * density[0] + d.mie[c] * density[1])
    }

    fn build_transmittance(&self) -> Vec<[f32; 3]> {
        let (width, height) = TRANSMITTANCE_SIZE;
        let (bottom, top) = (self.radius - SHADOW_DEPTH, self.top());
        let horizon = ((top - bottom) * (top + bottom)).sqrt();
        let mut depths = Vec::with_capacity(width * height);
        for j in 0..height {
            let rho = horizon * j as f64 / (height - 1) as f64;
            let r = (rho * rho + bottom * bottom).sqrt();
            let (d_min, d_max) = (top - r, rho + horizon);
            for i in 0..width {
                let d = d_min + (d_max - d_min) * i as f64 / (width - 1) as f64;
                let mu = if d <= 0.0 {
                    1.0
                } else {
                    ((horizon * horizon - rho * rho - d * d) / (2.0 * r * d)).clamp(-1.0, 1.0)
                };
                // Midpoint rule along the ray, by distance from the point.
                let ds = d / TRANSMITTANCE_STEPS as f64;
                let mut depth = [0.0; 3];
                for k in 0..TRANSMITTANCE_STEPS {
                    let t = ds * (k as f64 + 0.5);
                    let height = (t * t + 2.0 * r * mu * t + r * r).sqrt() - self.radius;
                    depth = add(depth, scale(self.extinction(self.densities(height)), ds));
                }
                depths.push(depth.map(|x| x as f32));
            }
        }
        depths
    }

    /// The radius and sun cosine at the texel `(i, j)` of a sky table: uniform in the sun's
    /// cosine, and the root of the height so low air, where the layers change, gets more rows.
    fn sky_table_point(&self, i: usize, j: usize) -> (f64, f64) {
        let (width, height) = SKY_TABLE_SIZE;
        let mu = 2.0 * i as f64 / (width - 1) as f64 - 1.0;
        let v = j as f64 / (height - 1) as f64;
        (self.radius + self.def.height * v * v, mu)
    }

    /// Light a unit of isotropic scattering sends out at a point, from all the orders above
    /// the first (Hillaire's Ψ).
    fn build_multiple(&self) -> Vec<[f32; 3]> {
        let (width, height) = SKY_TABLE_SIZE;
        let mut table = Vec::with_capacity(width * height);
        for j in 0..height {
            for i in 0..width {
                let (r, mu) = self.sky_table_point(i, j);
                let eye = DVec3::new(0.0, 0.0, r);
                let sun = DVec3::new((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu);
                // Light arriving as second order, and the share of light leaving that comes
                // back scattered once more.
                let mut second = [0.0; 3];
                let mut repeat = [0.0; 3];
                for k in 0..SKY_TABLE_DIRECTIONS {
                    let dir = sphere_direction(k, SKY_TABLE_DIRECTIONS);
                    let (l, f) = self.second_order(eye, dir, sun);
                    second = add(second, l);
                    repeat = add(repeat, f);
                }
                let n = SKY_TABLE_DIRECTIONS as f64;
                let series = zip(scale(second, 1.0 / n), scale(repeat, 1.0 / n), |l, f| {
                    l / (1.0 - f.min(MAX_REPEAT))
                });
                table.push(series.map(|x| x as f32));
            }
        }
        table
    }

    /// Along one direction from `eye`: light scattered once from the sun towards `eye` with an
    /// isotropic phase function (ground bounce included), and the share of light sent out along
    /// it that scatters back isotropically.
    fn second_order(&self, eye: DVec3, dir: DVec3, sun: DVec3) -> (Rgb, Rgb) {
        let end = self.ray_end(eye, dir);
        let ds = end.0 / SKY_TABLE_STEPS as f64;
        let (mut light, mut repeat) = ([0.0; 3], [0.0; 3]);
        let mut through = [1.0; 3];
        for k in 0..SKY_TABLE_STEPS {
            let p = eye + dir * (ds * (k as f64 + 0.5));
            let density = self.densities(p.length() - self.radius);
            let extinction = self.extinction(density);
            let scattering = self.scattering(density);
            let lit = self.sun_transmittance(p, sun);
            for c in 0..3 {
                let once = step_integral(scattering[c], extinction[c], ds);
                light[c] += through[c] * once * lit[c] / (4.0 * PI);
                repeat[c] += through[c] * once;
                through[c] *= (-extinction[c] * ds).exp();
            }
        }
        if end.1 {
            let p = eye + dir * end.0;
            let cosine = p.normalize().dot(sun).max(0.0);
            let lit = self.sun_transmittance(p, sun);
            for c in 0..3 {
                light[c] += through[c] * lit[c] * cosine * self.def.ground_albedo[c] / PI;
            }
        }
        (light, repeat)
    }

    /// Distance from `eye` to where a ray from inside the air ends, and whether that is the
    /// ground rather than the top.
    fn ray_end(&self, eye: DVec3, dir: DVec3) -> (f64, bool) {
        if let Some((near, _)) = sphere_hits(eye, dir, self.radius)
            && near > 0.0
        {
            return (near, true);
        }
        let exit = sphere_hits(eye, dir, self.top()).map_or(0.0, |(_, far)| far.max(0.0));
        (exit, false)
    }

    /// The light the sky puts on level ground, per unit of sunlight above the air.
    fn build_irradiance(&self) -> Vec<[f32; 3]> {
        let (width, height) = SKY_TABLE_SIZE;
        let mut table = Vec::with_capacity(width * height);
        for j in 0..height {
            for i in 0..width {
                let (r, mu) = self.sky_table_point(i, j);
                let eye = DVec3::new(0.0, 0.0, r);
                let sun = DVec3::new((1.0 - mu * mu).max(0.0).sqrt(), 0.0, mu);
                let mut sum = [0.0; 3];
                for k in 0..SKY_TABLE_DIRECTIONS {
                    // Cosine-weighted over the sky, so the average times pi is the integral.
                    let disc = sphere_direction(k, SKY_TABLE_DIRECTIONS);
                    let z = (1.0 - disc.x * disc.x - disc.y * disc.y).max(0.0).sqrt();
                    let dir = DVec3::new(disc.x, disc.y, z);
                    let (end, _) = self.ray_end(eye, dir);
                    let ray = self.scatter(eye, dir, 0.0, end, sun, SKY_TABLE_STEPS);
                    sum = add(sum, ray.inscatter);
                }
                let e = scale(sum, PI / SKY_TABLE_DIRECTIONS as f64);
                table.push(e.map(|x| x as f32));
            }
        }
        table
    }

    /// Where in the transmittance table the point at `r`, with the sun at cosine `mu` from its
    /// zenith, is stored, in texels (centres at `k + 0.5`).
    fn transmittance_texel(&self, r: f64, mu: f64) -> (f64, f64) {
        let (width, height) = TRANSMITTANCE_SIZE;
        let (bottom, top) = (self.radius - SHADOW_DEPTH, self.top());
        let r = r.clamp(bottom, top);
        let horizon = ((top - bottom) * (top + bottom)).sqrt();
        let rho = ((r - bottom) * (r + bottom)).max(0.0).sqrt();
        let d = (-r * mu + ((top - r) * (top + r) + r * r * mu * mu).max(0.0).sqrt()).max(0.0);
        let (d_min, d_max) = (top - r, rho + horizon);
        let x_mu = (d - d_min) / (d_max - d_min);
        let x_r = rho / horizon;
        // Unit range to texel space with the end texel centres on 0 and 1.
        let texel = |x: f64, n: usize| 0.5 + x * (n - 1) as f64;
        (texel(x_mu, width), texel(x_r, height))
    }

    /// Where a sky table stores the radius `r` and the sun cosine `mu`, in texels.
    fn sky_table_texel(&self, r: f64, mu: f64) -> (f64, f64) {
        let (width, height) = SKY_TABLE_SIZE;
        let v = ((r - self.radius) / self.def.height).clamp(0.0, 1.0).sqrt();
        let u = mu.clamp(-1.0, 1.0) * 0.5 + 0.5;
        (0.5 + u * (width - 1) as f64, 0.5 + v * (height - 1) as f64)
    }

    /// Bilinear lookup with clamped edges, as the GPU samples it.
    fn lookup(table: &[[f32; 3]], size: (usize, usize), (u, v): (f64, f64)) -> Rgb {
        let (width, height) = size;
        let split = |x: f64, n: usize| {
            let x = (x - 0.5).clamp(0.0, (n - 1) as f64);
            let i = (x.floor() as usize).min(n - 2);
            (i, x - i as f64)
        };
        let ((i, fu), (j, fv)) = (split(u, width), split(v, height));
        let at = |i: usize, j: usize| table[j * width + i].map(f64::from);
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fu;
            let bottom = c[k] + (d[k] - c[k]) * fu;
            top + (bottom - top) * fv
        })
    }

    /// Fraction of sunlight (per colour) reaching `p`, relative to the planet's centre, through
    /// the air: extinction, and the planet's shadow. Mirrors the shader.
    pub fn sun_transmittance(&self, p: DVec3, sun: DVec3) -> Rgb {
        let radius = self.radius;
        let r = p.length();
        let b = p.dot(sun);
        // Lowest the sun ray comes to the centre, when the sun is below the horizon.
        let along = (-b).max(0.0);
        let closest = ((r - along) * (r + along)).max(0.0).sqrt();
        let shadow = smoothstep(radius - SHADOW_DEPTH, radius - SHADOW_EDGE, closest);
        if shadow <= 0.0 {
            return [0.0; 3];
        }
        let top = self.top();
        let (mut r, mut mu) = (r, b / r.max(1.0));
        if r > top {
            // From outside, the light only dims from where the ray enters the air.
            let c = (r - top) * (r + top);
            let disc = b * b - c;
            if disc < 0.0 || b >= 0.0 {
                return [shadow; 3];
            }
            let entry = p + sun * (-b - disc.sqrt());
            r = top;
            mu = entry.dot(sun) / top;
        }
        let texel = self.transmittance_texel(r, mu);
        Self::lookup(&self.transmittance, TRANSMITTANCE_SIZE, texel).map(|x| (-x).exp() * shadow)
    }

    /// Light from all orders above the first at `p`, per unit of scattering. Mirrors the shader.
    fn multiple_scattering(&self, p: DVec3, sun: DVec3) -> Rgb {
        let r = p.length();
        let texel = self.sky_table_texel(r, p.dot(sun) / r.max(1.0));
        Self::lookup(&self.multiple, SKY_TABLE_SIZE, texel)
    }

    /// The light the sky puts on level ground at `p`, per unit of sunlight. Mirrors the shader.
    pub fn sky_irradiance(&self, p: DVec3, sun: DVec3) -> Rgb {
        let r = p.length();
        let texel = self.sky_table_texel(r, p.dot(sun) / r.max(1.0));
        Self::lookup(&self.irradiance, SKY_TABLE_SIZE, texel)
    }

    /// Scattering along the ray segment `origin + dir * [t0, t1]`, in `steps` steps at most.
    /// Mirrors the shader's `scatter`.
    ///
    /// Air is densest where a ray comes closest to the planet, and thins out both ways. So the
    /// segment is split there, half the steps to each side, and the steps grow with the cube of
    /// their distance from it, which keeps them short where the light comes from. A ray that
    /// only leaves that point (looking up from the ground) has just one side to do. Against a
    /// fine march that is within 2.2% anywhere at 32 steps, and 1% at 48.
    pub fn scatter(
        &self,
        origin: DVec3,
        dir: DVec3,
        t0: f64,
        t1: f64,
        sun: DVec3,
        steps: usize,
    ) -> Ray {
        let mut ray = Ray {
            inscatter: [0.0; 3],
            transmittance: [1.0; 3],
        };
        if t1 <= t0 {
            return ray;
        }
        let mu = dir.dot(sun);
        let d = &self.def;
        let (rayleigh, mie) = (phase_rayleigh(mu), d.mie_g.map(|g| phase_mie(mu, g)));
        let closest = (-origin.dot(dir)).clamp(t0, t1);
        let half = (steps / 2).max(1);
        let span = |x: f64, side: f64| side * x * x * x;
        // From the near end towards the closest point, then away from it.
        let toward = closest - t0;
        let away = t1 - closest;
        let mut march = |a: f64, b: f64| {
            let ds = b - a;
            let p = origin + dir * ((a + b) / 2.0);
            let density = self.densities(p.length() - self.radius);
            let extinction = self.extinction(density);
            let lit = self.sun_transmittance(p, sun);
            let more = self.multiple_scattering(p, sun);
            for c in 0..3 {
                let molecules = d.rayleigh[c] * density[0];
                let aerosols = d.mie[c] * density[1];
                let source = lit[c] * (molecules * rayleigh + aerosols * mie[c])
                    + more[c] * (molecules + aerosols);
                ray.inscatter[c] += ray.transmittance[c] * step_integral(source, extinction[c], ds);
                ray.transmittance[c] *= (-extinction[c] * ds).exp();
            }
        };
        if toward > 0.0 {
            for k in 0..half {
                let (a, b) = (
                    (half - k) as f64 / half as f64,
                    (half - k - 1) as f64 / half as f64,
                );
                march(closest - span(a, toward), closest - span(b, toward));
            }
        }
        if away > 0.0 {
            for k in 0..half {
                let (a, b) = (k as f64 / half as f64, (k + 1) as f64 / half as f64);
                march(closest + span(a, away), closest + span(b, away));
            }
        }
        ray
    }

    /// The tables, for the GPU: optical depth, then the two sky tables.
    pub fn transmittance_table(&self) -> &[[f32; 3]] {
        &self.transmittance
    }

    pub fn multiple_table(&self) -> &[[f32; 3]] {
        &self.multiple
    }

    pub fn irradiance_table(&self) -> &[[f32; 3]] {
        &self.irradiance
    }
}

/// The `k`th of `n` directions spread evenly over the sphere (a Fibonacci lattice).
fn sphere_direction(k: usize, n: usize) -> DVec3 {
    const GOLDEN_ANGLE: f64 = 2.399963229728653;
    let z = 1.0 - 2.0 * (k as f64 + 0.5) / n as f64;
    let ring = (1.0 - z * z).max(0.0).sqrt();
    let around = GOLDEN_ANGLE * k as f64;
    DVec3::new(ring * around.cos(), ring * around.sin(), z)
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    const R: f64 = 6_371_000.0;

    fn earth_def() -> ScatteringDef {
        ScatteringDef {
            height: 100_000.0,
            rayleigh: [5.802e-6, 13.558e-6, 33.1e-6],
            rayleigh_height: 8000.0,
            mie: [3.996e-6; 3],
            mie_absorption: [0.444e-6; 3],
            mie_height: 1200.0,
            mie_g: [0.8; 3],
            ozone: [0.650e-6, 1.881e-6, 0.085e-6],
            ozone_height: 25_000.0,
            ozone_width: 15_000.0,
            ground_albedo: [0.12; 3],
            plasma: None,
        }
    }

    fn mars_def() -> ScatteringDef {
        ScatteringDef {
            height: 100_000.0,
            rayleigh: [0.2e-6, 0.46e-6, 1.1e-6],
            rayleigh_height: 11_100.0,
            mie: [4.37e-5, 4.1e-5, 3.6e-5],
            mie_absorption: [0.13e-5, 0.4e-5, 0.9e-5],
            mie_height: 11_100.0,
            mie_g: [0.65, 0.69, 0.76],
            ozone: [0.0; 3],
            ozone_height: 0.0,
            ozone_width: 1.0,
            ground_albedo: [0.3, 0.17, 0.1],
            plasma: None,
        }
    }

    static EARTH: LazyLock<Air> = LazyLock::new(|| Air::new(&earth_def(), R));
    static MARS: LazyLock<Air> = LazyLock::new(|| Air::new(&mars_def(), 3_389_500.0));

    /// Brute-force march towards the sun from `p`, for checking the table.
    fn reference_sun(p: DVec3, sun: DVec3) -> Rgb {
        let air = &*EARTH;
        let r = p.length();
        let b = p.dot(sun);
        let along = (-b).max(0.0);
        let closest = ((r - along) * (r + along)).max(0.0).sqrt();
        let shadow = smoothstep(R - SHADOW_DEPTH, R - SHADOW_EDGE, closest);
        let top = air.top();
        let exit = (-b + (b * b - (r - top) * (r + top)).max(0.0).sqrt()).max(0.0);
        let steps = 100_000;
        let ds = exit / steps as f64;
        let mut depth = [0.0; 3];
        for i in 0..steps {
            let p = p + sun * (ds * (i as f64 + 0.5));
            let e = air.extinction(air.densities(p.length() - R));
            depth = add(depth, scale(e, ds));
        }
        depth.map(|x| (-x).exp() * shadow)
    }

    #[test]
    fn transmittance_table_matches_a_fine_march() {
        // Ground, mountains, flight, the stratosphere and orbit; sun high, low, setting and
        // just below the horizon (lit through the shadow's soft edge).
        for height in [0.0, 3_000.0, 12_000.0, 40_000.0, 400_000.0] {
            for elevation in [80.0f64, 30.0, 10.0, 3.0, 1.0, 0.0, -0.5, -1.5] {
                let p = DVec3::new(0.0, 0.0, R + height);
                let e = elevation.to_radians();
                let sun = DVec3::new(e.cos(), 0.0, e.sin());
                let (table, exact) = (EARTH.sun_transmittance(p, sun), reference_sun(p, sun));
                for k in 0..3 {
                    // Within 1% of the light, or 2% of what gets through.
                    let error = (table[k] - exact[k]).abs();
                    assert!(
                        error < 0.01 || error < 0.02 * exact[k],
                        "h {height} m, sun {elevation} deg: {table:?} vs {exact:?}"
                    );
                }
            }
        }
    }

    #[test]
    fn overhead_sun_is_bright_and_slightly_yellow() {
        let p = DVec3::new(0.0, 0.0, R + 10.0);
        let t = EARTH.sun_transmittance(p, DVec3::Z);
        assert!(t[0] > 0.9 && t[2] > 0.6 && t[0] > t[2], "{t:?}");
    }

    #[test]
    fn low_sun_is_red_and_set_sun_is_dark() {
        let p = DVec3::new(0.0, 0.0, R + 10.0);
        let low = EARTH.sun_transmittance(p, DVec3::new(1.0, 0.0, 0.03).normalize());
        assert!(low[0] > 3.0 * low[2], "{low:?}");
        let set = EARTH.sun_transmittance(p, DVec3::new(1.0, 0.0, -0.1).normalize());
        assert_eq!(set, [0.0; 3]);
    }

    #[test]
    fn space_is_unaffected_unless_in_shadow() {
        let lit = EARTH.sun_transmittance(DVec3::new(0.0, 0.0, R + 400e3), DVec3::X);
        assert!(lit.iter().all(|&t| t > 0.999), "{lit:?}");
        let shadow = EARTH.sun_transmittance(DVec3::new(-(R + 400e3), 0.0, 0.0), DVec3::X);
        assert_eq!(shadow, [0.0; 3]);
        // Far out, sunlight grazing the atmosphere on its way is reddened.
        let grazing = DVec3::new(-2.0 * R, 0.0, R + 20e3);
        let t = EARTH.sun_transmittance(grazing, DVec3::X);
        assert!(t[2] < 0.5 && t[0] > t[2], "{t:?}");
    }

    /// The whole ray at a very fine, even step, the sky tables still standing in for the
    /// higher orders.
    fn reference_ray(air: &Air, eye: DVec3, dir: DVec3, t0: f64, t1: f64, sun: DVec3) -> Ray {
        let steps = 40_000;
        let ds = (t1 - t0) / steps as f64;
        let mu = dir.dot(sun);
        let mie = air.def.mie_g.map(|g| phase_mie(mu, g));
        let mut ray = Ray {
            inscatter: [0.0; 3],
            transmittance: [1.0; 3],
        };
        for i in 0..steps {
            let p = eye + dir * (t0 + ds * (i as f64 + 0.5));
            let density = air.densities(p.length() - air.radius);
            let extinction = air.extinction(density);
            let lit = air.sun_transmittance(p, sun);
            let more = air.multiple_scattering(p, sun);
            for c in 0..3 {
                let molecules = air.def.rayleigh[c] * density[0];
                let aerosols = air.def.mie[c] * density[1];
                let source = lit[c] * (molecules * phase_rayleigh(mu) + aerosols * mie[c])
                    + more[c] * (molecules + aerosols);
                ray.inscatter[c] += ray.transmittance[c] * step_integral(source, extinction[c], ds);
                ray.transmittance[c] *= (-extinction[c] * ds).exp();
            }
        }
        ray
    }

    /// The view from a height, looking at an elevation above the local horizon, with the sun at
    /// an elevation and the view turned `away` from it (degrees).
    fn view(
        air: &Air,
        height: f64,
        look: f64,
        sun_elevation: f64,
        away: f64,
    ) -> (DVec3, DVec3, DVec3) {
        let eye = DVec3::new(0.0, 0.0, air.radius + height);
        let (look, sun_e, away) = (
            look.to_radians(),
            sun_elevation.to_radians(),
            away.to_radians(),
        );
        let dir = DVec3::new(look.cos(), 0.0, look.sin());
        let sun = DVec3::new(
            sun_e.cos() * away.cos(),
            sun_e.cos() * away.sin(),
            sun_e.sin(),
        );
        (eye, dir, sun)
    }

    fn sky(air: &Air, height: f64, look: f64, sun_elevation: f64, away: f64) -> Ray {
        let (eye, dir, sun) = view(air, height, look, sun_elevation, away);
        let hits = sphere_hits(eye, dir, air.top()).unwrap();
        let (end, _) = air.ray_end(eye, dir);
        air.scatter(
            eye,
            dir,
            hits.0.max(0.0),
            if height > air.def.height {
                hits.1.min(end)
            } else {
                end
            },
            sun,
            32,
        )
    }

    #[test]
    fn the_short_march_matches_a_fine_one_everywhere_you_can_look() {
        // From the ground, from a mountain, in flight and in orbit, up, level, at the horizon
        // and down, sun high, low and setting.
        for height in [2.0, 3_000.0, 30_000.0, 300_000.0] {
            for look in [90.0f64, 45.0, 10.0, 2.0, 0.0, -2.0, -15.0, -45.0, -90.0] {
                for sun in [70.0f64, 15.0, 2.0, -3.0] {
                    for away in [0.0, 90.0, 180.0] {
                        let air = &*EARTH;
                        let (eye, dir, sun_dir) = view(air, height, look, sun, away);
                        let Some(hits) = sphere_hits(eye, dir, air.top()) else {
                            continue;
                        };
                        let t0 = hits.0.max(0.0);
                        let (end, _) = air.ray_end(eye, dir);
                        // From outside the air, `ray_end` means nothing past the far side.
                        let t1 = end.min(hits.1);
                        if t1 <= t0 {
                            continue;
                        }
                        let ray = air.scatter(eye, dir, t0, t1, sun_dir, 32);
                        let exact = reference_ray(air, eye, dir, t0, t1, sun_dir);
                        for k in 0..3 {
                            let error = (ray.inscatter[k] - exact.inscatter[k]).abs();
                            // A tenth of a percent of the sun's light is well under what shows;
                            // otherwise within 3% of what is there.
                            let share = error / exact.inscatter[k].max(1e-3);
                            assert!(
                                share < 0.03,
                                "h {height} look {look} sun {sun} away {away}: {:?} vs {:?}",
                                ray.inscatter,
                                exact.inscatter
                            );
                            let te = (ray.transmittance[k] - exact.transmittance[k]).abs();
                            assert!(
                                te < 0.02 || te < 0.05 * exact.transmittance[k],
                                "transmittance h {height} look {look}: {:?} vs {:?}",
                                ray.transmittance,
                                exact.transmittance
                            );
                        }
                    }
                }
            }
        }
    }

    #[test]
    fn multiple_scattering_adds_a_good_share_to_the_noon_sky() {
        // Without it the sky is too dark; it is a fair share of the zenith's light, not most.
        let air = &*EARTH;
        let (eye, dir, sun) = view(air, 2.0, 90.0, 60.0, 90.0);
        let (end, _) = air.ray_end(eye, dir);
        let all = air.scatter(eye, dir, 0.0, end, sun, 64);
        let mut single_only = air.clone();
        single_only.multiple.iter_mut().for_each(|x| *x = [0.0; 3]);
        let single = single_only.scatter(eye, dir, 0.0, end, sun, 64);
        for c in 0..3 {
            let share = all.inscatter[c] / single.inscatter[c];
            assert!(share > 1.05 && share < 2.0, "channel {c}: {share}");
        }
    }

    #[test]
    fn the_noon_sky_is_blue_and_paler_at_the_horizon() {
        let zenith = sky(&EARTH, 2.0, 90.0, 60.0, 90.0).inscatter;
        let horizon = sky(&EARTH, 2.0, 2.0, 60.0, 90.0).inscatter;
        assert!(zenith[2] > zenith[1] && zenith[1] > zenith[0], "{zenith:?}");
        let saturation = |c: Rgb| c[2] / c[0];
        assert!(
            saturation(zenith) > saturation(horizon),
            "{zenith:?} {horizon:?}"
        );
        assert!(horizon[1] > zenith[1], "{zenith:?} {horizon:?}");
    }

    #[test]
    fn sunset_glows_towards_the_sun_and_the_zenith_stays_blue() {
        let toward = sky(&EARTH, 2.0, 2.0, 1.0, 0.0).inscatter;
        let zenith = sky(&EARTH, 2.0, 90.0, 1.0, 0.0).inscatter;
        assert!(toward[0] > toward[2], "towards the sun {toward:?}");
        assert!(zenith[2] > zenith[0], "zenith {zenith:?}");
    }

    #[test]
    fn the_sky_keeps_some_light_after_the_sun_has_set() {
        // Civil twilight, 3 degrees down: lit from above by the air that still sees the sun.
        let dusk = sky(&EARTH, 2.0, 30.0, -3.0, 0.0).inscatter;
        assert!(dusk[2] > 1e-6, "{dusk:?}");
        // And deep night none to speak of.
        let night = sky(&EARTH, 2.0, 30.0, -40.0, 0.0).inscatter;
        assert!(night[2] < 1e-7, "{night:?}");
    }

    #[test]
    fn daylight_on_the_ground_is_about_an_eighth_from_the_sky() {
        // Clear-sky diffuse light is about 10-20% of the total at high sun.
        let p = DVec3::new(0.0, 0.0, R + 2.0);
        let sky_light = EARTH.sky_irradiance(p, DVec3::Z);
        let sun = EARTH.sun_transmittance(p, DVec3::Z);
        let green = sky_light[1] / (sky_light[1] + sun[1]);
        assert!(green > 0.08 && green < 0.25, "{green}: {sky_light:?}");
        // And it is blue.
        assert!(sky_light[2] > 1.3 * sky_light[0], "{sky_light:?}");
    }

    #[test]
    fn the_martian_day_is_butterscotch_and_its_sunset_is_blue() {
        let zenith = sky(&MARS, 2.0, 70.0, 50.0, 90.0).inscatter;
        assert!(zenith[0] > zenith[1] && zenith[1] > zenith[2], "{zenith:?}");
        // Right around the setting sun, a blue aureole against the red sky.
        let aureole = sky(&MARS, 2.0, 5.0, 4.0, 0.0).inscatter;
        assert!(aureole[2] > aureole[0], "{aureole:?}");
        let opposite = sky(&MARS, 2.0, 5.0, 4.0, 180.0).inscatter;
        assert!(opposite[0] > opposite[2], "{opposite:?}");
    }

    #[test]
    fn dense_air_stays_finite_and_in_range() {
        // Venus-like: optically thick from top to bottom, hardly any absorption.
        let thick = ScatteringDef {
            height: 200_000.0,
            rayleigh: [1e-6, 2e-6, 5e-6],
            rayleigh_height: 15_900.0,
            mie: [2.5e-4, 2.3e-4, 1.7e-4],
            mie_absorption: [2e-7, 2e-6, 1.2e-5],
            mie_height: 15_900.0,
            mie_g: [0.75; 3],
            ozone: [0.0; 3],
            ozone_height: 0.0,
            ozone_width: 1.0,
            ground_albedo: [0.2, 0.15, 0.1],
            plasma: None,
        };
        let air = Air::new(&thick, 6_051_800.0);
        for height in [2.0, 50_000.0, 500_000.0] {
            for look in [90.0, 2.0, -5.0, -90.0] {
                for sun in [60.0, 2.0, -5.0] {
                    let (eye, dir, sun) = view(&air, height, look, sun, 0.0);
                    let Some(hits) = sphere_hits(eye, dir, air.top()) else {
                        continue;
                    };
                    let (end, _) = air.ray_end(eye, dir);
                    let ray = air.scatter(eye, dir, hits.0.max(0.0), end.min(hits.1), sun, 32);
                    for c in 0..3 {
                        assert!(ray.inscatter[c].is_finite() && ray.inscatter[c] >= 0.0);
                        // Bright, but no brighter than sunlight spread over the sky can be.
                        assert!(ray.inscatter[c] < 1.0, "{:?}", ray.inscatter);
                        assert!((0.0..=1.0).contains(&ray.transmittance[c]));
                    }
                }
            }
        }
    }
}
