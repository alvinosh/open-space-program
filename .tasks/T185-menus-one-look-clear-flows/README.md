# T185: Menus: one look, clear flows

The menus and the editor panel were added to piece by piece and it shows. This epic replaces them with
one small set of rules, one way of moving between screens, and flows that do what the player expects.
This file is the spec; the tasks under it are the order of work.

## What is wrong now

Measured on the current build (`OSP_CAPTURE_PLAY=1`, `OSP_CAPTURE_SETTINGS=1`).

Look:

- 10 font sizes across the menu and editor (10.5 to 28), 17 in the whole theme.
- 7 button looks: `menu_tab`, `menu_option`, plain `button`, `menu_item`, `menu_item menu_place`, `editor_place`, `editor_launch`.
- 3 layers of box: dimmed backdrop, panel, then a bordered card with its own background for every row.
- 7 icons that only decorate (Play, Load, Vessels, Fly a new vessel, Pad, Runway, Orbit). Load game is drawn with the flight log's icon, Vessels with the Flight window's gauge.
- The panel re-centres on every page: its top edge moves from 255 to 313 px between Play and Sound.
- List cards stop 12 px short of the buttons under them, because of the list's scrollbar padding.

Back:

- A small `menu_option` beside the heading on Play steps, a full-width plain button under Launch in the editor, and missing on Settings and Tutorials.
- Esc does not follow it: from a sub-page in a game it closes the whole menu, on the first menu it does nothing, in the editor it goes back.

Flow:

- The tab row and the Resume / Save / Save as new / Quit row stay on every page, so choosing a sound level sits beside saving the game.
- Tutorials can be started in a game, and that throws the game away without asking. So do Load and Quit.
- A vessel is chosen in a menu list, then the editor only shows it. New game goes through a Vessels page that is always empty.
- `menu.page` is not reset on close, so Esc can open the menu on the Sound tab.
- "Nothing flying yet" shows under New game while something is flying. "Vessel" and "craft" are used for the same thing.

Structure:

- `main_menu.json5` is one 590-line file; 90 of its 108 buttons are `menu_option` written out by hand, 5 per setting.
- Two step systems (`menu.page`, `menu.play`) and two ways to change vessel (`game.fly` reframes the camera, `vessel.fly` does not).

## The rules

