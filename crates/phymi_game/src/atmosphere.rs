//! Sky and sunlight for Earth-like atmospheres.
//!
//! The sky is a sphere enclosing the atmosphere drawn with `shaders/sky.wesl` (single
//! scattering). Sunlight is dimmed and reddened for the air between the sun and the
//! floating origin (the active vessel), so the vessel and everything around it is lit like
//! the ground it stands on: orange at sunset, dark at night. The terrain shader undoes that
//! and applies each fragment's own value, using [`sun_transmittance`]'s twin in
//! `shaders/atmosphere.wesl`.

use bevy::{
    asset::embedded_asset,
    camera::{Exposure, visibility::NoFrustumCulling},
    light::{NotShadowCaster, NotShadowReceiver, light_consts::lux},
    math::DVec3,
    mesh::MeshVertexBufferLayoutRef,
    pbr::{MaterialPipeline, MaterialPipelineKey},
    prelude::*,
    render::render_resource::{
        AsBindGroup, RenderPipelineDescriptor, SpecializedMeshPipelineError,
    },
    shader::ShaderRef,
};
use phymi_sim::body::BodyId;

use crate::{
    FrameSet,
    camera::{CameraRig, MainCamera},
    frame::{RenderFrame, Sunlight},
    sim::Sim,
    surface::Surface,
};

/// Height of the scattering atmosphere above the surface (`THICKNESS` in the shader).
const THICKNESS: f64 = 100_000.0;
const RAYLEIGH: [f64; 3] = [5.802e-6, 13.558e-6, 33.1e-6];
const RAYLEIGH_HEIGHT: f64 = 8000.0;
const MIE_EXTINCTION: f64 = 4.44e-6;
const MIE_HEIGHT: f64 = 1200.0;
const OZONE: [f64; 3] = [0.650e-6, 1.881e-6, 0.085e-6];
const SUN_STEPS: usize = 6;
/// Lower bound of the light's dimming, so the shaders can divide it back out.
const MIN_TRANSMITTANCE: f64 = 1e-4;
/// Ambient light in full daylight (and in sunlit space).
const AMBIENT: f32 = 1_500.0;
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
        app.add_plugins(MaterialPlugin::<SkyMaterial>::default())
            .add_systems(Startup, spawn_skies)
            .add_systems(
                Update,
                (place_skies, dim_sunlight)
                    .after(crate::frame::update_sunlight)
                    .in_set(FrameSet::Render),
            );
    }
}

#[derive(Asset, AsBindGroup, TypePath, Debug, Clone)]
pub struct SkyMaterial {
    /// x: planet mean radius.
    #[uniform(0)]
    params: Vec4,
}

