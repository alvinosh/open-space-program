//! Settings, saved between runs. Graphics: a preset (Low to Ultra) or each setting picked by hand
//! on the main menu's Graphics page. Sound: how loud each part of it is, on the Sound page. Keys,
//! rebound on the Controls page. The interface's scale and the alerts, on the Interface page.
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

use std::{collections::BTreeMap, path::PathBuf, time::Instant};

use bevy::{
    prelude::*,
    render::renderer::RenderAdapterInfo,
    window::{PresentMode, PrimaryWindow},
};
use osp_ui::{ActionState, KeyBindingDef, UiAction, UiData, UiDocuments, record};
use serde::{Deserialize, Serialize};

use crate::{FrameSet, sim::Sim, tutorial::key_label};

pub struct SettingsPlugin;

impl Plugin for SettingsPlugin {
    fn build(&self, app: &mut App) {
        let (settings, source) = load();
        app.insert_resource(settings.graphics)
            .insert_resource(settings.audio)
            .insert_resource(settings.interface)
            .insert_resource(source)
            .init_resource::<KeyEdit>()
            .add_systems(
                Update,
                (
                    detect.run_if(resource_exists::<RenderAdapterInfo>.and_then(run_once)),
                    (actions, audio_actions, interface_actions, key_actions)
                        .in_set(FrameSet::Input),
                    (apply_window, publish).run_if(resource_changed::<GraphicsSettings>),
                    publish_audio.run_if(resource_changed::<AudioSettings>),
                    apply_interface.run_if(resource_changed::<InterfaceSettings>),
                    apply_keys.run_if(resource_changed::<UiDocuments>),
                    stop_warp_at_signal_changes,
                ),
            )
            .add_systems(Last, limit_frame_rate);
    }
}

/// What the settings file holds. Graphics come first and flat, as the file began with them alone.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
struct Settings {
    #[serde(flatten)]
    graphics: GraphicsSettings,
    #[serde(default)]
    audio: AudioSettings,
    #[serde(default)]
    interface: InterfaceSettings,
    /// Keys the player has rebound, by [`binding_ids`].
    #[serde(default)]
    keys: BTreeMap<String, KeyCode>,
}

/// The interface, and what it warns of.
#[derive(Resource, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct InterfaceSettings {
    /// How big it all is, percent: the menu and the HUD alike.
    pub scale: u32,
    /// Alerts over the flight view (see `crate::alerts`), and a tone as each appears.
    pub alerts: bool,
    pub alert_tone: bool,
    /// Time warp stops when a signal comes or goes.
    pub warp_stops_at_signal: bool,
}

impl Default for InterfaceSettings {
    fn default() -> Self {
        Self {
            scale: 100,
            alerts: true,
            alert_tone: true,
            warp_stops_at_signal: false,
        }
    }
}

impl InterfaceSettings {
    /// Apply one `key=value` (the `interface.set:<key>:<value>` action).
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let on_off = |v: &str| match v {
            "on" => Ok(true),
            "off" => Ok(false),
            _ => Err(format!("`{v}` is not on or off")),
        };
        match key {
            "scale" => {
                let percent = value
                    .parse::<u32>()
                    .map_err(|_| format!("`{value}` is not a number"))?;
                self.scale = percent.clamp(50, 200);
            }
            "alerts" => self.alerts = on_off(value)?,
            "alert_tone" => self.alert_tone = on_off(value)?,
            "warp_stops_at_signal" => self.warp_stops_at_signal = on_off(value)?,
            _ => return Err(format!("unknown interface setting `{key}`")),
        }
        Ok(())
    }
}

/// How loud each part of the sound is, percent (see `crate::sound`).
#[derive(Resource, Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct AudioSettings {
    pub master: u32,
    pub engines: u32,
    pub wind: u32,
    pub effects: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            master: 75,
            engines: 100,
            wind: 100,
            effects: 100,
        }
    }
}

