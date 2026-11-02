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

const AERO: &[Check] = &[Check::Aero];

pub const RULES: &[Rule] = &[
    Rule {
        row: "wing",
        columns: &["fairing"],
        except: false,
        expect: Expect::Grey("too narrow for what it holds"),
        why: "a wing a metre across does not fit in a fairing a metre wide: the fairing tool \
              refuses it",
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