impl Material for SkyMaterial {
    fn fragment_shader() -> ShaderRef {
        "embedded://phymi/shaders/sky.wesl".into()
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
) {
    let mesh = meshes.add(Sphere::new(1.0).mesh().ico(5).unwrap());
    for body in sim.universe.system.bodies() {
        if !Surface::is_earthlike(body) {
            continue;
        }
        commands.spawn((
            Sky(body.id),
            Mesh3d(mesh.clone()),
            MeshMaterial3d(materials.add(SkyMaterial {
                params: Vec4::new(body.radius as f32, 0.0, 0.0, 0.0),
            })),
            Transform::default(),
            NotShadowCaster,
            NotShadowReceiver,
            NoFrustumCulling,
            Name::new(format!("{} sky", body.name)),
        ));
    }
}

fn place_skies(sim: Res<Sim>, frame: Res<RenderFrame>, mut skies: Query<(&Sky, &mut Transform)>) {
    if frame.origin_body.is_none() {
        return;
    }
    for (sky, mut transform) in &mut skies {
        let body = sim.universe.system.body(sky.0);
        transform.translation = frame.to_render(sky.0, DVec3::ZERO);
        // The icosphere's faces dip inside the unit sphere; leave margin over the shell.
        transform.scale = Vec3::splat(((body.radius + THICKNESS) * 1.02) as f32);
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

/// Fraction of sunlight (per RGB channel) reaching `p` (relative to the planet centre)
/// through the atmosphere, including the planet's shadow. Mirrors the shader.
pub fn sun_transmittance(p: DVec3, sun: DVec3, radius: f64) -> [f64; 3] {
    let r = p.length();
    let b = p.dot(sun);
    let along = (-b).max(0.0);
    let closest = ((r - along) * (r + along)).max(0.0).sqrt();
    let shadow = smoothstep(radius - 6000.0, radius - 1000.0, closest);
    if shadow <= 0.0 {
        return [0.0; 3];
    }
    let top = radius + THICKNESS;
    let c = (r - top) * (r + top);
    let exit = (-b + (b * b - c).max(0.0).sqrt()).max(0.0);
    let ds = exit / SUN_STEPS as f64;
    let mut depth = [0.0; 3];
    for i in 0..SUN_STEPS {
        let q = p + sun * (ds * (i as f64 + 0.5));
        let d = densities(q.length() - radius);
        for k in 0..3 {
            depth[k] += (RAYLEIGH[k] * d[0] + MIE_EXTINCTION * d[1] + OZONE[k] * d[2]) * ds;
        }
    }
    depth.map(|x| (-x).exp() * shadow)
}

/// Dim the sun (and the ambient light) for the air above the floating origin, and adapt
/// the exposure to how much light there is.
fn dim_sunlight(
    sim: Res<Sim>,
    frame: Res<RenderFrame>,
    time: Res<Time>,
    mut light: Single<&mut DirectionalLight, With<Sunlight>>,
    mut exposure: Single<&mut Exposure, With<MainCamera>>,
    rig: Res<CameraRig>,
    mut ambient: ResMut<GlobalAmbientLight>,
) {
    let Some(origin_body) = frame.origin_body else {
        return;
    };
    let system = &sim.universe.system;
    let body = system.body(origin_body);
    let to_sun = frame.body_offset(system.root()).normalize();
    let origin = -frame.body_offset(origin_body);
    let transmittance = if Surface::is_earthlike(body) {
        sun_transmittance(origin, to_sun, body.radius).map(|t| t.max(MIN_TRANSMITTANCE))
    } else {
        [1.0; 3]
    };
    let [r, g, b] = transmittance.map(|t| t as f32);
    light.color = Color::linear_rgb(r, g, b);
    light.illuminance = lux::RAW_SUNLIGHT;
    // Skylight fades through twilight; in space only direct sunlight lights things.
    let up = origin.normalize_or(DVec3::Z);
    let daylight = smoothstep(-0.12, 0.1, up.dot(to_sun)) as f32;
    let lit = (0.2126 * r + 0.7152 * g + 0.0722 * b).clamp(0.0, 1.0);
    let level = if Surface::is_earthlike(body) && origin.length() < body.radius + THICKNESS {
        daylight
    } else {
        lit
    };
    ambient.brightness = AMBIENT * (0.008 + 0.992 * level);

    // Expose for the ground below: the sunlit disc in the map view, otherwise day or night
    // where the vessel is (in orbit over the night side, that shows the city lights).
    let scene = if rig.map_view { 1.0 } else { daylight };
    let target = NIGHT_EV + (DAY_EV - NIGHT_EV) * scene.sqrt();
    let blend = 1.0 - (-time.delta_secs() / ADAPTATION).exp();
    exposure.ev100 += (target - exposure.ev100) * blend;
}

#[cfg(test)]
mod tests {
    use super::*;

    const R: f64 = 6_371_000.0;

    #[test]
    fn overhead_sun_is_bright_and_slightly_yellow() {
        let p = DVec3::new(0.0, 0.0, R + 10.0);
        let t = sun_transmittance(p, DVec3::Z, R);
        assert!(t[0] > 0.9 && t[2] > 0.6 && t[0] > t[2], "{t:?}");
    }

    #[test]
    fn low_sun_is_red_and_set_sun_is_dark() {
        let p = DVec3::new(0.0, 0.0, R + 10.0);
        let low = sun_transmittance(p, DVec3::new(1.0, 0.0, 0.03).normalize(), R);
        assert!(low[0] > 3.0 * low[2], "{low:?}");
        let set = sun_transmittance(p, DVec3::new(1.0, 0.0, -0.1).normalize(), R);
        assert_eq!(set, [0.0; 3]);
    }

    #[test]
    fn space_is_unaffected_unless_in_shadow() {
        let lit = sun_transmittance(DVec3::new(0.0, 0.0, R + 400e3), DVec3::X, R);
        assert!(lit.iter().all(|&t| t > 0.999), "{lit:?}");
        let shadow = sun_transmittance(DVec3::new(-(R + 400e3), 0.0, 0.0), DVec3::X, R);
        assert_eq!(shadow, [0.0; 3]);
    }
}
