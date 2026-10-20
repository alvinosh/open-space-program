//! Launch-site structures: a small pad built around every site that has one.
//!
//! The deck, its embankment and the roads are terrain (see `phymi_sim::terrain`). This
//! adds what stands on them: the launch mount and flame-trench grate under the vehicle, a
//! lattice service tower with a crew access arm and lightning mast, floodlights, a service
//! building, and off the deck a water tower and propellant tanks with their feed line.
//! Everything is generated from simple shapes in the site frame (`x` right, `y` up, `-z`
//! forward along the site heading) and merged into two meshes.

use bevy::{math::DVec3, prelude::*};
use phymi_sim::{
    data::LaunchSiteDef,
    terrain::{site_axes, site_point},
};

use crate::{
    frame::to_render,
    meshgen::MeshBuilder,
    planet::SurfaceAnchor,
    sim::{Sim, WorldData},
};

pub struct LaunchSitePlugin;

impl Plugin for LaunchSitePlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Startup, spawn_sites);
    }
}

fn rgb(r: f32, g: f32, b: f32) -> LinearRgba {
    LinearRgba::rgb(r, g, b)
}

const TOWER_RED: [f32; 3] = [0.30, 0.055, 0.03];
const STEEL: [f32; 3] = [0.09, 0.09, 0.095];
const GREY: [f32; 3] = [0.28, 0.28, 0.27];
const CONCRETE: [f32; 3] = [0.40, 0.39, 0.36];
const WHITE: [f32; 3] = [0.75, 0.75, 0.73];
const GRATE: [f32; 3] = [0.025, 0.025, 0.025];

fn c(color: [f32; 3]) -> LinearRgba {
    rgb(color[0], color[1], color[2])
}

/// Builds one site's structures in the site frame.
struct Site<'a> {
    def: &'a LaunchSiteDef,
    radius: f64,
    deck: f64,
    ground: &'a dyn Fn(DVec3) -> f64,
    painted: MeshBuilder,
    lamps: MeshBuilder,
    /// Floodlight positions (site frame), aimed at the vehicle.
    floodlights: Vec<Vec3>,
}

