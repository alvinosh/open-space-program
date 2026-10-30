//! Graphics settings: a preset (Low to Ultra) or each setting picked by hand on the main
//! menu's Graphics page, saved between runs.
//!
//! Each setting is applied by the module it tunes (render scale, anti-aliasing, ambient
//! occlusion and bloom in `camera`, shadows in `frame`, terrain detail in `planet`, trees in `trees`, the sky's and
//! plumes' ray-marching steps in `atmosphere` and `plume`); this module owns the values,
//! vsync and the frame cap.
//!
//! The first run picks a preset from the GPU (see [`recommended`]). The settings live in
//! `settings.json` in the platform's config directory (`OSP_SETTINGS=<file>` overrides it).
//! `OSP_GRAPHICS=<preset>[,<key>=<value>...]` (e.g. `low` or `high,shadows=off,trees=low`)
//! starts with those settings without saving them; scripted captures (`OSP_CAPTURE`) use it
//! or High and never touch the file, so they stay reproducible.

use std::{path::PathBuf, time::Instant};

use bevy::{
    prelude::*,
    render::renderer::RenderAdapterInfo,
    window::{PresentMode, PrimaryWindow},
};
use osp_ui::{UiAction, UiData};
use serde::{Deserialize, Serialize};

use crate::FrameSet;

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let (settings, source) = load();
        app.insert_resource(settings)
            .insert_resource(source)
            .add_systems(
                Update,
                (
                    detect.run_if(resource_exists::<RenderAdapterInfo>.and_then(run_once)),
                    actions.in_set(FrameSet::Input),
                    (apply_window, publish).run_if(resource_changed::<GraphicsSettings>),
                ),
            )
            .add_systems(Last, limit_frame_rate);
    }
}

/// A level of detail for one setting. Not every setting offers every level.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Quality {
    Off,
    Low,
    Medium,
    #[default]
    High,
    Ultra,
}

impl Quality {
    pub const ALL: [Quality; 5] = [
        Quality::Off,
        Quality::Low,
        Quality::Medium,
        Quality::High,
        Quality::Ultra,
    ];

    pub fn name(self) -> &'static str {
        match self {
            Quality::Off => "off",
            Quality::Low => "low",
            Quality::Medium => "medium",
            Quality::High => "high",
            Quality::Ultra => "ultra",
        }
    }

    fn parse(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|q| q.name() == s)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Antialiasing {
    Off,
    /// A cheap post-process pass that softens edges (and a little of everything else).
    Fxaa,
    /// 4x multisampling: sharp, but costs memory bandwidth on every pass, and rules out
    /// ambient occlusion.
    Msaa,
    /// Temporal: each frame is jittered by a fraction of a pixel and blended with the last
    /// few, which smooths edges, specular sparkle and the noise of screen-space effects. Can
    /// smear a little in fast motion.
    Taa,
}

impl Antialiasing {
    pub fn name(self) -> &'static str {
        match self {
            Antialiasing::Off => "off",
            Antialiasing::Fxaa => "fxaa",
            Antialiasing::Msaa => "msaa",
            Antialiasing::Taa => "taa",
        }
    }
}

#[derive(Resource, Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct GraphicsSettings {
    /// The 3D view's resolution as a share of the window's, percent; the UI stays sharp.
    pub render_scale: u32,
    pub antialiasing: Antialiasing,
    pub shadows: Quality,
    /// Screen-space darkening of creases and contacts, where the sky's light can't reach.
    pub ambient_occlusion: Quality,
    /// How far out the terrain keeps its finer chunks.
    pub terrain: Quality,
    /// How far out trees are drawn, and drawn in detail.
    pub trees: Quality,
    /// Ray-marching steps for the sky, the haze over the ground and engine plumes.
    pub effects: Quality,
    pub bloom: bool,
    pub vsync: bool,
    /// Frames per second cap, 0 for none.
    pub fps_limit: u32,
}

impl Default for GraphicsSettings {
    fn default() -> Self {
        Self::preset(Quality::High)
    }
}

/// The presets, in the order the menu shows them.
pub const PRESETS: [Quality; 4] = [Quality::Low, Quality::Medium, Quality::High, Quality::Ultra];

