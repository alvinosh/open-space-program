//! Styles and themes.
//!
//! A [`StyleDef`] is a set of optional properties (CSS-like, all fields optional). A node's
//! final style is resolved by layering, later wins:
//! 1. the theme class named after the widget (`label`, `button`, `window`, ...),
//! 2. each class in the node's `class` string, in order,
//! 3. the node's own inline `style`.
//!
//! Themes are records in the `themes` collection; all theme records are merged in load
//! order, so a mod can restyle the game by adding or patching classes.

use std::collections::BTreeMap;

use bevy::{
    prelude::*,
    text::{FontFeatureTag, FontFeatures},
};
use serde::{Deserialize, Serialize};

/// A length: a number of logical pixels, or a string like `"50%"`, `"12px"`, `"20vw"`,
/// `"10vh"` or `"auto"`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Length {
    Px(f32),
    Text(String),
}

impl Length {
    pub fn to_val(&self) -> Val {
        match self {
            Length::Px(px) => Val::Px(*px),
            Length::Text(s) => {
                let s = s.trim();
                let num = |suffix: &str| {
                    s.strip_suffix(suffix)
                        .and_then(|n| n.trim().parse::<f32>().ok())
                };
                if s == "auto" {
                    Val::Auto
                } else if let Some(n) = num("%") {
                    Val::Percent(n)
                } else if let Some(n) = num("vw") {
                    Val::Vw(n)
                } else if let Some(n) = num("vh") {
                    Val::Vh(n)
                } else if let Some(n) = num("px").or_else(|| s.parse().ok()) {
                    Val::Px(n)
                } else {
                    Val::Auto
                }
            }
        }
    }
}

/// Edge sizes: one number for all sides, `[vertical, horizontal]`, or
/// `[top, right, bottom, left]`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Edges {
    All(f32),
    Sides(Vec<f32>),
}

impl Edges {
    pub fn to_rect(&self) -> UiRect {
        let px = Val::Px;
        match self {
            Edges::All(v) => UiRect::all(px(*v)),
            Edges::Sides(v) => match v.as_slice() {
                [a] => UiRect::all(px(*a)),
                [vert, horiz] => UiRect::axes(px(*horiz), px(*vert)),
                [t, r, b, l] => UiRect::new(px(*l), px(*r), px(*t), px(*b)),
                _ => UiRect::DEFAULT,
            },
        }
    }
}

/// A colour: `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, or `[r, g, b]` / `[r, g, b, a]` in 0..1 sRGB.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ColorDef {
    Hex(String),
    Rgba(Vec<f32>),
}

impl ColorDef {
    pub fn to_color(&self) -> Color {
        match self {
            ColorDef::Hex(hex) => Srgba::hex(hex.trim_start_matches('#'))
                .map(Color::from)
                .unwrap_or(Color::srgb(1.0, 0.0, 1.0)),
            ColorDef::Rgba(c) => match c.as_slice() {
                [r, g, b] => Color::srgb(*r, *g, *b),
                [r, g, b, a] => Color::srgba(*r, *g, *b, *a),
                _ => Color::srgb(1.0, 0.0, 1.0),
            },
        }
    }
}

