//! Sky and sunlight for Earth-like atmospheres.
//!
//! The sky is a sphere enclosing the atmosphere drawn with `shaders/sky.wesl` (single
//! scattering). Sunlight is dimmed and reddened for the air between the sun and the
//! floating origin (the active vessel), so the vessel and everything around it is lit like
//! the ground it stands on: orange at sunset, dark at night. The terrain shader undoes that
//! and applies each fragment's own value.
//!
//! How much sunlight gets through depends only on height and the sun's angle, so it is
//! integrated once per body into a [`TransmittanceLut`] that the shaders sample
//! (`shaders/atmosphere.wesl`) instead of marching towards the sun from every point of
//! every view ray. The CPU dims the light from the same table, so the shaders divide the
//! dimming back out exactly.

use bevy::{
    asset::{RenderAssetUsages, embedded_asset},
    camera::{Exposure, visibility::NoFrustumCulling},
    image::{ImageAddressMode, ImageFilterMode, ImageSampler, ImageSamplerDescriptor},
    light::{NotShadowCaster, NotShadowReceiver, light_consts::lux},
    math::DVec3,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    platform::collections::HashMap,
    prelude::*,
    render::render_resource::{
        AsBindGroup, Extent3d, RenderPipelineDescriptor, SpecializedMeshPipelineError,
        TextureDimension, TextureFormat,
    },
    shader::ShaderRef,
};
use half::f16;
use osp_sim::body::BodyId;

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    frame::{RenderFrame, Sunlight},
    settings::{GraphicsSettings, Quality},
    sim::Sim,
    surface::Surface,
};

/// Height of the scattering atmosphere above the surface (`THICKNESS` in the shader).
const THICKNESS: f64 = 100_000.0;
/// The sky mesh's radius over the atmosphere's top (`SKY_MESH_SCALE` in the shader): enough
/// for its flat faces to clear the shell. The shaders decide what to draw by which side of
/// the mesh the camera is on, so it must match.
const SKY_MESH_SCALE: f64 = 1.002;
/// Icosphere subdivisions of the sky mesh: each edge of the icosahedron in 21 pieces, so
/// the faces dip at most about 0.05% inside its vertices.
const SKY_MESH_SUBDIVISIONS: u32 = 20;
const RAYLEIGH: [f64; 3] = [5.802e-6, 13.558e-6, 33.1e-6];
const RAYLEIGH_HEIGHT: f64 = 8000.0;
const MIE_EXTINCTION: f64 = 4.44e-6;
const MIE_HEIGHT: f64 = 1200.0;
const OZONE: [f64; 3] = [0.650e-6, 1.881e-6, 0.085e-6];
/// Sun rays grazing deeper than this below the mean surface are in the planet's shadow
/// (the shadow's soft edge spans `SHADOW_DEPTH` to `SHADOW_EDGE`), so the transmittance
/// table starts there (`LUT_DEPTH` in the shader).
const SHADOW_DEPTH: f64 = 6000.0;
const SHADOW_EDGE: f64 = 1000.0;
/// Transmittance table size: sun angle across, height down (`LUT_SIZE` in the shader).
const LUT_SIZE: (usize, usize) = (256, 64);
/// Integration steps along each ray of the table.
const LUT_STEPS: usize = 128;
/// Lower bound of the light's dimming, so the shaders can divide it back out.
const MIN_TRANSMITTANCE: f64 = 1e-4;
/// Camera exposure (EV100) in daylight and in darkness: the view adapts like an eye or an
/// auto-exposure camera, so night scenes show floodlights and city lights.
const DAY_EV: f32 = 15.0;
const NIGHT_EV: f32 = 9.0;
/// Time constant of that adaptation, s.
const ADAPTATION: f32 = 1.5;

pub struct AtmospherePlugin;

