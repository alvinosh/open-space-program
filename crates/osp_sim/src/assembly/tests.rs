//! Parts joined, placed, rerooted and split.

use std::{
    f64::consts::{FRAC_PI_2, PI},
    sync::Arc,
};

use glam::{DQuat, DVec3};

use super::*;
use crate::{geometry::ShapeDef, test_data::records};

fn shape(text: &str) -> Arc<Geometry> {
    let def: ShapeDef = records(text).remove(0);
    Arc::new(def.build().unwrap_or_else(|e| panic!("{e}")))
}

fn part(id: &str, text: &str) -> AssemblyPart {
    AssemblyPart::new(id, shape(text))
}

fn node(part: usize, name: &str) -> End {
    End {
        part,
        mount: Mount::Node(name.into()),
    }
}

fn surface(part: usize, name: &str, u: f64, v: f64) -> End {
    End {
        part,
        mount: Mount::Surface {
            surface: name.into(),
            side: Side::Front,
            u,
            v,
            standoff: 0.0,
        },
    }
}

#[track_caller]
fn assert_near(a: DVec3, b: DVec3) {
    assert!((a - b).length() < 1e-9, "{a} vs {b}");
}

#[track_caller]
fn assert_turned(a: DQuat, b: DQuat) {
    assert!(a.dot(b).abs() > 1.0 - 1e-12, "{a} vs {b}");
}

const LOWER: &str = "{ cylinder_tank: { diameter: 3, length: 12, aft_skirt: 2 } }";
const UPPER: &str = "{ cylinder_tank: { diameter: 3, length: 5 } }";
const NOSE: &str = "{ shell: { bottom_diameter: 3, sections: [{ length: 4, top_diameter: 0, profile: 'ogive' }] } }";

fn top(part: &AssemblyPart) -> f64 {
    part.node("top").unwrap().position.z
}

fn stack() -> Assembly {
    let parts = vec![
        part("lower", LOWER),
        part("upper", UPPER),
        part("nose", NOSE),
    ];
    let joints = vec![
        Joint::new(node(0, "top"), node(1, "bottom")),
        // Either end can come first.
        Joint::new(node(2, "bottom"), node(1, "top")),
    ];
    Assembly::new(parts, joints, 0).unwrap()
}

#[test]
fn a_stack_meets_node_to_node() {
    let stack = stack();
    let poses = stack.poses();
    let (lower, upper) = (top(&stack.parts()[0]), top(&stack.parts()[1]));
    assert_near(poses[1].position, DVec3::Z * lower);
    assert_near(poses[2].position, DVec3::Z * (lower + upper));
    for pose in &poses {
        assert_turned(pose.rotation, DQuat::IDENTITY);
    }
    // Rolled a quarter turn, the upper tank turns about the axis, and the nose with it.
    let mut rolled = stack.clone();
    rolled.joints[0].roll = FRAC_PI_2;
    let poses = rolled.poses();
    assert_turned(poses[1].rotation, DQuat::from_rotation_z(FRAC_PI_2));
    assert_turned(poses[2].rotation, DQuat::from_rotation_z(FRAC_PI_2));
    assert_near(poses[2].position, DVec3::Z * (lower + upper));
    let parents = stack.parents();
    assert_eq!(parents, vec![None, Some((0, 0)), Some((1, 1))]);
}

