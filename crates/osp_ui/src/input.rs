//! Text boxes: the `input` widget, for a value typed in.
//!
//! An `input` shows its `text` (a template, as a label's) until it is clicked. Then it shows
//! what is being typed, with a caret; the first key typed replaces what was there. Enter, or
//! a click anywhere else, fires its `action` with `$text` standing for what was typed
//! (`"plan.set_dv:prograde:$text"`). Escape, or handing it in with nothing typed, leaves it
//! as it was. `numeric: true` takes only what goes into a number: digits, a sign, a point and
//! an exponent.
//!
//! While a box is being typed into, keys belong to it: keybindings do nothing
//! ([`TextFocus::typing`]).
//!
//! Classes: `input` (the box: `hover_background` under the pointer, `active_background` and
//! `active_border_color` while typing into it, `active_color` for what it showed until a
//! key replaces it) and `input_text` (the text node inside).

use bevy::{
    input::{
        ButtonState,
        keyboard::{Key, KeyboardInput},
    },
    picking::{Pickable, events::PointerPress, hover::Hovered, pointer::PointerButton},
    prelude::*,
};

use crate::{
    data::UiData,
    format::{Formatters, Template},
    runtime::{StateColors, UiAction},
    style::ColorDef,
    widgets::WidgetContext,
};

/// What a box takes at most, characters.
const MAX_CHARS: usize = 24;
/// The caret's blink, s on and s off.
const BLINK: f32 = 0.53;

/// A text box. `text` is the node showing what it says.
#[derive(Component)]
pub struct TextInput {
    shown: Template,
    action: Template,
    numeric: bool,
    text: Entity,
    /// Background: normal, under the pointer, being typed into.
    background: [Color; 3],
    /// Border, if the style has one: normal, being typed into.
    border: Option<[Color; 2]>,
    /// Text: normal, and what it showed until a key replaces it.
    color: [Color; 2],
}

impl TextInput {
    /// Whether `c` may be typed into this box.
    fn takes(&self, c: char) -> bool {
        if self.numeric {
            c.is_ascii_digit() || matches!(c, '.' | ',' | '-' | '+' | 'e' | 'E')
        } else {
            !c.is_control()
        }
    }

    /// Its `action` as written, `$text` and all.
    pub fn action(&self) -> &Template {
        &self.action
    }

    /// The action for handing in `typed`.
    fn filled(&self, typed: &str, data: &UiData, formatters: &Formatters) -> String {
        let typed = typed.trim().replace(',', ".");
        self.action
            .render(data, formatters)
            .replace("$text", &typed)
    }
}

/// The text box being typed into, if any.
#[derive(Resource, Default)]
pub struct TextFocus {
    typing: Option<Typing>,
    /// A box was pressed this frame (so the press does not count as a click elsewhere).
    pressed: bool,
}

struct Typing {
    input: Entity,
    text: String,
    /// Still what the box showed: the first key replaces it.
    fresh: bool,
    /// When it was clicked, s (the caret blinks from then).
    since: f32,
}

impl TextFocus {
    /// Whether a text box has the keyboard.
    pub fn typing(&self) -> bool {
        self.typing.is_some()
    }
}

pub(crate) fn input(ctx: &mut WidgetContext) -> Entity {
    let color = |c: &Option<ColorDef>| c.as_ref().map(ColorDef::to_color);
    let style = ctx.style;
    let background = color(&style.background).unwrap_or(Color::NONE);
    let border = color(&style.border_color)
        .map(|normal| [normal, color(&style.active_border_color).unwrap_or(normal)]);
    let text_color = style.text_color();
    let (text, _) = ctx.child(ctx.entity, "input_text", "");
    let font = ctx.theme.text_style(style);
    ctx.commands.entity(text).insert((
        Text::new(String::new()),
        font.text_font(),
        TextColor(text_color),
        Pickable::IGNORE,
    ));
    let input = TextInput {
        shown: Template::parse(ctx.def.text.as_deref().unwrap_or_default()),
        action: Template::parse(ctx.def.action.as_deref().unwrap_or_default()),
        numeric: ctx.def.extra.get("numeric").and_then(|v| v.as_bool()) == Some(true),
        text,
        background: [
            background,
            color(&style.hover_background).unwrap_or(background),
            color(&style.active_background).unwrap_or(background),
        ],
        border,
        color: [text_color, color(&style.active_color).unwrap_or(text_color)],
    };
    let mut e = ctx.commands.entity(ctx.entity);
    // It keeps its own look (hover, typing), not a button's.
    e.remove::<StateColors>();
    e.insert((input, Hovered::default(), BackgroundColor(background)));
    if let Some([normal, _]) = border {
        e.insert(BorderColor::all(normal));
    }
    e.observe(press_input);
    ctx.entity
}