impl Plugin for AtmospherePlugin {
    fn build(&self, app: &mut App) {
        embedded_asset!(app, "shaders/sky.wesl");
        // Built here rather than at startup so terrain and skies can rely on them.
        let luts: Vec<_> = app
            .world()
            .resource::<Sim>()
            .universe
            .system
            .bodies()
            .iter()
            .filter(|body| Surface::is_earthlike(body))
            .map(|body| (body.id, TransmittanceLut::new(body.radius)))
            .collect();
        let mut images = app.world_mut().resource_mut::<Assets<Image>>();
        let luts = luts
            .into_iter()
            .map(|(id, lut)| {
                let image = images.add(lut.image());
                (id, (lut, image))
            })
            .collect();
        app.insert_resource(TransmittanceLuts(luts))
            .add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .add_systems(Startup, spawn_skies)
            .add_systems(
                Update,
                apply_effects_setting.run_if(resource_changed::<GraphicsSettings>),
            )
            .add_systems(
                Update,
                (place_skies, dim_sunlight)
                    .after(crate::frame::update_sunlight)
                    .in_set(FrameSet::Render),
            );
    }
}

/// Ray-marching steps through the air, for the sky and the haze over the ground, at each
/// `effects` setting.
pub fn view_steps(quality: Quality) -> u32 {
    match quality {
        Quality::Off | Quality::Low => 6,
        Quality::Medium => 8,
        Quality::High => 12,
        Quality::Ultra => 16,
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct SkyMaterial {
    /// x: planet mean radius, y: ray-marching steps.
    #[uniform(0)]
    params: Vec4,
    /// The body's [`TransmittanceLut`].
    #[texture(110)]
    #[sampler(111)]
    transmittance: Handle<Image>,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://osp/shaders/sky.wesl".into()
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
        // Seen from inside, only back faces exist.
        descriptor.primitive.cull_mode = None;
        Ok(())
    }
}

#[derive(Component)]
struct Sky(BodyId);

fn spawn_skies(
    mut commands: Commands,
    sim: Res<Sim>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<SkyMaterial>>,
    luts: Res<TransmittanceLuts>,
) {
    // Fine enough that its flat faces stay within `SKY_MESH_SCALE` of the sphere.
    let mesh = meshes.add(Sphere::new(1.0).mesh().ico(SKY_MESH_SUBDIVISIONS).unwrap());
    for body in sim.universe.system.bodies() {
        if !Surface::is_earthlike(body) {
            continue;
        }
        commands.spawn((
            Sky(body.id),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(SkyMaterial {
                // The steps are set by `apply_effects_setting`.
                params: Vec4::new(body.radius as f32, 0.0, 0.0, 0.0),
                transmittance: luts.texture(body.id),
            })),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
            NoFrustumCulling,
            Name::new(format!("{} sky", body.name)),
        ));
    }
}

fn apply_effects_setting(
    settings: Res<GraphicsSettings>,
    mut materials: ResMut<Assets<SkyMaterial>>,
) {
    let steps = view_steps(settings.effects) as f32;
    let stale: Vec<_> = materials
        .iter()
        .filter(|(_, m)| m.params.y != steps)
        .map(|(id, _)| id)
        .collect();
    for id in stale {
        if let Some(mut material) = materials.get_mut(id) {
            material.params.y = steps;
        }
    }
}

fn place_skies(sim: Res<Sim>, frame: Res<RenderFrame>, mut skies: Query<(&Sky, &mut Transform)>) {
    if frame.origin_body.is_none() {
        return;
    }
    for (sky, mut transform) in &mut skies {
        let body = sim.universe.system.body(sky.0);
        transform.translation = frame.to_render(sky.0, DVec3::ZERO);
        transform.scale = Vec3::splat(((body.radius + THICKNESS) * SKY_MESH_SCALE) as f32);
    }
}

fn densities(height: f64) -> [f64; 3] {
    let h = height.max(0.0);
    let ozone = (1.0 - (h - 25_000.0).abs() / 15_000.0).max(0.0);
    [(-h / RAYLEIGH_HEIGHT).exp(), (-h / MIE_HEIGHT).exp(), ozone]
}

