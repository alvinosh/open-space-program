//! The clouds right around the camera, as EnvironmentalVisualEnhancements draws them: a grid of big
//! soft sprites at the height of the sheet (`shaders/cloud_puffs.wesl`), which you fly through. Beyond
//! them the sheet of `crate::clouds` takes over. The grid stays around the camera in whole steps of a
//! lattice fixed on the planet, so a puff stays where it is as you go by.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::visibility::NoFrustumCulling,
    image::ImageAddressMode,
    light::{NotShadowCaster, NotShadowReceiver},
    math::{DQuat, DVec3},
    mesh::{Indices, MeshVertexBufferLayoutRef, PrimitiveTopology},
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, ShaderType, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};

use crate::{
    FrameSet,
    atmosphere::{BodyAirs, map_turns},
    camera::CameraRig,
    clouds::{nearest_cloudy_body, picture},
    content::GameContent,
    frame::RenderFrame,
    settings::{GraphicsSettings, Quality},
    sim::Sim,
};

/// The size of a puff and how many times bigger the biggest is: its `size = 2000,2.2`.
const PUFF_SIZE: f32 = 2000.0;
const BIGGEST_PUFF: f32 = 2.2;
/// How far a puff strays from its place on the lattice, m: more than its `maxTranslation = 100,200,100`
/// sideways, where the lattice would show from below.
const STRAYS: Vec2 = Vec2::new(450.0, 200.0);
/// Its `_DistFade` and `_DistFadeVert`, and `_InvFade`: fading out to about 11.7 km from the camera,
/// and softly where it meets the scene.
const FADE: Vec3 = Vec3::new(1.0, 0.000085, 0.008);
/// The camera must be nearer the sheet than this for there to be any puffs in sight, m.
const IN_SIGHT: f64 = 13_000.0;
/// How often the detail tiles over the puffs, m: the sheet's big copy.
const DETAIL_TILE: f64 = 40_000.0;

/// How far the grid reaches and the distance between puffs, m. At the top, its `area = 18000,4`.
fn grid_size(quality: Quality) -> (f32, f32) {
    match quality {
        Quality::Off | Quality::Low => (14_000.0, 1_800.0),
        Quality::Medium => (16_000.0, 1_400.0),
        Quality::High => (18_000.0, 1_125.0),
        Quality::Ultra => (18_000.0, 1_000.0),
    }
}

pub struct CloudPuffsPlugin;

impl Plugin for CloudPuffsPlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/cloud_puffs.wesl");
        let detail = picture(
            &app.world().resource::<GameContent>().0,
            "clouds/detail.png",
            ImageAddressMode::Repeat,
        );
        let sprite = picture(
            &app.world().resource::<GameContent>().0,
            "clouds/puff.png",
            ImageAddressMode::ClampToEdge,
        );
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let textures = PuffTextures {
            detail: images.add(detail.unwrap_or_default()),
            sprite: images.add(sprite.unwrap_or_default()),
        };
        app.insert_resource(textures)
            .init_resource::<Puffs>()
            .add_plugins(MaterialPlugin::<PuffMaterial>::default())
            .add_systems(
                Update,
                place_puffs
                    .after(crate::frame::update_sunlight)
                    .in_set(FrameSet::Render),
            );
    }
}

#[derive(Resource)]
struct PuffTextures {
    detail: Handle<Image>,
    sprite: Handle<Image>,
}

#[derive(Debug, Clone, Copy, Default, ShaderType)]
struct PuffParams {
    origin: Vec4,
    grid_x: Vec4,
    grid_y: Vec4,
    up: Vec4,
    cell: Vec4,
    spread: Vec4,
    sun: Vec4,
    sunlight: Vec4,
    skylight: Vec4,
    sun_height: Vec4,
    fade: Vec4,
    map_from_render: Mat3,
    copy_from_map: Mat3,
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
struct PuffMaterial {
    #[uniform(0)]
    params: PuffParams,
    #[texture(1, dimension = "cube")]
    #[sampler(2)]
    map: Handle<Image>,
    #[texture(3)]
    #[sampler(4)]
    detail: Handle<Image>,
    #[texture(5)]
    #[sampler(6)]
    sprite: Handle<Image>,
}

impl Material for PuffMaterial {
    fn vertex_shader() -> ShaderRef {
        "embedded://osp/shaders/cloud_puffs.wesl".into()
    }

    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/cloud_puffs.wesl".into()
    }

