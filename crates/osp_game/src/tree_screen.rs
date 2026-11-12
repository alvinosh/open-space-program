//! The tech tree screen (`layouts/tech_tree.json5`): a career's tree over everything, its nodes
//! laid out left to right (by tier, or by year with a column a year), a line from each to what it
//! builds on, each one unlocked, ready or locked. Drag looks round it, the wheel zooms about the
//! pointer. A click on a node picks it; the panel beside the tree says what it unlocks, what it
//! builds on and leads to, and unlocks it for its science.
//!
//! The tree is the `tech_tree` widget, drawn here from layout nodes made on the fly
//! ([`osp_ui::runtime::spawn_node`]), so its cards have the theme's look and their clicks are
//! actions like any other: `tree.pick:<node>`. It is drawn again when what is unlocked or the zoom
//! changes; moving it only moves it.
//!
//! Actions: `tree.open` (from the editor bar and the pause menu, in a career), `tree.close` (Back,
//! Esc), `tree.pick:<node>`, `tree.unlock` (the picked node), `tree.zoom:<in|out|fit>` (the buttons
//! in the corner), and `debug.science:<amount>`, which
//! gives the career science to try the tree with. Data: `career.active`, `career.science`,
//! `tree.picked`, `tree.notice`, and the picked node's `tree.node.*`.

use std::collections::BTreeSet;

use bevy::{
    input::mouse::MouseScrollUnit,
    picking::events::{PointerDrag, PointerScroll},
    prelude::*,
    ui::RelativeCursorPosition,
};
use osp_sim::tech::ScienceTree;
use osp_ui::{
    LayoutVisibility, NodeDef, UiAction, UiData, UiDocuments, UiRecord, WidgetContext,
    WidgetRegistry, record, runtime::spawn_node,
};
use serde_json::json;

use crate::{
    FrameSet,
    career::CareerTree,
    menu::{Menu, menu_actions},
    sim::Sim,
};

/// The screen's layout.
pub const LAYOUT: &str = "tech_tree";
/// A node's card at full size, px.
const CARD: Vec2 = Vec2::new(190.0, 72.0);
/// Between columns and between rows, px.
const GAP: Vec2 = Vec2::new(48.0, 14.0);
/// Round the tree, and the row of years over it, px.
const MARGIN: f32 = 32.0;
const HEADS: f32 = 30.0;
/// How far it zooms out and in.
const ZOOM: (f32, f32) = (0.3, 1.3);
/// The least zoom the tree is fitted at, which still reads.
const READABLE: f32 = 0.7;
/// How much of the tree stays in view however far it is dragged, px.
const KEEP: f32 = 160.0;

pub struct TreeScreenPlugin;

impl Plugin for TreeScreenPlugin {
    fn build(&self, app: &mut App) {
        app.world_mut()
            .resource_mut::<WidgetRegistry>()
            .register("tech_tree", tree_widget);
        app.init_resource::<TreeScreen>().add_systems(
            Update,
            (
                tree_actions.in_set(FrameSet::Input).after(menu_actions),
                (publish_tree, draw_tree).chain().in_set(FrameSet::Render),
            ),
        );
    }
}

/// The tree screen: whether it is open, the node picked, and how the tree is viewed.
#[derive(Resource)]
pub struct TreeScreen {
    open: bool,
    picked: Option<String>,
    /// Scale of the cards and the gaps between them.
    zoom: f32,
    /// Where the tree's top left corner is in the view, px.
    pan: Vec2,
    /// Fit the tree to the view when it is next drawn.
    fit: bool,
    notice: String,
    /// How big the view was when last drawn, px.
    view: Vec2,
}

impl Default for TreeScreen {
    fn default() -> Self {
        Self {
            open: false,
            picked: None,
            zoom: 1.0,
            pan: Vec2::ZERO,
            fit: true,
            notice: String::new(),
            view: Vec2::ZERO,
        }
    }
}

impl TreeScreen {
    /// It is open: Esc and Back are for it.
    pub fn is_open(&self) -> bool {
        self.open
    }

