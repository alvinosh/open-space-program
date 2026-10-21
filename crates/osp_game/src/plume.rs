//! Engine exhaust plumes: a volume per engine, ray-marched in `shaders/plume.wesl`.
//!
//! The look comes from a `plumes` record (colours, brightness, soot); the shape from the
//! engine's state: its thrust level (a start grows the plume out of the nozzle, throttling
//! shortens and dims it) and its exit pressure against the ambient pressure. A jet that
//! leaves the nozzle at or below the ambient pressure stays a narrow column with shock
//! diamonds; as the air thins it overtakes the ambient pressure, the cells stretch and the
//! plume fans out and fades, until in vacuum it is a wide, faint cone.

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
    /// Bounding cone: length, radius at the nozzle and growth per metre.
    pub length: f32,
    pub bound_radius: f32,
    pub bound_spread: f32,
}

/// How much wider the jet gets where it splashes on the ground.
const SPLASH: f32 = 2.5;

impl PlumeShape {
    /// `exit_pressure` is the engine's at full thrust, `ambient` the air's, both Pa;
    /// `level` the thrust share; `ground` how far the ground is from the nozzle exit along
    /// the axis, m (infinite when clear).
    pub fn new(
        exit_radius: f32,
        exit_pressure: f64,
        level: f64,
        ambient: f64,
        ground: f32,
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
        Self {
            exit_radius,
            expansion,
            cell,
            shock,
            fade,
            spread,
            glow: 1.0 - 0.85 * expansion,
            ground,
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
}

impl PlumeParams {
    pub fn new(shape: &PlumeShape, look: &PlumeDef, level: f32, seed: f32) -> Self {
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

    #[test]
    fn sea_level_plume_is_a_column_with_shock_diamonds() {
        let p = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, ATM, f32::INFINITY);
        assert_eq!(p.expansion, 0.0);
        assert!(p.shock > 0.9, "{}", p.shock);
        assert!(p.spread < 0.15);
    }

    #[test]
    fn plume_fans_out_and_loses_its_diamonds_with_altitude() {
        let low = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, 50_000.0, f32::INFINITY);
        let high = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, 1_000.0, f32::INFINITY);
        let vacuum = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, 0.0, f32::INFINITY);
        assert!(low.expansion < high.expansion && high.expansion < vacuum.expansion);
        assert!(low.spread < high.spread && high.spread < vacuum.spread);
        assert!(low.shock > high.shock);
        assert_eq!(vacuum.shock, 0.0);
        assert_eq!(vacuum.expansion, 1.0);
        // Cells stretch as the jet's pressure overtakes the air's.
        assert!(low.cell > PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, ATM, f32::INFINITY).cell);
    }

    #[test]
    fn the_ground_cuts_the_plume_and_its_bounds_make_room_for_the_splash() {
        let clear = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, ATM, f32::INFINITY);
        let pad = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, ATM, 5.0);
        assert_eq!(pad.length, 5.0);
        let at_ground = pad.bound_radius + pad.bound_spread * 5.0;
        assert!(at_ground > (1.0 + SPLASH) * (0.45 + 5.0 * pad.spread));
        assert!(clear.length > pad.length && clear.length.is_finite());
    }

    #[test]
    fn throttling_down_shortens_the_plume_and_tightens_the_cells() {
        let full = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 1.0, 60_000.0, f32::INFINITY);
        let low = PlumeShape::new(0.45, SEA_LEVEL_ENGINE, 0.4, 60_000.0, f32::INFINITY);
        assert!(low.fade < full.fade);
        assert!(low.cell < full.cell);
        assert!(low.expansion <= full.expansion);
    }
}
