//! The tutorial's face in the game: the step and its hint in a window, the control it points
//! at outlined, and the buttons to move on, skip a step or give up. The steps are judged in
//! the sim (`osp_sim::tutorial`, after every tick); a step about the interface (`ui`) is
//! judged here against the UI data, since the sim cannot see the map or the warp.
//!
//! The text names keys as actions, `[vessel.stage]`, and shows the key the player has that
//! action on, so rebinding a key does not break a tutorial.

use bevy::{input::keyboard::KeyCode, prelude::*};
use osp_sim::tutorial::{Tutorial, TutorialState};
use osp_ui::{LayoutVisibility, UiAction, UiData, UiDocuments, UiHighlight};

use crate::{
    FrameSet,
    sim::{Sim, SimRestarted},
};

/// The window with the step.
const WINDOW: &str = "tutorial";

pub struct TutorialPlugin;

impl Plugin for TutorialPlugin {
    fn build(&self, app: &mut App) {
        app.add_systems(Update, run_tutorial.in_set(FrameSet::Render));
    }
}

#[allow(clippy::too_many_arguments)]
fn run_tutorial(
    mut sim: ResMut<Sim>,
    mut actions: MessageReader<UiAction>,
    mut restarted: MessageReader<SimRestarted>,
    docs: Res<UiDocuments>,
    mut ui: ResMut<LayoutVisibility>,
    mut data: ResMut<UiData>,
    mut highlight: ResMut<UiHighlight>,
    mut started: Local<bool>,
) {
    let restart = restarted.read().count() > 0;
    if restart || !*started {
        *started = true;
        ui.layouts
            .insert(WINDOW.into(), sim.universe.tutorial.is_some());
    }
    let Some(tutorial) = sim.universe.tutorial.as_mut() else {
        data.set("tutorial.active", false);
        highlight.set_if_neq(UiHighlight(None));
        return;
    };
    for action in actions.read() {
        match action.name() {
            "tutorial.next" => tutorial.complete(),
            "tutorial.skip" => {
                tutorial.end();
                ui.layouts.insert(WINDOW.into(), false);
            }
            _ => {}
        }
    }
    // A step about the interface is done when the UI says so.
    if let Some(condition) = tutorial.step().and_then(|s| s.ui.as_deref())
        && data.condition(condition)
    {
        tutorial.complete();
    }
    tutorial.take_events();
    publish(&mut data, tutorial, &docs);
    highlight.set_if_neq(UiHighlight(
        tutorial.step().and_then(|s| s.highlight.clone()),
    ));
}

/// `tutorial.*`, see `docs/modding.md`.
fn publish(data: &mut UiData, tutorial: &Tutorial, docs: &UiDocuments) {
    let def = tutorial.def();
    let step = tutorial.step();
    let name = if def.name.is_empty() {
        &def.id
    } else {
        &def.name
    };
    let state = match tutorial.state() {
        TutorialState::Running => "running",
        TutorialState::Finished => "finished",
        TutorialState::Ended => "ended",
    };
    data.set("tutorial.active", tutorial.is_running());
    data.set("tutorial.state", state);
    data.set(
        "tutorial.finished",
        tutorial.state() == TutorialState::Finished,
    );
    data.set("tutorial.name", name.as_str());
    data.set("tutorial.count", def.steps.len());
    data.set("tutorial.step", (tutorial.index() + 1).min(def.steps.len()));
    data.set(
        "tutorial.text",
        step.map_or(String::new(), |s| with_keys(&s.text, &docs.bindings)),
    );
    let hint = step.filter(|_| tutorial.hint_visible());
    data.set(
        "tutorial.hint",
        hint.map_or(String::new(), |s| with_keys(&s.hint, &docs.bindings)),
    );
    // A step to read needs the Continue button; any other can be skipped.
    data.set("tutorial.manual", step.is_some_and(|s| s.is_manual()));
    data.set("tutorial.skippable", step.is_some_and(|s| !s.is_manual()));
}

/// `text` with every `[action]` replaced by the keys bound to that action, as the player
/// has them now (`Z`, `Shift or W`), or `(no key)` if none is. Brackets around anything else
/// are left alone.
fn with_keys(text: &str, bindings: &[osp_ui::KeyBindingDef]) -> String {
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(open) = rest.find('[') {
        let Some(len) = rest[open..].find(']') else {
            break;
        };
        let name = &rest[open + 1..open + len];
        out.push_str(&rest[..open]);
        if is_action(name) {
            let keys: Vec<_> = (bindings.iter())
                .filter(|b| b.action == name)
                .map(|b| key_label(b.key))
                .collect();
            out.push_str(&if keys.is_empty() {
                "(no key)".to_owned()
            } else {
                keys.join(" or ")
            });
        } else {
            out.push_str(&rest[open..=open + len]);
        }
        rest = &rest[open + len + 1..];
    }
    out.push_str(rest);
    out
}

/// Looks like an action name (`vessel.stage`, `ui.toggle:staging`): no spaces and a dot.
fn is_action(name: &str) -> bool {
    name.contains('.') && !name.contains(char::is_whitespace)
}

