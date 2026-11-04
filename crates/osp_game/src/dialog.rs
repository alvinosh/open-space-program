//! One question at a time, over whatever is showing. Nothing is thrown away without asking, and
//! everything that asks does it here, so it looks and answers the same way everywhere.
//!
//! The actions that do something (`game.exit`, `game.delete_save:<file>`, `vessel.destroy:<id>`,
//! `editor.clear`) do it at once. A button that should ask first has the action to ask:
//!
//! - `ask.leave:<action>` does the action, asking first to save a game that would lose something:
//!   Save and continue, Don't save, Cancel.
//! - `ask.delete_save:<file>` and `ask.destroy:<vessel>` ask whether to. `ask.clear` is the editor's
//!   (see `editor`), which knows what stands in the way.
//!
//! `dialog.answer:<index>` takes an answer and `dialog.cancel` closes it, as Esc does: the menu and
//! the editor leave Esc and Back alone while a question is open, and this runs after them so the
//! same press does not reach them once it has closed.

use bevy::prelude::*;
use osp_sim::vessel::VesselId;
use osp_ui::{LayoutVisibility, UiAction, UiData, record};

use crate::{
    FrameSet,
    editor::editor_actions,
    menu::{Menu, menu_actions},
    saves::Saves,
    sim::Sim,
};

/// The layout it shows in.
const LAYOUT: &str = "dialog";

pub struct DialogPlugin;

impl Plugin for DialogPlugin {
    fn build(&self, app: &mut App) {
        app.init_resource::<Dialog>().add_systems(
            Update,
            (
                dialog_actions
                    .in_set(FrameSet::Input)
                    .after(menu_actions)
                    .after(editor_actions),
                publish_dialog,
            ),
        );
    }
}

/// How an answer's button looks.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Look {
    Plain,
    Primary,
    Danger,
}