    /// Zoom by `factor` about `at`, a point of the view.
    fn zoom_about(&mut self, at: Vec2, factor: f32) {
        let before = self.zoom;
        let after = (before * factor).clamp(ZOOM.0, ZOOM.1);
        self.pan = at - (at - self.pan) * (after / before);
        self.zoom = after;
    }
}

/// The view the tree is drawn in, and what it was drawn for.
#[derive(Component, Default)]
struct TreeView {
    drawn: Option<Drawn>,
    content: Option<Entity>,
}

#[derive(Clone, PartialEq)]
struct Drawn {
    tree: String,
    unlocked: BTreeSet<String>,
    zoom: u32,
}

fn tree_widget(ctx: &mut WidgetContext) -> Entity {
    ctx.commands
        .entity(ctx.entity)
        .insert((TreeView::default(), RelativeCursorPosition::default()))
        .observe(pan)
        .observe(zoom);
    ctx.entity
}

fn pan(drag: On<PointerDrag>, mut screen: ResMut<TreeScreen>) {
    screen.pan += drag.delta;
}

/// Zoom about the point under the pointer.
fn zoom(
    mut scroll: On<PointerScroll>,
    mut screen: ResMut<TreeScreen>,
    views: Query<(&ComputedNode, &RelativeCursorPosition), With<TreeView>>,
) {
    scroll.propagate(false);
    let Ok((node, cursor)) = views.single() else {
        return;
    };
    let steps = match scroll.unit {
        MouseScrollUnit::Line => scroll.y,
        MouseScrollUnit::Pixel => scroll.y / 40.0,
    };
    let size = node.size() * node.inverse_scale_factor;
    let at = (cursor.normalized).map_or(size / 2.0, |n| (n + Vec2::splat(0.5)) * size);
    screen.zoom_about(at, 1.15_f32.powf(steps));
}

/// Where each shown node's card goes at zoom 1, its top left corner, px; and how big the whole
/// tree is.
fn places(tree: &ScienceTree) -> (Vec<(String, Vec2)>, Vec2) {
    let layout = tree.layout();
    let low = layout.values().map(|p| p[1]).fold(f64::INFINITY, f64::min);
    let heads = if tree.column_labels().is_empty() {
        0.0
    } else {
        HEADS
    };
    let at = |[column, row]: [f64; 2]| {
        Vec2::new(
            MARGIN + column as f32 * (CARD.x + GAP.x),
            MARGIN + heads + (row - low) as f32 * (CARD.y + GAP.y),
        )
    };
    let places: Vec<(String, Vec2)> = (layout.iter())
        .map(|(id, place)| (id.clone(), at(*place)))
        .collect();
    let far = (places.iter()).fold(Vec2::ZERO, |far, (_, p)| far.max(*p + CARD));
    (places, far + Vec2::splat(MARGIN))
}

/// How a node stands for a career that has `unlocked`.
#[derive(Clone, Copy, PartialEq)]
enum Standing {
    Unlocked,
    Ready,
    Locked,
}

fn standing(tree: &ScienceTree, id: &str, unlocked: &BTreeSet<String>) -> Standing {
    if unlocked.contains(id) {
        Standing::Unlocked
    } else if tree.can_unlock(id, unlocked) {
        Standing::Ready
    } else {
        Standing::Locked
    }
}

/// Science as a player reads it: whole, or to a tenth.
fn science(amount: f64) -> String {
    if (amount - amount.round()).abs() < 0.05 {
        format!("{amount:.0}")
    } else {
        format!("{amount:.1}")
    }
}

/// What a node unlocks, as a player reads it: its parts by name, then its other records.
fn unlocks(tree: &ScienceTree, career: &CareerTree, id: &str) -> Vec<String> {
    (tree.unlocks(id).iter())
        .map(|entry| career.name(entry).to_owned())
        .chain(tree.records(id).iter().cloned())
        .collect()
}

/// What a node lets procedural parts be made as big as, if anything.
fn sizes(tree: &ScienceTree, id: &str) -> Option<String> {
    let limits = tree.node(id)?.sizes?;
    let metres = |v: f64| format!("{v}").trim_end_matches(".0").to_owned();
    match (limits.diameter, limits.length) {
        (Some(d), Some(l)) => Some(format!(
            "Tanks up to {} m wide and {} m long",
            metres(d),
            metres(l)
        )),
        (Some(d), None) => Some(format!("Tanks up to {} m wide", metres(d))),
        (None, Some(l)) => Some(format!("Tanks up to {} m long", metres(l))),
        (None, None) => None,
    }
}

