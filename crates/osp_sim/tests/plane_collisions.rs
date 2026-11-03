//! Real aircraft and real parts hitting each other: momentum goes where it should, nothing stays
//! stuck to anything, and every part collides as it is drawn, however it was made.

use std::{path::Path, sync::Arc};

use osp_sim::{
    DQuat, DVec3,
    assembly::Pose,
    collision::{Bounds, Collider, Colliders, Mesh, PlacedMesh},
    data::{CraftDef, FlightStartDef, LaunchSiteDef, PartCatalog, SystemDef},
    propulsion::Propulsion,
    system::SolarSystem,
    time::SimTime,
    universe::{PHYSICS_STEP, Universe},
    vessel::{Vessel, VesselEventKind},
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

struct Game {
    system: SystemDef,
    sites: Vec<LaunchSiteDef>,
    catalog: PartCatalog,
    craft: Vec<CraftDef>,
    files: osp_sim::collision::ModelFiles,
}

fn game() -> Game {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let mut diagnostics = Vec::new();
    Game {
        system: SystemDef {
            bodies: content.registry.all("bodies", &mut diagnostics),
        },
        sites: content.registry.all("launch_sites", &mut diagnostics),
        catalog: record::catalog(&content.registry),
        craft: content.registry.all("craft", &mut diagnostics),
        files: Arc::new(content.file_reader()),
    }
}

impl Game {
    fn comet(&self) -> &CraftDef {
        self.craft.iter().find(|c| c.id == "comet").unwrap()
    }

    /// The runway, its near end `back` metres further back than it really is.
    fn runway(&self, back: f64) -> LaunchSiteDef {
        let mut site = (self.sites.iter())
            .find(|s| s.id == "cape_slf")
            .unwrap()
            .clone();
        site.runway.as_mut().unwrap().size[0] += 2.0 * back;
        site
    }

    fn universe(&self, system: SolarSystem, vessels: Vec<Vessel>) -> Universe {
        let mut universe = Universe::new(system, vessels, SimTime::J2000);
        universe.colliders.set_files(self.files.clone());
        universe.focus = Some(universe.vessels[0].id());
        universe
    }

    /// One Comet on its way down the runway under power, `behind` metres behind another parked with
    /// its brakes on. Equal masses, to begin with.
    fn taxi_into_parked(&self, behind: f64) -> Universe {
        let system = SolarSystem::from_def(&self.system).unwrap();
        let t = SimTime::J2000;
        let on_runway = |back: f64| {
            Vessel::on_runway(self.comet(), &self.catalog, &system, &self.runway(back), t).unwrap()
        };
        let mut moving = on_runway(behind);
        let parked = on_runway(0.0);
        moving.activate_next_stage(&system, t);
        moving.set_throttle(1.0);
        moving.brakes = false;
        self.universe(system, vec![moving, parked])
    }

    /// Two Comets in the air, one over the other's left wing and catching up on it.
    fn catching_up_in_the_air(&self) -> Universe {
        let system = SolarSystem::from_def(&self.system).unwrap();
        let t = SimTime::J2000;
        let start = |speed: f64, latitude: f64, longitude: f64| FlightStartDef {
            body: "Earth".into(),
            latitude,
            longitude,
            altitude: 2_000.0,
            heading: 0.0,
            speed,
            climb: 0.0,
        };
        // About 111 km to the degree. The second is 40 m ahead and 3 m to the side.
        let (first, second) = (start(110.0, 0.0, 3e-5), start(100.0, 40.0 / 111_000.0, 0.0));
        let fly = |start: &FlightStartDef| {
            Vessel::in_flight(self.comet(), &self.catalog, &system, start, t).unwrap()
        };
        let (a, b) = (fly(&first), fly(&second));
        self.universe(system, vec![a, b])
    }
}

fn ground_speed(u: &Universe, k: usize) -> f64 {
    let body = u.system.body(u.vessels[k].parent());
    let state = u.vessels[k].state_at(&u.system, u.time());
    (state.velocity - body.surface_velocity(state.position)).length()
}

fn apart(u: &Universe) -> f64 {
    let t = u.time();
    let at = |k: usize| u.vessels[k].state_at(&u.system, t).position;
    at(0).distance(at(1))
}

/// Where vessel `k` is on the turning body it is on.
fn fixed_place(u: &Universe, k: usize) -> DVec3 {
    let t = u.time();
    let body = u.system.body(u.vessels[k].parent());
    body.orientation(t).inverse() * u.vessels[k].state_at(&u.system, t).position
}

fn ticks(seconds: f64) -> usize {
    (seconds / PHYSICS_STEP.as_seconds()).round() as usize
}

fn bumps(events: &[VesselEventKind]) -> usize {
    (events.iter())
        .filter(|e| matches!(e, VesselEventKind::Bumped { .. }))
        .count()
}

/// A slow bump shoves what it hits along, and the two come to rest again on their brakes.
#[test]
fn a_parked_plane_is_shoved_by_a_slow_one() {
    let game = game();
    let mut u = game.taxi_into_parked(30.0);
    assert!(u.vessels[1].is_landed());
    let parked_at = fixed_place(&u, 1);
    let mut top = 0.0_f64;
    let mut hit = None;
    for _ in 0..ticks(40.0) {
        if ground_speed(&u, 0) >= 3.0 {
            u.vessels[0].set_throttle(0.0);
        }
        top = top.max(ground_speed(&u, 0));
        for event in u.step_physics() {
            if let VesselEventKind::Bumped { speed } = event.event.kind {
                hit.get_or_insert(speed);
            }
        }
        if hit.is_some() && u.vessels[0].is_landed() && u.vessels[1].is_landed() {
            break;
        }
    }
    let hit = hit.expect("they met");
    assert!(hit > 0.3, "{hit} m/s");
    // The one that was parked went somewhere, along the ground.
    let moved = fixed_place(&u, 1).distance(parked_at);
    assert!((0.05..30.0).contains(&moved), "it moved {moved} m");
    let height = fixed_place(&u, 1).length() - parked_at.length();
    assert!(height.abs() < 0.1, "{height} m off the ground it was on");
    assert!(
        u.vessels[0].is_landed() && u.vessels[1].is_landed(),
        "they come to rest: {} and {} m/s",
        ground_speed(&u, 0),
        ground_speed(&u, 1)
    );
    assert!(!u.vessels[0].is_destroyed() && !u.vessels[1].is_destroyed());
    assert!(apart(&u) > 10.0, "{} m between them", apart(&u));
}

/// Hit hard, a plane on its brakes takes half of what hit it, as equal masses should.
#[test]
fn a_hard_hit_shares_the_momentum() {
    let game = game();
    let mut u = game.taxi_into_parked(80.0);
    let mut before = 0.0;
    let mut after = None;
    for _ in 0..ticks(12.0) {
        before = ground_speed(&u, 0);
        u.step_physics();
        if ground_speed(&u, 1) > 1.0 {
            after = Some((ground_speed(&u, 0), ground_speed(&u, 1)));
            break;
        }
    }
    let (a, b) = after.expect("it was moved");
    assert!(before > 15.0, "{before} m/s");
    // The tail end of the hit, with the Comet's nose going in at a slant: well short of all of it,
    // and neither is left standing still.
    assert!(a < 0.9 * before && a > 0.2 * before, "{before} to {a} m/s");
    assert!(
        b > 0.2 * before && b < 0.9 * before,
        "{before} gave {b} m/s"
    );
    assert!(
        (a + b - before).abs() < 0.25 * before,
        "{before} became {a} and {b} m/s"
    );
    assert!(u.vessels[0].is_destroyed() && u.vessels[1].is_destroyed());
}

/// Two planes that clip each other in the air tumble off their own ways: they used to fly on stuck
/// together, one's wing through the other, being told about it every tick.
#[test]
fn planes_that_clip_each_other_part() {
    let game = game();
    let mut u = game.catching_up_in_the_air();
    let mut events = Vec::new();
    let mut closest = f64::INFINITY;
    let mut hardest_spin = 0.0_f64;
    let mut last_hit = 0;
    for tick in 0..ticks(12.0) {
        let fresh: Vec<_> = u.step_physics().into_iter().map(|e| e.event.kind).collect();
        if !fresh.is_empty() {
            last_hit = tick;
        }
        events.extend(fresh);
        closest = closest.min(apart(&u));
        if !events.is_empty() {
            let t = u.time();
            let spin = u.vessels[1].attitude_state(&u.system, t).angular_velocity;
            hardest_spin = hardest_spin.max(spin.length());
        }
    }
    let count = bumps(&events);
    assert!((1..=12).contains(&count), "{count} bumps: {events:?}");
    assert!(closest > 3.0, "{closest} m apart at the closest");
    // A glancing hit, so they do not part fast, but they do part, and it is over.
    assert!(
        ticks(12.0) - last_hit > ticks(5.0),
        "still hitting at tick {last_hit}"
    );
    assert!(apart(&u) > closest + 10.0, "{} m apart after", apart(&u));
    assert!(hardest_spin > 0.3, "{hardest_spin} rad/s");
}

/// Near enough to be handed to the solver but never touching, a vessel flies exactly as it would on
/// its own, down to the last bit.
#[test]
fn flying_close_changes_nothing_until_they_touch() {
    let game = game();
    let system = SolarSystem::from_def(&game.system).unwrap();
    let t = SimTime::J2000;
    let fly = |longitude: f64| {
        let start = FlightStartDef {
            body: "Earth".into(),
            latitude: 0.0,
            longitude,
            altitude: 2_000.0,
            heading: 0.0,
            speed: 100.0,
            climb: 0.0,
        };
        Vessel::in_flight(game.comet(), &game.catalog, &system, &start, t).unwrap()
    };
    let mut colliders = Colliders::default();
    colliders.set_files(game.files.clone());
    let width = {
        let plane = fly(0.0);
        let bounds = colliders.of(plane.propulsion.parts()).bounds().unwrap();
        let size = bounds.max - bounds.min;
        size.x.max(size.y)
    };
    // Half a metre clear of each other, and neither moves against the other.
    let gap = (width + 0.5) / 111_320.0;
    let mut together = game.universe(
        SolarSystem::from_def(&game.system).unwrap(),
        vec![fly(0.0), fly(gap)],
    );
    let mut alone = game.universe(SolarSystem::from_def(&game.system).unwrap(), vec![fly(0.0)]);
    for _ in 0..ticks(2.0) {
        let events = together.step_physics();
        assert_eq!(
            bumps(&events.iter().map(|e| e.event.kind).collect::<Vec<_>>()),
            0
        );
        alone.step_physics();
    }
    let now = alone.time();
    let (a, b) = (
        together.vessels[0].state_at(&together.system, now),
        alone.vessels[0].state_at(&alone.system, now),
    );
    assert_eq!(a.position, b.position);
    assert_eq!(a.velocity, b.velocity);
}

fn block(half: f64) -> Collider {
    let corners = (0..8)
        .map(|k| {
            DVec3::new(
                if k & 1 == 0 { -half } else { half },
                if k & 2 == 0 { -half } else { half },
                if k & 4 == 0 { -half } else { half },
            )
        })
        .collect();
    let mesh = Mesh::new(
        corners,
        vec![
            [0, 2, 1],
            [1, 2, 3],
            [4, 5, 6],
            [5, 7, 6],
            [0, 1, 4],
            [1, 5, 4],
            [2, 6, 3],
            [3, 6, 7],
            [0, 4, 2],
            [2, 4, 6],
            [1, 3, 5],
            [3, 7, 5],
        ],
    );
    let bounds = mesh.bounds().unwrap();
    Collider {
        parts: vec![PlacedMesh {
            mesh: Arc::new(mesh),
            pose: Pose::IDENTITY,
            bounds,
        }],
    }
}

/// Every part of every craft in the game gets a collider as big as it is drawn, whether it is a
/// procedural shape or a model, a flat fin or an engine bell, with nothing about it special-cased.
#[test]
fn every_part_of_every_craft_collides() {
    let game = game();
    let mut colliders = Colliders::default();
    colliders.set_files(game.files.clone());
    let mut parts = 0;
    let craft: Vec<_> = (game.craft.iter())
        .filter_map(|def| Some((def, Propulsion::from_craft(def, &game.catalog).ok()?)))
        .map(|(def, craft)| (def, colliders.of(craft.parts())))
        .collect();
    // All at once: there are over a thousand of them to cut.
    for (_, collider) in &craft {
        colliders.prepare(collider);
    }
    for (def, collider) in &craft {
        for placed in &collider.parts {
            parts += 1;
            let drawn: Bounds = placed.mesh.bounds().unwrap();
            let size = drawn.max - drawn.min;
            let middle = (drawn.max + drawn.min) / 2.0;
            let name = format!("{} at {:?}", def.id, placed.pose.position);
            assert!(placed.mesh.piece_count() > 0, "{name}: no pieces");
            // Its pieces reach as far as it is drawn, each way, and no further.
            let edges = placed.mesh.piece_edges();
            assert!(!edges.is_empty(), "{name}: no edges");
            let mut low = DVec3::INFINITY;
            let mut high = DVec3::NEG_INFINITY;
            for point in edges.iter().flatten() {
                low = low.min(*point);
                high = high.max(*point);
            }
            let slack = 0.05 * size.max_element() + 0.03;
            assert!(
                (low - drawn.min).abs().max_element() < slack
                    && (high - drawn.max).abs().max_element() < slack,
                "{name}: drawn {drawn:?}, cut to {low} .. {high}, around {middle}"
            );
        }
    }
    assert!(parts > 500, "{parts} parts");
}

/// A procedural part that is open at the end has room inside it, as a real interstage does, and a
/// procedural part that is closed does not. Nothing tells the two apart but their shapes.
#[test]
fn procedural_parts_are_hollow_where_they_are_open() {
    let game = game();
    let mut colliders = Colliders::default();
    colliders.set_files(game.files.clone());
    let def = game.craft.iter().find(|c| c.id == "saturn_v").unwrap();
    let craft = Propulsion::from_craft(def, &game.catalog).unwrap();
    let tree = craft.parts();
    let whole = colliders.of(tree);
    let of_named = |name: &str| {
        let part = (tree.parts().iter())
            .find(|p| p.name.to_lowercase().contains(name))
            .unwrap_or_else(|| panic!("no part called {name}"));
        let placed = (whole.parts.iter())
            .find(|m| m.pose.position == part.position)
            .unwrap()
            .clone();
        Collider {
            parts: vec![placed],
        }
    };
    let inside = |c: &Collider, half: f64, across: f64| {
        let bounds = c.bounds().unwrap();
        let middle = (bounds.min + bounds.max) / 2.0;
        let at = Pose::new(middle + DVec3::X * across, DQuat::IDENTITY);
        block(half).touches(at, c, Pose::IDENTITY)
    };
    let interstage = of_named("interstage");
    let radius = {
        let bounds = interstage.bounds().unwrap();
        (bounds.max.x - bounds.min.x) / 2.0
    };
    assert!(!inside(&interstage, 0.5, 0.0), "the middle of it is empty");
    assert!(inside(&interstage, 0.5, radius), "its wall is not");
}
