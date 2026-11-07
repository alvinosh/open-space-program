//! The parts list in the editor's rail, as KSP's: the palette's entries (`osp_sim::palette`) as a
//! grid of pictures under tabs by what they do (`editor.group:<group>`), or those a search finds in
//! every tab (`editor.search:<text>`), narrowed by how wide they are (`editor.size:<size>`) and by
//! what they burn (`editor.propellant:<kind>`); `any` takes either filter off, as a click on the
//! choice that is on does. A part with no width or propellant of its own, as the procedural ones
//! that take what they go on, is never filtered out. The pictures are the renderer's
//! (`osp_game::part_pictures`): each part drawn once, as it starts. Pointing at a tile shows its
//! figures and what it is under the grid. A click on a tile (`editor.add:<entry>`) or a drag out
//! of it (`palette:<entry>`) takes the part in hand, which is [`crate::building`]'s.
//!
//! Publishes `editor.palette_open`; `editor.group`, the tab, or `search` while searching;
//! `editor.palette`, each `id`, `name` (its config where parts share a name) and `held`, at most
//! [`MOST_TILES`]; `editor.palette_more`; `editor.search_shown`; the filters, `editor.size` and
//! `editor.propellant` (the one on, or `any`) with their choices `editor.sizes` and
//! `editor.propellants` (each `id`, `label`, `on` and the `action` a click fires; only what the
//! tab has to narrow by), shown while `editor.sizes_shown` and `editor.propellants_shown`; and what
//! is pointed at, `editor.pointed` with `editor.pointed_name`, `editor.pointed_detail` and
//! `editor.pointed_about`.
//!
//! The `part_picture` widget (`part: "{item.id}"`) shows the picture of an entry.

use std::collections::HashMap;

use bevy::prelude::*;
use osp_game::{
    FrameSet,
    content::{GameContent, catalog},
    part_pictures::PartPictures,
};
use osp_sim::palette::{self, Entry, Group, Propellant};
use osp_ui::{
    Formatters, Template, UiAction, UiData, UiHover, UiRecord, WidgetContext, WidgetRegistry,
    record,
};

use crate::{building::Placing, hangar::Editor};

/// The most tiles the list shows at once.
pub const MOST_TILES: usize = 60;
/// What a tile says it is while the pointer is over it (`hover`): `part:<entry>`.
const POINTED: &str = "part:";
/// The widths the list narrows to: as actions write each, from and up to, m, and what the player
/// reads.
const SIZES: [(&str, f64, f64, &str); 5] = [
    ("under_1", 0.0, 1.0, "<1 m"),
    ("1_2", 1.0, 2.0, "1–2 m"),
    ("2_3", 2.0, 3.0, "2–3 m"),
    ("3_4", 3.0, 4.0, "3–4 m"),
    ("over_4", 4.0, f64::INFINITY, "4 m+"),
];

pub struct PartsListPlugin;

impl Plugin for PartsListPlugin {
    fn build(&self, app: &mut App) {
        if let Some(mut widgets) = app.world_mut().get_resource_mut::<WidgetRegistry>() {
            widgets.register("part_picture", part_picture);
        }
        app.init_resource::<PartsList>().add_systems(
            Update,
            (
                list_actions.in_set(FrameSet::Input),
                (publish_list, show_pictures).chain(),
            ),
        );
    }
}

/// What the list shows: a tab or a search, narrowed by the filters.
#[derive(Resource, Debug, Clone, PartialEq)]
struct PartsList {
    group: Group,
    search: String,
    /// Which of [`SIZES`], if one.
    size: Option<usize>,
    propellant: Option<Propellant>,
}

impl Default for PartsList {
    fn default() -> Self {
        Self {
            group: Group::Pods,
            search: String::new(),
            size: None,
            propellant: None,
        }
    }
}