fn smoothstep(e0: f64, e1: f64, x: f64) -> f64 {
    let t = ((x - e0) / (e1 - e0)).clamp(0.0, 1.0);
    t * t * (3.0 - 2.0 * t)
}

/// Sunlight transmittance tables for Earth-like bodies, with their GPU copies.
#[derive(Resource)]
pub struct TransmittanceLuts(HashMap<BodyId, (TransmittanceLut, Handle<Image>)>);

impl TransmittanceLuts {
    /// The body's table for a material; a placeholder for bodies without one (their
    /// shaders never sample it).
    pub fn texture(&self, body: BodyId) -> Handle<Image> {
        self.0
            .get(&body)
            .map_or_else(Handle::default, |(_, image)| image.clone())
    }

    pub fn get(&self, body: BodyId) -> Option<&TransmittanceLut> {
        self.0.get(&body).map(|(lut, _)| lut)
    }
}

/// Optical depth (per RGB channel) from a point to the top of the atmosphere, towards the
/// sun, tabulated by the point's distance from the centre `r` and the cosine `mu` of the
/// sun's angle from its zenith. Uses Bruneton's parametrisation, which packs texels where
/// the light changes fastest (near the horizon), with the shadow depth as its floor:
/// rays that pass deeper are in the planet's shadow anyway. Inside the planet the air is
/// as dense as at the surface, like the per-point march it replaces.
pub struct TransmittanceLut {
    radius: f64,
    /// Row-major, `LUT_SIZE.0` per row.
    depths: Vec<[f32; 3]>,
}

impl TransmittanceLut {
    pub fn new(radius: f64) -> Self {
        let (width, height) = LUT_SIZE;
        let (bottom, top) = (radius - SHADOW_DEPTH, radius + THICKNESS);
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
                let ds = d / LUT_STEPS as f64;
                let mut depth = [0.0; 3];
                for k in 0..LUT_STEPS {
                    let t = ds * (k as f64 + 0.5);
                    let height = (t * t + 2.0 * r * mu * t + r * r).sqrt() - radius;
                    let density = densities(height);
                    for (c, depth) in depth.iter_mut().enumerate() {
                        *depth += (RAYLEIGH[c] * density[0]
                            + MIE_EXTINCTION * density[1]
                            + OZONE[c] * density[2])
                            * ds;
                    }
                }
                depths.push(depth.map(|x| x as f32));
            }
        }
        Self { radius, depths }
    }

    /// The table as a filterable texture for the shaders.
    fn image(&self) -> Image {
        let (width, height) = LUT_SIZE;
        let data = self
            .depths
            .iter()
            .flat_map(|d| [d[0], d[1], d[2], 0.0])
            .flat_map(|x| f16::from_f32(x).to_le_bytes())
            .collect();
        let mut image = Image::new(
            Extent3d {
                width: width as u32,
                height: height as u32,
                depth_or_array_layers: 1,
            },
            TextureDimension::D2,
            data,
            TextureFormat::Rgba16Float,
            RenderAssetUsages::RENDER_WORLD,
        );
        image.sampler = ImageSampler::Descriptor(ImageSamplerDescriptor {
            address_mode_u: ImageAddressMode::ClampToEdge,
            address_mode_v: ImageAddressMode::ClampToEdge,
            mag_filter: ImageFilterMode::Linear,
            min_filter: ImageFilterMode::Linear,
            ..default()
        });
        image
    }

    /// Texture coordinates of `(r, mu)`, in texels (centres at `k + 0.5`).
    fn texel(&self, r: f64, mu: f64) -> (f64, f64) {
        let (width, height) = LUT_SIZE;
        let (bottom, top) = (self.radius - SHADOW_DEPTH, self.radius + THICKNESS);
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

    /// Bilinear lookup with clamped edges, as the GPU samples it.
    fn depth(&self, r: f64, mu: f64) -> [f64; 3] {
        let (width, height) = LUT_SIZE;
        let (u, v) = self.texel(r, mu);
        let split = |x: f64, n: usize| {
            let x = (x - 0.5).clamp(0.0, (n - 1) as f64);
            let i = (x.floor() as usize).min(n - 2);
            (i, x - i as f64)
        };
        let ((i, fu), (j, fv)) = (split(u, width), split(v, height));
        let at = |i: usize, j: usize| self.depths[j * width + i].map(f64::from);
        let (a, b, c, d) = (at(i, j), at(i + 1, j), at(i, j + 1), at(i + 1, j + 1));
        std::array::from_fn(|k| {
            let top = a[k] + (b[k] - a[k]) * fu;
            let bottom = c[k] + (d[k] - c[k]) * fu;
            top + (bottom - top) * fv
        })
    }

    /// Fraction of sunlight (per RGB channel) reaching `p` (relative to the planet centre)
    /// through the atmosphere, including the planet's shadow. Mirrors the shader.
    pub fn sun_transmittance(&self, p: DVec3, sun: DVec3) -> [f64; 3] {
        let radius = self.radius;
        let r = p.length();
        let b = p.dot(sun);
        let along = (-b).max(0.0);
        let closest = ((r - along) * (r + along)).max(0.0).sqrt();
        let shadow = smoothstep(radius - SHADOW_DEPTH, radius - SHADOW_EDGE, closest);
        if shadow <= 0.0 {
            return [0.0; 3];
        }
        let top = radius + THICKNESS;
        let (mut r, mut mu) = (r, b / r.max(1.0));
        if r > top {
            // From outside, the light only dims from where the ray enters the atmosphere.
            let c = (r - top) * (r + top);
            let disc = b * b - c;
            if disc < 0.0 || b >= 0.0 {
                return [shadow; 3];
            }
            let entry = p + sun * (-b - disc.sqrt());
            r = top;
            mu = entry.dot(sun) / top;
        }
        self.depth(r, mu).map(|x| (-x).exp() * shadow)
    }
}

