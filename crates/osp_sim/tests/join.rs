//! Any two craft can join at any free nodes and come apart again, neither of them changed.

use std::{collections::BTreeSet, path::Path};

use osp_sim::{
    DQuat, DVec3,
    attitude::mass_properties,
    data::{CraftDef, PartCatalog},
    parts::PartId,
    propulsion::{Port, Propulsion},
    staging::Activation,
};

#[path = "golden/record.rs"]
#[allow(dead_code)]
mod record;

/// Every craft in the game, as flown.
fn fleet() -> Vec<(String, Propulsion)> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
    let registry = &content.registry;
    let catalog: PartCatalog = record::catalog(registry);
    let craft: Vec<CraftDef> = registry.all("craft", &mut Vec::new());
    let fleet: Vec<_> = craft
        .iter()
        .filter_map(|def| Some((def.id.clone(), Propulsion::from_craft(def, &catalog).ok()?)))
        .collect();
    assert!(fleet.len() >= 20, "only {} craft", fleet.len());
    fleet
}

/// A small generator, so that a failure comes back the same.
struct Dice(u64);

impl Dice {
    fn roll(&mut self, below: usize) -> usize {
        self.0 = self.0.wrapping_mul(6_364_136_223_846_793_005);
        self.0 = self.0.wrapping_add(1_442_695_040_888_963_407);
        ((self.0 >> 33) % below.max(1) as u64) as usize
    }
}

/// Everything about a vessel's parts and what they are doing: what must come back.
#[derive(Debug, PartialEq)]
struct Print {
    facts: Vec<String>,
    figures: Vec<(String, f64)>,
}

fn vector(figures: &mut Vec<(String, f64)>, name: String, v: DVec3) {
    figures.push((format!("{name}.x"), v.x));
    figures.push((format!("{name}.y"), v.y));
    figures.push((format!("{name}.z"), v.z));
}

