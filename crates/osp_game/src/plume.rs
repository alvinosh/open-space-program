//! Engine exhaust plumes: a volume per engine, ray-marched in `shaders/plume.wesl`.
//!
//! The look comes from a `plumes` record (colours, brightness, soot); the shape from the
//! engine's state: its thrust level (a start grows the plume out of the nozzle, throttling
//! shortens and dims it) and its exit pressure against the ambient pressure. A jet that
//! leaves the nozzle at or below the ambient pressure stays a narrow column with shock
//! diamonds; as the air thins it overtakes the ambient pressure, the cells stretch and the
//! plume fans out and fades, until in vacuum it is a wide, faint cone.
//!
//! The plume hangs off the vessel, not the gimbal. Its axis is where the exhaust has been
//! going lately (the nozzle axis smoothed over time), bent towards the airflow in air; the
//! jet leaves along the nozzle's current axis and turns onto it within a few metres, so a
//! quick gimbal move shows near the nozzle instead of swinging the whole plume.

use bevy::{
    asset::embedded_asset,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use serde::{Deserialize, Serialize};

use crate::settings::Quality;

/// 1 atm, Pa.
const ATM: f64 = 101_325.0;

pub struct PlumePlugin;

impl Plugin for PlumePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/plume.wesl");
        app.add_plugins(MaterialPlugin::<PlumeMaterial>::default());
    }
}

/// How an exhaust looks (a `plumes` record). Colours are linear RGB.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlumeDef {
    pub id: String,
    /// The hot core just behind the nozzle.
    pub core: [f32; 3],
    /// The body of the plume.
    pub flame: [f32; 3],
    /// Shock diamonds.
    pub shock: [f32; 3],
    /// Brightness of a metre of flame at full thrust, cd/m² per m.
    pub luminance: f32,
    /// How much the cooling exhaust darkens what is behind it, 0..1.
    #[serde(default)]
    pub soot: f32,
}

/// The shape of a plume for one engine state, in metres.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PlumeShape {
    pub exit_radius: f32,
    /// 0 for a jet at or below the ambient pressure (a column), 1 in vacuum (a wide fan).
    pub expansion: f32,
    /// Length of a shock cell, and how strongly the cells show, 0..1.
    pub cell: f32,
    pub shock: f32,
    /// Distance over which the glow falls to 1/e.
    pub fade: f32,
    /// Tangent of the half-angle the jet's edge spreads at.
    pub spread: f32,
    /// Brightness relative to the plume at sea level: exhaust expanding into vacuum cools
    /// fast and has no air to keep burning with.
    pub glow: f32,
    /// Distance from the nozzle exit along the axis to the ground (infinite when clear): the
    /// jet stops there and spreads out along it.
    pub ground: f32,
    /// The nozzle axis against the plume's, as the tangent of the angle between them in the
    /// plume's cross-section (x, z): the jet leaves along the nozzle and turns onto the plume
    /// axis over `bend_length`.
    pub bend: Vec2,
    pub bend_length: f32,
    /// Bounding cone: length, radius at the nozzle and growth per metre.
    pub length: f32,
    pub bound_radius: f32,
    pub bound_spread: f32,
}

/// How much wider the jet gets where it splashes on the ground.
const SPLASH: f32 = 2.5;
/// Share of the fade length over which the jet turns from the nozzle axis onto the plume's.
const BEND_LENGTH: f32 = 0.3;
/// Largest bend drawn, as the tangent of the angle between nozzle and plume axis.
const MAX_BEND: f32 = 1.0;
/// Seconds over which the plume's axis follows the nozzle's.
pub const AXIS_LAG: f32 = 0.4;
/// How strongly the airflow bends the plume, against the square root of the ratio of the
/// air's dynamic pressure to the jet's momentum flux, and the most it can (below 1, so
/// the airflow never turns a plume blowing into it all the way round).
const CROSSFLOW: f32 = 2.0;
const MAX_CROSSFLOW: f32 = 0.6;

