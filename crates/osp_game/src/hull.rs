//! Procedural stage hulls: what a stage looks like, built from the stage itself and its
//! `appearance` (see `osp_sim::data::StageAppearance`).
//!
//! A stage's tanks are painted barrels with weld seams, the structure above them is an
//! interstage (dark composite around the next stage's engines, tapering if the next stage
//! is narrower) or more hull. A stage with a fairing is the fairing until it is jettisoned,
//! then the payload inside shows; a top stage without tanks, engines, pod or fairing looks
//! like one too. The top of the stack gets a tangent-ogive nose. Engine stages get a dark base
//! ring and heat shield. Optional extras: folded landing legs, stowed grid fins and a cable
//! raceway. Parts come out as three vertex-coloured meshes, one per material: glossy paint,
//! matte composite and bare metal.
//!
//! Coordinates are the stage's: `y` up from its base; vessel X/Y map to render X/−Z like
//! the engine and RCS layouts, so angle `a` points along `(cos a, 0, −sin a)`.

use std::f32::consts::{FRAC_PI_2, FRAC_PI_4, TAU};

use bevy::prelude::*;
use osp_sim::{data::TankShape, propulsion::Stage};

use crate::meshgen::MeshBuilder;

/// Segments around a hull.
const SEGMENTS: usize = 48;
const WHITE: [f32; 3] = [0.85, 0.85, 0.83];
const COMPOSITE: [f32; 3] = [0.028, 0.028, 0.032];
const TITANIUM: [f32; 3] = [0.55, 0.55, 0.57];
const GOLD_FOIL: [f32; 3] = [0.85, 0.62, 0.25];
const SOLAR_CELLS: [f32; 3] = [0.03, 0.05, 0.14];
/// Barrel sections between weld seams are about this tall, m.
const BARREL_SECTION: f32 = 6.0;
/// Nose length as a multiple of the diameter.
const NOSE_FINENESS: f32 = 1.3;

/// A stage's hull, by material.
#[derive(Default)]
pub struct Hull {
    pub paint: MeshBuilder,
    pub matte: MeshBuilder,
    pub metal: MeshBuilder,
}

fn rgb(c: [f32; 3]) -> LinearRgba {
    LinearRgba::rgb(c[0], c[1], c[2])
}

fn shade(c: LinearRgba, f: f32) -> LinearRgba {
    LinearRgba::rgb(c.red * f, c.green * f, c.blue * f)
}

/// Outward direction at angle `a` around the stage axis.
fn out(a: f32) -> Vec3 {
    Vec3::new(a.cos(), 0.0, -a.sin())
}

/// A surface of revolution through `profile` points `(radius, y)` from bottom to top,
/// smooth-shaded, open at both ends.
fn revolve(mesh: &mut MeshBuilder, profile: &[(f32, f32)], color: LinearRgba) {
    for w in profile.windows(2) {
        let [(r0, y0), (r1, y1)] = [w[0], w[1]];
        if (y1 - y0).abs() < 1e-5 && (r1 - r0).abs() < 1e-5 {
            continue;
        }
        // Normal of the profile segment, pointing outwards.
        let (dr, dy) = (r1 - r0, y1 - y0);
        let len = (dr * dr + dy * dy).sqrt();
        let (nr, ny) = (dy / len, -dr / len);
        for k in 0..SEGMENTS {
            let (a0, a1) = (
                TAU * k as f32 / SEGMENTS as f32,
                TAU * (k + 1) as f32 / SEGMENTS as f32,
            );
            let p = |r: f32, y: f32, a: f32| out(a) * r + Vec3::Y * y;
            let n = |a: f32| (out(a) * nr + Vec3::Y * ny).normalize();
            let (a, b) = (p(r0, y0, a0), p(r0, y0, a1));
            let (c, d) = (p(r1, y1, a1), p(r1, y1, a0));
            mesh.triangle_smooth([(a, n(a0)), (b, n(a1)), (c, n(a1))], color);
            if r1 > 1e-4 {
                mesh.triangle_smooth([(a, n(a0)), (c, n(a1)), (d, n(a0))], color);
            }
        }
    }
}

/// A flat disc at height `y`, facing down or up.
fn disc(mesh: &mut MeshBuilder, y: f32, radius: f32, down: bool, color: LinearRgba) {
    let center = Vec3::Y * y;
    for k in 0..SEGMENTS {
        let a0 = TAU * k as f32 / SEGMENTS as f32;
        let a1 = TAU * (k + 1) as f32 / SEGMENTS as f32;
        let (p0, p1) = (center + out(a0) * radius, center + out(a1) * radius);
        if down {
            mesh.triangle(center, p0, p1, color);
        } else {
            mesh.triangle(center, p1, p0, color);
        }
    }
}