impl Site<'_> {
    /// Site-frame position of a point `right`, `forward` from the centre at `height`
    /// above the deck.
    fn at(right: f32, height: f32, forward: f32) -> Vec3 {
        Vec3::new(right, height, -forward)
    }

    /// Height of the natural ground (or deck) below a point, relative to the deck.
    fn ground_at(&self, right: f32, forward: f32) -> f32 {
        let dir = site_point(self.def, self.radius, right as f64, forward as f64);
        ((self.ground)(dir) - self.deck) as f32
    }

    fn launch_mount(&mut self) {
        // Grating over the flame trench, running along the heading under the vehicle.
        self.painted.cuboid(
            Self::at(0.0, 0.03, 0.0),
            Vec3::new(3.6, 0.06, 24.0),
            Quat::IDENTITY,
            c(GRATE),
        );
        // Octagonal hold-down ring around the vehicle's base.
        let ring = 3.0;
        for k in 0..8 {
            let a0 = std::f32::consts::TAU * k as f32 / 8.0;
            let a1 = std::f32::consts::TAU * (k + 1) as f32 / 8.0;
            let p0 = Self::at(ring * a0.cos(), 1.2, ring * a0.sin());
            let p1 = Self::at(ring * a1.cos(), 1.2, ring * a1.sin());
            self.painted.beam(p0, p1, 0.5, c(STEEL));
            self.painted.beam(
                Self::at(ring * a0.cos(), 0.0, ring * a0.sin()),
                p0,
                0.45,
                c(STEEL),
            );
        }
        // Hold-down clamps reaching in to the vehicle.
        for k in 0..4 {
            let a = std::f32::consts::TAU * (k as f32 + 0.5) / 4.0;
            self.painted.beam(
                Self::at(ring * a.cos(), 1.2, ring * a.sin()),
                Self::at(2.0 * a.cos(), 1.6, 2.0 * a.sin()),
                0.35,
                c(STEEL),
            );
        }
    }

    /// Lattice tower centred at (`x`, `z`), `height` tall, `half` wide.
    fn tower(&mut self, x: f32, z: f32, half: f32, height: f32) {
        let red = c(TOWER_RED);
        let corners = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        let corner = |k: usize, y: f32| {
            let (dx, dz) = corners[k % 4];
            Self::at(x + dx * half, y, z + dz * half)
        };
        for k in 0..4 {
            self.painted
                .beam(corner(k, 0.0), corner(k, height), 0.7, red);
        }
        let step = 6.0;
        let levels = (height / step) as usize;
        for level in 0..levels {
            let (y0, y1) = (level as f32 * step, (level + 1) as f32 * step);
            for k in 0..4 {
                self.painted
                    .beam(corner(k, y1), corner(k + 1, y1), 0.35, red);
                self.painted
                    .beam(corner(k, y0), corner(k + 1, y1), 0.22, red);
                self.painted
                    .beam(corner(k + 1, y0), corner(k, y1), 0.22, red);
            }
            if level % 2 == 1 {
                // Working platform.
                self.painted.cuboid(
                    Self::at(x, y1, z),
                    Vec3::new(half, 0.12, half),
                    Quat::IDENTITY,
                    c(STEEL),
                );
            }
        }
        // Elevator and stair core.
        self.painted.cuboid(
            Self::at(x, height / 2.0, z),
            Vec3::new(1.3, height / 2.0, 1.3),
            Quat::IDENTITY,
            c(GREY),
        );
        // Lightning mast.
        self.painted.frustum(
            Self::at(x, height, z),
            Vec3::Y * 20.0,
            (0.45, 0.08),
            6,
            false,
            c(GREY),
        );
        // Hammerhead crane on the roof.
        self.painted.beam(
            Self::at(x + half, height + 2.5, z),
            Self::at(x - half - 6.0, height + 2.5, z),
            0.9,
            c([0.6, 0.45, 0.05]),
        );
    }

    /// Crew access arm from the tower face at `x_face` to the vehicle, at `y`.
    fn access_arm(&mut self, x_face: f32, y: f32) {
        self.painted.cuboid(
            Self::at((x_face + 3.2) / 2.0, y, 0.0),
            Vec3::new((x_face - 3.2) / 2.0, 1.1, 0.9),
            Quat::IDENTITY,
            c(GREY),
        );
        // White room at the hatch.
        self.painted.cuboid(
            Self::at(2.7, y + 0.3, 0.0),
            Vec3::new(0.9, 1.5, 1.4),
            Quat::IDENTITY,
            c(WHITE),
        );
    }

    fn floodlight(&mut self, x: f32, z: f32, height: f32) {
        self.painted.frustum(
            Self::at(x, 0.0, z),
            Vec3::Y * height,
            (0.3, 0.16),
            6,
            false,
            c(GREY),
        );
        // Lamp bank facing the pad centre: local +Z turned towards (-x, 0, z).
        let facing = Quat::from_rotation_y((-x).atan2(z));
        let head = Self::at(x, height, z);
        self.floodlights
            .push(head + facing * Vec3::new(0.0, 0.0, 0.5));
        self.painted
            .cuboid(head, Vec3::new(1.4, 0.8, 0.25), facing, c(STEEL));
        self.lamps.cuboid(
            head + facing * Vec3::new(0.0, 0.0, 0.3),
            Vec3::new(1.2, 0.6, 0.05),
            facing,
            rgb(1.0, 1.0, 1.0),
        );
    }

    fn building(&mut self, x: f32, z: f32) {
        let half = Vec3::new(7.0, 2.8, 4.0);
        self.painted
            .cuboid(Self::at(x, half.y, z), half, Quat::IDENTITY, c(CONCRETE));
        self.painted.cuboid(
            Self::at(x - 3.0, 4.0 + half.y, z),
            Vec3::new(1.2, 0.6, 1.2),
            Quat::IDENTITY,
            c(GREY),
        );
        // Door.
        self.painted.cuboid(
            Self::at(x + 2.0, 1.3, z - half.z),
            Vec3::new(1.2, 1.3, 0.05),
            Quat::IDENTITY,
            c(STEEL),
        );
    }

    fn water_tower(&mut self, x: f32, z: f32) {
        let ground = self.ground_at(x, z);
        let top = ground + 24.0;
        let legs = [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)];
        for (k, (dx, dz)) in legs.iter().enumerate() {
            let foot = Self::at(x + dx * 5.0, ground - 0.5, z + dz * 5.0);
            let head = Self::at(x + dx * 3.0, top, z + dz * 3.0);
            self.painted.beam(foot, head, 0.5, c(GREY));
            let (nx, nz) = legs[(k + 1) % 4];
            let next_foot = Self::at(x + nx * 4.0, ground + 8.0, z + nz * 4.0);
            self.painted.beam(
                Self::at(x + dx * 4.0, ground + 8.0, z + dz * 4.0),
                next_foot,
                0.25,
                c(GREY),
            );
        }
        self.painted.blob(
            Self::at(x, top + 5.0, z),
            Vec3::new(7.0, 6.0, 7.0),
            Quat::IDENTITY,
            2,
            c(WHITE),
            |_| 1.0,
        );
    }

    fn propellant_tank(&mut self, x: f32, z: f32, radius: f32) {
        let ground = self.ground_at(x, z);
        let center = Self::at(x, ground + 3.0 + radius, z);
        for k in 0..4 {
            let a = std::f32::consts::TAU * (k as f32 + 0.5) / 4.0;
            let (dx, dz) = (a.cos() * radius * 0.7, a.sin() * radius * 0.7);
            self.painted.beam(
                Self::at(x + dx, ground - 0.5, z + dz),
                center + Vec3::new(dx, -radius * 0.5, -dz),
                0.5,
                c(GREY),
            );
        }
        self.painted.blob(
            center,
            Vec3::splat(radius),
            Quat::IDENTITY,
            2,
            c(WHITE),
            |_| 1.0,
        );
    }

    /// A pipe from (`x0`, `z0`) to (`x1`, `z1`), resting on the ground on short supports.
    fn pipe(&mut self, from: (f32, f32), to: (f32, f32)) {
        let steps = 12;
        let points: Vec<Vec3> = (0..=steps)
            .map(|k| {
                let t = k as f32 / steps as f32;
                let (x, z) = (from.0 + (to.0 - from.0) * t, from.1 + (to.1 - from.1) * t);
                Self::at(x, self.ground_at(x, z).max(0.0) + 0.8, z)
            })
            .collect();
        for pair in points.windows(2) {
            let (a, b) = (pair[0], pair[1]);
            self.painted.beam(a, b, 0.45, c(STEEL));
            self.painted.beam(a - Vec3::Y * 1.2, a, 0.2, c(GREY));
        }
    }

    fn build(&mut self) {
        let Some(pad) = &self.def.pad else { return };
        // Deck half extents: across (right) and along (forward).
        let (across, along) = (pad.size[1] as f32 / 2.0, pad.size[0] as f32 / 2.0);
        self.launch_mount();
        let tower_half = 3.5;
        let tower_x = 6.5 + tower_half;
        self.tower(tower_x, 0.0, tower_half, 78.0);
        self.access_arm(tower_x - tower_half, 60.0);
        for (sx, sz) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
            self.floodlight(sx * (across - 5.0), sz * (along - 5.0), 22.0);
        }
        self.building(-(across - 12.0), along - 9.0);
        self.water_tower(-(across + 45.0), along);
        let tanks = (across + 30.0, -(along + 8.0));
        self.propellant_tank(tanks.0, tanks.1, 5.5);
        self.propellant_tank(tanks.0 + 14.0, tanks.1, 4.5);
        self.pipe((tanks.0 - 6.0, tanks.1), (4.0, -12.0));
    }
}