fn print(p: &Propulsion) -> Print {
    let mut facts = Vec::new();
    let mut figures = Vec::new();
    for part in p.parts().parts() {
        let connectors: Vec<_> = (part.connectors.iter())
            .map(|c| (&c.name, c.kind, c.capabilities))
            .collect();
        facts.push(format!(
            "part {} {} stage {} joint {:?} connectors {connectors:?} kinds {} frame {:?}",
            part.id.0,
            part.name,
            part.stage,
            part.joint,
            part.kinds.len(),
            part.frame,
        ));
        vector(
            &mut figures,
            format!("part {} at", part.id.0),
            part.position,
        );
        // Two quaternions can be the same turn.
        vector(
            &mut figures,
            format!("part {} x", part.id.0),
            part.rotation * DVec3::X,
        );
        vector(
            &mut figures,
            format!("part {} z", part.id.0),
            part.rotation * DVec3::Z,
        );
        for system in p.systems.on(part.id) {
            facts.push(format!("part {} system {}", part.id.0, system.kind()));
            for reading in system.state() {
                figures.push((format!("part {} {}", part.id.0, reading.id), reading.value));
            }
        }
        facts.push(format!(
            "part {} actions {:?}",
            part.id.0,
            p.part_actions(part.id, false)
        ));
    }
    for e in &p.engines {
        facts.push(format!(
            "engine {} {} section {} running {} ignitions {:?} armed {}",
            e.part.0,
            e.def.id,
            e.section,
            e.running,
            e.ignitions_left,
            p.engine_armed(e.part),
        ));
        vector(&mut figures, format!("engine {} at", e.part.0), e.mount);
        vector(
            &mut figures,
            format!("engine {} points", e.part.0),
            e.direction,
        );
    }
    for b in &p.rcs {
        facts.push(format!("rcs {} section {}", b.part.0, b.section));
        vector(&mut figures, format!("rcs {} at", b.part.0), b.mount);
        vector(
            &mut figures,
            format!("rcs {} x", b.part.0),
            b.rotation * DVec3::X,
        );
        vector(
            &mut figures,
            format!("rcs {} z", b.part.0),
            b.rotation * DVec3::Z,
        );
    }
    for t in &p.tanks {
        facts.push(format!("tank {} section {}", t.part.0, t.section));
        for (k, amount) in t.amounts.iter().enumerate() {
            figures.push((format!("tank {} holds {k}", t.part.0), *amount));
        }
    }
    for s in &p.sections {
        facts.push(format!(
            "section {} {} parent {:?} beside {} engines {}",
            s.id, s.name, s.parent, s.beside, s.engines,
        ));
        vector(&mut figures, format!("section {} bottom", s.id), s.bottom);
        figures.push((format!("section {} length", s.id), s.length));
        figures.push((format!("section {} dry", s.id), s.dry_mass()));
    }
    for (k, row) in p.stage_info().iter().enumerate() {
        figures.push((format!("row {k} delta-v"), row.delta_v_vac));
        figures.push((format!("row {k} burn"), row.burn_time));
    }
    facts.push(format!("sequences {:?}", p.sequences.list()));
    facts.push(format!("next number {}", p.sequences.number(0)));
    facts.push(format!("firing {:?}", p.firing()));
    facts.push(format!("fluid {:?}", p.parts().fluid_groups()));
    facts.push(format!("power {:?}", p.parts().power_groups()));
    let bottom: Vec<_> = p.bottom().map(|s| s.id).collect();
    facts.push(format!("bottom {bottom:?}"));
    facts.push(format!("joined {:?}", p.joined_parts().collect::<Vec<_>>()));
    figures.push(("throttle".into(), p.throttle()));
    figures.push(("mass".into(), p.mass()));
    figures.push(("length".into(), p.length()));
    let mass = mass_properties(p);
    figures.push(("com.x".into(), mass.center_of_mass.x));
    figures.push(("com.y".into(), mass.center_of_mass.y));
    figures.push(("com.z".into(), mass.center_of_mass.z));
    let tensor = mass.inertia.tensor;
    for (axis, column) in [tensor.x_axis, tensor.y_axis, tensor.z_axis]
        .iter()
        .enumerate()
    {
        vector(&mut figures, format!("inertia {axis}"), *column);
    }
    Print { facts, figures }
}

fn assert_same(before: &Print, after: &Print, what: &str) {
    for (a, b) in before.facts.iter().zip(&after.facts) {
        assert_eq!(a, b, "{what}");
    }
    assert_eq!(before.facts.len(), after.facts.len(), "{what}");
    assert_eq!(before.figures.len(), after.figures.len(), "{what}");
    for ((name, a), (other, b)) in before.figures.iter().zip(&after.figures) {
        assert_eq!(name, other, "{what}");
        // To the bit: going home is not an approximation of where they were.
        assert_eq!(a.to_bits(), b.to_bits(), "{what}: {name} was {a}, is {b}");
    }
}

/// A craft's free node, chosen by `dice`.
fn pick(p: &Propulsion, dice: &mut Dice) -> Option<(PartId, String)> {
    let free = p.parts().free_nodes();
    (!free.is_empty()).then(|| free[dice.roll(free.len())].clone())
}

/// Part of a craft's flight, chosen by `dice`: a few of its sequences fired.
fn flown(p: &Propulsion, dice: &mut Dice) -> Propulsion {
    let mut p = p.clone();
    for _ in 0..dice.roll(3) {
        p.activate_next_sequence();
    }
    p
}