/// The tree as layout nodes at `zoom`: the years over the columns, the lines, then the cards.
fn content(
    tree: &ScienceTree,
    career: &CareerTree,
    unlocked: &BTreeSet<String>,
    zoom: f32,
    pan: Vec2,
) -> NodeDef {
    let (places, size) = places(tree);
    let place = |id: &str| (places.iter()).find(|(p, _)| p == id).map(|(_, at)| *at);
    let px = |v: f32| (v * zoom).round();
    let mut children = Vec::new();
    for (column, year) in tree.column_labels() {
        let x = MARGIN + column as f32 * (CARD.x + GAP.x);
        children.push(json!({
            "widget": "label", "class": "tree_year", "text": year,
            "style": { "position": "absolute", "left": px(x), "top": px(MARGIN - 8.0),
                       "font_size": (12.0 * zoom).max(8.0) },
        }));
    }
    let thick = (2.0 * zoom).max(1.0);
    let mut line = |from: Vec2, to: Vec2, lit: bool| {
        let (low, high) = (from.min(to), from.max(to));
        let class = if lit {
            "tree_line tree_line_lit"
        } else {
            "tree_line"
        };
        children.push(json!({
            "widget": "column", "class": class,
            "style": { "position": "absolute", "left": px(low.x) - thick / 2.0, "top": px(low.y) - thick / 2.0,
                       "width": px(high.x - low.x) + thick, "height": px(high.y - low.y) + thick },
        }));
    };
    for node in tree.shown() {
        let Some(at) = place(&node.id) else {
            continue;
        };
        for parent in tree.builds_on(&node.id) {
            let Some(from) = place(parent) else {
                continue;
            };
            let lit = unlocked.contains(parent);
            if from.x + CARD.x < at.x {
                // Out of the parent's right side, across in the gap before this one, into its left.
                let (y0, y1) = (from.y + CARD.y / 2.0, at.y + CARD.y / 2.0);
                let bend = at.x - GAP.x / 2.0;
                line(Vec2::new(from.x + CARD.x, y0), Vec2::new(bend, y0), lit);
                line(Vec2::new(bend, y0), Vec2::new(bend, y1), lit);
                line(Vec2::new(bend, y1), Vec2::new(at.x, y1), lit);
            } else {
                // The same year: down (or up) its left edge.
                let x = at.x + 12.0;
                let (y0, y1) = if from.y < at.y {
                    (from.y + CARD.y, at.y)
                } else {
                    (at.y + CARD.y, from.y)
                };
                line(Vec2::new(x, y0), Vec2::new(x, y1), lit);
            }
        }
    }
    let years = !tree.column_labels().is_empty();
    for node in tree.shown() {
        let Some(at) = place(&node.id) else {
            continue;
        };
        let stands = standing(tree, &node.id, unlocked);
        let class = match stands {
            Standing::Unlocked => "tree_card tree_card_unlocked",
            Standing::Ready => "tree_card tree_card_ready",
            Standing::Locked => "tree_card tree_card_locked",
        };
        let title = if node.title.is_empty() {
            &node.id
        } else {
            &node.title
        };
        let title_class = match stands {
            Standing::Locked => "tree_card_title tree_card_title_locked",
            _ => "tree_card_title",
        };
        let cost = format!("{} science", science(tree.cost(&node.id)));
        let detail = match (stands, node.year.filter(|_| years)) {
            (Standing::Unlocked, _) => "Unlocked".to_owned(),
            (_, Some(year)) => format!("{cost} · {year}"),
            (_, None) => cost,
        };
        let mut gives = unlocks(tree, career, &node.id);
        gives.extend(sizes(tree, &node.id));
        children.push(json!({
            "widget": "button", "class": class,
            "action": format!("tree.pick:{}", node.id),
            "active": format!("tree.picked == {}", node.id),
            "style": {
                "position": "absolute", "left": px(at.x), "top": px(at.y),
                "width": px(CARD.x), "height": px(CARD.y),
                "padding": [px(7.0), px(10.0)], "gap": px(2.0),
            },
            "children": [
                { "widget": "label", "class": title_class, "text": title,
                  "style": { "font_size": 15.0 * zoom } },
                { "widget": "label", "class": "tree_card_line", "text": detail,
                  "style": { "font_size": 12.0 * zoom } },
                { "widget": "label", "class": "tree_card_line", "text": gives.join(", "),
                  "style": { "font_size": 12.0 * zoom } },
            ],
        }));
    }
    let def = json!({
        "widget": "column", "class": "tree_content",
        "style": { "position": "absolute", "left": pan.x, "top": pan.y,
                   "width": px(size.x), "height": px(size.y) },
        "children": children,
    });
    serde_json::from_value(def).expect("the tree's nodes are layout nodes")
}