/// A barrel from `y0` to `y1` with a thin, slightly proud weld seam at each section joint.
fn barrel(mesh: &mut MeshBuilder, radius: f32, y0: f32, y1: f32, color: LinearRgba) {
    revolve(mesh, &[(radius, y0), (radius, y1)], color);
    let sections = ((y1 - y0) / BARREL_SECTION).round().max(1.0) as usize;
    let seam = shade(color, 0.72);
    for k in 0..=sections {
        let y = y0 + (y1 - y0) * k as f32 / sections as f32;
        let r = radius + 0.006;
        let (lo, hi) = ((y - 0.025).max(y0), (y + 0.025).min(y1));
        revolve(mesh, &[(r, lo), (r, hi)], seam);
    }
}

/// Tangent-ogive nose of `radius` and `length` from `y0`, with a small rounded tip.
fn nose_profile(radius: f32, length: f32, y0: f32) -> Vec<(f32, f32)> {
    let rho = (radius * radius + length * length) / (2.0 * radius);
    let steps = 24;
    (0..=steps)
        .map(|i| {
            let x = length * i as f32 / steps as f32;
            // Radius at height x above the base of the nose.
            let r = (rho * rho - x * x).max(0.0).sqrt() + radius - rho;
            // Blunt the very tip a little.
            let r = if i == steps { 0.0 } else { r.max(0.0) };
            (r, y0 + x)
        })
        .collect()
}

/// Seam strips along a profile at angles `angles` (fairing halves).
fn profile_seams(
    mesh: &mut MeshBuilder,
    profile: &[(f32, f32)],
    angles: &[f32],
    color: LinearRgba,
) {
    let half = 0.012;
    for &a in angles {
        let side = out(a + FRAC_PI_2);
        for w in profile.windows(2) {
            let [(r0, y0), (r1, y1)] = [w[0], w[1]];
            let lift = 0.008;
            let p0 = out(a) * (r0 + lift) + Vec3::Y * y0;
            let p1 = out(a) * (r1 + lift) + Vec3::Y * y1;
            mesh.quad(
                p0 - side * half,
                p0 + side * half,
                p1 + side * half,
                p1 - side * half,
                color,
            );
        }
    }
}

/// A cable raceway: a low box up the side of the stage at angle `a`.
fn raceway(mesh: &mut MeshBuilder, radius: f32, y0: f32, y1: f32, a: f32, color: LinearRgba) {
    let (h, depth, width) = (y1 - y0, 0.11, 0.3);
    let center = out(a) * (radius + depth / 2.0) + Vec3::Y * (y0 + h / 2.0);
    let rotation = Quat::from_rotation_y(a);
    mesh.cuboid(
        center,
        Vec3::new(depth / 2.0, h / 2.0, width / 2.0),
        rotation,
        color,
    );
}

/// A landing leg folded up against the hull at angle `a`: a tapered prism from just below
/// the base up to `length`, with a foot and an actuator ridge.
fn leg(hull: &mut Hull, radius: f32, length: f32, a: f32) {
    let color = rgb(COMPOSITE);
    let (o, side) = (out(a), out(a + FRAC_PI_2));
    let (y0, y1) = (-0.5, length);
    // (half width, thickness) at the bottom and top.
    let ((w0, t0), (w1, t1)) = ((0.75, 0.32), (0.34, 0.18));
    let r = radius + 0.02;
    let p = |y: f32, w: f32, t: f32, s: f32| o * (r + t) + side * (w * s) + Vec3::Y * y;
    let q = |y: f32, w: f32, s: f32| o * r + side * (w * s) + Vec3::Y * y;
    let mesh = &mut hull.matte;
    // Outer face, sides, and the end caps.
    mesh.quad(
        p(y0, w0, t0, -1.0),
        p(y0, w0, t0, 1.0),
        p(y1, w1, t1, 1.0),
        p(y1, w1, t1, -1.0),
        color,
    );
    mesh.quad(
        q(y0, w0, 1.0),
        q(y1, w1, 1.0),
        p(y1, w1, t1, 1.0),
        p(y0, w0, t0, 1.0),
        color,
    );
    mesh.quad(
        q(y0, w0, -1.0),
        p(y0, w0, t0, -1.0),
        p(y1, w1, t1, -1.0),
        q(y1, w1, -1.0),
        color,
    );
    mesh.quad(
        q(y1, w1, -1.0),
        p(y1, w1, t1, -1.0),
        p(y1, w1, t1, 1.0),
        q(y1, w1, 1.0),
        color,
    );
    mesh.quad(
        q(y0, w0, -1.0),
        q(y0, w0, 1.0),
        p(y0, w0, t0, 1.0),
        p(y0, w0, t0, -1.0),
        color,
    );
    // The telescoping actuator along its middle.
    let ridge = shade(color, 1.8);
    let along = |y: f32| {
        let f = (y - y0) / (y1 - y0);
        o * (r + t0 + (t1 - t0) * f + 0.05) + Vec3::Y * y
    };
    mesh.beam(along(y0 + 0.6), along(y1 - 0.8), 0.14, ridge);
    // The foot, in bare metal.
    hull.metal.cuboid(
        o * (r + t0 * 0.6) + Vec3::Y * (y0 + 0.12),
        Vec3::new(0.24, 0.12, w0 * 0.85),
        Quat::from_rotation_y(a),
        rgb(TITANIUM),
    );
}