/// A press on a box gives it the keyboard; one being typed into before is handed in.
fn press_input(
    mut event: On<PointerPress>,
    inputs: Query<&TextInput>,
    mut focus: ResMut<TextFocus>,
    data: Res<UiData>,
    formatters: Res<Formatters>,
    time: Res<Time>,
    mut actions: MessageWriter<UiAction>,
) {
    if event.button != PointerButton::Primary {
        return;
    }
    let Ok(input) = inputs.get(event.entity) else {
        return;
    };
    event.propagate(false);
    focus.pressed = true;
    if focus
        .typing
        .as_ref()
        .is_some_and(|t| t.input == event.entity)
    {
        return;
    }
    if let Some(typing) = focus.typing.take() {
        hand_in(typing, &inputs, &data, &formatters, &mut actions);
    }
    focus.typing = Some(Typing {
        input: event.entity,
        text: input.shown.render(&data, &formatters),
        fresh: true,
        since: time.elapsed_secs(),
    });
}

/// Fire a box's action with what was typed into it, if anything was.
fn hand_in(
    typing: Typing,
    inputs: &Query<&TextInput>,
    data: &UiData,
    formatters: &Formatters,
    actions: &mut MessageWriter<UiAction>,
) {
    if typing.fresh || typing.text.trim().is_empty() {
        return;
    }
    if let Ok(input) = inputs.get(typing.input) {
        let action = input.filled(&typing.text, data, formatters);
        if !action.is_empty() {
            actions.write(UiAction(action));
        }
    }
}

/// Whether `entity` or a node above it is not displayed.
fn hidden(entity: Entity, nodes: &Query<(&Node, Option<&ChildOf>)>) -> bool {
    let mut at = Some(entity);
    while let Some(e) = at {
        let Ok((node, parent)) = nodes.get(e) else {
            return true;
        };
        if node.display == Display::None {
            return true;
        }
        at = parent.map(ChildOf::parent);
    }
    false
}

/// Keys into the box being typed into. Runs after the keybindings, which leave the keys
/// alone while a box has them, so the Escape that puts a box away does not open the menu.
#[allow(clippy::too_many_arguments)]
pub fn type_into_inputs(
    mut keys: MessageReader<KeyboardInput>,
    mouse: Res<ButtonInput<MouseButton>>,
    mut focus: ResMut<TextFocus>,
    inputs: Query<&TextInput>,
    nodes: Query<(&Node, Option<&ChildOf>)>,
    data: Res<UiData>,
    formatters: Res<Formatters>,
    mut actions: MessageWriter<UiAction>,
) {
    let pressed = std::mem::take(&mut focus.pressed);
    let Some(typing) = focus.typing.as_mut() else {
        keys.read().for_each(drop);
        return;
    };
    let Ok(input) = inputs.get(typing.input) else {
        focus.typing = None;
        return;
    };
    // Put away with what it was on: the box is gone.
    if hidden(typing.input, &nodes) {
        focus.typing = None;
        return;
    }
    let mut done = mouse.get_just_pressed().next().is_some() && !pressed;
    let mut cancel = false;
    for key in keys.read() {
        if key.state != ButtonState::Pressed || done || cancel {
            continue;
        }
        match &key.logical_key {
            Key::Enter => done = true,
            Key::Escape => cancel = true,
            Key::Backspace => {
                if typing.fresh {
                    typing.text.clear();
                }
                typing.fresh = false;
                typing.text.pop();
            }
            _ => {
                let Some(text) = &key.text else {
                    continue;
                };
                for c in text.chars().filter(|&c| input.takes(c)) {
                    if typing.fresh {
                        typing.text.clear();
                        typing.fresh = false;
                    }
                    if typing.text.chars().count() < MAX_CHARS {
                        typing.text.push(c);
                    }
                }
            }
        }
    }
    if cancel {
        focus.typing = None;
    } else if done && let Some(typing) = focus.typing.take() {
        hand_in(typing, &inputs, &data, &formatters, &mut actions);
    }
}

/// Show each box's text (what is typed into it, with a blinking caret, while it has the
/// keyboard) and its look.
pub fn show_inputs(
    focus: Res<TextFocus>,
    data: Res<UiData>,
    formatters: Res<Formatters>,
    time: Res<Time>,
    mut inputs: Query<(
        Entity,
        &TextInput,
        &Hovered,
        &mut BackgroundColor,
        Option<&mut BorderColor>,
    )>,
    mut texts: Query<(&mut Text, &mut TextColor)>,
) {
    for (entity, input, hovered, mut background, border) in &mut inputs {
        let typing = focus.typing.as_ref().filter(|t| t.input == entity);
        let (shown, color) = match typing {
            Some(typing) => {
                let lit = (((time.elapsed_secs() - typing.since) / BLINK) as u32).is_multiple_of(2);
                let caret = if lit { "|" } else { " " };
                let color = input.color[usize::from(typing.fresh)];
                (format!("{}{caret}", typing.text), color)
            }
            None => (input.shown.render(&data, &formatters), input.color[0]),
        };
        if let Ok((mut text, mut text_color)) = texts.get_mut(input.text) {
            if text.0 != shown {
                text.0 = shown;
            }
            text_color.set_if_neq(TextColor(color));
        }
        let look = if typing.is_some() {
            2
        } else {
            usize::from(hovered.get())
        };
        background.set_if_neq(BackgroundColor(input.background[look]));
        if let (Some(mut border), Some(pair)) = (border, input.border) {
            border.set_if_neq(BorderColor::all(pair[usize::from(typing.is_some())]));
        }
    }
}

