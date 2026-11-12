# T296: New game screen: a real Start button and a clear Sandbox or Career choice

Said by the user on 2026-11-12, after T289 put Sandbox and Career on the New game screen: "The start
button is horrible, and the selector is also not very UX friendly".

Today the screen is one column of menu rows: Sandbox, Career, a divider, Casual, Realistic, Custom, then
Start as one more row, then the mods one by one. Nothing sets the two choices apart from each other or
from the button that acts, and the button looks like any other row.

## Scope

- The game's kind (Sandbox or Career) and its mode (Casual, Realistic, Custom) as two clearly titled
  choices, each one picked at a glance (segmented buttons or cards with a line on what each means),
  not rows in a list.
- Start as the screen's one primary button, fixed where it is always seen (bottom right, as the
  menus' other confirm buttons), never scrolled away by the mods list.
- Same look as the rest of the menus (T185, HUD design system).

## Done when

- `OSP_CAPTURE_MODS` and `OSP_CAPTURE_CAREER` pass, and their shots of the screen show both choices
  and Start without scrolling, with many mods installed.

## Done (2026-11-12)

- The `screen` component takes a `footer`: nodes under a hairline at the bottom of the panel, never
  scrolled away (`menu_footer`). Any menu can use it for what it is for.
- New game: two titled choices as rows of segments, Game (Sandbox, Career) and How real (the
  modes), each with a line under it saying what the lit one means (`menu_hint`), and "Custom: your
  own choice of the mods below" when the mods are no mode. The mods list follows. Start is one big
  primary button in the footer, "Start career" or "Start sandbox" (`btn_large`).
- A lit control keeps its lit colour under the pointer (`update_states`): before, a chosen segment
  or a mod that was on looked unchosen while the pointer was on it, which made the old Career row
  look grey after a click. This holds in every menu.
- Tests: `osp_ui` (a footer stays under the body; the menu rules hold for the new classes).
  Captures: `OSP_CAPTURE_MODS`, `OSP_CAPTURE_CAREER` and `OSP_CAPTURE_MENUS` pass; the shots show both
  choices and Start without scrolling, with seven mods installed.
