//! Draws the Casual and Realistic tech trees, as the game loads them, to
//! `target/tech_trees/<mode>.svg`, to look at before the game's own screen shows them.
//!
//! `cargo run -p osp_sim --example tech_tree_preview`

use std::{collections::BTreeMap, fmt::Write as _, path::Path};

use osp_sim::{
    data::{TechNodeDef, TechTreeDef},
    tech::{ScienceTree, science_tree},
};

#[path = "../tests/golden/record.rs"]
#[allow(dead_code)]
mod record;

const WIDTH: f64 = 190.0;
const HEIGHT: f64 = 64.0;
const GAP_X: f64 = 60.0;
const GAP_Y: f64 = 14.0;

fn content(root: &Path, realistic: bool) -> osp_mods::Content {
    let core = root.join("assets/core");
    if !realistic {
        return osp_mods::load(&core, None);
    }
    let mods = root.join("mods");
    let base = osp_mods::load(&core, None);
    let mode: serde_json::Value = base.registry.get("game_modes", "realistic").unwrap();
    let ids: Vec<String> = serde_json::from_value(mode["mods"].clone()).unwrap();
    let choices = osp_mods::exactly(&osp_mods::installed(&mods), &ids);
    osp_mods::load_chosen(&core, Some(&mods), &choices)
}

fn escape(text: &str) -> String {
    text.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn svg(tree: &ScienceTree, names: &BTreeMap<String, String>) -> String {
    let layout = tree.layout();
    let (min_row, max_row, max_column) = layout.values().fold(
        (f64::INFINITY, f64::NEG_INFINITY, 0.0_f64),
        |(lo, hi, right), [column, row]| (lo.min(*row), hi.max(*row), right.max(*column)),
    );
    let at = |id: &str| {
        let [column, row] = layout[id];
        (
            20.0 + column * (WIDTH + GAP_X),
            50.0 + (row - min_row) * (HEIGHT + GAP_Y),
        )
    };
    let width = 40.0 + (max_column + 1.0) * (WIDTH + GAP_X);
    let height = 80.0 + (max_row - min_row + 1.0) * (HEIGHT + GAP_Y);
    let mut out = String::new();
    let _ = writeln!(
        out,
        r##"<svg xmlns="http://www.w3.org/2000/svg" width="{width}" height="{height}" font-family="Helvetica, Arial, sans-serif"><rect width="100%" height="100%" fill="#10151c"/>"##
    );
    let _ = writeln!(
        out,
        r##"<text x="20" y="30" fill="#e8eef5" font-size="20">{} tree: {} nodes shown</text>"##,
        escape(&tree.def.name),
        layout.len()
    );
    // Lines first, from the right edge of what a node builds on to its left edge.
    for node in tree.shown() {
        let (x, y) = at(&node.id);
        for parent in tree.builds_on(&node.id) {
            let (px, py) = at(parent);
            let (x0, y0, x1, y1) = (px + WIDTH, py + HEIGHT / 2.0, x, y + HEIGHT / 2.0);
            let bend = (x1 - x0).abs().max(40.0) / 2.0;
            let _ = writeln!(
                out,
                r##"<path d="M{x0},{y0} C{},{y0} {},{y1} {x1},{y1}" stroke="#4a6378" stroke-width="1.5" fill="none"/>"##,
                x0 + bend,
                x1 - bend
            );
        }
    }
    let roots = tree.roots();
    for node in tree.shown() {
        let (x, y) = at(&node.id);
        let parts = tree.unlocks(&node.id);
        let fill = if roots.contains(&node.id) {
            "#1f4d3a"
        } else {
            "#1b2733"
        };
        let title = if node.title.is_empty() {
            &node.id
        } else {
            &node.title
        };
        let detail = match node.year {
            Some(year) => format!("{} science · {year}", tree.cost(&node.id)),
            None => format!("{} science", tree.cost(&node.id)),
        };
        let shown: Vec<&str> = (parts.iter())
            .map(|p| names.get(p).map_or(p.as_str(), String::as_str))
            .collect();
        let mut list = shown.join(", ");
        if list.chars().count() > 30 {
            list = format!("{}…", list.chars().take(29).collect::<String>());
        }
        let _ = writeln!(
            out,
            r##"<g><title>{}</title><rect x="{x}" y="{y}" width="{WIDTH}" height="{HEIGHT}" rx="6" fill="{fill}" stroke="#3fb6a8" stroke-width="1"/><text x="{}" y="{}" fill="#e8eef5" font-size="13" font-weight="bold">{}</text><text x="{}" y="{}" fill="#9fb3c4" font-size="11">{}</text><text x="{}" y="{}" fill="#7f95a8" font-size="11">{}</text></g>"##,
            escape(&format!("{}: {}", node.description, shown.join(", "))),
            x + 8.0,
            y + 18.0,
            escape(title),
            x + 8.0,
            y + 36.0,
            escape(&detail),
            x + 8.0,
            y + 54.0,
            escape(&list),
        );
    }
    out.push_str("</svg>\n");
    out
}

fn main() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
    let out = root.join("target/tech_trees");
    std::fs::create_dir_all(&out).unwrap();
    for (name, realistic) in [("casual", false), ("realistic", true)] {
        let content = content(&root, realistic);
        let catalog = record::catalog(&content.registry);
        let mut problems = Vec::new();
        let trees: Vec<TechTreeDef> = content.registry.all("tech_trees", &mut problems);
        let nodes: Vec<TechNodeDef> = content.registry.all("tech_nodes", &mut problems);
        let tree = science_tree(&catalog, &trees, &nodes)
            .unwrap()
            .expect("a tree");
        let names: BTreeMap<String, String> = (osp_sim::palette::entries(&catalog).into_iter())
            .map(|e| (e.id, e.name))
            .collect();
        let path = out.join(format!("{name}.svg"));
        std::fs::write(&path, svg(&tree, &names)).unwrap();
        let entries: usize = tree.shown().iter().map(|n| tree.unlocks(&n.id).len()).sum();
        println!(
            "{name}: {} nodes shown, {entries} parts, the dearest {} science: {}",
            tree.shown().len(),
            tree.shown().last().map_or(0.0, |n| tree.cost(&n.id)),
            path.display()
        );
    }
}