/// The zoom and the place that show as much of `tree` as is readable in a view `view` big: all of
/// it if it fits, else its start, the roots, at the left.
fn fit(tree: &ScienceTree, view: Vec2) -> (f32, Vec2) {
    let (_, size) = places(tree);
    let whole = (view / size).min_element();
    let zoom = whole.clamp(READABLE, 1.0);
    let shown = size * zoom;
    let pan = Vec2::new(
        if shown.x <= view.x {
            (view.x - shown.x) / 2.0
        } else {
            0.0
        },
        if shown.y <= view.y {
            (view.y - shown.y) / 2.0
        } else {
            0.0
        },
    );
    (zoom, pan)
}

#[allow(clippy::too_many_arguments)]
pub fn tree_actions(
    mut actions: MessageReader<UiAction>,
    mut screen: ResMut<TreeScreen>,
    mut ui: ResMut<LayoutVisibility>,
    mut career: ResMut<CareerTree>,
    menu: Res<Menu>,
    mut sim: ResMut<Sim>,
) {
    for action in actions.read() {
        match action.name() {
            "tree.open" if career.tree.is_some() => {
                screen.open = true;
                screen.fit = true;
                screen.notice.clear();
                // The cheapest it can unlock next, or the start.
                if screen.picked.is_none() {
                    let unlocked = career.unlocked(&menu, &sim).unwrap_or_default();
                    let tree = career.tree.as_ref();
                    screen.picked = tree.and_then(|t| {
                        (t.ready(&unlocked).first().map(|n| n.id.clone()))
                            .or_else(|| t.roots().into_iter().next())
                    });
                }
                ui.layouts.insert(LAYOUT.into(), true);
            }
            "tree.close" | "game.menu" | "menu.back" if screen.open => {
                screen.open = false;
                ui.layouts.insert(LAYOUT.into(), false);
            }
            // The buttons in the corner: about the middle of the view, or all of it in view.
            "tree.zoom" => match action.arg() {
                Some("fit") => screen.fit = true,
                Some(way) => {
                    let factor = if way == "in" { 1.25 } else { 0.8 };
                    let middle = screen.view / 2.0;
                    screen.zoom_about(middle, factor);
                }
                None => {}
            },
            "tree.pick" => {
                screen.picked = action.arg().map(str::to_owned);
                screen.notice.clear();
            }
            "tree.unlock" => {
                let Some(node) = screen.picked.clone() else {
                    continue;
                };
                let CareerTree { tree, pending, .. } = &mut *career;
                let Some(tree) = tree.as_ref() else {
                    continue;
                };
                let playing = if menu.new_game {
                    pending.as_mut().filter(|_| menu.career)
                } else {
                    sim.universe.career.as_mut()
                };
                screen.notice = match playing.map(|c| c.unlock(&node, tree)) {
                    Some(Ok(())) => String::new(),
                    Some(Err(e)) => format!("Not unlocked: {e}"),
                    None => "Not unlocked: this game is not a career".into(),
                };
            }
            "debug.science" => {
                let amount = action.arg().and_then(|a| a.parse::<f64>().ok());
                if let (Some(amount), Some(playing)) = (amount, career.current_mut(&menu, &mut sim))
                {
                    playing.science += amount;
                }
            }
            _ => {}
        }
    }
}

