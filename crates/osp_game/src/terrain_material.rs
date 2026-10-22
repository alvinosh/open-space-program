//! The planet surface material: standard PBR extended with per-chunk data textures
//! (`shaders/terrain.wesl`).

use bevy::{
    asset::embedded_asset,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::{ShaderRef, load_shader_library},
};

use crate::{atmosphere::view_steps, settings::GraphicsSettings};

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExtension>;

pub struct TerrainMaterialPlugin;

impl Plugin for TerrainMaterialPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "shaders/atmosphere.wesl");
        embedded_asset!(app, "shaders/terrain.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default())
            .add_systems(
                Update,
                apply_effects_setting.run_if(resource_changed::<GraphicsSettings>),
            );
    }
}

/// The haze's steps on every chunk (new chunks get them when they spawn).
fn apply_effects_setting(
    settings: Res<GraphicsSettings>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let steps = view_steps(settings.effects) as f32;
    // Only the ones that differ: `iter_mut` would mark every chunk's material modified.
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.params.quality.x != steps)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut material) = materials.get_mut(id) {
            material.extension.params.quality.x = steps;
        }
    }
}

#[derive(Debug, Clone, Copy, Default, ShaderType, Reflect)]
pub struct TerrainParams {
    /// xyz: planet centre in chunk-local coordinates; w: mean radius.
    pub planet: Vec4,
    /// xyz: chunk centre in body-fixed axes modulo `planet::DETAIL_PERIOD`.
    pub detail_origin: Vec4,
    /// x: chunk level, y: colour data level, z: cover data level, w: 1 for an Earth-like
    /// atmosphere.
    pub levels: Vec4,
    /// x: ray-marching steps for the haze (`atmosphere::view_steps`).
    pub quality: Vec4,
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExtension {
    #[uniform(100)]
    pub params: TerrainParams,
    /// rgb: sRGB imagery, a: night lights.
    #[texture(101)]
    #[sampler(102)]
    pub color: Handle<Image>,
    /// Normal (east, north), elevation (m), water fraction.
    #[texture(103)]
    #[sampler(104)]
    pub surface: Handle<Image>,
    /// Land-cover class per sample.
    #[texture(105, sample_type = "u_int")]
    pub cover: Handle<Image>,
    /// The body's sunlight transmittance (`atmosphere::TransmittanceLut`).
    #[texture(110)]
    #[sampler(111)]
    pub transmittance: Handle<Image>,
}

impl MaterialExtension for TerrainExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/terrain.wesl".into()
    }
}
