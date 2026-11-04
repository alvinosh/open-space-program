# T186: Theme tokens, type scale and one set of controls

The building blocks the rules in T185 ask for. Nothing changes on screen yet.

## Scope

- Theme: a `colors` map of named tokens, usable as `"$accent"` wherever a colour goes, merged like classes so a mod can patch one value.
- Theme classes for the type scale (28, 20, 15, 12) and the controls: `menu_rail`, `back`, `menu_row` and its value and second line, `btn`, `btn_primary`, `btn_danger`, `choice`, `tab`, hairline. Solid surface colours, since UI alpha blends in linear light.
- Layout expansion in `osp_ui::layout`, run on load so `#id` patches still work on the record as written: `screen` (the frame: Back, heading, body), `menu_row`, `setting_choice` (label plus a `choice` built from `key`, `action` and `options`).
- A test over `layouts/menu/` and `layouts/editor.json5`: no `image`, no inline `font_size`, colour, border or padding; every clickable node uses a listed control class. Another over the theme: the menu classes use only the four sizes. Each with a case that must fail.

## Done so far

- Everything above, in `osp_ui::components`, `style.rs` and the theme. Existing colours that matched a token exactly now name it, with no visible change. The rail is 560 px wide, solid, with a hard right edge.

## Done when

- A throwaway screen written with these builds and looks as T185 says, and the tests pass and catch a stray font size and a stray icon.
- `docs/modding.md` describes the tokens and the controls.