fn spawn_sites(
    mut commands: Commands,
    sim: Res<Sim>,
    world: Res<WorldData>,
    mut meshes: ResMut<Assets<Mesh>>,
    mut materials: ResMut<Assets<StandardMaterial>>,
) {
    let painted = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        perceptual_roughness: 0.65,
        metallic: 0.15,
        ..default()
    });
    let lamps = materials.add(StandardMaterial {
        base_color: Color::WHITE,
        emissive: LinearRgba::rgb(30_000.0, 27_000.0, 22_000.0),
        ..default()
    });
    let system = &sim.universe.system;
    for def in &world.sites {
        let Some(body_id) = system.find(&def.body) else {
            continue;
        };
        if def.pad.is_none() {
            continue;
        }
        let body = system.body(body_id);
        let terrain = body.terrain.clone();
        let ground = move |dir: DVec3| terrain.height(dir);
        let (center_dir, right, forward) = site_axes(def);
        let deck = body.terrain.height(center_dir);
        let mut site = Site {
            def,
            radius: body.radius,
            deck,
            ground: &ground,
            painted: MeshBuilder::default(),
            lamps: MeshBuilder::default(),
            floodlights: Vec::new(),
        };
        site.build();
        // Site frame (x right, y up, -z forward) to body-fixed render axes.
        let rotation = Quat::from_mat3(&Mat3::from_cols(
            to_render(right),
            to_render(center_dir),
            to_render(-forward),
        ));
        let center = center_dir * (body.radius + deck);
        commands
            .spawn((
                SurfaceAnchor {
                    body: body_id,
                    center,
                },
                Transform::default(),
                Visibility::default(),
                Name::new(def.display_name().to_owned()),
            ))
            .with_children(|parent| {
                let frame = Transform::from_rotation(rotation);
                parent.spawn((
                    Mesh3d(meshes.add(site.painted.build())),
                    MeshMaterial3d(painted.clone()),
                    frame,
                ));
                parent.spawn((
                    Mesh3d(meshes.add(site.lamps.build())),
                    MeshMaterial3d(lamps.clone()),
                    frame,
                ));
                // Floodlights on the vehicle; they only matter at night.
                let target = Site::at(0.0, 30.0, 0.0);
                for position in &site.floodlights {
                    parent.spawn((
                        SpotLight {
                            intensity: 3.0e6,
                            range: 250.0,
                            outer_angle: 0.45,
                            inner_angle: 0.3,
                            shadow_maps_enabled: false,
                            color: Color::srgb(1.0, 0.93, 0.82),
                            ..default()
                        },
                        Transform::from_rotation(rotation)
                            * Transform::from_translation(*position).looking_at(target, Vec3::Y),
                    ));
                }
            });
    }
}