#[test]
fn boosters_on_a_core_are_placed_alike_whoever_placed_them() {
    // A core with four radial decouplers on its barrel, each holding a booster by the
    // booster's side: written out one by one, as symmetry would write them.
    let core = "{ cylinder_tank: { diameter: 3, length: 20 } }";
    let pylon = "{ box: { size: [0.3, 0.4, 0.6] } }";
    let booster = "{ cylinder_tank: { diameter: 2, length: 14 } }";
    let mut parts = vec![part("core", core)];
    let mut joints = Vec::new();
    for k in 0..4 {
        let u = k as f64 / 4.0;
        parts.push(part(&format!("pylon {k}"), pylon));
        parts.push(part(&format!("booster {k}"), booster));
        let (p, b) = (parts.len() - 2, parts.len() - 1);
        joints.push(Joint::new(surface(0, "barrel", u, 0.5), node(p, "bottom")));
        // The booster's far side (u = 1/2) faces the core; rolled upright.
        let mut hold = Joint::new(node(p, "top"), surface(b, "barrel", 0.5, 0.5));
        hold.roll = PI;
        joints.push(hold);
    }
    let assembly = Assembly::new(parts, joints, 0).unwrap();
    let poses = assembly.poses();
    let (core_mid, booster_mid) = (10.0, 7.0);
    for k in 0..4 {
        let turn = DQuat::from_rotation_z(k as f64 * FRAC_PI_2);
        let booster = poses[2 + 2 * k];
        // Upright, its axis 1.5 + 0.6 + 1 m out, its middle level with the core's.
        assert_turned(booster.rotation, turn);
        let axis = turn * DVec3::X * (1.5 + 0.6 + 1.0);
        assert_near(booster.position, axis + DVec3::Z * (core_mid - booster_mid));
        // The pylon stands straight out from the core.
        let pylon = poses[1 + 2 * k];
        assert_near(pylon.rotation * DVec3::Z, turn * DVec3::X);
    }
    // Two placed the same way are the same, however they came to be: all four boosters are
    // the first one turned about the axis.
    let first = poses[2];
    for k in 1..4 {
        let turn = Pose::new(DVec3::ZERO, DQuat::from_rotation_z(k as f64 * FRAC_PI_2));
        let expected = turn * first;
        assert_near(poses[2 + 2 * k].position, expected.position);
        assert_turned(poses[2 + 2 * k].rotation, expected.rotation);
    }
}

#[test]
fn a_wing_root_meets_the_hull_square_on() {
    let parts = vec![
        part(
            "fuselage",
            "{ cylinder_tank: { diameter: 2, length: 10, dome: 0 } }",
        ),
        part("wing", "{ wing: { root_chord: 3, tip_chord: 1, span: 5 } }"),
    ];
    let joints = vec![Joint::new(surface(0, "barrel", 0.0, 0.4), node(1, "root"))];
    let poses = Assembly::new(parts, joints, 0).unwrap().poses();
    // On the right of the hull, its span straight out along +X, its chord back down −Z.
    assert_near(poses[1].position, DVec3::new(1.0, 0.0, 4.0));
    assert_turned(poses[1].rotation, DQuat::IDENTITY);
}

#[test]
fn rerooting_changes_no_joint_and_moves_nothing() {
    let mut assembly = stack();
    let before = assembly.poses();
    assembly.reroot(2);
    let after = assembly.poses();
    assert_eq!(after[2], Pose::IDENTITY);
    assert_eq!(assembly.parents()[0], Some((0, 1)));
    // Everything sits where it did, seen from the new root.
    let from_nose = before[2].inverse();
    for (b, a) in before.iter().zip(&after) {
        let expected = from_nose * *b;
        assert_near(a.position, expected.position);
        assert_turned(a.rotation, expected.rotation);
    }
}

#[test]
fn a_released_joint_leaves_the_side_without_the_root() {
    let mut assembly = stack();
    // Nothing on the joint releases: it holds.
    assert!(assembly.clone().release(0).is_none());
    assembly.parts[1].releases.insert("bottom".into());
    let mut kept = assembly.clone();
    let gone = kept.release(0).unwrap();
    assert_eq!(
        kept.parts()
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["lower"]
    );
    assert_eq!(
        gone.parts()
            .iter()
            .map(|p| p.id.as_str())
            .collect::<Vec<_>>(),
        ["upper", "nose"]
    );
    // The leaving side is rooted where the joint held it, with its joint renumbered.
    assert_eq!(gone.parts()[gone.root()].id, "upper");
    assert_eq!(gone.joints().len(), 1);
    assert_near(gone.poses()[1].position, DVec3::Z * top(&gone.parts()[0]));
    // Rooted at the nose instead, the nose's side stays and the lower tank leaves.
    assembly.reroot(2);
    let gone = assembly.split(0);
    assert_eq!(gone.parts()[gone.root()].id, "lower");
    assert_eq!(assembly.parts()[assembly.root()].id, "nose");
    assert_eq!(assembly.parts().len(), 2);
}

