//! Pictures of parts, for what lists parts by how they look (the editor's parts list). Whoever
//! wants them says which parts, each under a name of its own ([`PartPictures::want`]), and whether
//! any are on screen ([`PartPictures::set_shown`]). The renderer (`osp_render`) draws each part
//! once, framed and lit plainly, into one image of them all, and says where in it each finished
//! one is ([`PartPictures::picture`]); it keeps pictures it has drawn for a while after they stop
//! being wanted, so going back to them costs nothing.
//!
//! Nothing here draws: without the renderer the game runs as it did, with no pictures.

use std::collections::HashMap;

use bevy::prelude::*;
use osp_sim::data::CraftPartDef;

/// What is in hand in the editor, when it is not shown on the craft: drawn by the renderer as the
/// hangar draws its parts, at the pointer.
#[derive(Resource, Default)]
pub struct PartInHand {
    parts: Vec<CraftPartDef>,
    place: Option<(Vec3, Quat)>,
    middle: Option<Vec3>,
}

impl PartInHand {
    /// Draw `parts`, the first holding the rest, with their middle at the point and turned so; or
    /// nothing, with none. Without a place they stay ready but are not drawn.
    pub fn set(&mut self, parts: Vec<CraftPartDef>, place: Option<(Vec3, Quat)>) {
        if parts != self.parts {
            self.parts = parts;
            self.middle = None;
        }
        self.place = place;
    }

    /// Where the renderer finds the middle of what it draws, from the first part's place in the
    /// drawing (the frame `osp_render` draws them in, as the hangar's), m: what is put at the point.
    /// None until it has drawn them.
    pub fn middle(&self) -> Option<Vec3> {
        self.middle
    }

    /// Said by the renderer, as it draws them.
    pub fn set_middle(&mut self, middle: Option<Vec3>) {
        if middle != self.middle {
            self.middle = middle;
        }
    }

    pub fn parts(&self) -> &[CraftPartDef] {
        &self.parts
    }

    pub fn place(&self) -> Option<(Vec3, Quat)> {
        self.place
    }
}

/// Parts taken off the craft in the editor and left about the hangar (`osp_sim::builder::Loose`),
/// drawn by the renderer see-through, as the hangar draws its parts. Each group is its parts, the
/// first holding the rest, and where its middle is drawn and how it is turned; one with no place
/// is not drawn (it is in hand). A group's drawing carries [`ShowsLoose`] with its index, so the
/// editor can tell what the pointer is on.
#[derive(Resource, Default)]
pub struct LooseParts {
    groups: Vec<(Vec<CraftPartDef>, Option<(Vec3, Quat)>)>,
}

impl LooseParts {
    pub fn set(&mut self, groups: Vec<(Vec<CraftPartDef>, Option<(Vec3, Quat)>)>) {
        if groups != self.groups {
            self.groups = groups;
        }
    }

    pub fn groups(&self) -> &[(Vec<CraftPartDef>, Option<(Vec3, Quat)>)] {
        &self.groups
    }
}

/// On the drawing of loose group `.0` of [`LooseParts`].
#[derive(Component, Debug, Clone, Copy, PartialEq, Eq)]
pub struct ShowsLoose(pub usize);

#[derive(Resource, Default)]
pub struct PartPictures {
    /// The parts wanted now, by name, in the order asked: the first are drawn first.
    wanted: Vec<(String, CraftPartDef)>,
    /// Counts the changes to `wanted`.
    asked: u64,
    shown: bool,
    /// The image they are drawn in, and where each finished picture is in it, px.
    atlas: Option<Handle<Image>>,
    finished: HashMap<String, Rect>,
}

impl PartPictures {
    /// Ask for pictures of `parts`, each under its name, instead of what was asked before.
    pub fn want(&mut self, parts: Vec<(String, CraftPartDef)>) {
        if parts != self.wanted {
            self.wanted = parts;
            self.asked += 1;
        }
    }

    /// The parts wanted, and which asking that is: it changes whenever they do.
    pub fn wanted(&self) -> (&[(String, CraftPartDef)], u64) {
        (&self.wanted, self.asked)
    }

    /// Say whether any pictures are on screen: they are drawn only while some are.
    pub fn set_shown(&mut self, shown: bool) {
        self.shown = shown;
    }

    pub fn shown(&self) -> bool {
        self.shown
    }

    /// The picture of the part wanted under `name`, once it is drawn: the image and where it is
    /// in it, px.
    pub fn picture(&self, name: &str) -> Option<(Handle<Image>, Rect)> {
        let rect = self.finished.get(name)?;
        Some((self.atlas.clone()?, *rect))
    }

    /// For the renderer: the image the pictures are drawn in.
    pub fn set_atlas(&mut self, atlas: Handle<Image>) {
        self.atlas = Some(atlas);
    }

    /// For the renderer: the picture of what is wanted under `name` is finished, at `rect` in the
    /// image (px), or is gone for `None`.
    pub fn set_finished(&mut self, name: &str, rect: Option<Rect>) {
        match rect {
            Some(rect) => {
                self.finished.insert(name.to_owned(), rect);
            }
            None => {
                self.finished.remove(name);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn part(name: &str) -> CraftPartDef {
        serde_json::from_value(serde_json::json!({ "id": "", "name": name, "template": "x" }))
            .unwrap()
    }

    #[test]
    fn asking_again_for_the_same_parts_is_not_a_new_asking() {
        let mut pictures = PartPictures::default();
        pictures.want(vec![("a".into(), part("a"))]);
        let (_, first) = pictures.wanted();
        pictures.want(vec![("a".into(), part("a"))]);
        assert_eq!(pictures.wanted().1, first);
        pictures.want(vec![("b".into(), part("b"))]);
        assert_eq!(pictures.wanted().1, first + 1);
    }

    #[test]
    fn a_picture_is_there_only_once_it_is_finished_and_drawn_somewhere() {
        let mut pictures = PartPictures::default();
        let rect = Rect::new(0.0, 0.0, 10.0, 10.0);
        pictures.set_finished("a", Some(rect));
        assert!(pictures.picture("a").is_none(), "no image yet");
        pictures.set_atlas(Handle::default());
        assert_eq!(pictures.picture("a").map(|(_, r)| r), Some(rect));
        pictures.set_finished("a", None);
        assert!(pictures.picture("a").is_none());
    }
}