macro_rules! style_def {
    ($($(#[$meta:meta])* $field:ident: $ty:ty),* $(,)?) => {
        #[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct StyleDef {
            $($(#[$meta])* pub $field: Option<$ty>,)*
        }

        impl StyleDef {
            /// Overlay `other` on top of `self`.
            pub fn merge(&mut self, other: &StyleDef) {
                $(if other.$field.is_some() {
                    self.$field = other.$field.clone();
                })*
            }
        }
    };
}

style_def! {
    /// `"row"` or `"column"`.
    direction: String,
    /// `"relative"` (default) or `"absolute"`.
    position: String,
    left: Length,
    right: Length,
    top: Length,
    bottom: Length,
    width: Length,
    height: Length,
    min_width: Length,
    min_height: Length,
    max_width: Length,
    max_height: Length,
    padding: Edges,
    margin: Edges,
    border: Edges,
    /// Space between children, px.
    gap: f32,
    /// `"start"`, `"end"`, `"center"`, `"stretch"`, `"baseline"`.
    align_items: String,
    /// `"start"`, `"end"`, `"center"`, `"space_between"`, `"space_around"`, `"space_evenly"`.
    justify_content: String,
    align_self: String,
    grow: f32,
    shrink: f32,
    wrap: bool,
    /// Clip overflowing children.
    clip: bool,
    /// Clip overflowing children and scroll them vertically with the mouse wheel.
    scroll: bool,
    radius: f32,
    background: ColorDef,
    hover_background: ColorDef,
    border_color: ColorDef,
    /// Text colour.
    color: ColorDef,
    /// Colours while the node's `active` condition holds.
    active_background: ColorDef,
    active_border_color: ColorDef,
    active_color: ColorDef,
    font_size: f32,
    /// Font family, or a CSS-style list of them (`"Barlow, sans-serif"`). Font files are
    /// listed in the theme's `fonts`.
    font: String,
    /// 100 (thin) to 900 (black); 400 is regular.
    font_weight: f32,
    /// Extra space between letters, px.
    letter_spacing: f32,
    /// Show text in capitals (titles, captions).
    uppercase: bool,
    /// Background gradient from top to bottom, stops spread evenly (drawn over
    /// `background`).
    gradient: Vec<ColorDef>,
    /// Border gradient from top to bottom (instead of `border_color`): a lit top edge.
    border_gradient: Vec<ColorDef>,
    /// Drop shadow behind the node.
    shadow: ShadowDef,
    /// Stacking order for layout roots.
    z_index: i32,
}

/// A drop shadow: offset, blur and spread in px.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ShadowDef {
    pub color: ColorDef,
    pub x: f32,
    pub y: f32,
    pub blur: f32,
    pub spread: f32,
}

impl Default for ShadowDef {
    fn default() -> Self {
        Self {
            color: ColorDef::Rgba(vec![0.0, 0.0, 0.0, 0.5]),
            x: 0.0,
            y: 2.0,
            blur: 8.0,
            spread: 0.0,
        }
    }
}

impl ShadowDef {
    pub fn to_box_shadow(&self) -> BoxShadow {
        BoxShadow::new(
            self.color.to_color(),
            Val::Px(self.x),
            Val::Px(self.y),
            Val::Px(self.spread),
            Val::Px(self.blur),
        )
    }
}

/// A top-to-bottom gradient through `stops`, spread evenly.
pub fn vertical_gradient(stops: &[ColorDef]) -> Gradient {
    let last = stops.len().saturating_sub(1).max(1) as f32;
    LinearGradient::to_bottom(
        stops
            .iter()
            .enumerate()
            .map(|(i, c)| ColorStop::new(c.to_color(), Val::Percent(i as f32 * 100.0 / last)))
            .collect(),
    )
    .into()
}

impl StyleDef {
    /// Write the layout properties into a Bevy `Node`.
    pub fn apply(&self, node: &mut Node) {
        if let Some(d) = &self.direction {
            node.flex_direction = match d.as_str() {
                "row" => FlexDirection::Row,
                "row_reverse" => FlexDirection::RowReverse,
                "column_reverse" => FlexDirection::ColumnReverse,
                _ => FlexDirection::Column,
            };
        }
        if let Some(p) = &self.position {
            node.position_type = if p == "absolute" {
                PositionType::Absolute
            } else {
                PositionType::Relative
            };
        }
        let set = |slot: &mut Val, value: &Option<Length>| {
            if let Some(v) = value {
                *slot = v.to_val();
            }
        };
        set(&mut node.left, &self.left);
        set(&mut node.right, &self.right);
        set(&mut node.top, &self.top);
        set(&mut node.bottom, &self.bottom);
        set(&mut node.width, &self.width);
        set(&mut node.height, &self.height);
        set(&mut node.min_width, &self.min_width);
        set(&mut node.min_height, &self.min_height);
        set(&mut node.max_width, &self.max_width);
        set(&mut node.max_height, &self.max_height);
        if let Some(e) = &self.padding {
            node.padding = e.to_rect();
        }
        if let Some(e) = &self.margin {
            node.margin = e.to_rect();
        }
        if let Some(e) = &self.border {
            node.border = e.to_rect();
        }
        if let Some(g) = self.gap {
            node.row_gap = Val::Px(g);
            node.column_gap = Val::Px(g);
        }
        if let Some(a) = &self.align_items {
            node.align_items = match a.as_str() {
                "start" => AlignItems::FlexStart,
                "end" => AlignItems::FlexEnd,
                "center" => AlignItems::Center,
                "baseline" => AlignItems::Baseline,
                _ => AlignItems::Stretch,
            };
        }
        if let Some(a) = &self.align_self {
            node.align_self = match a.as_str() {
                "start" => AlignSelf::FlexStart,
                "end" => AlignSelf::FlexEnd,
                "center" => AlignSelf::Center,
                "stretch" => AlignSelf::Stretch,
                _ => AlignSelf::Auto,
            };
        }
        if let Some(j) = &self.justify_content {
            node.justify_content = match j.as_str() {
                "end" => JustifyContent::FlexEnd,
                "center" => JustifyContent::Center,
                "space_between" => JustifyContent::SpaceBetween,
                "space_around" => JustifyContent::SpaceAround,
                "space_evenly" => JustifyContent::SpaceEvenly,
                _ => JustifyContent::FlexStart,
            };
        }
        if let Some(g) = self.grow {
            node.flex_grow = g;
        }
        if let Some(s) = self.shrink {
            node.flex_shrink = s;
        }
        if let Some(w) = self.wrap {
            node.flex_wrap = if w { FlexWrap::Wrap } else { FlexWrap::NoWrap };
        }
        if self.clip == Some(true) {
            node.overflow = Overflow::clip();
        }
        if self.scroll == Some(true) {
            node.overflow = Overflow::scroll_y();
        }
        if let Some(r) = self.radius {
            node.border_radius = BorderRadius::all(Val::Px(r));
        }
    }

    pub fn text_color(&self) -> Color {
        self.color.as_ref().map_or(Color::WHITE, ColorDef::to_color)
    }

    pub fn font_size(&self) -> f32 {
        self.font_size.unwrap_or(14.0)
    }

    /// The font for text nodes. Numbers use tabular figures so readouts do not jitter.
    pub fn text_font(&self) -> TextFont {
        let mut font = TextFont::from_font_size(self.font_size());
        font.font_features = FontFeatures::builder()
            .enable(FontFeatureTag::TABULAR_FIGURES)
            .build();
        if let Some(family) = &self.font {
            font.font = FontSource::Families(family.as_str().into());
        }
        if let Some(weight) = self.font_weight {
            font.weight = FontWeight(weight.clamp(1.0, 1000.0) as u16);
        }
        font
    }
}

/// A theme record: named style classes.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ThemeDef {
    pub id: String,
    /// Font files to load (`.ttf`, `.otf`), relative to the mod root. Styles name them by
    /// family (`font`).
    #[serde(default)]
    pub fonts: Vec<String>,
    #[serde(default)]
    pub classes: BTreeMap<String, StyleDef>,
}

impl ThemeDef {
    /// Merge another theme's classes over this one (class by class, property by property).
    pub fn merge(&mut self, other: &ThemeDef) {
        for font in &other.fonts {
            if !self.fonts.contains(font) {
                self.fonts.push(font.clone());
            }
        }
        for (name, style) in &other.classes {
            self.classes.entry(name.clone()).or_default().merge(style);
        }
    }

    /// Style of a text node: the `text` class under the node's own style, so one place
    /// sets the font for every label, button and title.
    pub fn text_style(&self, style: &StyleDef) -> StyleDef {
        let mut text = self.classes.get("text").cloned().unwrap_or_default();
        text.merge(style);
        text
    }

    /// Resolve the final style for a node.
    pub fn resolve(&self, widget: &str, classes: &str, inline: &StyleDef) -> StyleDef {
        let mut style = StyleDef::default();
        for name in std::iter::once(widget).chain(classes.split_whitespace()) {
            if let Some(class) = self.classes.get(name) {
                style.merge(class);
            }
        }
        style.merge(inline);
        style
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lengths_and_colors() {
        assert_eq!(Length::Px(5.0).to_val(), Val::Px(5.0));
        assert_eq!(Length::Text("50%".into()).to_val(), Val::Percent(50.0));
        assert_eq!(Length::Text("12px".into()).to_val(), Val::Px(12.0));
        assert_eq!(Length::Text("auto".into()).to_val(), Val::Auto);
        assert_eq!(
            ColorDef::Hex("#ff0000".into()).to_color(),
            Color::srgb(1.0, 0.0, 0.0)
        );
    }

    #[test]
    fn class_resolution_order() {
        let mut theme = ThemeDef::default();
        theme.classes.insert(
            "label".into(),
            StyleDef {
                font_size: Some(12.0),
                color: Some(ColorDef::Hex("#fff".into())),
                ..default()
            },
        );
        theme.classes.insert(
            "big".into(),
            StyleDef {
                font_size: Some(20.0),
                ..default()
            },
        );
        let inline = StyleDef {
            color: Some(ColorDef::Hex("#000".into())),
            ..default()
        };
        let s = theme.resolve("label", "big", &inline);
        assert_eq!(s.font_size, Some(20.0));
        assert_eq!(s.color, Some(ColorDef::Hex("#000".into())));
    }
}