impl PartsList {
    /// The entries of the tab, or those the search finds, before the filters.
    fn chosen<'a>(&self, entries: &'a [Entry]) -> Vec<&'a Entry> {
        let search = self.search.trim();
        (entries.iter())
            .filter(|e| match search {
                "" => e.group == self.group,
                _ => e.matches(search),
            })
            .collect()
    }

    /// Whether `entry` is left by the filters.
    fn keeps(&self, entry: &Entry) -> bool {
        let wide = |(_, from, to, _): &(&str, f64, f64, &str)| {
            entry.width.is_none_or(|w| (*from..*to).contains(&w))
        };
        let burns = |p: Propellant| entry.propellant.is_none_or(|q| q == p);
        self.size.is_none_or(|s| wide(&SIZES[s])) && self.propellant.is_none_or(burns)
    }
}

fn list_actions(mut list: ResMut<PartsList>, mut actions: MessageReader<UiAction>) {
    for action in actions.read() {
        let arg = action.arg().unwrap_or("");
        match action.name() {
            "editor.group" => {
                if let Some(group) = Group::parse(arg) {
                    list.group = group;
                    list.search.clear();
                }
            }
            "editor.search" => list.search = arg.trim().to_owned(),
            "editor.size" => list.size = SIZES.iter().position(|s| s.0 == arg),
            "editor.propellant" => list.propellant = Propellant::parse(arg),
            _ => {}
        }
    }
}

/// The palette, with what each tile is called: its name, or where other entries share that name
/// (an engine's configurations), its own id.
struct Palette {
    entries: Vec<Entry>,
    labels: HashMap<String, String>,
}

impl Palette {
    fn new(entries: Vec<Entry>) -> Self {
        let mut named: HashMap<&str, usize> = HashMap::new();
        for entry in &entries {
            *named.entry(entry.name.as_str()).or_default() += 1;
        }
        let labels = (entries.iter())
            .map(|e| {
                let own = e.id.split_once(':').map_or(e.id.as_str(), |(_, id)| id);
                let label = if named[e.name.as_str()] > 1 {
                    own
                } else {
                    &e.name
                };
                (e.id.clone(), label.to_owned())
            })
            .collect();
        Self { entries, labels }
    }

    fn label<'a>(&'a self, entry: &'a Entry) -> &'a str {
        self.labels
            .get(&entry.id)
            .map_or(&entry.name, String::as_str)
    }
}

/// A filter's choices, each by id and label, for the filter `action` writes, with the one `on` lit:
/// a click puts one on, or takes it off again if it is on.
fn choices<'a>(
    action: &str,
    on: Option<&str>,
    all: impl Iterator<Item = (&'a str, &'a str)>,
) -> Vec<UiRecord> {
    all.map(|(id, label)| {
        let lit = on == Some(id);
        let to = if lit { "any" } else { id };
        record([
            ("id", id.into()),
            ("label", label.into()),
            ("on", lit.into()),
            ("action", format!("{action}:{to}").into()),
        ])
    })
    .collect()
}

/// The start of what is said of a part, as much as fits a few lines under the grid: whole
/// sentences if they fit, or words up to an ellipsis.
fn brief(about: &str) -> String {
    const MOST: usize = 150;
    if about.chars().count() <= MOST {
        return about.to_owned();
    }
    let cut: String = about.chars().take(MOST).collect();
    if let Some(end) = cut.rfind(". ").filter(|end| *end >= MOST / 3) {
        return cut[..=end].to_owned();
    }
    let words = cut.rfind(' ').map_or(cut.as_str(), |at| &cut[..at]);
    format!("{}…", words.trim_end_matches([',', ';', ':']))
}