/// A grid fin stowed flat against the hull at angle `a`, centred at height `y`: a frame
/// with a diagonal lattice, on a dark actuator housing.
fn grid_fin(hull: &mut Hull, radius: f32, y: f32, a: f32) {
    let (o, side) = (out(a), out(a + FRAC_PI_2));
    let (half_w, half_h, standoff) = (0.7, 0.8, 0.18);
    let plane = |s: f32, u: f32| o * (radius + standoff) + side * s + Vec3::Y * (y + u);
    let metal = rgb(TITANIUM);
    let bar = 0.05;
    let mesh = &mut hull.metal;
    // Frame.
    let corners = [
        (-half_w, -half_h),
        (half_w, -half_h),
        (half_w, half_h),
        (-half_w, half_h),
    ];
    for i in 0..4 {
        let (s0, u0) = corners[i];
        let (s1, u1) = corners[(i + 1) % 4];
        mesh.beam(plane(s0, u0), plane(s1, u1), bar * 1.6, metal);
    }
    // Lattice at ±45°, clipped to the frame.
    let pitch = 0.22;
    let n = ((half_w + half_h) * 2.0 / pitch) as i32;
    for k in -n..=n {
        let c = k as f32 * pitch;
        for dir in [1.0f32, -1.0] {
            // Points on the line u = dir·s + c inside the rectangle.
            let mut ends = Vec::new();
            for s in [-half_w, half_w] {
                let u = dir * s + c;
                if u.abs() <= half_h {
                    ends.push((s, u));
                }
            }
            for u in [-half_h, half_h] {
                let s = (u - c) * dir;
                if s.abs() <= half_w {
                    ends.push((s, u));
                }
            }
            ends.dedup_by(|p, q| (p.0 - q.0).abs() < 1e-4 && (p.1 - q.1).abs() < 1e-4);
            if ends.len() >= 2 {
                let (s0, u0) = ends[0];
                let (s1, u1) = ends[ends.len() - 1];
                mesh.beam(plane(s0, u0), plane(s1, u1), bar, metal);
            }
        }
    }
    // Actuator housing above it.
    hull.matte.cuboid(
        o * (radius + 0.08) + Vec3::Y * (y + half_h + 0.25),
        Vec3::new(0.08, 0.22, 0.32),
        Quat::from_rotation_y(a),
        shade(rgb(COMPOSITE), 1.5),
    );
}

/// What a jettisoned fairing leaves: a separation ring and adapter cone, and a satellite in
/// gold foil with its solar panels folded against it.
fn payload(hull: &mut Hull, radius: f32, height: f32) {
    let dark = rgb(COMPOSITE);
    let adapter = 0.9;
    let top_radius = 0.5 * radius;
    revolve(
        &mut hull.matte,
        &[(radius, 0.0), (radius, 0.12), (top_radius, adapter)],
        dark,
    );
    disc(&mut hull.matte, 0.0, radius, true, dark);
    disc(&mut hull.matte, adapter, top_radius, false, dark);
    let half = 0.45 * radius;
    let bus = (height - adapter).clamp(1.0, 4.5);
    let middle = Vec3::Y * (adapter + bus / 2.0);
    hull.metal.cuboid(
        middle,
        Vec3::new(half, bus / 2.0, half),
        Quat::IDENTITY,
        rgb(GOLD_FOIL),
    );
    for a in [0.0, std::f32::consts::PI] {
        hull.paint.cuboid(
            middle + out(a) * (half + 0.05),
            Vec3::new(0.025, bus * 0.46, half * 0.9),
            Quat::from_rotation_y(a),
            rgb(SOLAR_CELLS),
        );
    }
    // A small antenna on top.
    let top = adapter + bus;
    revolve(
        &mut hull.paint,
        &[(0.6 * half, top + 0.05), (0.0, top + 0.3)],
        rgb(WHITE),
    );
}

/// Height of the top of the stage's stacked cylindrical tanks above its base, m.
fn tanks_top(stage: &Stage) -> f32 {
    stage
        .tanks
        .iter()
        .filter(|t| t.shape == TankShape::Cylinder)
        .map(|t| (t.base + t.length) as f32)
        .fold(0.0, f32::max)
}