#[test]
fn any_two_craft_join_and_come_apart_unchanged() {
    let fleet = fleet();
    let mut dice = Dice(1);
    let mut joins = 0;
    for (a_name, a) in &fleet {
        for (b_name, b) in &fleet {
            let (a, b) = (flown(a, &mut dice), flown(b, &mut dice));
            let (Some(at), Some(to)) = (pick(&a, &mut dice), pick(&b, &mut dice)) else {
                continue;
            };
            let roll = dice.roll(628) as f64 / 100.0;
            let what = format!("{b_name} {to:?} on {a_name} {at:?} turned {roll}");
            let (a_print, b_print) = (print(&a), print(&b));
            let (a_mass, b_mass) = (a.mass(), b.mass());

            let mut both = a.clone();
            let port = both
                .join(
                    b.clone(),
                    Port {
                        part: at.0,
                        node: &at.1,
                    },
                    Port {
                        part: to.0,
                        node: &to.1,
                    },
                    roll,
                )
                .unwrap_or_else(|e| panic!("{what}: {e}"));
            joins += 1;
            assert!((both.mass() - (a_mass + b_mass)).abs() < 1e-6, "{what}");
            let tree = both.parts();
            assert_eq!(
                tree.parts().len(),
                a.parts().parts().len() + b.parts().parts().len(),
                "{what}"
            );
            assert_eq!(
                tree.subtree(tree.root().unwrap().id).len(),
                tree.parts().len(),
                "{what}: not all one tree"
            );
            // Neither gets in the way of the other: what could fire on each still can.
            let firing = both.firing();
            assert!(a.firing().iter().all(|e| firing.contains(e)), "{what}");
            // It takes the other's engines off, and flies by its own.
            let bottom: BTreeSet<_> = both.bottom().map(|s| s.id).collect();
            assert_eq!(bottom, a.bottom().map(|s| s.id).collect(), "{what}");

            let staged = both
                .perform(Activation::undock(port), false)
                .unwrap_or_else(|| panic!("{what}: nothing lets go"));
            let [released] = <[_; 1]>::try_from(staged.released)
                .unwrap_or_else(|_| panic!("{what}: not one piece"));
            let back = Propulsion::come_home(released);
            assert_same(
                &a_print,
                &print(&both),
                &format!("{what}: the one that stayed"),
            );
            assert_same(
                &b_print,
                &print(&back),
                &format!("{what}: the one that left"),
            );
        }
    }
    assert!(joins > 1000, "only {joins} joins");
}

/// The node `node` of `part`, where it is in the vessel's frame and which way it faces.
fn node_at(p: &Propulsion, part: PartId, node: &str) -> (DVec3, DVec3) {
    let found = p.parts().get(part).unwrap();
    let built = found.built.as_ref().unwrap();
    let node = (built.nodes.iter())
        .chain(&built.geometry.nodes)
        .find(|n| n.name == node)
        .unwrap();
    (
        found.position + found.rotation * node.position,
        found.rotation * node.direction,
    )
}

#[test]
fn the_nodes_meet_face_to_face_at_any_roll() {
    let fleet = fleet();
    let mut dice = Dice(7);
    for (_, a) in &fleet {
        for (_, b) in &fleet {
            let (Some(at), Some(to)) = (pick(a, &mut dice), pick(b, &mut dice)) else {
                continue;
            };
            let roll = dice.roll(628) as f64 / 100.0;
            let mut both = a.clone();
            let port = both
                .join(
                    b.clone(),
                    Port {
                        part: at.0,
                        node: &at.1,
                    },
                    Port {
                        part: to.0,
                        node: &to.1,
                    },
                    roll,
                )
                .unwrap();
            let (near, out) = node_at(&both, at.0, &at.1);
            let (far, back) = node_at(&both, port, &to.1);
            assert!((near - far).length() < 1e-9, "{near} {far}");
            assert!((out + back).length() < 1e-9, "{out} {back}");
        }
    }
}

/// The Saturn V and the lunar module, for what the Moon mission does with a join.
fn apollo() -> (Propulsion, Propulsion) {
    let fleet = fleet();
    let find = |id: &str| fleet.iter().find(|(n, _)| n == id).unwrap().1.clone();
    (find("saturn_v"), find("lunar_module"))
}