/// Whether the game is a career, its science, and what the panel says of the picked node.
#[allow(clippy::too_many_arguments)]
fn publish_tree(
    screen: Res<TreeScreen>,
    career: Res<CareerTree>,
    menu: Res<Menu>,
    sim: Res<Sim>,
    mut data: ResMut<UiData>,
    mut published: Local<Option<(Option<String>, Option<BTreeSet<String>>, u64, String, bool)>>,
) {
    let unlocked = career.unlocked(&menu, &sim);
    let have = career.current(&menu, &sim).map_or(0.0, |c| c.science);
    let now = (
        screen.picked.clone(),
        unlocked.clone(),
        have.to_bits(),
        screen.notice.clone(),
        career.tree.is_some(),
    );
    if published.as_ref() == Some(&now) && !career.is_changed() {
        return;
    }
    *published = Some(now);
    let active = career.tree.is_some() && unlocked.is_some();
    data.set("career.active", active);
    data.set("career.science", science(have));
    data.set("tree.notice", screen.notice.as_str());
    let (Some(tree), Some(unlocked)) = (career.tree.as_ref(), unlocked) else {
        data.set("tree.node.picked", false);
        return;
    };
    let picked = (screen.picked.as_deref()).and_then(|id| tree.node(id));
    data.set("tree.picked", screen.picked.as_deref().unwrap_or(""));
    data.set("tree.node.picked", picked.is_some());
    let Some(node) = picked else {
        return;
    };
    let id = node.id.as_str();
    let title = |n: &osp_sim::data::TechNodeDef| {
        if n.title.is_empty() {
            n.id.clone()
        } else {
            n.title.clone()
        }
    };
    let cost = tree.cost(id);
    let stands = standing(tree, id, &unlocked);
    let years = !tree.column_labels().is_empty();
    let tier = tree.layout().get(id).map_or(0.0, |p| p[0]);
    let when = match node.year.filter(|_| years) {
        Some(year) => format!("{year} · {} science", science(cost)),
        None => format!("Tier {tier} · {} science", science(cost)),
    };
    let waiting: Vec<String> = (tree.builds_on(id).iter())
        .filter(|p| !unlocked.contains(*p))
        .filter_map(|p| tree.node(p).map(title))
        .collect();
    let joined = |names: &[String]| match names {
        [] => String::new(),
        [one] => one.clone(),
        [rest @ .., last] => format!("{} and {last}", rest.join(", ")),
    };
    let state = match stands {
        Standing::Unlocked => "Unlocked: its parts are in the parts list.".to_owned(),
        Standing::Ready if have + 1e-9 >= cost => "Ready to unlock.".to_owned(),
        Standing::Ready => format!(
            "Ready to unlock once there is {} more science.",
            science(cost - have)
        ),
        Standing::Locked if node.any_parent => {
            format!("Needs one of {} first.", waiting.join(" or "))
        }
        Standing::Locked => format!("Needs {} first.", joined(&waiting)),
    };
    let can =
        stands == Standing::Ready && have + 1e-9 >= cost && career.current(&menu, &sim).is_some();
    let blocked = match stands {
        Standing::Unlocked => "Unlocked".to_owned(),
        Standing::Ready => format!("Needs {} more science", science(cost - have)),
        Standing::Locked => "Needs what it builds on".to_owned(),
    };
    let linked = |nodes: Vec<&osp_sim::data::TechNodeDef>| -> Vec<UiRecord> {
        (nodes.into_iter())
            .map(|n| {
                let state = match standing(tree, &n.id, &unlocked) {
                    Standing::Unlocked => "Unlocked".to_owned(),
                    _ => format!("{} science", science(tree.cost(&n.id))),
                };
                record([
                    ("id", n.id.as_str().into()),
                    ("title", title(n).into()),
                    ("state", state.into()),
                ])
            })
            .collect()
    };
    let parents: Vec<_> = (tree.builds_on(id).iter())
        .filter_map(|p| tree.node(p))
        .collect();
    let parts: Vec<UiRecord> = (unlocks(tree, &career, id).into_iter())
        .map(|name| record([("name", name.into())]))
        .collect();
    data.set("tree.node.title", title(node));
    data.set("tree.node.when", when);
    data.set("tree.node.state", state);
    data.set("tree.node.description", node.description.as_str());
    data.set("tree.node.parts", parts);
    data.set("tree.node.sizes", sizes(tree, id).unwrap_or_default());
    data.set("tree.node.builds_on", linked(parents));
    data.set("tree.node.leads_to", linked(tree.leads_to(id)));
    data.set("tree.node.cost", format!("{} science", science(cost)));
    data.set("tree.node.can_unlock", can);
    data.set(
        "tree.node.blocked",
        if can { String::new() } else { blocked },
    );
}

