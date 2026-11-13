//! The planet surface material: standard PBR extended with per-chunk data textures
//! (`shaders/terrain.wesl`). One per body and page of chunk layers (see `terrain_layers`), so a
//! body's chunks are drawn together.

use bevy::{
    asset::embedded_asset,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{ExtendedMaterial, MaterialExtension, MaterialExtensionKey, MaterialExtensionPipeline},
    prelude::*,
    render::{
        render_resource::{AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError},
        storage::ShaderBuffer,
    },
    shader::{ShaderDefVal, ShaderRef, load_shader_library},
};

use crate::{
    atmosphere::{AirUniform, view_steps},
    settings::{GraphicsSettings, Quality},
};

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

/// The reflected sky's steps and whether clouds shade the ground, on every chunk's material (new
/// materials get them when they are made).
fn apply_effects_setting(
    settings: Res<GraphicsSettings>,
    mut materials: ResMut<Assets<TerrainMaterial>>,
) {
    let steps = view_steps(settings.effects) as f32;
    let clouds = settings.clouds != Quality::Off;
    // Only the ones that differ: `iter_mut` would mark every chunk's material modified.
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.extension.quality.x != steps || (m.extension.clouds.z > 0.0) != clouds)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut material) = materials.get_mut(id) {
            material.extension.quality.x = steps;
            material.extension.clouds.z = f32::from(u8::from(clouds));
            material.extension.clouds.y = material.extension.clouds.z * material.extension.clouds.w;
        }
    }
}

#[derive(Asset, AsBindGroup, Reflect, Debug, Clone)]
pub struct TerrainExtension {
    /// x: ray-marching steps for the sky reflected in water (`atmosphere::view_steps`).
    #[uniform(100)]
    pub quality: Vec4,
    /// sRGB imagery, a layer per chunk (`terrain_layers`), as are the next three.
    #[texture(101, dimension = "2d_array")]
    #[sampler(102)]
    pub color: Handle<Image>,
    /// Normal (east, north), elevation (m), water fraction.
    #[texture(103, dimension = "2d_array")]
    #[sampler(104)]
    pub surface: Handle<Image>,
    /// Land-cover class per sample.
    #[texture(105, dimension = "2d_array", sample_type = "u_int")]
    pub cover: Handle<Image>,
    /// Night lights, finer than the rest, as they are all that shows at night (`planet`).
    #[texture(106, dimension = "2d_array")]
    #[sampler(107)]
    pub lights: Handle<Image>,
    /// Every chunk's `terrain_layers::ChunkData`, at its mesh's tag.
    #[storage(108, read_only)]
    pub chunks: Handle<ShaderBuffer>,
    /// The body's air (`atmosphere::BodyAir`, as `shaders/atmosphere.wesl` binds it).
    #[texture(110)]
    #[sampler(111)]
    pub transmittance: Handle<Image>,
    #[texture(112)]
    pub multiple: Handle<Image>,
    #[texture(113)]
    pub irradiance: Handle<Image>,
    #[uniform(114)]
    pub air: AirUniform,
    /// The body's weather and where its clouds are (`atmosphere::BodyAir`).
    #[texture(115, dimension = "cube")]
    pub weather: Handle<Image>,
    #[uniform(116)]
    pub clouds: Vec4,
    /// The waves' slopes and foam (`ocean`), shared by every chunk.
    #[texture(120, dimension = "2d_array")]
    #[sampler(121)]
    pub waves: Handle<Image>,
}

impl MaterialExtension for TerrainExtension {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp_render/shaders/terrain.wesl".into()
    }

    fn specialize(
        _pipeline: &MaterialExtensionPipeline,
        descriptor: &mut RenderPipelineDescriptor,
        _layout: &MeshVertexBufferLayoutRef,
        _key: MaterialExtensionKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        // The camera's environment map is the sky as seen from the vessel (`sky_light`);
        // the terrain reaches far beyond it and works out its own skylight.
        if let Some(fragment) = &mut descriptor.fragment {
            fragment.shader_defs.retain(
                |def| !matches!(def, ShaderDefVal::Bool(name, _) if name == "ENVIRONMENT_MAP"),
            );
        }
        Ok(())
    }
}