/// A top node nothing holds, on the highest part that has one.
fn top_node(p: &Propulsion) -> (PartId, String) {
    let tree = p.parts();
    tree.free_nodes()
        .into_iter()
        .filter(|(_, node)| node == "top")
        .max_by(|(a, _), (b, _)| {
            let height = |id| tree.get(id).unwrap().position.z;
            height(*a).total_cmp(&height(*b))
        })
        .expect("a free top node")
}

/// A stack turned round on the nose of another: its parts point the other way, its engines
/// still push along their own axes, and neither blocks the other. Staging it from there lets go
/// of the piece on the far end, the right way up to itself.
#[test]
fn a_stack_turned_round_flies_and_sheds_pieces_in_their_own_frame() {
    let (csm_side, lander) = apollo();
    let mut stack = csm_side.clone();
    // Up to the stage that is on top of the whole stack.
    while stack.sections.len() > 1 {
        let staged = stack.activate_next_sequence().unwrap();
        if staged.released.is_empty() && stack.sequences.is_empty() {
            break;
        }
    }
    let (at, to) = (top_node(&stack), top_node(&lander));
    let turned_axis = {
        let mut both = stack.clone();
        both.join(
            lander.clone(),
            Port {
                part: at.0,
                node: &at.1,
            },
            Port {
                part: to.0,
                node: &to.1,
            },
            0.0,
        )
        .unwrap();
        // The lander stands on its head on the stack.
        let engine = (both.engines.iter())
            .find(|e| e.def.id.contains("LMDE") || e.def.id.contains("LM"))
            .expect("the lander's engine");
        engine.direction
    };
    assert!(turned_axis.z < -0.999, "{turned_axis}");

    // A lander with its stages: its descent stage comes off the far end.
    let mut both = stack.clone();
    let port = both
        .join(
            lander.clone(),
            Port {
                part: at.0,
                node: &at.1,
            },
            Port {
                part: to.0,
                node: &to.1,
            },
            0.0,
        )
        .unwrap();
    assert_eq!(both.joined_parts().collect::<Vec<_>>(), [port]);
    let lander_sections = lander.sections.len();
    assert_eq!(both.sections.len(), stack.sections.len() + lander_sections);
    let descent = both.sequences.list().iter().flatten().copied().find(|a| {
        a.is_decoupler()
            && both
                .parts()
                .get(a.part())
                .is_some_and(|p| p.stage > stack.sections.iter().map(|s| s.id).max().unwrap())
    });
    let descent = descent.expect("the lander's own staging, among the sequences");
    let staged = both.perform(descent, false).unwrap();
    assert_eq!(staged.released.len(), 1);
    let piece = Propulsion::come_home(staged.released.into_iter().next().unwrap());
    // In its own frame again: everything of it upright, as the lander's own parts were.
    for part in piece.parts().parts() {
        assert_eq!(
            part.frame,
            osp_sim::assembly::Pose::IDENTITY,
            "{}",
            part.name
        );
    }
    let z = |q: DQuat| q * DVec3::Z;
    for engine in &piece.engines {
        assert!(z(piece.parts().get(engine.part).unwrap().rotation).z > 0.99);
    }
    // What is left still undocks, a lander without its descent stage.
    let staged = both.perform(Activation::undock(port), false).unwrap();
    let ascent = Propulsion::come_home(staged.released.into_iter().next().unwrap());
    assert!(ascent.mass() < lander.mass());
    assert_same(&print(&stack), &print(&both), "the stack that stayed");
    for part in ascent.parts().parts() {
        let own = lander.parts().get(part.id).expect("its own id");
        assert_eq!(own.name, part.name);
        assert!(
            (own.position - part.position).length() < 1e-9,
            "{}",
            part.name
        );
    }
}