#[test]
fn hinges_turn_and_slides_extend() {
    let moving = |motion, travel| {
        let parts = vec![part("tank", UPPER), part("nose", NOSE)];
        // A joint moves its second end in the first one's frame.
        let mut joint = Joint::new(node(0, "top"), node(1, "bottom"));
        joint.motion = motion;
        joint.travel = travel;
        Assembly::new(parts, vec![joint], 0).unwrap().poses()[1]
    };
    let joint = DVec3::Z * top(&part("tank", UPPER));
    // A quarter turn about the joint's X axis, at the joint.
    let hinged = moving(Motion::Hinge, FRAC_PI_2);
    assert_near(hinged.position, joint);
    assert_near(hinged.rotation * DVec3::Z, DVec3::NEG_Y);
    // Drawn out half a metre along the line the parts meet along.
    assert_near(moving(Motion::Slide, 0.5).position, joint + DVec3::Z * 0.5);
    // A rigid joint ignores its travel.
    assert_near(moving(Motion::Rigid, 0.5).position, joint);
}

#[test]
fn joints_pass_what_both_ends_pass() {
    let parts = vec![
        part("tank", LOWER),
        part("upper", UPPER),
        part("box", "{ box: { size: [0.3, 0.4, 0.6] } }"),
    ];
    let joints = vec![
        Joint::new(node(0, "top"), node(1, "bottom")),
        Joint::new(surface(0, "barrel", 0.0, 0.5), node(2, "bottom")),
    ];
    let mut assembly = Assembly::new(parts, joints, 0).unwrap();
    assert_eq!(assembly.carries(0), Carries::NODE);
    assert_eq!(assembly.carries(1), Carries::SURFACE);
    assembly.parts[1]
        .carries
        .insert("bottom".into(), Carries::NOTHING);
    assert_eq!(assembly.carries(0), Carries::NOTHING);
}

#[test]
fn bad_assemblies_are_refused() {
    let three = || vec![part("a", UPPER), part("b", UPPER), part("c", UPPER)];
    let refused = |parts, joints| Assembly::new(parts, joints, 0).unwrap_err();
    let both_on_top = vec![
        Joint::new(node(0, "top"), node(1, "bottom")),
        Joint::new(node(0, "top"), node(2, "bottom")),
    ];
    assert!(refused(three(), both_on_top).contains("holds two joints"));
    let unjoined = vec![Joint::new(node(0, "top"), node(1, "bottom"))];
    assert!(refused(three(), unjoined).contains("`c` is not joined"));
    let looped = vec![
        Joint::new(node(0, "top"), node(1, "bottom")),
        Joint::new(node(1, "top"), node(2, "bottom")),
        Joint::new(node(2, "top"), node(0, "bottom")),
    ];
    assert!(refused(three(), looped).contains("loop"));
    let two_surfaces = vec![
        Joint::new(
            surface(0, "barrel", 0.0, 0.5),
            surface(1, "barrel", 0.5, 0.5),
        ),
        Joint::new(node(1, "top"), node(2, "bottom")),
    ];
    assert!(refused(three(), two_surfaces).contains("one end must be a node"));
    let nowhere = vec![
        Joint::new(node(0, "side"), node(1, "bottom")),
        Joint::new(node(1, "top"), node(2, "bottom")),
    ];
    assert!(refused(three(), nowhere).contains("no `side`"));
    let off_the_surface = vec![
        Joint::new(surface(0, "barrel", 0.0, 1.5), node(1, "bottom")),
        Joint::new(node(1, "top"), node(2, "bottom")),
    ];
    assert!(refused(three(), off_the_surface).contains("from 0 to 1"));
    let twins = vec![part("a", UPPER), part("a", UPPER)];
    assert!(refused(twins, vec![Joint::new(node(0, "top"), node(1, "bottom"))]).contains("unique"));
}
