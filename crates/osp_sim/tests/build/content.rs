//! The game's content, loaded once for every cell: the part catalogue, the solar system and
//! the launch sites a cell is flown on.

use std::{path::Path, sync::OnceLock};

use osp_sim::data::{LaunchSiteDef, PartCatalog, ScenarioDef, SystemDef};

use crate::record;

pub struct Content {
    pub catalog: PartCatalog,
    pub system: SystemDef,
    pub sites: Vec<LaunchSiteDef>,
    /// The pad scenario, whose craft a cell replaces.
    pub scenario: ScenarioDef,
}

/// Every mod, as the game loads them.
pub fn get() -> &'static Content {
    static CONTENT: OnceLock<Content> = OnceLock::new();
    CONTENT.get_or_init(|| {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
        let registry = &content.registry;
        let mut diagnostics = Vec::new();
        let system = SystemDef {
            bodies: registry.all("bodies", &mut diagnostics),
        };
        let sites = registry.all("launch_sites", &mut diagnostics);
        Content {
            catalog: record::catalog(registry),
            system,
            sites,
            scenario: registry.get("scenarios", "pad").unwrap(),
        }
    })
}
