//! The star background: NASA's Deep Star Map, reprojected from plate carrée to a cubemap
//! and saved as six square faces stacked top to bottom (+X, −X, +Y, −Y, +Z, −Z), the layout
//! Bevy reinterprets as a cube texture.
//!
//! The faces are in Bevy's render axes for the ICRF (Y = celestial north pole), as
//! `frame::to_render` maps sim axes: the game only rotates them from the equator to the
//! ecliptic.

use std::{io, path::Path};

use glam::DVec3;
use image::{ImageBuffer, Rgb, Rgb32FImage};
use rayon::prelude::*;

use crate::sources::{Source, fetch};

/// Texels per face edge: the 8k map's 0.044° per pixel at the face centres.
const FACE: u32 = 2048;
/// Samples per texel edge, so each texel averages the map pixels it covers.
const SUPERSAMPLE: u32 = 2;
/// Linear map value written as white. The map's stars are much brighter than the Milky
/// Way; this keeps the band visible and lets only the brightest stars clip.
const WHITE: f32 = 0.6;

/// NASA SVS Deep Star Maps 2020: Hipparcos-2, Tycho-2 and Gaia DR2 stars, celestial axes.
fn starmap() -> Source {
    Source {
        file: "starmap_2020_8k.exr".to_owned(),
        url: "https://svs.gsfc.nasa.gov/vis/a000000/a004800/a004851/starmap_2020_8k.exr".to_owned(),
    }
}

pub const ATTRIBUTION: &str = "Deep Star Maps 2020: NASA/Goddard Space Flight Center Scientific Visualization Studio, \
     with Gaia DR2 (ESA/Gaia/DPAC), Hipparcos-2 and Tycho-2";

pub fn bake(cache: &Path, out: &Path) -> io::Result<()> {
    let path = fetch(cache, &starmap(), false)?.expect("required download");
    println!("Loading star map");
    let map = image::open(&path).map_err(io::Error::other)?.into_rgb32f();
    println!("Reprojecting to a {FACE}² cubemap");
    let faces: Vec<Vec<[u8; 3]>> = (0..6)
        .map(|face| {
            (0..FACE * FACE)
                .into_par_iter()
                .map(|i| encode(texel(&map, face, i % FACE, i / FACE)))
                .collect()
        })
        .collect();
    let image = ImageBuffer::<Rgb<u8>, _>::from_raw(
        FACE,
        FACE * 6,
        faces.into_iter().flatten().flatten().collect::<Vec<u8>>(),
    )
    .expect("six faces");
    std::fs::create_dir_all(out.parent().unwrap_or(Path::new(".")))?;
    image.save(out).map_err(io::Error::other)?;
    println!("Wrote {}", out.display());
    Ok(())
}

/// Average of the map over one cube texel.
fn texel(map: &Rgb32FImage, face: u32, x: u32, y: u32) -> [f32; 3] {
    let mut sum = [0.0; 3];
    for sy in 0..SUPERSAMPLE {
        for sx in 0..SUPERSAMPLE {
            let u = (x as f64 + (sx as f64 + 0.5) / SUPERSAMPLE as f64) / FACE as f64;
            let v = (y as f64 + (sy as f64 + 0.5) / SUPERSAMPLE as f64) / FACE as f64;
            let c = bilinear(map, render_to_icrf(cube_dir(face, u, v)));
            for k in 0..3 {
                sum[k] += c[k];
            }
        }
    }
    sum.map(|s| s / (SUPERSAMPLE * SUPERSAMPLE) as f32)
}

/// Direction of a cube texel, as Bevy's skybox looks it up (`sample_cube_dir` in
/// `bevy_pbr`, then its left-handed z flipped back).
fn cube_dir(face: u32, u: f64, v: f64) -> DVec3 {
    let (a, b) = (2.0 * u - 1.0, 2.0 * v - 1.0);
    let d = match face {
        0 => DVec3::new(1.0, -b, -a),
        1 => DVec3::new(-1.0, -b, a),
        2 => DVec3::new(a, 1.0, b),
        3 => DVec3::new(a, -1.0, -b),
        4 => DVec3::new(a, -b, 1.0),
        _ => DVec3::new(-a, -b, -1.0),
    };
    DVec3::new(d.x, d.y, -d.z).normalize()
}

/// Render axes (Y up) back to sim axes (Z up): the inverse of `frame::to_render`.
fn render_to_icrf(r: DVec3) -> DVec3 {
    DVec3::new(r.x, -r.z, r.y)
}

/// The map at a direction: right ascension 0 in the middle, increasing to the left.
fn bilinear(map: &Rgb32FImage, d: DVec3) -> [f32; 3] {
    let (w, h) = map.dimensions();
    let ra = d.y.atan2(d.x);
    let dec = d.z.clamp(-1.0, 1.0).asin();
    let px = (0.5 - ra / std::f64::consts::TAU) * w as f64 - 0.5;
    let py = (0.5 - dec / std::f64::consts::PI) * h as f64 - 0.5;
    let (x0, y0) = (px.floor(), py.floor());
    let (fx, fy) = ((px - x0) as f32, (py - y0) as f32);
    let at = |x: f64, y: f64| {
        let x = (x as i64).rem_euclid(w as i64) as u32;
        let y = (y as i64).clamp(0, h as i64 - 1) as u32;
        map.get_pixel(x, y).0
    };
    let (a, b, c, e) = (
        at(x0, y0),
        at(x0 + 1.0, y0),
        at(x0, y0 + 1.0),
        at(x0 + 1.0, y0 + 1.0),
    );
    std::array::from_fn(|k| {
        let top = a[k] + (b[k] - a[k]) * fx;
        let bottom = c[k] + (e[k] - c[k]) * fx;
        top + (bottom - top) * fy
    })
}

/// Linear map value to 8-bit sRGB.
fn encode(c: [f32; 3]) -> [u8; 3] {
    c.map(|x| {
        let x = (x / WHITE).clamp(0.0, 1.0);
        let s = if x <= 0.003_130_8 {
            12.92 * x
        } else {
            1.055 * x.powf(1.0 / 2.4) - 0.055
        };
        (s * 255.0).round() as u8
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cube_faces_point_along_their_axes() {
        // Bevy flips z before sampling, so the +Z face holds what lies along −Z.
        let centres = [
            DVec3::X,
            -DVec3::X,
            DVec3::Y,
            -DVec3::Y,
            -DVec3::Z,
            DVec3::Z,
        ];
        for (face, centre) in centres.into_iter().enumerate() {
            let d = cube_dir(face as u32, 0.5, 0.5);
            assert!(d.distance(centre) < 1e-12, "face {face}: {d}");
        }
    }

    #[test]
    fn render_up_is_the_celestial_pole() {
        assert!(render_to_icrf(DVec3::Y).distance(DVec3::Z) < 1e-12);
    }
}
