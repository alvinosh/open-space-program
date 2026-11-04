//! Colliders against the real models: vessels touch where they are drawn to, ports sit on their
//! hardware, and a vessel can come all the way in to dock.

use std::{path::Path, sync::Arc};

use osp_sim::{
    DQuat, DVec3,
    assembly::Pose,
    collision::{Collider, Colliders},
    data::{CraftDef, PartCatalog},
    docking::DockingPort,
    propulsion::Propulsion,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

struct Game {
    colliders: Colliders,
    catalog: PartCatalog,
    craft: Vec<CraftDef>,
}

fn game() -> Game {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let mut colliders = Colliders::default();
    colliders.set_files(Arc::new(content.file_reader()));
    Game {
        colliders,
        catalog: record::catalog(&content.registry),
        craft: content.registry.all("craft", &mut Vec::new()),
    }
}

impl Game {
    fn craft(&self, id: &str) -> Propulsion {
        let def = self.craft.iter().find(|c| c.id == id).unwrap();
        Propulsion::from_craft(def, &self.catalog).unwrap()
    }

    /// The command and service modules.
    fn csm(&self) -> Propulsion {
        let mut p = self.craft("saturn_v");
        for _ in 0..8 {
            p.activate_next_sequence();
        }
        p
    }

    /// The lunar module's ascent stage.
    fn eagle(&self) -> Propulsion {
        let mut p = self.craft("lunar_module");
        for _ in 0..2 {
            p.activate_next_sequence();
        }
        p
    }
}

fn the_port(p: &Propulsion) -> DockingPort {
    p.docking_ports()[0].clone()
}

/// `a` on the axis of `b`'s port, `gap` metres off it and `aside` metres across it, facing it, as a
/// frame for `a`'s collider. `b`'s is left where it is.
fn lined_up(a: &DockingPort, b: &DockingPort, gap: f64, aside: f64) -> Pose {
    let side = b.direction.any_orthonormal_vector();
    let target = b.position + b.direction * gap + side * aside;
    let turn = DQuat::from_rotation_arc(a.direction, -b.direction);
    Pose::new(target - turn * a.position, turn)
}

fn touching(a: &Collider, at: Pose, b: &Collider) -> bool {
    a.touches(at, b, Pose::IDENTITY)
}

/// How far off its port's plane a vessel coming straight in first touches, m.
fn first_touch(a: &Collider, pa: &DockingPort, b: &Collider, pb: &DockingPort, aside: f64) -> f64 {
    let (mut clear, mut hit) = (6.0, -2.0);
    assert!(!touching(a, lined_up(pa, pb, clear, aside), b));
    if !touching(a, lined_up(pa, pb, hit, aside), b) {
        return f64::NEG_INFINITY;
    }
    for _ in 0..30 {
        let middle = (clear + hit) / 2.0;
        if touching(a, lined_up(pa, pb, middle, aside), b) {
            hit = middle;
        } else {
            clear = middle;
        }
    }
    clear
}

/// Coming straight in, what first touches is the two rims, at the ports: nothing stands in the way,
/// whichever way round it comes.
#[test]
fn on_the_axis_the_rims_are_what_meet() {
    let mut game = game();
    let (csm, eagle) = (game.csm(), game.eagle());
    let (a, b) = (
        game.colliders.of(csm.parts()),
        game.colliders.of(eagle.parts()),
    );
    let (pa, pb) = (the_port(&csm), the_port(&eagle));
    let touch = first_touch(&a, &pa, &b, &pb, 0.0);
    assert!(touch.abs() < 0.03, "first touch {touch} m off the ports");
}

/// The stand-in for the cabin was a 4 m drum as high as its aerial. The nose can go down past where
/// its top was, a metre clear of the cabin to the side of the tunnel.
#[test]
fn the_cabin_is_its_real_shape() {
    let mut game = game();
    let (csm, eagle) = (game.csm(), game.eagle());
    let (a, b) = (
        game.colliders.of(csm.parts()),
        game.colliders.of(eagle.parts()),
    );
    let (pa, pb) = (the_port(&csm), the_port(&eagle));
    // Every triangle of the model is there, not the few of its stand-in.
    let cabin = eagle
        .parts()
        .parts()
        .iter()
        .find(|p| p.name == "Ascent Stage");
    let cabin = b
        .parts
        .iter()
        .find(|m| m.pose.position == cabin.unwrap().position);
    assert!(cabin.unwrap().mesh.triangle_count() >= 19_000);
    // Where the top of the drum was, 80 cm above the rim, and half a metre to one side.
    assert!(!touching(&a, lined_up(&pa, &pb, 0.5, 0.5), &b));
    assert!(!touching(&a, lined_up(&pa, &pb, 0.1, 0.0), &b));
    // Into the roof it does hit, well to the side of the tunnel.
    assert!(touching(&a, lined_up(&pa, &pb, -0.5, 1.0), &b));
}

/// What the air's outline left out collides too: an engine's bell, a leg.
#[test]
fn engines_and_legs_collide() {
    let mut game = game();
    let lander = game.craft("lunar_module");
    let tree = lander.parts();
    let whole = game.colliders.of(tree);
    let probe = game.eagle();
    let port = the_port(&probe);
    let nose = game.colliders.of(probe.parts());
    // The eagle's tunnel rim, put against a point of `tree`'s part called `name`, from `from`.
    let against = |name: &str, want: bool| {
        let part = tree.parts().iter().find(|p| p.name.contains(name)).unwrap();
        let mesh = whole
            .parts
            .iter()
            .find(|m| m.pose.position == part.position)
            .unwrap();
        let middle = (mesh.bounds.min + mesh.bounds.max) / 2.0;
        let frame = Pose::new(middle - port.position, DQuat::IDENTITY);
        assert_eq!(nose.touches(frame, &whole, Pose::IDENTITY), want, "{name}");
    };
    against("Descent engine", true);
    against("leg", true);
    // Far off, nothing.
    let far = Pose::new(DVec3::X * 100.0, DQuat::IDENTITY);
    assert!(!nose.touches(far, &whole, Pose::IDENTITY));
}

/// A port is on its hardware: nothing of its own part stands out in front of it within its ring, and
/// its part reaches it.
#[test]
fn every_port_is_where_its_part_is() {
    let mut game = game();
    for id in ["saturn_v", "saturn_ib", "lunar_module"] {
        let mut p = game.craft(id);
        // The Saturn V's escape tower and its cover stand over the port until they go.
        if id == "saturn_v" {
            for _ in 0..4 {
                p.activate_next_sequence();
            }
        }
        let port = the_port(&p);
        let part = p.parts().get(port.part).unwrap();
        let tree = p.parts();
        let collider = game.colliders.of(tree);
        let mut nearest = f64::INFINITY;
        for mesh in &collider.parts {
            for t in mesh.mesh.triangles() {
                for corner in t {
                    let at = mesh.pose.point(corner);
                    let off = at - port.position;
                    let ahead = off.dot(port.direction);
                    let across = (off - port.direction * ahead).length();
                    if across < port.diameter / 2.0 {
                        assert!(ahead < 0.02, "{id}: {ahead} m in front of the port");
                    }
                    if across < 1.0 {
                        nearest = nearest.min(ahead.abs());
                    }
                }
            }
        }
        assert!(
            nearest < 0.05,
            "{id} `{}`: {nearest} m to its port",
            part.name
        );
    }
}
