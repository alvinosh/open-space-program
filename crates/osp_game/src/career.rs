//! The game's side of a career ([`osp_sim::career`]): the tech tree the loaded records give, kept up
//! with them, and the career the editor builds for. A new career is kept here until its first
//! launch carries it into the universe and its save; one in progress is the universe's.

use std::collections::{BTreeMap, BTreeSet};

use bevy::prelude::*;
use osp_sim::{
    builder::Allowed,
    career::Career,
    data::{TechNodeDef, TechTreeDef},
    palette,
    tech::{ScienceTree, science_tree},
};

use crate::{
    content::{GameContent, catalog},
    menu::Menu,
    sim::Sim,
};

pub struct CareerPlugin;

impl Plugin for CareerPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<CareerTree>()
            .add_systems(PreUpdate, (follow_the_records, keep_the_new_career).chain());
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
    }
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