/// One vessel on another on a third: the middle one takes the third with it, and each comes
/// back to itself in the end.
#[test]
fn a_vessel_that_leaves_takes_what_joined_it() {
    let fleet = fleet();
    let find = |id: &str| fleet.iter().find(|(n, _)| n == id).unwrap().1.clone();
    let (a, b, c) = (find("pathfinder"), find("satellite"), find("lunar_module"));
    let (a_print, b_print, c_print) = (print(&a), print(&b), print(&c));
    let free = |p: &Propulsion, k: usize| p.parts().free_nodes()[k].clone();

    // The third joins the middle one, then the middle one joins the first.
    let mut middle = b.clone();
    let (b_at, c_to) = (free(&middle, 0), free(&c, 0));
    let c_port = middle
        .join(
            c.clone(),
            Port {
                part: b_at.0,
                node: &b_at.1,
            },
            Port {
                part: c_to.0,
                node: &c_to.1,
            },
            1.0,
        )
        .unwrap();
    let (a_at, b_to) = (top_node(&a), free(&b, 1));
    let mut all = a.clone();
    let b_port = all
        .join(
            middle,
            Port {
                part: a_at.0,
                node: &a_at.1,
            },
            Port {
                part: b_to.0,
                node: &b_to.1,
            },
            2.0,
        )
        .unwrap();
    assert_eq!(all.joined_parts().count(), 2);

    let staged = all.perform(Activation::undock(b_port), false).unwrap();
    assert_same(&a_print, &print(&all), "the first");
    let mut middle = Propulsion::come_home(staged.released.into_iter().next().unwrap());
    assert_eq!(middle.joined_parts().collect::<Vec<_>>(), [c_port]);
    let staged = middle.perform(Activation::undock(c_port), false).unwrap();
    assert_same(&b_print, &print(&middle), "the middle one");
    let third = Propulsion::come_home(staged.released.into_iter().next().unwrap());
    assert_same(&c_print, &print(&third), "the third");
}

/// A node holds one joint.
#[test]
fn a_node_that_holds_a_joint_cannot_join() {
    let fleet = fleet();
    let a = fleet[0].1.clone();
    let b = fleet[1].1.clone();
    let (at, to) = (
        a.parts().free_nodes()[0].clone(),
        b.parts().free_nodes()[0].clone(),
    );
    let mut both = a.clone();
    both.join(
        b.clone(),
        Port {
            part: at.0,
            node: &at.1,
        },
        Port {
            part: to.0,
            node: &to.1,
        },
        0.0,
    )
    .unwrap();
    let other = b.parts().free_nodes()[0].clone();
    let before = print(&both);
    let error = both
        .join(
            b.clone(),
            Port {
                part: at.0,
                node: &at.1,
            },
            Port {
                part: other.0,
                node: &other.1,
            },
            0.0,
        )
        .unwrap_err();
    assert!(error.contains("holds a joint"), "{error}");
    assert_same(&before, &print(&both), "after the refused join");
    // A node on the craft's own joints is no more free.
    let held = a
        .parts()
        .parts()
        .iter()
        .find_map(|part| Some((part.id, part.connectors[part.joint?.connector].name.clone())))
        .unwrap();
    assert!(
        a.join_frame(
            Port {
                part: held.0,
                node: &held.1,
            },
            &b,
            Port {
                part: to.0,
                node: &to.1,
            },
            0.0,
        )
        .is_err()
    );
}

/// Only a payload rides above its holder's base: a stage hanging under another, as nearly all do,
/// does not count, in any craft in the game.
#[test]
fn only_the_lander_in_its_adapter_is_carried() {
    for (id, p) in fleet() {
        let carried: Vec<&str> = (p.sections.iter())
            .filter(|s| s.carried)
            .map(|s| s.name.as_str())
            .collect();
        let want: &[&str] = if id == "saturn_v" {
            &["Lunar Module"]
        } else {
            &[]
        };
        assert_eq!(carried, want, "{id}");
    }
}