1. **One surface.** A screen is one rail: full height, left edge, 560 px wide, always in the same place. It looks about 90% opaque, with a hard right edge and no fade (the user's call, 2026-11-04). Blending is in linear light, so the colour's own alpha is 97%: measure a capture rather than trusting the number. The flight stays visible beside it. Nothing inside the rail has a border or a background at rest. A row gets a background only under the pointer or when selected. Groups are split by a hairline.
2. **Four type sizes, two weights.** 28 (the game's name, title screen only), 20 (screen heading), 15 (everything you read or click), 12 (secondary text and section captions). Weights 500 and 600. Capitals only on the game's name and section captions.
3. **A short list of controls, all the same height family:**
   - `back`: the frame's own, see rule 4.
   - `menu_row`: full width, left aligned, 44 px. Optional line under it and optional value on the right. This is how you go anywhere.
   - `btn`: 36 px. Variants differ only in colour: plain, `primary` (at most one per screen), `danger`.
   - `choice`: joined segments, 32 px, for a value from a short list.
   - `slider`: for a percentage.
   - `tab`: text with a teal underline, inside Settings only.
4. **Back is one thing.** "‹ Back" at the top left of the rail, same spot on every screen, the editor included. Esc does exactly what it does. It is absent only on the roots.
5. **No pictures in menus.** No `image` in any menu layout or the editor panel. Words only.
6. **One word per thing.** The player sees "vessel" everywhere. "Craft" stays in code and files.
7. **Colours stay the HUD's.** Accent #38e0c4, text #e4edf4, dim #8fa3b4, control #1a2a3a, hover #23374a, selected #0e3a35, spacing 4, 8, 16, 24, 48. They become named tokens (T186).

A test enforces 2, 3 and 5 so this does not drift back.

## The screens

```
Title (no game in progress)        Pause (Esc in a game)
----------------------------       ----------------------------
OPEN SPACE PROGRAM                 Paused
                                   Game 2, flying Kestrel
Continue        Game 2, Saturn V
New game                           Resume
Load game                          ----------------------------
Tutorials                          Vessels
----------------------------       Fly a new vessel
Settings                           ----------------------------
Quit                               Save game
                                   Load game
                                   New game
                                   ----------------------------
                                   Settings
                                   Exit to main menu
                                   Quit
```

In a tutorial, Pause is only Resume, Restart tutorial, Settings, Exit to main menu, Quit.

```
Editor
+----------------------+                               +----------------------+
| ‹ Back               |                               | Saturn V             |
| Fly a new vessel     |                               | Height          112 m|
|                      |            rocket             | Mass          2939 t |
| Apollo 11            |                               | Parts             70 |
| Comet                |                               |                      |
| Saturn V        <--  |                               | LAUNCH FROM          |
| ...                  |                               | [Pad][Runway][Orbit] |
|                      |   staging                     | Cape Canaveral LC-39A|
|                      |   beside the list             | [      Launch      ] |
+----------------------+                               +----------------------+
```

| From | Choice | Goes to | Notes |
| --- | --- | --- | --- |
| Title | Continue | Flight | Newest save. Hidden when there are none. |
| Title | New game | Editor | Heading says "New game". The hangar opens empty; the first launch starts the game. |
| Title | Load game | Load | |
| Title | Tutorials | Tutorials | Only here. |
| Pause | Resume, or Esc | Flight | |
| Pause | Vessels | Vessels | Fly another or destroy any (T193). Debris left out, the one flown marked. |
| Pause | Fly a new vessel | Editor | Back returns here with the game untouched. |
| Pause | Save game | Save | "New save" on top, then the saves. The current one is overwritten at once, any other after a question. |
| Pause | Load game | Load | Asks to save first when a save is clicked. |
| Pause | New game | Editor | Asks to save first. Back returns here with the game untouched. |
| Pause | Exit to main menu | Title | Asks to save first. |
| Pause, Title | Quit | Closes | Pause asks to save first. |
| Both | Settings | Settings | Tabs: Graphics, Sound, Controls, Interface. |
| Editor | Launch | Flight | Not allowed until a vessel is chosen, nor from a pad or runway with something on it (T193). |
| Editor | Back, or Esc | Where it came from | |

Esc, in the order it is taken: cancel a key being rebound, close an open dialog (Cancel), close a popup, then
`menu.back`. On Pause that resumes, on Title it does nothing, in flight it opens Pause.

"Asks to save first" is one dialog: Save and continue, Don't save, Cancel. It is skipped when nothing has
changed since the last save.

## How it is built, so it stays easy to add to

- **A screen is a layout file** in `layouts/menu/`, `group: "menu"`, wrapped in the `screen` frame (heading and Back). Moving between them is a stack: `menu.open:<id>`, `menu.back`. Only the top one shows.
- **Adding a screen needs no Rust:** one new file, and one `menu_row` patched into `#title_items` or `#pause_items`. Mods do the same.
- **Adding a setting is one line:** `{ widget: "setting_choice", label: "Shadows", key: "graphics.shadows", action: "graphics.set:shadows", options: ["off", "low", "medium", "high", "ultra"] }`. 20 settings rows replace about 90 hand-written buttons.
- **Any question goes through one dialog** (`Dialog` resource, `dialog` layout): delete a save, overwrite a save, leave a game. The Manifest's confirm rows can move to it later.
- **Colours are named tokens** in the theme, so a mod recolours the game by patching a few values.
- **The look is tested:** see T186.

## Order

Agreed with the user on 2026-11-04: left panel, Resume on top, all of this before the Apollo work. T186 tokens and controls, T187 shell and the screens on it (signed off here, on captures), T188 leaving a game, T189 the Save screen and the slider, T190 choosing the vessel in the editor. T191 boxes around vessels stands alone and can go at any time. T192 takes the same rules to the flight windows and toolbar, once wanted.

## Where it comes from

- The user's review of the menus, 2026-11-04.
