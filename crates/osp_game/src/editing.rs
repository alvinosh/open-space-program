//! What the rest of the game knows of the editor, which is `osp_editor`'s: whether it is open,
//! what a craft on show weighs by, and the last launch, which the pause menu can revert to. The
//! editor keeps these; the menu, saves, controls and flight UI only read them, so nothing here
//! needs the editor to build or run.

use bevy::prelude::*;
use osp_sim::{
    data::CraftDef, staging::Sequences, time::SimTime, universe::Snapshot, vessel::VesselId,
};

pub struct EditingPlugin;

impl Plugin for EditingPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<EditorStatus>()
            .init_resource::<LastLaunch>();
    }
}

/// Where the editor takes the UI's actions, in `FrameSet::Input` after the menu's. The dialog
/// takes its answers after it, so that a question the editor asks shows the same frame.
#[derive(SystemSet, Debug, Clone, PartialEq, Eq, Hash)]
pub struct EditorActions;

/// The editor as the game sees it, kept by the editor as it opens and closes.
#[derive(Resource, Debug, Default, Clone, PartialEq)]
pub struct EditorStatus {
    /// What the craft on show weighs by, m/s², while the editor is open.
    open: Option<f64>,
}

impl EditorStatus {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// What the craft on show will weigh by, m/s², while the editor is open: its figures are for
    /// the launch, not for where it stands on show.
    pub fn gravity(&self) -> Option<f64> {
        self.open
    }

    /// The editor is open, with a craft on show weighing by `gravity`, or is closed for `None`.
    pub fn set(&mut self, gravity: Option<f64>) {
        self.open = gravity;
    }
}

/// Where a launch goes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchPlace {
    Pad,
    Runway,
    Orbit,
}

impl LaunchPlace {
    pub fn parse(text: &str) -> Option<Self> {
        match text {
            "pad" => Some(Self::Pad),
            "runway" => Some(Self::Runway),
            "orbit" => Some(Self::Orbit),
            _ => None,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::Pad => "pad",
            Self::Runway => "runway",
            Self::Orbit => "orbit",
        }
    }
}

/// What the game can go back to: the last launch, from the moment it left.
pub struct LaunchPoint {
    /// The whole game at that moment, the launched vessel in it.
    pub game: Snapshot,
    pub launched: VesselId,
    /// What it is called, and when in the game it left.
    pub name: String,
    pub time: SimTime,
    /// How it was launched, to put it back on show.
    pub craft: CraftDef,
    pub sequences: Option<Sequences>,
    pub place: LaunchPlace,
    /// The vessel the hangar goes back to, in `game`: none if the launch started the game, or the
    /// game had nothing else to fly.
    pub flown: Option<VesselId>,
}

/// The launch the game in progress can be reverted to. A game loaded from a save has none, as a save
/// holds the game and not how it got there.
#[derive(Resource, Default)]
pub struct LastLaunch(Option<LaunchPoint>);

impl LastLaunch {
    /// The game is no longer the one that launched it.
    pub fn forget(&mut self) {
        self.0 = None;
    }

    /// The game goes back to this from now on.
    pub fn set(&mut self, point: LaunchPoint) {
        self.0 = Some(point);
    }

    pub fn point(&self) -> Option<&LaunchPoint> {
        self.0.as_ref()
    }

    /// What was launched, if the game can go back to it.
    pub fn name(&self) -> Option<&str> {
        self.0.as_ref().map(|point| point.name.as_str())
    }

    /// When in the game it left.
    pub fn time(&self) -> Option<SimTime> {
        self.0.as_ref().map(|point| point.time)
    }
}
