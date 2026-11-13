//! The game's side of a career ([`osp_sim::career`]): the tech tree and facilities the loaded records
//! give, kept up with them, and the career the editor builds for. A new career is kept here until
//! its first launch carries it into the universe and its save; one in progress is the universe's.
//!
//! Actions: `career.upgrade:<facility>` (pays for the facility's next level), `debug.funds:<amount>`
//! (for captures). Data: `career.facilities` (each `id`, `name`, `level` ("Level 1 of 4"), `limits`,
//! `has_next`, `next` (the next level's limits), `price` and `affordable`), `career.notice`.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use osp_sim::{
    builder::Allowed,
    career::{Career, money},
    data::{FacilityDef, TechNodeDef, TechTreeDef},
    palette,
    tech::{ScienceTree, science_tree},
};
use osp_ui::{UiAction, UiData, record};

use crate::{
    FrameSet,
    content::{GameContent, catalog},
    menu::{Menu, menu_actions},
    sim::Sim,
};

pub struct CareerPlugin;

impl Plugin for CareerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CareerTree>()
            .add_systems(PreUpdate, (follow_the_records, keep_the_new_career).chain())
            .add_systems(
                Update,
                (
                    career_actions.in_set(FrameSet::Input).after(menu_actions),
                    publish_facilities.in_set(FrameSet::Render),
                ),
            );
    }
}

/// The tree a career is played on with the records loaded now (the one their rules name, if they
/// name one and it can be built), the names of the parts it unlocks, and a new career not launched
/// yet.
#[derive(Resource, Default)]
pub struct CareerTree {
    pub tree: Option<ScienceTree>,
    /// Each parts-list entry's name, by its id.
    pub names: BTreeMap<String, String>,
    /// The career a new game will be, until its first launch.
    pub pending: Option<Career>,
    /// What a career launches and builds with.
    pub facilities: Vec<FacilityDef>,
    /// What the last upgrade did, or why it did not.
    pub notice: String,
}

impl CareerTree {
    /// The career the editor builds for: a new one, or the one in progress; none in a sandbox.
    pub fn current<'a>(&'a self, menu: &Menu, sim: &'a Sim) -> Option<&'a Career> {
        if menu.new_game {
            return self.pending.as_ref().filter(|_| menu.career);
        }
        sim.universe.career.as_ref()
    }

    /// The same, to change: unlock a node, add science.
    pub fn current_mut<'a>(&'a mut self, menu: &Menu, sim: &'a mut Sim) -> Option<&'a mut Career> {
        if menu.new_game {
            return self.pending.as_mut().filter(|_| menu.career);
        }
        sim.universe.career.as_mut()
    }

    /// What the game the editor builds for has unlocked: the career's nodes, the tree's roots for a
    /// new career, or None for a sandbox, which has every part.
    pub fn unlocked(&self, menu: &Menu, sim: &Sim) -> Option<BTreeSet<String>> {
        if let Some(career) = self.current(menu, sim) {
            return Some(career.unlocked.clone());
        }
        (menu.new_game && menu.career)
            .then(|| self.tree.as_ref().map(ScienceTree::roots))
            .flatten()
    }

    /// What the game the editor builds for may build: anything in a sandbox, a career's sizes and
    /// tank types otherwise.
    pub fn allowed(&self, menu: &Menu, sim: &Sim) -> Allowed {
        match (&self.tree, self.unlocked(menu, sim)) {
            (Some(tree), Some(unlocked)) => tree.allowed(&unlocked),
            _ => Allowed::everything(),
        }
    }

    /// A parts-list entry's name.
    pub fn name<'a>(&'a self, entry: &'a str) -> &'a str {
        self.names.get(entry).map_or(entry, String::as_str)
    }
}

/// The tree of the records in `content`, and the names of their parts-list entries.
pub fn tree_of(content: &GameContent) -> (Option<ScienceTree>, BTreeMap<String, String>) {
    let registry = &content.0.registry;
    let mut problems = Vec::new();
    let catalog = catalog(registry, &mut problems);
    let trees: Vec<TechTreeDef> = registry.all("tech_trees", &mut problems);
    let nodes: Vec<TechNodeDef> = registry.all("tech_nodes", &mut problems);
    let names = (palette::entries(&catalog).into_iter())
        .map(|e| (e.id, e.name))
        .collect();
    match science_tree(&catalog, &trees, &nodes) {
        Ok(tree) => (tree, names),
        Err(e) => {
            warn!("no tech tree for careers: {e}");
            (None, names)
        }
    }
}

