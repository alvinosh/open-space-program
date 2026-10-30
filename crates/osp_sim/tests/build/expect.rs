//! What each cell is expected to do today. A red cell names the task that is meant to fix
//! it; when that task lands the cell passes, and the test then fails until the rule below
//! is removed (the expectation flips). A cell that fails in a way no rule expects fails the
//! test at once.
//!
//! The rules are read top to bottom and the first that matches a cell is its expectation.
//! A cell no rule matches is expected to be green.

use crate::{
    cell::{Column, Row},
    checks::Check,
};

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Expect {
    Green,
    /// The sim refuses with a rule of building, whose text has this in it.
    Grey(&'static str),
    /// These checks fail, until the task makes them pass.
    Red(&'static [Check], &'static str),
}

/// A rule: the cells it is about, what is expected of them, and why.
pub struct Rule {
    pub row: &'static str,
    /// Columns by id; empty for all of them.
    pub columns: &'static [&'static str],
    /// The rule is about every column but these.
    pub except: bool,
    pub expect: Expect,
    pub why: &'static str,
}

const LOADS: &[Check] = &[Check::Loads];
const KEPT: &[Check] = &[Check::Kept];
const AERO: &[Check] = &[Check::Aero];

pub const RULES: &[Rule] = &[
    // A stage is the parts between joints that let go, with one body: a container with a
    // bottom and a top node, standing upright. A second container is a second body.
    Rule {
        row: "container",
        columns: &[],
        except: false,
        expect: Expect::Red(LOADS, "T014"),
        why: "the stage gatherer allows one container with a bottom and a top node, so a \
              second is refused",
    },
    Rule {
        row: "gear",
        columns: &[],
        except: false,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer only takes a gear leg standing straight out of its stage's side",
    },
    Rule {
        row: "wing",
        columns: &["fairing"],
        except: false,
        expect: Expect::Grey("too narrow for what it holds"),
        why: "a wing a metre across does not fit in a fairing a metre wide: the fairing tool \
              refuses it",
    },
    Rule {
        row: "wing",
        columns: &[],
        except: false,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer only takes a wing untwisted, uncambered and with its root on its \
              stage's side, and a mirrored one only beside its partner",
    },
    Rule {
        row: "engine",
        columns: &["bottom"],
        except: true,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer only takes an engine standing upright and firing down its stage",
    },
    Rule {
        row: "rcs",
        columns: &["radial", "ring2", "ring3", "ring4", "ring6", "mirror"],
        except: false,
        expect: Expect::Red(AERO, "T014"),
        why: "the block's catalogue model has no surface of its own for the air, so it is \
              left out of the outline",
    },
    Rule {
        row: "rcs",
        columns: &[],
        except: false,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer only takes an RCS block standing on its stage's rim",
    },
    Rule {
        row: "storage",
        columns: &["top", "bottom", "inside", "fairing"],
        except: true,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer only takes a tank standing upright",
    },
    Rule {
        row: "battery",
        columns: &[],
        except: false,
        expect: Expect::Red(KEPT, "T014"),
        why: "the physics has no battery: the part weighs but its charge is dropped",
    },
    Rule {
        row: "crew",
        columns: &[],
        except: false,
        expect: Expect::Red(KEPT, "T014"),
        why: "only a pod seats a crew: the seats of any other part are dropped",
    },
    Rule {
        row: "pod",
        columns: &["fairing"],
        except: false,
        expect: Expect::Grey("too narrow for what it holds"),
        why: "a capsule 1.9 m wide does not fit a fairing 1 m wide: the fairing tool refuses it",
    },
    Rule {
        row: "pod",
        columns: &["inside"],
        except: false,
        expect: Expect::Red(AERO, "T015"),
        why: "a capsule 1.9 m wide is put inside a container 1 m wide, sticks out through its \
              wall, and nothing refuses it: the builder's fit check will",
    },
    Rule {
        row: "pod",
        columns: &["top", "bottom"],
        except: true,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer only takes a pod standing upright on the axis of its stage",
    },
    Rule {
        row: "two",
        columns: &[],
        except: false,
        expect: Expect::Red(LOADS, "T014"),
        why: "the gatherer takes a part that is one thing (a tank, an engine, an RCS block): a \
              tank with thrusters is \"a stage has no part like it\"",
    },
    Rule {
        row: "decoupler",
        columns: &["bottom"],
        except: false,
        expect: Expect::Red(LOADS, "T014"),
        why: "a decoupler holding nothing below is a stage with no body, which the gatherer \
              refuses",
    },
];

/// What is expected of the cell, and the rule's reason.
pub fn expected(row: &Row, column: Column) -> (Expect, &'static str) {
    let id = column.id();
    for rule in RULES {
        let listed = rule.columns.is_empty() || rule.columns.contains(&id.as_str());
        if rule.row == row.id && listed != rule.except {
            return (rule.expect, rule.why);
        }
    }
    (Expect::Green, "")
}