/// The hull of `stage` from its base up to `height` (below its pod, if it has one).
/// `above` is the next stage up; `top` whether nothing is above it.
pub fn stage_hull(stage: &Stage, above: Option<&Stage>, height: f32) -> Hull {
    let mut hull = Hull::default();
    let look = &stage.appearance;
    let paint = rgb(look.paint.unwrap_or(WHITE));
    let interstage = rgb(look.interstage.unwrap_or(COMPOSITE));
    let radius = (stage.diameter / 2.0) as f32;
    let fairing = match &stage.fairing {
        Some(f) => f.attached,
        None => {
            above.is_none()
                && stage.pod.is_none()
                && stage.engines.is_empty()
                && stage.tanks.is_empty()
        }
    };
    if stage.fairing.as_ref().is_some_and(|f| !f.attached) {
        payload(&mut hull, radius, height);
        return hull;
    }
    let tanks = tanks_top(stage).min(height);

    // Painted barrel over the tanks (or the whole fairing).
    let barrel_top = if fairing { height } else { tanks };
    if barrel_top > 0.0 {
        barrel(&mut hull.paint, radius, 0.0, barrel_top, paint);
    }
    // Above the tanks: an interstage around the next stage's engines, or more hull.
    if barrel_top < height {
        let next_radius = above.map_or(radius, |s| (s.diameter / 2.0) as f32);
        let houses_engines = above.is_some_and(|s| !s.engines.is_empty());
        let (mesh, color) = if houses_engines {
            (&mut hull.matte, interstage)
        } else {
            (&mut hull.paint, paint)
        };
        revolve(mesh, &[(radius, barrel_top), (next_radius, height)], color);
        if houses_engines {
            // Lighter rim where the stages meet.
            let rim = shade(color, 3.0);
            revolve(
                &mut hull.matte,
                &[
                    (next_radius + 0.008, height - 0.06),
                    (next_radius + 0.008, height),
                ],
                rim,
            );
        }
    }
    if fairing {
        // Seams between the halves, and the separation joint at the base.
        let mut profile = vec![(radius, 0.0), (radius, height)];
        let nose = nose_profile(radius, NOSE_FINENESS * 2.0 * radius, height);
        profile.extend(nose.iter().skip(1).copied());
        profile_seams(
            &mut hull.paint,
            &profile,
            &[FRAC_PI_2, 3.0 * FRAC_PI_2],
            shade(paint, 0.6),
        );
        revolve(
            &mut hull.matte,
            &[(radius + 0.01, 0.0), (radius + 0.01, 0.08)],
            rgb(COMPOSITE),
        );
    }
    // A nose on top of the stack, unless a pod is there.
    if above.is_none() && stage.pod.is_none() {
        let profile = nose_profile(radius, NOSE_FINENESS * 2.0 * radius, height);
        revolve(&mut hull.paint, &profile, paint);
    }

    if !stage.engines.is_empty() {
        // Dark base ring and the heat shield the engines hang from.
        let base = rgb(COMPOSITE);
        revolve(
            &mut hull.matte,
            &[(radius + 0.004, 0.0), (radius + 0.004, 0.55)],
            base,
        );
        disc(&mut hull.matte, 0.0, radius, true, shade(base, 1.4));
    } else if barrel_top > 0.0 || fairing {
        disc(&mut hull.paint, 0.0, radius, true, shade(paint, 0.6));
    }
    if look.raceway && tanks > 1.0 {
        raceway(
            &mut hull.paint,
            radius,
            0.7,
            tanks - 0.2,
            FRAC_PI_4 * 0.5,
            shade(paint, 0.94),
        );
    }
    for k in 0..look.legs {
        let a = FRAC_PI_4 + TAU * k as f32 / look.legs as f32;
        leg(&mut hull, radius, (0.23 * stage.length as f32).min(10.0), a);
    }
    for k in 0..look.grid_fins {
        let a = TAU * k as f32 / look.grid_fins as f32;
        grid_fin(&mut hull, radius, height - 1.4, a);
    }
    hull
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ogive_nose_meets_the_hull_and_closes_at_the_tip() {
        let profile = nose_profile(1.85, 4.8, 10.0);
        let (r0, y0) = profile[0];
        let (r1, y1) = *profile.last().unwrap();
        assert!((r0 - 1.85).abs() < 1e-4 && (y0 - 10.0).abs() < 1e-6);
        assert!(r1 == 0.0 && (y1 - 14.8).abs() < 1e-4);
        // Tangent at the base: the radius barely changes over the first step.
        assert!(r0 - profile[1].0 < 0.02, "{:?}", &profile[..2]);
        assert!(profile.windows(2).all(|w| w[1].0 <= w[0].0));
    }
}