/// Draw the tree in its view when it is open: again when what is unlocked or the zoom changes,
/// only moved when it is dragged.
#[allow(clippy::too_many_arguments)]
fn draw_tree(
    mut commands: Commands,
    mut screen: ResMut<TreeScreen>,
    career: Res<CareerTree>,
    menu: Res<Menu>,
    sim: Res<Sim>,
    (widgets, docs): (Res<WidgetRegistry>, Res<UiDocuments>),
    mut views: Query<(Entity, &mut TreeView, &ComputedNode)>,
    mut nodes: Query<&mut Node>,
) {
    if !screen.open {
        return;
    }
    let Some(tree) = career.tree.as_ref() else {
        return;
    };
    let unlocked = career.unlocked(&menu, &sim).unwrap_or_default();
    for (entity, mut view, computed) in &mut views {
        let size = computed.size() * computed.inverse_scale_factor;
        if size.min_element() <= 0.0 {
            continue;
        }
        if screen.view != size {
            screen.view = size;
        }
        if screen.fit {
            (screen.zoom, screen.pan) = fit(tree, size);
            screen.fit = false;
        }
        // Never dragged out of sight.
        let shown = places(tree).1 * screen.zoom;
        let least = Vec2::splat(KEEP) - shown;
        let most = size - Vec2::splat(KEEP);
        let pan = screen.pan.max(least.min(most)).min(most.max(least));
        if pan != screen.pan {
            screen.pan = pan;
        }
        let drawn = Drawn {
            tree: tree.def.id.clone(),
            unlocked: unlocked.clone(),
            zoom: screen.zoom.to_bits(),
        };
        if view.drawn.as_ref() != Some(&drawn) || docs.is_changed() {
            if let Some(old) = view.content.take() {
                commands.entity(old).despawn();
            }
            let def = content(tree, &career, &unlocked, screen.zoom, screen.pan);
            let root = spawn_node(
                &mut commands,
                &widgets,
                &docs.theme,
                LAYOUT,
                &def,
                Some(entity),
            );
            view.content = Some(root);
            view.drawn = Some(drawn);
            continue;
        }
        if let Some(mut node) = view.content.and_then(|c| nodes.get_mut(c).ok()) {
            let (left, top) = (Val::Px(screen.pan.x), Val::Px(screen.pan.y));
            if node.left != left || node.top != top {
                node.left = left;
                node.top = top;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every node shown gets a card, no two cards overlap, and the tree fits in what it says.
    #[test]
    fn every_node_has_a_place_of_its_own() {
        let core = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let content = crate::content::GameContent(osp_mods::load(&core, None));
        let (tree, _) = crate::career::tree_of(&content);
        let tree = tree.expect("the base game has a tree");
        let (places, size) = places(&tree);
        assert_eq!(places.len(), tree.shown().len());
        for (i, (a, at)) in places.iter().enumerate() {
            assert!(at.x + CARD.x < size.x && at.y + CARD.y < size.y, "{a}");
            for (b, other) in &places[i + 1..] {
                let apart = (at.x - other.x).abs() >= CARD.x || (at.y - other.y).abs() >= CARD.y;
                assert!(apart, "{a} and {b} overlap");
            }
        }
        // A big view shows all of it, centred; a small one starts readable at the left.
        let (zoom, pan) = fit(&tree, Vec2::new(4000.0, 3000.0));
        assert_eq!(zoom, 1.0);
        assert!(pan.x > 0.0 && pan.y > 0.0);
        let (zoom, pan) = fit(&tree, Vec2::new(400.0, 300.0));
        assert_eq!((zoom, pan.x), (READABLE, 0.0));
    }
}