#[cfg(test)]
mod tests {
    use bevy::input::{InputPlugin, keyboard::NativeKeyCode};
    use serde_json::json;

    use super::*;
    use crate::{UiDocuments, UiPlugin};

    fn app() -> App {
        let mut app = App::new();
        app.add_plugins((MinimalPlugins, InputPlugin, UiPlugin));
        let layout = json!({
            "id": "panel",
            "root": {
                "children": [{
                    "widget": "input",
                    "text": "{dv|fixed:1}",
                    "action": "set:prograde:$text",
                    "numeric": true,
                }],
            },
        });
        app.insert_resource(UiDocuments {
            layouts: vec![serde_json::from_value(layout).unwrap()],
            ..default()
        });
        app.world_mut().resource_mut::<UiData>().set("dv", 12.34);
        app.update();
        app
    }

    fn the_box(app: &mut App) -> Entity {
        let mut query = app.world_mut().query_filtered::<Entity, With<TextInput>>();
        query.single(app.world()).unwrap()
    }

    fn shown(app: &mut App) -> String {
        let input = the_box(app);
        let text = app.world().get::<TextInput>(input).unwrap().text;
        app.world().get::<Text>(text).unwrap().0.clone()
    }

    /// As a click on the box would.
    fn click(app: &mut App) {
        let input = the_box(app);
        let mut focus = app.world_mut().resource_mut::<TextFocus>();
        focus.typing = Some(Typing {
            input,
            text: "12.3".into(),
            fresh: true,
            since: 0.0,
        });
    }

    fn key(app: &mut App, logical_key: Key, text: Option<&str>) {
        let window = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Unidentified(NativeKeyCode::Unidentified),
            logical_key,
            state: ButtonState::Pressed,
            text: text.map(Into::into),
            repeat: false,
            window,
        });
        app.update();
    }

    fn typed(app: &mut App, text: &str) {
        for c in text.chars() {
            let s = c.to_string();
            key(app, Key::Character(s.as_str().into()), Some(&s));
        }
    }

    fn actions(app: &mut App) -> Vec<String> {
        let messages = app.world().resource::<Messages<UiAction>>();
        let mut cursor = messages.get_cursor();
        cursor.read(messages).map(|a| a.0.clone()).collect()
    }

    #[test]
    fn a_box_shows_its_value_and_hands_in_what_is_typed() {
        let mut app = app();
        assert_eq!(shown(&mut app), "12.3");
        click(&mut app);
        app.update();
        assert!(app.world().resource::<TextFocus>().typing());
        // The first key replaces the value; letters do not go into a number.
        typed(&mut app, "-4x5.5");
        assert!(shown(&mut app).starts_with("-45.5"));
        key(&mut app, Key::Backspace, None);
        assert!(shown(&mut app).starts_with("-45."));
        key(&mut app, Key::Enter, None);
        assert_eq!(actions(&mut app), ["set:prograde:-45."]);
        assert!(!app.world().resource::<TextFocus>().typing());
        assert_eq!(shown(&mut app), "12.3");
    }

    #[test]
    fn escape_or_nothing_typed_leaves_it_as_it_was() {
        let mut app = app();
        click(&mut app);
        typed(&mut app, "7");
        key(&mut app, Key::Escape, None);
        assert!(!app.world().resource::<TextFocus>().typing());
        click(&mut app);
        key(&mut app, Key::Enter, None);
        assert!(actions(&mut app).is_empty());
    }

    fn space(app: &mut App, state: ButtonState) {
        let window = app.world_mut().spawn_empty().id();
        app.world_mut().write_message(KeyboardInput {
            key_code: KeyCode::Space,
            logical_key: Key::Space,
            state,
            text: (state == ButtonState::Pressed).then(|| " ".into()),
            repeat: false,
            window,
        });
        app.update();
    }

    #[test]
    fn keybindings_wait_while_a_box_has_the_keys() {
        let mut app = app();
        app.world_mut()
            .resource_mut::<UiDocuments>()
            .bindings
            .push(serde_json::from_value(json!({ "key": "Space", "action": "stage" })).unwrap());
        app.update();
        space(&mut app, ButtonState::Pressed);
        assert_eq!(actions(&mut app), ["stage"]);
        space(&mut app, ButtonState::Released);
        app.world_mut().resource_mut::<Messages<UiAction>>().clear();
        click(&mut app);
        space(&mut app, ButtonState::Pressed);
        assert!(actions(&mut app).is_empty());
    }
}