    fn alpha_mode(&self) -> AlphaMode {
        AlphaMode::Blend
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
        layout: &MeshVertexBufferLayoutRef,
        _key: MaterialPipelineKey<Self>,
    ) -> Result<(), SpecializedMeshPipelineError> {
        descriptor.vertex.buffers = vec![layout.0.get_layout(&[
            Mesh::ATTRIBUTE_POSITION.at_shader_location(0),
            Mesh::ATTRIBUTE_UV_0.at_shader_location(1),
        ])?];
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

/// A quad for each puff of a hexagonal grid: its place on the lattice from the middle, as its position.
fn grid_mesh(reach: f32, spacing: f32) -> Mesh {
    let cells = (reach / spacing).ceil() as i32 + 1;
    let (mut positions, mut uvs, mut indices) = (Vec::new(), Vec::new(), Vec::new());
    for j in -cells..=cells {
        for i in -2 * cells..=2 * cells {
            let at = Vec2::new(i as f32 + j as f32 * 0.5, j as f32 * 0.8660254) * spacing;
            if at.length() > reach {
                continue;
            }
            let base = positions.len() as u32;
            for corner in [[0.0, 0.0], [1.0, 0.0], [1.0, 1.0], [0.0, 1.0]] {
                positions.push([i as f32, j as f32, 0.0]);
                uvs.push(corner);
            }
            indices.extend([base, base + 1, base + 2, base, base + 2, base + 3]);
        }
    }
    Mesh::new(
        PrimitiveTopology::TriangleList,
        RenderAssetUsages::RENDER_WORLD,
    )
    .with_inserted_attribute(Mesh::ATTRIBUTE_POSITION, positions)
    .with_inserted_attribute(Mesh::ATTRIBUTE_UV_0, uvs)
    .with_inserted_indices(Indices::U32(indices))
}

/// The drawn grid, and the quality it was built for.
#[derive(Resource, Default)]
struct Puffs(Option<(Entity, Handle<PuffMaterial>, Handle<Mesh>, Quality)>);

/// Where the grid goes for a camera over `over` (a unit vector in the body's fixed axes), on a sheet
/// of this radius: the lattice's nearest point to it, which face of the cube that is on, and the
/// lattice's axes there.
struct GridPlace {
    cell: (f64, f64),
    face: usize,
    centre: DVec3,
    axes: [DVec3; 2],
}

fn grid_place(over: DVec3, radius: f64, spacing: f64) -> GridPlace {
    let a = over.abs();
    let (face, axis, x, y) = if a.x >= a.y && a.x >= a.z {
        (0, DVec3::X * over.x.signum(), DVec3::Y, DVec3::Z)
    } else if a.y >= a.z {
        (1, DVec3::Y * over.y.signum(), DVec3::X, DVec3::Z)
    } else {
        (2, DVec3::Z * over.z.signum(), DVec3::X, DVec3::Y)
    };
    let along = over.dot(axis);
    let row_height = spacing * 0.8660254;
    let (u, v) = (
        over.dot(x).atan2(along) * radius,
        over.dot(y).atan2(along) * radius,
    );
    let j = (v / row_height).round();
    let i = (u / spacing - j * 0.5).round();
    let (cu, cv) = ((i + j * 0.5) * spacing, j * row_height);
    let centre = (axis + x * (cu / radius).tan() + y * (cv / radius).tan()).normalize();
    let grid_x = (x - centre * x.dot(centre)).normalize();
    let grid_y = centre.cross(grid_x);
    // The same way round as `y`, whichever hand the face's axes are.
    let grid_y = if grid_y.dot(y) >= 0.0 {
        grid_y
    } else {
        -grid_y
    };
    GridPlace {
        cell: (i, j),
        face: face * 2 + (over.dot(axis.abs()) >= 0.0) as usize,
        centre,
        axes: [grid_x, grid_y],
    }
}

fn place_puffs(
    mut commands: Commands,
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    rig: Res<CameraRig>,
    settings: Res<GraphicsSettings>,
    airs: Res<BodyAirs>,
    textures: Res<PuffTextures>,
    mut puffs: ResMut<Puffs>,
    mut materials: ResMut<Assets<PuffMaterial>>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut transforms: Query<&mut Transform>,
) {
    let (reach, spacing) = grid_size(settings.clouds);
    let hide = |puffs: &Puffs, materials: &mut Assets<PuffMaterial>| {
        if let Some((_, material, ..)) = &puffs.0
            && let Some(mut material) = materials.get_mut(material)
        {
            material.params.fade.w = 0.0;
        }
    };
    let on = frame.origin_body.is_some() && settings.clouds != Quality::Off && !rig.map_view;
    let found = on
        .then(|| nearest_cloudy_body(&sim, &frame, &rig, &airs))
        .flatten();
    let Some(id) = found else {
        hide(&puffs, &mut materials);
        return;
    };
    let Some(air) = airs.get(id) else {
        return;
    };
    let Some(clouds) = air.clouds.as_ref() else {
        return;
    };
    let system = &sim.universe.system;
    let body = system.body(id);
    let radius = body.radius + clouds.def.shadow_height();
    let eye = rig.position - frame.body_offset(id);
    if (eye.length() - radius).abs() > IN_SIGHT {
        hide(&puffs, &mut materials);
        return;
    }

    // Build it, or build it again for another quality.
    let stale = puffs
        .0
        .as_ref()
        .is_none_or(|(.., quality)| *quality != settings.clouds);
    if stale {
        let mesh = grid_mesh(reach, spacing);
        match &mut puffs.0 {
            Some((_, _, handle, quality)) => {
                if let Some(mut old) = meshes.get_mut(handle) {
                    *old = mesh;
                }
                *quality = settings.clouds;
            }
            None => {
                let mesh = meshes.add(mesh);
                let material = materials.add(PuffMaterial {
                    params: PuffParams::default(),
                    map: air.cloud_map.clone(),
                    detail: textures.detail.clone(),
                    sprite: textures.sprite.clone(),
                });
                let entity = commands
                    .spawn((
                        Mesh3d(mesh.clone()),
                        MeshMaterial3d(material.clone()),
                        Transform::default(),
                        NotShadowCaster,
                        NotShadowReceiver,
                        // The shader places them, around the camera.
                        NoFrustumCulling,
                        Name::new("Cloud puffs"),
                    ))
                    .id();
                puffs.0 = Some((entity, material, mesh, settings.clouds));
            }
        }
    }
    let Some((entity, material, ..)) = &puffs.0 else {
        return;
    };
    if let Ok(mut transform) = transforms.get_mut(*entity) {
        *transform = Transform::default();
    }

    // The sim's axes to the body's fixed axes as the renderer turns them (Y to the pole), and back.
    let basis = DQuat::from_rotation_x(-std::f64::consts::FRAC_PI_2);
    let to_fixed = basis * body.orientation(frame.time).inverse();
    let place = grid_place((to_fixed * eye).normalize(), radius, spacing as f64);
    let from_fixed = to_fixed.inverse();
    let centre = from_fixed * place.centre;
    let to_render_axes = |v: DVec3| Vec3::new(v.x as f32, v.z as f32, -v.y as f32);
    let origin = frame.to_render(id, centre * radius);
    let star = frame.body_offset(system.root());
    let to_sun = (star - frame.body_offset(id)).normalize_or(DVec3::Z);
    let at = centre * radius;
    let (reached, sky) = (
        air.air.sun_transmittance(at, to_sun),
        air.air.sky_irradiance(at, to_sun),
    );
    let light = bevy::light::light_consts::lux::RAW_SUNLIGHT;
    let lit = |c: [f64; 3]| {
        Vec4::new(
            c[0] as f32 * light,
            c[1] as f32 * light,
            c[2] as f32 * light,
            1.0,
        )
    };
    // Render axes to the map's: render to sim, sim to fixed, fixed to the map's.
    let (map_from_local, copy_from_map) = map_turns(&clouds.def);
    let map_from_render = map_from_local
        * Mat3::from_quat(crate::frame::rotation_to_render(
            body.orientation(frame.time).inverse(),
        ));
    if let Some(mut material) = materials.get_mut(material) {
        material.params = PuffParams {
            origin: origin.extend(spacing),
            grid_x: to_render_axes(from_fixed * place.axes[0]).extend(0.0),
            grid_y: to_render_axes(from_fixed * place.axes[1]).extend(0.0),
            up: to_render_axes(centre).extend(radius as f32),
            cell: Vec4::new(
                place.cell.0 as f32,
                place.cell.1 as f32,
                place.face as f32,
                PUFF_SIZE,
            ),
            spread: Vec4::new(
                BIGGEST_PUFF,
                STRAYS.x,
                STRAYS.y,
                (DETAIL_TILE / spacing as f64) as f32,
            ),
            sun: crate::frame::to_render(to_sun).extend(if clouds.def.map.is_some() {
                1.0
            } else {
                0.0
            }),
            sunlight: lit(reached),
            skylight: lit(sky),
            sun_height: Vec4::new(centre.dot(to_sun).max(0.0) as f32, 0.0, 0.0, 0.0),
            fade: FADE.extend(1.0),
            map_from_render,
            copy_from_map,
        };
    }
}