impl GraphicsSettings {
    /// A preset's settings; vsync on and no frame cap.
    pub fn preset(preset: Quality) -> Self {
        use Antialiasing::{Fxaa, Taa};
        use Quality::*;
        let (render_scale, antialiasing, shadows, terrain, trees, effects, bloom) = match preset {
            Off | Low => (50, Fxaa, Off, Low, Low, Low, false),
            Medium => (75, Fxaa, Low, Medium, Medium, Medium, true),
            High => (100, Taa, High, High, High, High, true),
            Ultra => (100, Taa, Ultra, Ultra, Ultra, Ultra, true),
        };
        let ambient_occlusion = match preset {
            Off | Low => Off,
            other => other,
        };
        Self {
            render_scale,
            antialiasing,
            shadows,
            ambient_occlusion,
            terrain,
            trees,
            effects,
            bloom,
            vsync: true,
            fps_limit: 0,
        }
    }

    /// The preset these settings match, ignoring vsync and the frame cap.
    pub fn matching_preset(&self) -> Option<Quality> {
        PRESETS.into_iter().find(|&p| {
            let preset = Self {
                vsync: self.vsync,
                fps_limit: self.fps_limit,
                ..Self::preset(p)
            };
            preset == *self
        })
    }

    /// Apply one `key=value` (also the `graphics.set:<key>:<value>` action).
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let on_off = |v: &str| match v {
            "on" | "true" | "1" => Ok(true),
            "off" | "false" | "0" => Ok(false),
            _ => Err(format!("`{v}` is not on or off")),
        };
        let quality = |v: &str| Quality::parse(v).ok_or_else(|| format!("unknown level `{v}`"));
        let number = |v: &str| {
            v.parse::<u32>()
                .map_err(|_| format!("`{v}` is not a number"))
        };
        match key {
            "render_scale" => self.render_scale = number(value)?.clamp(25, 100),
            "antialiasing" => {
                self.antialiasing = match value {
                    "off" => Antialiasing::Off,
                    "fxaa" => Antialiasing::Fxaa,
                    "msaa" => Antialiasing::Msaa,
                    "taa" => Antialiasing::Taa,
                    _ => return Err(format!("`{value}` is not off, fxaa, msaa or taa")),
                }
            }
            "shadows" => self.shadows = quality(value)?,
            "ambient_occlusion" => self.ambient_occlusion = quality(value)?,
            "terrain" => self.terrain = quality(value)?.max(Quality::Low),
            "trees" => self.trees = quality(value)?,
            "effects" => self.effects = quality(value)?.max(Quality::Low),
            "bloom" => self.bloom = on_off(value)?,
            "vsync" => self.vsync = on_off(value)?,
            "fps_limit" => self.fps_limit = number(value)?,
            _ => return Err(format!("unknown graphics setting `{key}`")),
        }
        Ok(())
    }

    /// One line for logs and profiles.
    pub fn describe(&self) -> String {
        format!(
            "{} (render scale {}%, anti-aliasing {}, shadows {}, ambient occlusion {}, terrain {}, trees {}, effects {}, bloom {}, vsync {}, fps limit {})",
            self.matching_preset().map_or("custom", Quality::name),
            self.render_scale,
            self.antialiasing.name(),
            self.shadows.name(),
            self.ambient_occlusion.name(),
            self.terrain.name(),
            self.trees.name(),
            self.effects.name(),
            on_off(self.bloom),
            on_off(self.vsync),
            if self.fps_limit == 0 {
                "none".to_owned()
            } else {
                self.fps_limit.to_string()
            },
        )
    }
}

fn on_off(on: bool) -> &'static str {
    if on { "on" } else { "off" }
}

/// Where the settings came from, and whether changes are saved.
#[derive(Resource)]
struct SettingsSource {
    file: Option<PathBuf>,
    /// No settings were saved yet: pick a preset from the GPU once it is known.
    detect: bool,
}

pub fn settings_file() -> Option<PathBuf> {
    if let Some(file) = std::env::var_os("OSP_SETTINGS") {
        return Some(file.into());
    }
    let dir = if cfg!(target_os = "windows") {
        PathBuf::from(std::env::var_os("APPDATA")?)
    } else if cfg!(target_os = "macos") {
        PathBuf::from(std::env::var_os("HOME")?).join("Library/Application Support")
    } else {
        std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| Some(PathBuf::from(std::env::var_os("HOME")?).join(".config")))?
    };
    Some(dir.join("OpenSpaceProgram").join("settings.json"))
}