impl AudioSettings {
    /// Apply one `key=value` (the `audio.set:<key>:<value>` action).
    pub fn set(&mut self, key: &str, value: &str) -> Result<(), String> {
        let percent = value
            .parse::<u32>()
            .map_err(|_| format!("`{value}` is not a number"))?
            .min(100);
        match key {
            "master" => self.master = percent,
            "engines" => self.engines = percent,
            "wind" => self.wind = percent,
            "effects" => self.effects = percent,
            _ => return Err(format!("unknown sound setting `{key}`")),
        }
        Ok(())
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
    /// What the file holds.
    saved: Settings,
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

fn load() -> (Settings, SettingsSource) {
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
        let settings = Settings {
            graphics: settings,
            ..default()
        };
        let source = SettingsSource {
            file: None,
            detect: false,
            saved: settings.clone(),
        };
        return (settings, source);
    }
    if capture {
        let source = SettingsSource {
            file: None,
            detect: false,
            saved: Settings::default(),
        };
        return (Settings::default(), source);
    }
    let file = settings_file();
    let saved = file.as_ref().and_then(|f| {
        let text = std::fs::read_to_string(f).ok()?;
        match serde_json::from_str::<Settings>(&text) {
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
        info!("graphics: {}", settings.graphics.describe());
    }
    let source = SettingsSource {
        file,
        detect,
        saved: settings.clone(),
    };
    (settings, source)
}

/// Write out what the file should hold.
fn save(source: &SettingsSource) {
    let Some(file) = &source.file else { return };
    let result = file
        .parent()
        .map_or(Ok(()), std::fs::create_dir_all)
        .and_then(|()| {
            let text =
                serde_json::to_string_pretty(&source.saved).map_err(std::io::Error::other)?;
            std::fs::write(file, text)
        });
    if let Err(e) = result {
        warn!("cannot save settings to {}: {e}", file.display());
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
    source.saved.graphics = settings.clone();
    save(&source);
}

/// `graphics.preset:<low|medium|high|ultra>`, `graphics.set:<key>:<value>`, and
/// `graphics.detect` (the recommended preset for this GPU).
fn actions(
    mut actions: MessageReader<UiAction>,
    mut settings: ResMut<GraphicsSettings>,
    mut source: ResMut<SettingsSource>,
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
            source.saved.graphics = next.clone();
            save(&source);
            *settings = next;
        }
    }
}

/// `audio.set:<key>:<value>`.
fn audio_actions(
    mut actions: MessageReader<UiAction>,
    mut settings: ResMut<AudioSettings>,
    mut source: ResMut<SettingsSource>,
) {
    for action in actions.read().filter(|a| a.name() == "audio.set") {
        let Some((key, value)) = action.arg().and_then(|a| a.split_once(':')) else {
            continue;
        };
        let mut next = settings.clone();
        if let Err(e) = next.set(key, value) {
            warn!("{e}");
        } else if next != *settings {
            source.saved.audio = next.clone();
            save(&source);
            *settings = next;
        }
    }
}

/// `interface.set:<key>:<value>`.
fn interface_actions(
    mut actions: MessageReader<UiAction>,
    mut settings: ResMut<InterfaceSettings>,
    mut source: ResMut<SettingsSource>,
) {
    for action in actions.read().filter(|a| a.name() == "interface.set") {
        let Some((key, value)) = action.arg().and_then(|a| a.split_once(':')) else {
            continue;
        };
        let mut next = settings.clone();
        if let Err(e) = next.set(key, value) {
            warn!("{e}");
        } else if next != *settings {
            source.saved.interface = next.clone();
            save(&source);
            *settings = next;
        }
    }
}

/// Scale the interface, and `interface.*`: each setting's value as text, for the menu's buttons
/// to compare against.
fn apply_interface(
    settings: Res<InterfaceSettings>,
    mut scale: ResMut<UiScale>,
    mut data: ResMut<UiData>,
) {
    scale.0 = settings.scale as f32 / 100.0;
    data.set("interface.scale", settings.scale.to_string());
    data.set("interface.alerts", on_off(settings.alerts));
    data.set("interface.alert_tone", on_off(settings.alert_tone));
    data.set(
        "interface.warp_stops_at_signal",
        on_off(settings.warp_stops_at_signal),
    );
}

/// A universe started afresh has the setting back at its default, so it is kept up here.
fn stop_warp_at_signal_changes(settings: Res<InterfaceSettings>, mut sim: ResMut<Sim>) {
    if sim.universe.warp_stops_at_links != settings.warp_stops_at_signal {
        sim.universe.warp_stops_at_links = settings.warp_stops_at_signal;
    }
}

/// The keys as they were before the player changed any, and the one being asked for.
#[derive(Resource, Default)]
struct KeyEdit {
    defaults: Vec<KeyBindingDef>,
    listening: Option<String>,
}

/// A name for each binding that stays the same when its key changes: its action, and which of
/// that action's bindings it is.
fn binding_ids(bindings: &[KeyBindingDef]) -> Vec<String> {
    let mut seen: BTreeMap<&str, usize> = BTreeMap::new();
    (bindings.iter())
        .map(|b| {
            let n = seen.entry(b.action.as_str()).or_default();
            *n += 1;
            format!("{}#{}", b.action, *n - 1)
        })
        .collect()
}

/// `defaults` with the player's keys over them.
fn rebound(defaults: &[KeyBindingDef], keys: &BTreeMap<String, KeyCode>) -> Vec<KeyBindingDef> {
    (defaults.iter().zip(binding_ids(defaults)))
        .map(|(binding, id)| KeyBindingDef {
            key: keys.get(&id).copied().unwrap_or(binding.key),
            ..binding.clone()
        })
        .collect()
}

/// `keys.rows`: what can be rebound, and which other action a key is also on.
fn publish_keys(data: &mut UiData, bindings: &[KeyBindingDef], listening: Option<&str>) {
    let rows: Vec<_> = (bindings.iter().zip(binding_ids(bindings)))
        .filter_map(|(binding, id)| {
            let label = binding.label.as_deref()?;
            let clash = (bindings.iter())
                .find(|b| b.key == binding.key && b.action != binding.action)
                .map_or("", |b| b.label.as_deref().unwrap_or(b.action.as_str()));
            let asked = listening == Some(id.as_str());
            let key = if asked {
                "Press a key".to_owned()
            } else {
                key_label(binding.key)
            };
            Some(record([
                ("id", id.into()),
                ("label", label.into()),
                ("key", key.into()),
                ("listening", asked.into()),
                ("clash", clash.into()),
            ]))
        })
        .collect();
    data.set("keys.rows", rows);
}

/// The documents are fresh from the records: put the player's keys over them.
fn apply_keys(
    mut docs: ResMut<UiDocuments>,
    mut edit: ResMut<KeyEdit>,
    source: Res<SettingsSource>,
    mut data: ResMut<UiData>,
) {
    edit.defaults = docs.bindings.clone();
    let keys = rebound(&edit.defaults, &source.saved.keys);
    // Not a change to the documents as far as anything else is concerned.
    docs.bypass_change_detection().bindings = keys;
    publish_keys(&mut data, &docs.bindings, edit.listening.as_deref());
}

/// `keys.listen:<id>`: the next key pressed goes on that binding, Esc leaves it. `keys.reset`.
#[allow(clippy::too_many_arguments)]
fn key_actions(
    mut actions: MessageReader<UiAction>,
    keys: Res<ButtonInput<KeyCode>>,
    mut docs: ResMut<UiDocuments>,
    mut edit: ResMut<KeyEdit>,
    mut source: ResMut<SettingsSource>,
    mut state: ResMut<ActionState>,
    mut data: ResMut<UiData>,
) {
    let mut changed = false;
    for action in actions.read() {
        match action.name() {
            "keys.listen" => {
                edit.listening = action.arg().map(str::to_owned);
                changed = true;
            }
            "keys.reset" => {
                source.saved.keys.clear();
                save(&source);
                edit.listening = None;
                docs.bypass_change_detection().bindings = edit.defaults.clone();
                changed = true;
            }
            _ => {}
        }
    }
    if let (Some(id), Some(&key)) = (edit.listening.clone(), keys.get_just_pressed().next()) {
        edit.listening = None;
        changed = true;
        if key != KeyCode::Escape {
            source.saved.keys.insert(id, key);
            save(&source);
            docs.bypass_change_detection().bindings = rebound(&edit.defaults, &source.saved.keys);
        }
    }
    state.suspended = edit.listening.is_some();
    if changed {
        publish_keys(&mut data, &docs.bindings, edit.listening.as_deref());
    }
}

/// `audio.*`: each setting's value as text, for the menu's buttons to compare against.
fn publish_audio(settings: Res<AudioSettings>, mut data: ResMut<UiData>) {
    data.set("audio.master", settings.master.to_string());
    data.set("audio.engines", settings.engines.to_string());
    data.set("audio.wind", settings.wind.to_string());
    data.set("audio.effects", settings.effects.to_string());
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

#[cfg(test)]
mod settings_file_tests {
    use super::*;

    fn binding(key: KeyCode, action: &str) -> KeyBindingDef {
        KeyBindingDef {
            key,
            action: action.into(),
            hold: false,
            label: Some(action.into()),
        }
    }

    #[test]
    fn a_file_from_before_there_was_sound_still_loads_and_keeps_its_graphics() {
        let old = r#"{ "render_scale": 67, "shadows": "low", "vsync": false }"#;
        let settings: Settings = serde_json::from_str(old).unwrap();
        assert_eq!(settings.graphics.render_scale, 67);
        assert_eq!(settings.graphics.shadows, Quality::Low);
        assert!(!settings.graphics.vsync);
        assert_eq!(settings.audio, AudioSettings::default());
        assert_eq!(settings.interface, InterfaceSettings::default());
        assert!(settings.keys.is_empty());
    }

    #[test]
    fn everything_comes_back_from_the_file_as_it_went_in() {
        let mut settings = Settings::default();
        settings.audio.set("wind", "25").unwrap();
        settings.interface.set("scale", "125").unwrap();
        settings.interface.set("alerts", "off").unwrap();
        settings.keys.insert("vessel.stage#0".into(), KeyCode::KeyP);
        let text = serde_json::to_string(&settings).unwrap();
        assert_eq!(serde_json::from_str::<Settings>(&text).unwrap(), settings);
    }

    #[test]
    fn settings_are_kept_in_range_and_bad_ones_refused() {
        let mut interface = InterfaceSettings::default();
        interface.set("scale", "900").unwrap();
        assert_eq!(interface.scale, 200);
        assert!(interface.set("scale", "big").is_err());
        assert!(interface.set("alerts", "maybe").is_err());
        assert!(interface.set("nothing", "on").is_err());
        let mut audio = AudioSettings::default();
        audio.set("master", "300").unwrap();
        assert_eq!(audio.master, 100);
        assert!(audio.set("nothing", "10").is_err());
    }

    #[test]
    fn a_rebound_key_stays_with_its_binding_and_the_others_keep_theirs() {
        let defaults = [
            binding(KeyCode::ShiftLeft, "throttle"),
            binding(KeyCode::ArrowUp, "throttle"),
            binding(KeyCode::Space, "stage"),
        ];
        assert_eq!(
            binding_ids(&defaults),
            ["throttle#0", "throttle#1", "stage#0"]
        );
        let keys = BTreeMap::from([("throttle#1".to_owned(), KeyCode::KeyU)]);
        let bound = rebound(&defaults, &keys);
        assert_eq!(bound[0].key, KeyCode::ShiftLeft);
        assert_eq!(bound[1].key, KeyCode::KeyU);
        assert_eq!(bound[2].key, KeyCode::Space);
        // A binding gone from the records takes nothing with it.
        let keys = BTreeMap::from([("gone#0".to_owned(), KeyCode::KeyU)]);
        assert_eq!(rebound(&defaults, &keys)[2].key, KeyCode::Space);
    }

    #[test]
    fn the_rows_say_what_a_key_clashes_with_and_which_one_is_asked_for() {
        let mut data = UiData::default();
        let bindings = [
            binding(KeyCode::KeyP, "stage"),
            binding(KeyCode::KeyP, "map"),
            KeyBindingDef {
                label: None,
                ..binding(KeyCode::KeyQ, "debug")
            },
        ];
        publish_keys(&mut data, &bindings, Some("map#0"));
        let text = |path: &str| match data.get(path) {
            Some(osp_ui::UiValue::Text(text)) => text.clone(),
            _ => String::new(),
        };
        assert_eq!(text("keys.rows[0].clash"), "map");
        assert_eq!(text("keys.rows[1].clash"), "stage");
        assert_eq!(text("keys.rows[1].key"), "Press a key");
        assert_eq!(text("keys.rows[0].key"), "P");
        // Without a label it is not offered.
        assert_eq!(text("keys.rows[2].key"), "");
    }

    #[test]
    fn every_key_the_game_ships_with_that_a_player_needs_can_be_rebound() {
        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../assets/core");
        let registry = osp_mods::load(&root, None).registry;
        let docs = UiDocuments::from_registry(&registry, &mut Vec::new());
        for action in [
            "vessel.stage",
            "vessel.pitch_up",
            "game.menu",
            "view.toggle_map",
        ] {
            assert!(
                (docs.bindings.iter()).any(|b| b.action == action && b.label.is_some()),
                "{action}"
            );
        }
    }
}