/// Dim the sun for the air above the floating origin, and adapt the exposure to how much
/// light there is. The sky's own light comes from `sky_light`.
fn dim_sunlight(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    time: Res<Time>,
    mut light: Single<&mut DirectionalLight, With<Sunlight>>,
    mut exposure: Single<&mut Exposure, With<MainCamera>>,
    rig: Res<CameraRig>,
    luts: Res<TransmittanceLuts>,
) {
    let Some(origin_body) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let to_sun = frame.body_offset(system.root()).normalize();
    let origin = -frame.body_offset(origin_body);
    let transmittance = luts.get(origin_body).map_or([1.0; 3], |lut| {
        lut.sun_transmittance(origin, to_sun)
            .map(|t| t.max(MIN_TRANSMITTANCE))
    });
    let [r, g, b] = transmittance.map(|t| t as f32);
    light.color = Color::linear_rgb(r, g, b);
    light.illuminance = lux::RAW_SUNLIGHT;
    let up = origin.normalize_or(DVec3::Z);
    let daylight = smoothstep(-0.12, 0.1, up.dot(to_sun)) as f32;

    // Expose for the ground below: the sunlit disc in the map view, otherwise day or night
    // where the vessel is (in orbit over the night side, that shows the city lights).
    let scene = if rig.map_view { 1.0 } else { daylight };
    let target = NIGHT_EV + (DAY_EV - NIGHT_EV) * scene.sqrt();
    let blend = 1.0 - (-time.delta_secs() / ADAPTATION).exp();
    exposure.ev100 += (target - exposure.ev100) * blend;
}

#[cfg(test)]
mod tests {
    use std::sync::LazyLock;

    use super::*;

    const R: f64 = 6_371_000.0;