impl PlumeShape {
    /// `exit_pressure` is the engine's at full thrust, `ambient` the air's, both Pa;
    /// `level` the thrust share; `ground` how far the ground is from the nozzle exit along
    /// the axis, m (infinite when clear); `bend` the nozzle axis against the plume's (see
    /// [`plume_axes`]).
    pub fn new(
        exit_radius: f32,
        exit_pressure: f64,
        level: f64,
        ambient: f64,
        ground: f32,
        bend: Vec2,
    ) -> Self {
        let level = level.clamp(0.0, 1.0);
        // Exit pressure scales with chamber pressure, so with thrust.
        let ratio = exit_pressure * level / ambient.max(1e-3);
        let expansion = smoothstep(0.0, 1.0, ratio.max(1.0).log10() / 2.5) as f32;
        let air = (ambient / ATM).clamp(0.0, 1.0);
        // Cells stretch as the jet's pressure overtakes the ambient and fade out once it
        // fans out; they show best in dense air.
        let cell = 2.0 * exit_radius * (0.8 + 0.7 * ratio.min(9.0).sqrt() as f32);
        let shock = smoothstep(0.02, 0.25, air) as f32 * (1.0 - expansion).powi(2);
        let fade = exit_radius * (24.0 + 36.0 * (level as f32).sqrt()) * (1.0 - 0.5 * expansion);
        // In air the jet mixes with it and widens slowly; in vacuum it fans out (the faint
        // edge goes wider still, but the glow stays within about 35°).
        let spread = 0.1 + 0.6 * expansion.powf(1.3);
        let ground = ground.max(0.0);
        let length = (4.0 * fade).min(ground);
        // Room for the shock-cell bulges and the soft edge, and for the splash.
        let bound_spread = 1.9 * spread;
        let mut bound_radius = 1.9 * exit_radius;
        if ground <= length {
            let splash = 1.9 * (1.0 + SPLASH) * (exit_radius + ground * spread);
            bound_radius = bound_radius.max(splash - bound_spread * ground);
        }
        // The bent jet's centre is at most `bend · bend_length` off the axis.
        let bend = bend.clamp_length_max(MAX_BEND);
        let bend_length = BEND_LENGTH * fade;
        bound_radius += bend.length() * bend_length;
        Self {
            exit_radius,
            expansion,
            cell,
            shock,
            fade,
            spread,
            glow: 1.0 - 0.85 * expansion,
            ground,
            bend,
            bend_length,
            length,
            bound_radius,
            bound_spread,
        }
    }

    /// Radius of the bounding cone at its far end.
    pub fn end_radius(&self) -> f32 {
        self.bound_radius + self.bound_spread * self.length
    }
}

/// Where the far part of a plume runs, in vessel axes: along `exhaust` (where the nozzle
/// has been pointing lately), bent towards `airflow` (the air's velocity past the vessel,
/// m/s) the more, the stronger the air is against the jet. `dynamic_pressure` is the air's,
/// `jet` the jet's momentum flux (its thrust over the exit area), both Pa.
pub fn plume_axis(exhaust: Vec3, airflow: Vec3, dynamic_pressure: f32, jet: f32) -> Vec3 {
    let Some(downwind) = airflow.try_normalize() else {
        return exhaust;
    };
    let push = if jet > 0.0 {
        (CROSSFLOW * (dynamic_pressure / jet).sqrt()).min(MAX_CROSSFLOW)
    } else {
        MAX_CROSSFLOW
    };
    (exhaust + push * downwind).normalize()
}

