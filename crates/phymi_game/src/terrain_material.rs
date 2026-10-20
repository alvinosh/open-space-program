//! The planet surface material: standard PBR extended with per-chunk data textures
//! (`shaders/terrain.wesl`).

use bevy::{
    asset::embedded_asset,
    pbr::{ExtendedMaterial, MaterialExtension},
    prelude::*,
    render::render_resource::{AsBindGroup, ShaderType},
    shader::{ShaderRef, load_shader_library},
};

pub type TerrainMaterial = ExtendedMaterial<StandardMaterial, TerrainExtension>;

pub struct TerrainMaterialPlugin;

impl Plugin for TerrainMaterialPlugin {
    fn build(&self, app: &mut App) {
        load_shader_library!(app, "shaders/atmosphere.wesl");
        embedded_asset!(app, "shaders/terrain.wesl");
        app.add_plugins(MaterialPlugin::<TerrainMaterial>::default());
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
}

impl MaterialExtension for TerrainExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://phymi/shaders/terrain.wesl".into()
    }
}