    #[test]
    fn sky_mesh_faces_clear_the_top_of_the_atmosphere() {
        use bevy::mesh::{Indices, VertexAttributeValues};
        let mesh = Sphere::new(1.0).mesh().ico(SKY_MESH_SUBDIVISIONS).unwrap();
        let Some(VertexAttributeValues::Float32x3(points)) =
            mesh.attribute(Mesh::ATTRIBUTE_POSITION)
        else {
            panic!("no positions");
        };
        let Some(Indices::U32(indices)) = mesh.indices() else {
            panic!("no indices");
        };
        // Closest any face comes to the centre: its plane's distance from it.
        let closest = indices
            .chunks(3)
            .map(|t| {
                let [a, b, c] = [0, 1, 2].map(|k| Vec3::from(points[t[k] as usize]));
                (b - a).cross(c - a).normalize().dot(a).abs()
            })
            .fold(1.0f32, f32::min) as f64;
        // Scaled to the mesh, every face stays outside the atmosphere with room to spare.
        assert!(closest * SKY_MESH_SCALE > 1.001, "{closest}");
    }
    static LUT: LazyLock<TransmittanceLut> = LazyLock::new(|| TransmittanceLut::new(R));

    fn sun_transmittance(p: DVec3, sun: DVec3) -> [f64; 3] {
        LUT.sun_transmittance(p, sun)
    }

    /// Brute-force march towards the sun from `p`, for checking the table.
    fn reference(p: DVec3, sun: DVec3) -> [f64; 3] {
        let r = p.length();
        let b = p.dot(sun);
        let along = (-b).max(0.0);
        let closest = ((r - along) * (r + along)).max(0.0).sqrt();
        let shadow = smoothstep(R - SHADOW_DEPTH, R - SHADOW_EDGE, closest);
        let top = R + THICKNESS;
        let exit = (-b + (b * b - (r - top) * (r + top)).max(0.0).sqrt()).max(0.0);
        let steps = 100_000;
        let ds = exit / steps as f64;
        let mut depth = [0.0; 3];
        for i in 0..steps {
            let d = densities((p + sun * (ds * (i as f64 + 0.5))).length() - R);
            for k in 0..3 {
                depth[k] += (RAYLEIGH[k] * d[0] + MIE_EXTINCTION * d[1] + OZONE[k] * d[2]) * ds;
            }
        }
        depth.map(|x| (-x).exp() * shadow)
    }

    #[test]
    fn table_matches_a_fine_march() {
        // Ground, mountains, flight, the stratosphere and orbit; sun high, low, setting and
        // just below the horizon (lit through the shadow's soft edge).
        for height in [0.0, 3_000.0, 12_000.0, 40_000.0, 400_000.0] {
            for elevation in [80.0f64, 30.0, 10.0, 3.0, 1.0, 0.0, -0.5, -1.5] {
                let p = DVec3::new(0.0, 0.0, R + height);
                let e = elevation.to_radians();
                let sun = DVec3::new(e.cos(), 0.0, e.sin());
                let (table, exact) = (sun_transmittance(p, sun), reference(p, sun));
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
        let t = sun_transmittance(p, DVec3::Z);
        assert!(t[0] > 0.9 && t[2] > 0.6 && t[0] > t[2], "{t:?}");
    }

    #[test]
    fn low_sun_is_red_and_set_sun_is_dark() {
        let p = DVec3::new(0.0, 0.0, R + 10.0);
        let low = sun_transmittance(p, DVec3::new(1.0, 0.0, 0.03).normalize());
        assert!(low[0] > 3.0 * low[2], "{low:?}");
        let set = sun_transmittance(p, DVec3::new(1.0, 0.0, -0.1).normalize());
        assert_eq!(set, [0.0; 3]);
    }

    #[test]
    fn space_is_unaffected_unless_in_shadow() {
        let lit = sun_transmittance(DVec3::new(0.0, 0.0, R + 400e3), DVec3::X);
        assert!(lit.iter().all(|&t| t > 0.999), "{lit:?}");
        let shadow = sun_transmittance(DVec3::new(-(R + 400e3), 0.0, 0.0), DVec3::X);
        assert_eq!(shadow, [0.0; 3]);
        // Far out, sunlight grazing the atmosphere on its way is reddened.
        let grazing = DVec3::new(-2.0 * R, 0.0, R + 20e3);
        let t = sun_transmittance(grazing, DVec3::X);
        assert!(t[2] < 0.5 && t[0] > t[2], "{t:?}");
    }
}