fn load() -> (GraphicsSettings, SettingsSource) {
    let capture = std::env::var_os("OSP_CAPTURE").is_some();
    if let Ok(spec) = std::env::var("OSP_GRAPHICS") {
        let mut settings = GraphicsSettings::default();
        for (i, part) in spec.split(',').map(str::trim).enumerate() {
            let result = match (i, part.split_once('=')) {
                (_, Some((key, value))) => settings.set(key.trim(), value.trim()),
                (0, None) => match Quality::parse(part) {
                    Some(preset) => {
                        settings = GraphicsSettings::preset(preset);
                        Ok(())
                    }
                    None => Err(format!("unknown preset `{part}`")),
                },
                (_, None) => Err(format!("expected <key>=<value>, got `{part}`")),
            };
            if let Err(e) = result {
                warn!("OSP_GRAPHICS: {e}");
            }
        }
        info!("graphics from OSP_GRAPHICS: {}", settings.describe());
        let source = SettingsSource {
            file: None,
            detect: false,
        };
        return (settings, source);
    }
    if capture {
        let source = SettingsSource {
            file: None,
            detect: false,
        };
        return (GraphicsSettings::default(), source);
    }
    let file = settings_file();
    let saved = file.as_ref().and_then(|f| {
        let text = std::fs::read_to_string(f).ok()?;
        match serde_json::from_str::<GraphicsSettings>(&text) {
            Ok(settings) => Some(settings),
            Err(e) => {
                warn!("ignoring {}: {e}", f.display());
                None
            }
        }
    });
    let detect = saved.is_none();
    let settings = saved.unwrap_or_default();
    if !detect {
        info!("graphics: {}", settings.describe());
    }
    (settings, SettingsSource { file, detect })
}

fn save(settings: &GraphicsSettings, source: &SettingsSource) {
    let Some(file) = &source.file else { return };
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let text = serde_json::to_string_pretty(settings).map_err(std::io::Error::other)?;
            std::fs::write(file, text)
        });
    if let Err(e) = result {
        warn!("cannot save graphics settings to {}: {e}", file.display());
    }
}

/// The preset for a GPU: discrete GPUs get High, Apple's M3 and later (and any Pro, Max
/// or Ultra) High and earlier ones Medium, other integrated GPUs and software or virtual
/// adapters Low. Measured on the pad at 2560x1440 on an M5: High 12 ms, Medium 6, Low 4.
pub fn recommended(adapter: &RenderAdapterInfo) -> Quality {
    // wgpu's `DeviceType`, which Bevy does not re-export.
    match format!("{:?}", adapter.device_type).as_str() {
        "DiscreteGpu" => Quality::High,
        "IntegratedGpu" => match apple_generation(&adapter.name) {
            Some((generation, big)) if generation >= 3 || big => Quality::High,
            Some(_) => Quality::Medium,
            None => Quality::Low,
        },
        _ => Quality::Low,
    }
}

/// "Apple M2 Pro" -> (2, true): the generation, and whether it is a Pro, Max or Ultra.
fn apple_generation(name: &str) -> Option<(u32, bool)> {
    let rest = name.strip_prefix("Apple M")?;
    let digits: String = rest.chars().take_while(char::is_ascii_digit).collect();
    let big = ["Pro", "Max", "Ultra"]
        .iter()
        .any(|tier| rest.contains(tier));
    Some((digits.parse().ok()?, big))
}

/// First run: once the GPU is known, start from its preset and save it.
fn detect(
    adapter: Res<RenderAdapterInfo>,
    mut source: ResMut<SettingsSource>,
    mut settings: ResMut<GraphicsSettings>,
    mut data: ResMut<UiData>,
) {
    let recommended = recommended(&adapter);
    data.set("graphics.recommended", recommended.name());
    if !source.detect {
        return;
    }
    source.detect = false;
    *settings = GraphicsSettings::preset(recommended);
    info!(
        "graphics: first run on {}, picked {}",
        adapter.name,
        settings.describe()
    );
    save(&settings, &source);
}

/// `graphics.preset:<low|medium|high|ultra>`, `graphics.set:<key>:<value>`, and
/// `graphics.detect` (the recommended preset for this GPU).
fn actions(
    mut actions: MessageReader<UiAction>,
    mut settings: ResMut<GraphicsSettings>,
    source: Res<SettingsSource>,
    adapter: Option<Res<RenderAdapterInfo>>,
) {
    for action in actions.read() {
        let mut next = settings.clone();
        match action.name() {
            "graphics.preset" => {
                let Some(preset) = action.arg().and_then(Quality::parse) else {
                    continue;
                };
                next = GraphicsSettings {
                    vsync: settings.vsync,
                    fps_limit: settings.fps_limit,
                    ..GraphicsSettings::preset(preset)
                };
            }
            "graphics.detect" => {
                let Some(adapter) = &adapter else { continue };
                next = GraphicsSettings {
                    vsync: settings.vsync,
                    fps_limit: settings.fps_limit,
                    ..GraphicsSettings::preset(recommended(adapter))
                };
            }
            "graphics.set" => {
                let Some((key, value)) = action.arg().and_then(|a| a.split_once(':')) else {
                    continue;
                };
                if let Err(e) = next.set(key, value) {
                    warn!("{e}");
                    continue;
                }
            }
            _ => continue,
        }
        if next != *settings {
            info!("graphics: {}", next.describe());
            save(&next, &source);
            *settings = next;
        }
    }
}