fn follow_the_records(content: Res<GameContent>, mut career: ResMut<CareerTree>) {
    if content.is_changed() {
        (career.tree, career.names) = tree_of(&content);
        let registry = &content.0.registry;
        career.facilities = registry.all::<FacilityDef>("facilities", &mut Vec::new());
    }
}

fn career_actions(
    mut actions: MessageReader<UiAction>,
    menu: Res<Menu>,
    mut sim: ResMut<Sim>,
    mut career: ResMut<CareerTree>,
) {
    for action in actions.read() {
        match action.name() {
            "career.upgrade" => {
                let id = action.arg().unwrap_or_default();
                let Some(facility) = career.facilities.iter().find(|f| f.id == id).cloned() else {
                    warn!("no facility `{id}`");
                    continue;
                };
                career.notice = match career.current_mut(&menu, &mut sim) {
                    Some(playing) => match playing.upgrade(&facility) {
                        Ok(done) => done,
                        Err(why) => format!("Not upgraded: {why}"),
                    },
                    None => "Not upgraded: this game is not a career".into(),
                };
            }
            "debug.funds" => {
                let amount = action.arg().and_then(|a| a.parse::<f64>().ok());
                if let (Some(amount), Some(playing)) = (amount, career.current_mut(&menu, &mut sim))
                {
                    playing.earn(amount);
                }
            }
            _ => {}
        }
    }
}

/// `career.facilities` and `career.notice`, when what they show changes.
fn publish_facilities(
    menu: Res<Menu>,
    sim: Res<Sim>,
    career: Res<CareerTree>,
    mut data: ResMut<UiData>,
    mut published: Local<Option<(Vec<usize>, u64, String)>>,
) {
    let playing = career.current(&menu, &sim);
    let levels: Vec<usize> = (career.facilities.iter())
        .map(|f| playing.map_or(usize::MAX, |c| c.level(f)))
        .collect();
    let funds = playing.map_or(0.0, |c| c.funds);
    let now = (levels, funds.to_bits(), career.notice.clone());
    if published.as_ref() == Some(&now) && !career.is_changed() {
        return;
    }
    *published = Some(now);
    data.set("career.notice", career.notice.as_str());
    let Some(playing) = playing else {
        data.set("career.facilities", Vec::<osp_ui::UiRecord>::new());
        return;
    };
    let rows: Vec<_> = (career.facilities.iter())
        .filter(|f| !f.levels.is_empty())
        .map(|facility| {
            let at = playing.level(facility);
            let next = facility.levels.get(at + 1);
            let price = next.map_or(0.0, |n| n.price);
            record([
                ("id", facility.id.as_str().into()),
                ("name", facility.title().into()),
                (
                    "level",
                    format!("Level {} of {}", at + 1, facility.levels.len()).into(),
                ),
                ("limits", facility.levels[at].describe().into()),
                ("has_next", next.is_some().into()),
                ("next", next.map_or(String::new(), |n| n.describe()).into()),
                ("price", money(price).into()),
                (
                    "affordable",
                    (next.is_some() && price <= playing.funds).into(),
                ),
            ])
        })
        .collect();
    data.set("career.facilities", rows);
}

/// A new career starts with its tree's roots and no science as soon as New game says it is one, and
/// is gone when the new game is.
fn keep_the_new_career(menu: Res<Menu>, mut career: ResMut<CareerTree>) {
    let wanted = menu.new_game && menu.career;
    let tree = career.tree.as_ref().map(|t| t.def.id.clone());
    let fits = (career.pending.as_ref()).is_some_and(|c| Some(&c.tree) == tree.as_ref());
    if wanted && !fits {
        let fresh = career.tree.as_ref().map(Career::new);
        career.pending = fresh;
    } else if !wanted && career.pending.is_some() {
        career.pending = None;
    }
}