/// What a key is called on a keycap.
pub(crate) fn key_label(key: KeyCode) -> String {
    let name = format!("{key:?}");
    let named = |prefix: &str| name.strip_prefix(prefix).map(str::to_owned);
    if let Some(letter) = named("Key") {
        return letter;
    }
    if let Some(digit) = named("Digit").or_else(|| named("Numpad")) {
        return digit;
    }
    let side = |what: &str| {
        name.strip_prefix(what)
            .filter(|r| matches!(*r, "Left" | "Right"))
    };
    for (what, label) in [("Shift", "Shift"), ("Control", "Ctrl"), ("Alt", "Alt")] {
        if side(what).is_some() {
            return label.into();
        }
    }
    match name.as_str() {
        "Comma" => ",".into(),
        "Period" => ".".into(),
        "Slash" => "/".into(),
        "BracketLeft" => "[".into(),
        "BracketRight" => "]".into(),
        "Escape" => "Esc".into(),
        "Equal" => "=".into(),
        "Minus" => "-".into(),
        "Backslash" => "\\".into(),
        "Semicolon" => ";".into(),
        "Quote" => "'".into(),
        "Backquote" => "`".into(),
        _ => match name.strip_prefix("Arrow") {
            Some(direction) => direction.to_owned(),
            None => name,
        },
    }
}

#[cfg(test)]
mod tests {
    use osp_ui::{KeyBindingDef, NodeDef};

    use super::*;

    /// `node` or something under it has the id.
    fn has_id(node: &NodeDef, id: &str) -> bool {
        node.id.as_deref() == Some(id) || node.children.iter().any(|c| has_id(c, id))
    }

    fn bind(key: KeyCode, action: &str) -> KeyBindingDef {
        KeyBindingDef {
            key,
            action: action.into(),
            hold: false,
            label: None,
        }
    }

    fn bindings() -> Vec<KeyBindingDef> {
        vec![
            bind(KeyCode::Space, "vessel.stage"),
            bind(KeyCode::KeyZ, "vessel.throttle_full"),
            bind(KeyCode::ShiftLeft, "vessel.throttle_up"),
            bind(KeyCode::ArrowUp, "vessel.throttle_up"),
            bind(KeyCode::F8, "ui.toggle:staging"),
        ]
    }

    #[test]
    fn keys_in_a_text_are_the_ones_the_player_has_bound() {
        let text = "Press [vessel.throttle_full], then [vessel.stage].";
        assert_eq!(with_keys(text, &bindings()), "Press Z, then Space.");
        // Rebound: the same words say the new key.
        let mut rebound = bindings();
        rebound[1].key = KeyCode::KeyU;
        rebound[0].key = KeyCode::Enter;
        assert_eq!(with_keys(text, &rebound), "Press U, then Enter.");
    }

    #[test]
    fn an_action_on_several_keys_or_none_or_with_an_argument() {
        let b = bindings();
        assert_eq!(with_keys("[vessel.throttle_up]", &b), "Shift or Up");
        assert_eq!(with_keys("[ui.toggle:staging]", &b), "F8");
        assert_eq!(with_keys("[vessel.brakes]", &b), "(no key)");
    }

    #[test]
    fn brackets_around_anything_else_are_left_alone() {
        let b = bindings();
        assert_eq!(
            with_keys("a [note] and [two words.] [", &b),
            "a [note] and [two words.] ["
        );
        assert_eq!(with_keys("no brackets", &b), "no brackets");
        assert_eq!(with_keys("", &b), "");
    }

    /// A typo in a shipped tutorial (a goal, a body, an SAS mode that is not there) would
    /// otherwise show up only when someone starts it.
    #[test]
    fn every_tutorial_in_the_base_game_can_start() {
        use osp_sim::{
            data::{BodyDef, LaunchSiteDef, ScenarioDef, SystemDef},
            system::SolarSystem,
            time::SimTime,
            tutorial::{Tutorial, TutorialDef},
        };

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../..");
        let content = osp_mods::load(&root.join("assets/core"), Some(&root.join("mods")));
        let registry = &content.registry;
        let mut diagnostics = Vec::new();
        let system = SolarSystem::from_def(&SystemDef {
            bodies: registry.all::<BodyDef>("bodies", &mut diagnostics),
        })
        .unwrap();
        let sites = registry.all::<LaunchSiteDef>("launch_sites", &mut diagnostics);
        let documents = UiDocuments::from_registry(registry, &mut diagnostics);
        let tutorials = registry.all::<TutorialDef>("tutorials", &mut diagnostics);
        assert!(!tutorials.is_empty(), "no tutorials in the base game");
        for def in tutorials {
            let scenario: ScenarioDef = registry.get("scenarios", &def.scenario).unwrap();
            let time = SimTime::from_seconds(scenario.start_time);
            if let Err(e) = Tutorial::new(&def, &scenario, &sites, &system, time) {
                panic!("{e}");
            }
            for step in &def.steps {
                let at = format!("{} / {}", def.id, step.id);
                // Every key the text names is bound, and every control it points at is there.
                for text in [&step.text, &step.hint] {
                    let shown = with_keys(text, &documents.bindings);
                    assert!(!shown.contains("(no key)"), "{at}: {text}");
                }
                if let Some(id) = &step.highlight {
                    let found = |node: &NodeDef| has_id(node, id);
                    assert!(
                        documents
                            .layouts
                            .iter()
                            .any(|l| l.id == *id || found(&l.root)),
                        "{at}: nothing has the id `{id}`"
                    );
                }
            }
        }
    }

    #[test]
    fn keys_are_named_as_on_a_keycap() {
        for (key, label) in [
            (KeyCode::KeyM, "M"),
            (KeyCode::Digit5, "5"),
            (KeyCode::Space, "Space"),
            (KeyCode::ShiftRight, "Shift"),
            (KeyCode::ControlLeft, "Ctrl"),
            (KeyCode::Period, "."),
            (KeyCode::Comma, ","),
            (KeyCode::Slash, "/"),
            (KeyCode::Escape, "Esc"),
            (KeyCode::F1, "F1"),
            (KeyCode::ArrowLeft, "Left"),
            (KeyCode::BracketRight, "]"),
        ] {
            assert_eq!(key_label(key), label);
        }
    }
}