/// The rotation that turns a plume volume (exhaust along −Y) onto `axis`, and the bend of
/// `nozzle` against it for [`PlumeShape::new`].
pub fn plume_axes(axis: Vec3, nozzle: Vec3) -> (Quat, Vec2) {
    let rotation = Quat::from_rotation_arc(Vec3::NEG_Y, axis);
    let local = rotation.inverse() * nozzle;
    let bend = Vec2::new(local.x, local.z) / (-local.y).max(0.5);
    (rotation, bend)
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

#[derive(Debug, Clone, Copy, Default, ShaderType, Reflect)]
pub struct PlumeParams {
    /// x: exit radius, y: bounding length, z: bounding radius at the far end (the mesh is a
    /// unit cylinder scaled to these), w: noise seed.
    pub shape: Vec4,
    /// x: bounding cone radius at the nozzle, y: its growth per metre.
    pub bounds: Vec4,
    /// x: thrust level, y: expansion, z: shock cell length, w: shock strength.
    pub flow: Vec4,
    /// x: glow fade length, y: spread (tan of the half-angle), z: luminance at this
    /// level (cd/m² per m), w: soot.
    pub glow: Vec4,
    pub core: Vec4,
    pub flame: Vec4,
    pub shock: Vec4,
    /// xy: the nozzle axis's bend against the plume axis (x, z), z: bend length.
    pub bend: Vec4,
    /// x: ray-marching steps ([`march_steps`]).
    pub quality: Vec4,
}

/// Ray-marching steps through a plume at each `effects` setting.
pub fn march_steps(quality: Quality) -> u32 {
    match quality {
        Quality::Off | Quality::Low => 16,
        Quality::Medium => 24,
        Quality::High => 40,
        Quality::Ultra => 56,
    }
}

impl PlumeParams {
    pub fn new(shape: &PlumeShape, look: &PlumeDef, level: f32, seed: f32, steps: u32) -> Self {
        let rgb = |c: [f32; 3]| Vec4::new(c[0], c[1], c[2], 0.0);
        Self {
            shape: Vec4::new(shape.exit_radius, shape.length, shape.end_radius(), seed),
            bounds: Vec4::new(
                shape.bound_radius,
                shape.bound_spread,
                shape.ground.min(1e9),
                SPLASH,
            ),
            flow: Vec4::new(level, shape.expansion, shape.cell, shape.shock),
            glow: Vec4::new(
                shape.fade,
                shape.spread,
                look.luminance * level * shape.glow,
                look.soot,
            ),
            core: rgb(look.core),
            flame: rgb(look.flame),
            shock: rgb(look.shock),
            bend: Vec4::new(shape.bend.x, shape.bend.y, shape.bend_length, 0.0),
            quality: Vec4::new(steps as f32, 0.0, 0.0, 0.0),
        }
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone, Default)]
pub struct PlumeMaterial {
    #[uniform(0)]
    pub params: PlumeParams,
}

impl Material for PlumeMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/plume.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Premultiplied
    }

    fn enable_prepass() -> bool {
        false
    }

    fn enable_shadows() -> bool {
        false
    }

    fn specialize(
        _pipeline: &MaterialPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // Front faces march the plume from outside, back faces from a camera inside it.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// Light the plume casts on the vessel and the ground, lm, from its look and shape: a
/// glowing column about as wide as the nozzle and as long as its fade length.
pub fn plume_lumens(shape: &PlumeShape, look: &PlumeDef, level: f32) -> f32 {
    let diameter = 2.0 * shape.exit_radius;
    let luminance = look.luminance * level * shape.glow * diameter;
    let area = diameter * shape.fade;
    std::f32::consts::PI * luminance * area
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEA_LEVEL_ENGINE: f64 = 70_000.0;

    /// A plume from a sea-level engine's nozzle, not bent.
    fn straight(level: f64, ambient: f64, ground: f32) -> PlumeShape {
        PlumeShape::new(0.45, SEA_LEVEL_ENGINE, level, ambient, ground, Vec2::ZERO)
    }

    /// The jet's centre `a` metres from the nozzle exit, as the shader puts it.
    fn centre(shape: &PlumeShape, a: f32) -> Vec2 {
        let b = shape.bend_length;
        shape.bend * b * (1.0 - (-a / b).exp())
    }

    #[test]
    fn sea_level_plume_is_a_column_with_shock_diamonds() {
        let p = straight(1.0, ATM, f32::INFINITY);
        assert_eq!(p.expansion, 0.0);
        assert!(p.shock > 0.9, "{}", p.shock);
        assert!(p.spread < 0.15);
    }

    #[test]
    fn plume_fans_out_and_loses_its_diamonds_with_altitude() {
        let low = straight(1.0, 50_000.0, f32::INFINITY);
        let high = straight(1.0, 1_000.0, f32::INFINITY);
        let vacuum = straight(1.0, 0.0, f32::INFINITY);
        assert!(low.expansion < high.expansion && high.expansion < vacuum.expansion);
        assert!(low.spread < high.spread && high.spread < vacuum.spread);
        assert!(low.shock > high.shock);
        assert_eq!(vacuum.shock, 0.0);
        assert_eq!(vacuum.expansion, 1.0);
        // Cells stretch as the jet's pressure overtakes the air's.
        assert!(low.cell > straight(1.0, ATM, f32::INFINITY).cell);
    }

    #[test]
    fn the_ground_cuts_the_plume_and_its_bounds_make_room_for_the_splash() {
        let clear = straight(1.0, ATM, f32::INFINITY);
        let pad = straight(1.0, ATM, 5.0);
        assert_eq!(pad.length, 5.0);
        let at_ground = pad.bound_radius + pad.bound_spread * 5.0;
        assert!(at_ground > (1.0 + SPLASH) * (0.45 + 5.0 * pad.spread));
        assert!(clear.length > pad.length && clear.length.is_finite());
    }

    #[test]
    fn throttling_down_shortens_the_plume_and_tightens_the_cells() {
        let full = straight(1.0, 60_000.0, f32::INFINITY);
        let low = straight(0.4, 60_000.0, f32::INFINITY);
        assert!(low.fade < full.fade);
        assert!(low.cell < full.cell);
        assert!(low.expansion <= full.expansion);
    }

    #[test]
    fn a_gimballed_jet_leaves_along_the_nozzle_and_stays_in_its_bounds() {
        let shape =
            |bend| PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, 20_000.0, f32::INFINITY, bend);
        let bend = Vec2::new(0.09, -0.03);
        let (plain, bent) = (shape(Vec2::ZERO), shape(bend));
        // Along the nozzle at first...
        let h = 1e-3;
        assert!((centre(&bent, h) / h - bend).length() < 1e-3);
        // ...then onto the plume axis: the far end is nowhere near where the nozzle points.
        let far = centre(&bent, bent.length);
        assert!(far.length() < 0.2 * bend.length() * bent.length);
        // The bounds make room for the offset.
        let room = bent.bound_radius - plain.bound_radius;
        for k in 0..=20 {
            let a = bent.length * k as f32 / 20.0;
            assert!(centre(&bent, a).length() <= room + 1e-4);
        }
    }

    #[test]
    fn the_airflow_bends_the_plume_and_vacuum_does_not() {
        let exhaust = Vec3::NEG_Y;
        // At an angle of attack: the air comes past 10° off the axis.
        let airflow = Quat::from_rotation_z(10f32.to_radians()) * Vec3::NEG_Y * 400.0;
        let jet = 845e3 / (std::f32::consts::PI * 0.45 * 0.45);
        assert_eq!(plume_axis(exhaust, airflow, 0.0, jet), exhaust);
        assert_eq!(plume_axis(exhaust, Vec3::ZERO, 0.0, jet), exhaust);
        let angle = |q| plume_axis(exhaust, airflow, q, jet).angle_between(exhaust);
        assert!(angle(30e3) > angle(5e3) && angle(5e3) > 0.0);
        assert!(angle(30e3) < 10f32.to_radians());
        // A plume blowing into the airflow is pushed back but never turned round.
        assert!(plume_axis(exhaust, -airflow, 1e6, jet).dot(exhaust) > 0.5);
    }

    #[test]
    fn plume_axes_measure_the_nozzle_against_the_plume() {
        let axis = Vec3::new(0.3, -1.0, 0.2).normalize();
        let nozzle = Quat::from_axis_angle(axis.any_orthonormal_vector(), 0.1) * axis;
        let (rotation, bend) = plume_axes(axis, nozzle);
        assert!((rotation * Vec3::NEG_Y - axis).length() < 1e-6);
        assert!((bend.length() - 0.1f32.tan()).abs() < 1e-5);
        let along = rotation * Vec3::new(bend.x, -1.0, bend.y);
        assert!(along.normalize().dot(nozzle) > 0.99999);
    }
}