/// See the module.
#[allow(clippy::too_many_arguments, clippy::type_complexity)]
fn publish_list(
    list: Res<PartsList>,
    (placing, editor): (Res<Placing>, Res<Editor>),
    content: Res<GameContent>,
    hover: Res<UiHover>,
    mut pictures: ResMut<PartPictures>,
    mut data: ResMut<UiData>,
    mut palette: Local<Option<Palette>>,
    mut published: Local<Option<(PartsList, bool, Option<String>)>>,
    mut pointed_at: Local<Option<Option<String>>>,
) {
    if palette.is_none() || content.is_changed() {
        let parts = catalog(&content.0.registry, &mut Vec::new());
        *palette = Some(Palette::new(palette::entries(&parts)));
        *published = None;
        *pointed_at = None;
    }
    let Some(palette) = palette.as_ref() else {
        return;
    };

    // What the pointer is on.
    let pointed = (hover.0.as_deref())
        .and_then(|h| h.strip_prefix(POINTED))
        .and_then(|id| palette.entries.iter().find(|e| e.id == id));
    let pointed_id = pointed.map(|e| e.id.clone());
    if pointed_at.as_ref() != Some(&pointed_id) {
        *pointed_at = Some(pointed_id);
        data.set("editor.pointed", pointed.is_some());
        let say = |text: Option<&str>| text.unwrap_or_default().to_owned();
        data.set(
            "editor.pointed_name",
            say(pointed.map(|e| palette.label(e))),
        );
        data.set(
            "editor.pointed_detail",
            say(pointed.map(|e| e.detail.as_str())),
        );
        data.set(
            "editor.pointed_about",
            pointed.map_or(String::new(), |e| brief(&e.about)),
        );
    }

    let open = placing.is_open() && editor.is_open();
    let now = (list.clone(), open, placing.held_entry().map(str::to_owned));
    if published.as_ref() == Some(&now) {
        return;
    }
    *published = Some(now);
    data.set("editor.palette_open", open);
    let searching = !list.search.trim().is_empty();
    data.set(
        "editor.group",
        if searching { "search" } else { list.group.id() },
    );
    data.set(
        "editor.search_shown",
        if searching {
            list.search.as_str()
        } else {
            "Search parts"
        },
    );

    let chosen = list.chosen(&palette.entries);
    let shown: Vec<&Entry> = chosen.iter().copied().filter(|e| list.keeps(e)).collect();
    let held = placing.held_entry();
    let tiles: Vec<UiRecord> = (shown.iter().take(MOST_TILES))
        .map(|e| {
            record([
                ("id", e.id.clone().into()),
                ("name", palette.label(e).into()),
                ("held", (held == Some(e.id.as_str())).into()),
            ])
        })
        .collect();
    data.set("editor.palette", tiles);
    let more = shown.len().saturating_sub(MOST_TILES);
    data.set(
        "editor.palette_more",
        if more > 0 {
            format!("{more} more: narrow them down or search")
        } else if shown.is_empty() {
            "Nothing found".to_owned()
        } else {
            String::new()
        },
    );
    pictures.want(
        (shown.iter().take(MOST_TILES))
            .map(|e| (e.id.clone(), e.part.clone()))
            .collect(),
    );

    // The filters, with only what the tab has to narrow by, and what is on whether it has or not.
    let size = list.size.map(|s| SIZES[s].0);
    data.set("editor.size", size.unwrap_or("any"));
    data.set(
        "editor.sizes",
        choices("editor.size", size, SIZES.iter().map(|s| (s.0, s.3))),
    );
    data.set(
        "editor.sizes_shown",
        chosen.iter().any(|e| e.width.is_some()),
    );
    let propellant = list.propellant;
    let mut burnt: Vec<Propellant> = chosen.iter().filter_map(|e| e.propellant).collect();
    burnt.extend(propellant);
    burnt.sort();
    burnt.dedup();
    data.set(
        "editor.propellant",
        propellant.map_or("any", Propellant::id),
    );
    data.set(
        "editor.propellants",
        choices(
            "editor.propellant",
            propellant.map(Propellant::id),
            burnt.iter().map(|p| (p.id(), p.name())),
        ),
    );
    data.set("editor.propellants_shown", !burnt.is_empty());
}

/// A node showing the picture of the palette entry its `part` names.
#[derive(Component)]
struct PartPicture(Template);

