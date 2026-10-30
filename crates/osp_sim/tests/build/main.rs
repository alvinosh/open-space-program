//! Build smoke tests: a placement matrix of every component the sim has against every way
//! to put a part on a craft. Each cell builds a small craft and checks it (see `checks.rs`);
//! the page at `target/smoke/build/index.html` draws the matrix. Written first, while most
//! of it fails: a red cell names the task that is meant to fix it (`expect.rs`), and when
//! that task lands the cell must flip.
//!
//! ```sh
//! cargo smoke build
//! ```
//!
//! See `docs/smoke-tests.md`.

mod cell;
mod checks;
mod content;
mod expect;
mod report;

#[path = "../smoke/site.rs"]
mod site;

#[path = "../golden/record.rs"]
#[allow(dead_code)]
mod record;

use checks::{CellResult, Check, Outcome, Pad};
use expect::Expect;

/// A cell run, with what was expected of it.
pub struct Run {
    pub row: usize,
    pub column: usize,
    pub result: CellResult,
    pub expect: Expect,
    pub why: &'static str,
    /// Why it is not as expected, if it is not.
    pub mismatch: Option<String>,
}

fn failures(result: &CellResult) -> String {
    let what: Vec<String> = result
        .checks
        .iter()
        .filter_map(|(c, o)| match o {
            Outcome::Fail(d) => Some(format!("{}: {d}", c.title())),
            _ => None,
        })
        .collect();
    what.join("; ")
}

/// Whether the cell did what its rule said.
fn judge(result: &CellResult, expect: Expect) -> Option<String> {
    let failed = result.failed();
    match expect {
        Expect::Green if failed.is_empty() && !result.grey() => None,
        Expect::Green if result.grey() => Some(format!(
            "expected green, refused: {}",
            result.refusal.as_ref().unwrap().reason
        )),
        Expect::Green => Some(format!("expected green, but {}", failures(result))),
        Expect::Grey(rule) => match &result.refusal {
            Some(r) if !r.limit && r.reason.contains(rule) => None,
            Some(r) => Some(format!(
                "expected a refusal naming `{rule}`, got: {}",
                r.reason
            )),
            None => Some(format!("expected a refusal naming `{rule}`, but it loaded")),
        },
        Expect::Red(checks, task) => {
            let want: Vec<Check> = checks.to_vec();
            let fixed: Vec<_> = want.iter().filter(|c| !failed.contains(c)).collect();
            let new: Vec<_> = failed.iter().filter(|c| !want.contains(c)).collect();
            if !fixed.is_empty() {
                Some(format!(
                    "{task} was to make {fixed:?} pass, and it does: remove or change the rule"
                ))
            } else if !new.is_empty() {
                Some(format!("also fails {new:?}: {}", failures(result)))
            } else {
                None
            }
        }
    }
}

pub fn run_matrix() -> Vec<Run> {
    let mut pad = Pad::new();
    let mut runs = Vec::new();
    for (r, row) in cell::ROWS.iter().enumerate() {
        for (c, column) in cell::COLUMNS.iter().enumerate() {
            let result = checks::run(row, *column, &mut pad);
            let (expect, why) = expect::expected(row, *column);
            let mismatch = judge(&result, expect);
            runs.push(Run {
                row: r,
                column: c,
                result,
                expect,
                why,
                mismatch,
            });
        }
    }
    runs
}

#[test]
fn build_matrix() {
    let runs = run_matrix();
    let path = report::write(&runs);
    let red = runs
        .iter()
        .filter(|r| matches!(r.expect, Expect::Red(..)))
        .count();
    eprintln!(
        "\nBuild: {} cells, {} red, {} grey, {} wrong\nreport: {}\n",
        runs.len(),
        red,
        runs.iter()
            .filter(|r| matches!(r.expect, Expect::Grey(_)))
            .count(),
        runs.iter().filter(|r| r.mismatch.is_some()).count(),
        path.display(),
    );
    if std::env::var_os("OSP_SMOKE_OPEN").is_some() {
        let opener = if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        };
        let _ = std::process::Command::new(opener).arg(&path).status();
    }
    let mut problems: Vec<String> = runs
        .iter()
        .filter_map(|run| {
            run.mismatch.as_ref().map(|why| {
                format!(
                    "{} × {}: {why}",
                    cell::ROWS[run.row].title,
                    cell::COLUMNS[run.column].title()
                )
            })
        })
        .collect();
    problems.extend(report::closed_tasks(&runs));
    assert!(problems.is_empty(), "{}", problems.join("\n"));
}

/// The checks are only worth having if they can fail: give each the physics of one craft and
/// the parts of another.
#[test]
fn build_checks_can_fail() {
    use osp_sim::{craft::Craft, propulsion::Propulsion};

    let catalog = &content::get().catalog;
    let fails = |outcome: Outcome| matches!(outcome, Outcome::Fail(_));
    let storage = cell::ROWS.iter().find(|r| r.id == "storage").unwrap();

    let built = cell::build(storage, cell::Column::Top).unwrap();
    let craft = Craft::build(&built.def, catalog).unwrap();
    let alone = Craft::build(&built.baseline, catalog).unwrap();
    let without = Propulsion::from_craft(&built.baseline, catalog).unwrap();
    assert!(fails(checks::kept(&craft, &without)), "kept");
    assert!(fails(checks::mass(&craft, &without)), "mass");
    assert!(fails(checks::balance(&craft, &without)), "balance");
    assert!(fails(checks::save(&built.def, &alone)), "save");

    // Outside the skin, and the air sees no difference: nothing was placed.
    let unchanged = cell::Built {
        def: built.baseline.clone(),
        baseline: built.baseline.clone(),
        subject: built.subject.clone(),
    };
    assert!(
        fails(checks::aero(
            storage,
            cell::Column::Top,
            &unchanged,
            &without
        )),
        "aero outside"
    );
    // Inside a fairing, and the air sees a difference: a tank left out in the open.
    let open = cell::Built {
        def: built.def.clone(),
        baseline: built.baseline.clone(),
        subject: built.subject.clone(),
    };
    let with = Propulsion::from_craft(&built.def, catalog).unwrap();
    assert!(
        fails(checks::aero(storage, cell::Column::Inside, &open, &with)),
        "aero inside"
    );

    // The pad: a craft of several stages, weighed without its tank.
    let staged = cell::build(storage, cell::Column::Fairing).unwrap();
    let craft = Craft::build(&staged.baseline, catalog).unwrap();
    let propulsion = Propulsion::from_craft(&staged.def, catalog).unwrap();
    let mut pad = Pad::new();
    assert!(
        fails(checks::on_the_pad(&staged, &craft, &propulsion, &mut pad)),
        "pad"
    );
}