/// `graphics.*`: each setting's value as text, for the menu's buttons to compare against.
fn publish(settings: Res<GraphicsSettings>, mut data: ResMut<UiData>) {
    data.set(
        "graphics.preset",
        settings.matching_preset().map_or("custom", Quality::name),
    );
    data.set("graphics.render_scale", settings.render_scale.to_string());
    data.set("graphics.antialiasing", settings.antialiasing.name());
    data.set("graphics.shadows", settings.shadows.name());
    data.set(
        "graphics.ambient_occlusion",
        settings.ambient_occlusion.name(),
    );
    data.set("graphics.terrain", settings.terrain.name());
    data.set("graphics.trees", settings.trees.name());
    data.set("graphics.effects", settings.effects.name());
    data.set("graphics.bloom", on_off(settings.bloom));
    data.set("graphics.vsync", on_off(settings.vsync));
    data.set("graphics.fps_limit", settings.fps_limit.to_string());
    data.set("graphics.summary", settings.describe());
}

fn apply_window(
    settings: Res<GraphicsSettings>,
    mut window: Single<&mut Window, With<PrimaryWindow>>,
) {
    // `OSP_UNCAPPED` measures how fast the game can go.
    if std::env::var_os("OSP_UNCAPPED").is_some() {
        return;
    }
    let mode = if settings.vsync {
        PresentMode::AutoVsync
    } else {
        PresentMode::AutoNoVsync
    };
    if window.present_mode != mode {
        window.present_mode = mode;
    }
}

/// Sleep out the rest of the frame when it would beat the frame cap.
fn limit_frame_rate(settings: Res<GraphicsSettings>, mut last: Local<Option<Instant>>) {
    if settings.fps_limit > 0 {
        let period = std::time::Duration::from_secs_f64(1.0 / settings.fps_limit as f64);
        if let Some(last) = *last {
            let elapsed = last.elapsed();
            if elapsed < period {
                std::thread::sleep(period - elapsed);
            }
        }
    }
    *last = Some(Instant::now());
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_round_trip() {
        for preset in PRESETS {
            let settings = GraphicsSettings::preset(preset);
            assert_eq!(settings.matching_preset(), Some(preset));
            let text = serde_json::to_string(&settings).unwrap();
            assert_eq!(
                serde_json::from_str::<GraphicsSettings>(&text).unwrap(),
                settings
            );
        }
        let mut custom = GraphicsSettings::preset(Quality::High);
        custom.set("shadows", "off").unwrap();
        assert_eq!(custom.matching_preset(), None);
        // Vsync and the frame cap are not part of a preset.
        let mut capped = GraphicsSettings::preset(Quality::Low);
        capped.set("fps_limit", "30").unwrap();
        capped.set("vsync", "off").unwrap();
        assert_eq!(capped.matching_preset(), Some(Quality::Low));
    }

    #[test]
    fn apple_generations() {
        assert_eq!(apple_generation("Apple M1"), Some((1, false)));
        assert_eq!(apple_generation("Apple M2 Max"), Some((2, true)));
        assert_eq!(apple_generation("Apple M5"), Some((5, false)));
        assert_eq!(apple_generation("Intel(R) Iris(R) Xe Graphics"), None);
    }

    #[test]
    fn set_validates() {
        let mut s = GraphicsSettings::default();
        assert!(s.set("shadows", "lots").is_err());
        assert!(s.set("nope", "on").is_err());
        s.set("antialiasing", "taa").unwrap();
        assert_eq!(s.antialiasing, Antialiasing::Taa);
        s.set("ambient_occlusion", "ultra").unwrap();
        assert_eq!(s.ambient_occlusion, Quality::Ultra);
        s.set("render_scale", "5").unwrap();
        assert_eq!(s.render_scale, 25);
        s.set("terrain", "off").unwrap();
        assert_eq!(s.terrain, Quality::Low);
        // Missing fields in a saved file take the defaults.
        let partial: GraphicsSettings =
            serde_json::from_str(r#"{ "antialiasing": "fxaa" }"#).unwrap();
        assert_eq!(partial.antialiasing, Antialiasing::Fxaa);
        assert_eq!(partial.shadows, GraphicsSettings::default().shadows);
    }
}