/// `widget: "part_picture"`: the picture of the entry `part` names (`"{item.id}"`), once it is
/// drawn; size it with the style's `width` and `height`.
fn part_picture(ctx: &mut WidgetContext) -> Entity {
    let part = ctx.def.extra.get("part").and_then(|v| v.as_str());
    let template = Template::parse(part.unwrap_or_default());
    ctx.commands
        .entity(ctx.entity)
        .insert(PartPicture(template));
    ctx.entity
}

/// Put each picture in its node once it is drawn, and tell the renderer whether any is on screen.
fn show_pictures(
    mut commands: Commands,
    mut pictures: ResMut<PartPictures>,
    data: Res<UiData>,
    formatters: Res<Formatters>,
    mut nodes: Query<(Entity, &PartPicture, Option<&mut ImageNode>, &ComputedNode)>,
) {
    let mut shown = false;
    for (entity, picture, image, computed) in &mut nodes {
        shown |= computed.size().x > 0.0;
        let drawn = pictures.picture(&picture.0.render(&data, &formatters));
        match (drawn, image) {
            (Some((atlas, rect)), Some(mut image)) => {
                if image.image != atlas || image.rect != Some(rect) {
                    image.image = atlas;
                    image.rect = Some(rect);
                }
            }
            (Some((atlas, rect)), None) => {
                let image = ImageNode {
                    rect: Some(rect),
                    ..ImageNode::new(atlas)
                };
                commands.entity(entity).try_insert(image);
            }
            (None, Some(_)) => {
                commands.entity(entity).try_remove::<ImageNode>();
            }
            (None, None) => {}
        }
    }
    if pictures.shown() != shown {
        pictures.set_shown(shown);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entries() -> Vec<Entry> {
        let mut entries = Vec::new();
        let nothing = serde_json::from_value(serde_json::json!({ "engines": [] })).unwrap();
        let tank = palette::entries(&nothing)
            .into_iter()
            .find(|e| e.id == "tank")
            .unwrap();
        for (id, name, width, propellant) in [
            ("engine:F-1-A", "F-1", Some(3.7), Some(Propellant::Kerolox)),
            ("engine:F-1-B", "F-1", Some(3.7), Some(Propellant::Kerolox)),
            ("engine:J-2", "J-2", Some(2.0), Some(Propellant::Hydrolox)),
            (
                "engine:RS-68",
                "RS-68",
                Some(3.3),
                Some(Propellant::Hydrolox),
            ),
        ] {
            entries.push(Entry {
                id: id.into(),
                name: name.into(),
                group: Group::Engines,
                width,
                propellant,
                ..tank.clone()
            });
        }
        entries.push(Entry {
            group: Group::Engines,
            ..tank
        });
        entries
    }

    fn shown(list: &PartsList, entries: &[Entry]) -> Vec<String> {
        (list.chosen(entries).into_iter())
            .filter(|e| list.keeps(e))
            .map(|e| e.id.clone())
            .collect()
    }

    #[test]
    fn a_hydrolox_engine_three_to_four_metres_across_is_found_by_the_two_filters() {
        let entries = entries();
        let mut list = PartsList {
            group: Group::Engines,
            ..default()
        };
        assert_eq!(shown(&list, &entries).len(), 5);
        list.propellant = Some(Propellant::Hydrolox);
        assert_eq!(
            shown(&list, &entries),
            ["engine:J-2", "engine:RS-68", "tank"]
        );
        list.size = SIZES.iter().position(|s| s.0 == "3_4");
        // What takes any width and holds any propellant stays.
        assert_eq!(shown(&list, &entries), ["engine:RS-68", "tank"]);
        list.search = "rs-".into();
        assert_eq!(shown(&list, &entries), ["engine:RS-68"]);
    }

    #[test]
    fn tiles_that_share_a_name_are_called_by_their_own_ids() {
        let palette = Palette::new(entries());
        let label = |id: &str| {
            let entry = palette.entries.iter().find(|e| e.id == id).unwrap();
            palette.label(entry).to_owned()
        };
        assert_eq!(label("engine:F-1-A"), "F-1-A");
        assert_eq!(label("engine:J-2"), "J-2");
        assert_eq!(label("tank"), "Tank");
    }
}