/// What an answer does.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Then {
    /// Nothing: the question closes.
    Cancel,
    Fire(String),
    /// Save the game and then fire this. If it cannot be saved the question stays and says why.
    SaveAndFire(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Answer {
    label: &'static str,
    look: Look,
    then: Then,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Question {
    title: String,
    text: String,
    answers: Vec<Answer>,
}

impl Question {
    /// Whether to do something that cannot be undone: `yes` fires `then`, or Cancel.
    pub fn destructive(title: String, text: &str, yes: &'static str, then: String) -> Self {
        Self {
            title,
            text: text.to_owned(),
            answers: vec![
                Answer {
                    label: yes,
                    look: Look::Danger,
                    then: Then::Fire(then),
                },
                cancel(),
            ],
        }
    }

    /// Whether to save before `then`, which leaves the game. `save` is the name of the save the game
    /// goes to, if it has one.
    fn save_first(save: Option<&str>, then: &str) -> Self {
        let text = match save {
            Some(save) => format!("Anything since it was last saved as {save} will be lost."),
            None => "It has not been saved yet, so all of it will be lost.".to_owned(),
        };
        Self {
            title: "Save this game first?".to_owned(),
            text,
            answers: vec![
                Answer {
                    label: "Save and continue",
                    look: Look::Primary,
                    then: Then::SaveAndFire(then.to_owned()),
                },
                Answer {
                    label: "Don't save",
                    look: Look::Danger,
                    then: Then::Fire(then.to_owned()),
                },
                cancel(),
            ],
        }
    }
}

fn cancel() -> Answer {
    Answer {
        label: "Cancel",
        look: Look::Plain,
        then: Then::Cancel,
    }
}

/// The question being asked.
#[derive(Resource, Default)]
pub struct Dialog(Option<Question>);

impl Dialog {
    pub fn is_open(&self) -> bool {
        self.0.is_some()
    }

    /// Ask this, in place of anything being asked.
    pub fn ask(&mut self, question: Question) {
        self.0 = Some(question);
    }

    /// The heading of the question being asked, "" with none.
    pub fn title(&self) -> &str {
        self.0.as_ref().map_or("", |q| q.title.as_str())
    }
}

/// Whether leaving now would lose anything: a game in progress that its save does not hold. There
/// is nothing to save in a running tutorial, which has no Save.
fn would_lose(menu: &Menu, sim: &Sim, saves: &Saves) -> bool {
    let tutorial = (sim.universe.tutorial.as_ref()).is_some_and(|tutorial| tutorial.is_running());
    menu.playing && !tutorial && !saves.up_to_date(sim)
}

fn dialog_actions(
    mut commands: Commands,
    mut actions: MessageReader<UiAction>,
    mut dialog: ResMut<Dialog>,
    mut saves: ResMut<Saves>,
    menu: Res<Menu>,
    sim: Res<Sim>,
) {
    for action in actions.read() {
        match action.name() {
            "ask.leave" => {
                let Some(then) = action.arg() else { continue };
                if would_lose(&menu, &sim, &saves) {
                    dialog.ask(Question::save_first(saves.current_name(), then));
                } else {
                    commands.write_message(UiAction(then.to_owned()));
                }
            }
            "ask.delete_save" => {
                let Some(file) = action.arg() else { continue };
                dialog.ask(Question::destructive(
                    format!("Delete {}?", saves.name_of(file)),
                    "A deleted save cannot be brought back.",
                    "Delete",
                    format!("game.delete_save:{file}"),
                ));
            }
            "ask.destroy" => {
                let id = action.arg().and_then(|id| id.parse().ok()).map(VesselId);
                let Some(vessel) = id.and_then(|id| sim.universe.vessel(id)) else {
                    warn!("no vessel to destroy in `{}`", action.0);
                    continue;
                };
                dialog.ask(Question::destructive(
                    format!("Destroy {}?", vessel.name),
                    "A destroyed vessel cannot be brought back.",
                    "Destroy",
                    format!("vessel.destroy:{}", vessel.id().0),
                ));
            }
            "dialog.answer" => {
                let index: Option<usize> = action.arg().and_then(|index| index.parse().ok());
                let Some(question) = dialog.0.as_mut() else {
                    continue;
                };
                let Some(answer) = index.and_then(|index| question.answers.get(index)) else {
                    warn!("no answer in `{}`", action.0);
                    continue;
                };
                match answer.then.clone() {
                    Then::Cancel => {}
                    Then::Fire(then) => {
                        commands.write_message(UiAction(then));
                    }
                    Then::SaveAndFire(then) => {
                        if !saves.save(&sim) {
                            // Going on would lose the game, so the question stays.
                            question.text.clone_from(&saves.notice);
                            continue;
                        }
                        commands.write_message(UiAction(then));
                    }
                }
                dialog.0 = None;
            }
            "dialog.cancel" | "game.menu" | "menu.back" | "editor.back" if dialog.is_open() => {
                dialog.0 = None;
            }
            _ => {}
        }
    }
}

/// `dialog.title`, `dialog.text` and `dialog.answers[]` (`index`, `label`, `plain`, `primary`,
/// `danger`), and the layout showing while there is a question.
fn publish_dialog(dialog: Res<Dialog>, mut data: ResMut<UiData>, mut ui: ResMut<LayoutVisibility>) {
    if !dialog.is_changed() {
        return;
    }
    if ui.shown(LAYOUT) != dialog.is_open() {
        ui.layouts.insert(LAYOUT.into(), dialog.is_open());
    }
    let Some(question) = &dialog.0 else { return };
    data.set("dialog.title", question.title.as_str());
    data.set("dialog.text", question.text.as_str());
    let answers: Vec<_> = (question.answers.iter().enumerate())
        .map(|(index, answer)| {
            record([
                ("index", index.into()),
                ("label", answer.label.into()),
                ("plain", (answer.look == Look::Plain).into()),
                ("primary", (answer.look == Look::Primary).into()),
                ("danger", (answer.look == Look::Danger).into()),
            ])
        })
        .collect();
    data.set("dialog.answers", answers);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saving_first_offers_all_three_ways_and_carries_the_action() {
        let question = Question::save_first(Some("Game 2"), "game.load:game_1");
        let labels: Vec<_> = question.answers.iter().map(|a| a.label).collect();
        assert_eq!(labels, ["Save and continue", "Don't save", "Cancel"]);
        assert_eq!(
            question.answers[0].then,
            Then::SaveAndFire("game.load:game_1".into())
        );
        assert_eq!(
            question.answers[1].then,
            Then::Fire("game.load:game_1".into())
        );
        assert_eq!(question.answers[2].then, Then::Cancel);
        assert!(question.text.contains("Game 2"), "{}", question.text);
        // At most one primary button on a screen.
        let primary = question.answers.iter().filter(|a| a.look == Look::Primary);
        assert_eq!(primary.count(), 1);
    }

    #[test]
    fn a_game_never_saved_says_so() {
        let question = Question::save_first(None, "game.exit");
        assert!(
            question.text.contains("not been saved"),
            "{}",
            question.text
        );
    }

    #[test]
    fn a_destructive_question_can_be_cancelled() {
        let question = Question::destructive("x".into(), "y", "Delete", "z".into());
        assert_eq!(question.answers.len(), 2);
        assert_eq!(question.answers[0].look, Look::Danger);
        assert_eq!(question.answers[1].then, Then::Cancel);
    }

    #[test]
    fn the_title_says_what_is_being_asked() {
        let mut dialog = Dialog::default();
        assert_eq!(dialog.title(), "");
        assert!(!dialog.is_open());
        dialog.ask(Question::destructive("x".into(), "y", "Yes", "z".into()));
        assert_eq!(dialog.title(), "x");
        assert!(dialog.is_open());
    }
}