/// A payload rides in the stack and does nothing until it has let go: the stages round it fire past
/// it, it takes what it still has to do with it when the stage carrying it leaves, and once docked
/// with it comes out on the nose of the vessel that came for it.
#[test]
fn the_lander_rides_in_its_adapter_until_it_is_let_go() {
    let (mut stack, _) = apollo();
    let lander = |p: &Propulsion| {
        (p.sections.iter())
            .find(|s| s.name == "Lunar Module")
            .map(|s| s.id)
            .expect("the lander")
    };
    let id = lander(&stack);
    assert!(stack.carried(id) && !stack.flown(id));
    let index = (stack.sections.iter()).position(|s| s.id == id).unwrap();
    assert!(!stack.clear(index), "held in by the adapter");
    assert!(osp_sim::gear::legs(&stack).is_empty(), "stands on nothing");
    // Up to the S-IVB, which fires with the lander above it.
    for _ in 0..6 {
        stack.activate_next_sequence().unwrap();
    }
    stack.set_throttle(1.0);
    let firing: Vec<&str> = (stack.engines.iter())
        .filter(|e| stack.firing().contains(&e.part))
        .map(|e| e.def.id.as_str())
        .collect();
    assert!(firing.iter().any(|e| e.contains("J-2")), "{firing:?}");
    // Nothing that arms what is at the bottom of the stack arms it.
    stack.arm_bottom();
    assert!(
        (stack.engines.iter())
            .filter(|e| stack.carried(e.section))
            .all(|e| !stack.firing().contains(&e.part)),
    );
    stack.set_throttle(0.0);

    // The panels and the S-IVB leave, the S-IVB with the lander and all its staging.
    let staged = stack.activate_next_sequence().unwrap();
    assert_eq!(staged.released.len(), 5, "four panels and the S-IVB");
    assert_eq!(stack.sequences.list().len(), 1, "the service module's own");
    let carrier = Propulsion::from_released(staged.released.into_iter().last().unwrap());
    assert_eq!(carrier.sequences.list().len(), 3);
    assert!(carrier.carried(lander(&carrier)));
    assert!(carrier.parts().has_seats());
    assert!(!stack.engines.iter().any(|e| e.def.id.contains("LM")));

    // Docked: nose to nose, still in the adapter.
    let free = |p: &Propulsion| {
        let port = (p.docking_ports().iter())
            .find(|d| d.free)
            .cloned()
            .unwrap();
        (port.part, port.node)
    };
    let (at, to) = (free(&stack), free(&carrier));
    let mut both = stack.clone();
    let port = both
        .join(
            carrier.clone(),
            Port {
                part: at.0,
                node: &at.1,
            },
            Port {
                part: to.0,
                node: &to.1,
            },
            0.0,
        )
        .unwrap();
    let id = lander(&both);
    assert!(both.carried(id));

    // Let go of what holds it: the S-IVB goes, and the lander is no longer carried.
    let tree = both.parts();
    let hold = (tree.parts().iter())
        .filter(|p| !both.carried(p.stage))
        .filter(|p| tree.parent(p.id).is_some_and(|h| both.carried(h.stage)))
        .flat_map(|p| both.part_actions(p.id, false))
        .find(|a| a.is_decoupler())
        .expect("the hold can be let go of");
    let staged = both.perform(hold, false).unwrap();
    assert_eq!(staged.released.len(), 1);
    let left = Propulsion::from_released(staged.released.into_iter().next().unwrap());
    assert!(!left.parts().has_seats(), "the S-IVB, with nobody on it");
    assert!(!both.carried(lander(&both)));
    // And it comes off the nose as a lander with its two stages' staging still to do.
    let staged = both.perform(Activation::undock(port), false).unwrap();
    let flying = Propulsion::come_home(staged.released.into_iter().next().unwrap());
    assert!(!flying.carried(lander(&flying)));
    assert_eq!(flying.sequences.list().len(), 2);
    assert!(osp_sim::gear::legs(&flying).is_empty(), "still folded");
    let mut flying = flying;
    flying.activate_next_sequence().unwrap();
    assert_eq!(osp_sim::gear::legs(&flying).len(), 4);
}
