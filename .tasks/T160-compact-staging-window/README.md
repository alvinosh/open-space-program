# T160: Compact staging window

The staging window is 270 px wide with a tall row per sequence (icon, count and a fuel bar under each icon), and it stacks one row per stage: a ten-stage rocket fills the left side of the screen. Make it small without losing what T155 adds.

## Scope

- Design it together with T155: captions and tooltips must fit in the compact form (caption on hover or only on the next row, not on every row).
- Rows about half as tall: icon and count on one line, the fuel as a thin bar under the row, not under every icon.
- Sequences already fired collapse into one line ("3 fired") that expands on click.
- Only the next few sequences are shown at full size; the rest are a short list.
- Width follows the content, 200 px at most at the default scale.

## Done when

- A Saturn V's full stack takes less than a third of the screen height at 1280x720 (captured screenshot), and the next sequence is still the first thing the eye lands on.

## Done

Rows are about 29 px instead of 47, icons 18 px, the fuel bar 2 px, and the stack is 216 px wide (the clock panel above it too, since a dock is as wide as its widest window). The Saturn V's nine sequences take 317 px of 900 (35%), not the third asked for; the next step would be to show only the next few and a "+N more" line, which needs a way to still drop an icon on a collapsed row. Not done: the one-line "fired" collapse (a fired sequence already leaves the list), and the fuel bar stays under each icon rather than one per row, because different icons in a row draw from different tanks.

## Redone (same day)

The first version was still a wide stack of tall rows. Rebuilt: 18 px tiles in a grid, four to a line and then the next line; rows 20 px; the panel is about 115 px wide (it no longer stretches to the clock panel above) and the Saturn V's nine sequences take about 250 px; the caption line under the stack is gone (it made the panel wide; the row's number and each tile have it as a tooltip); dV and TWR on two short lines; + and x are plain glyphs without a button background. Theme classes `seq_*` in `themes/default.json5`, layout `layouts/staging.json5`.

Second pass: no panel round the stack at all, every row is a box of its own and only as wide and tall as its tiles (a row hands its tiles to the layout in lines of four, `tile_lines` in `ui_data.rs`, because a wrapping container does not make its parent grow); the + is a box at the top that also takes a dropped tile; dV and TWR are a box of their own at the very bottom.

Third pass: with the space saved, tiles are 24 px with 16 px icons, rows have 3 px of vertical and 4 px of horizontal padding, the badge and the dV box text are a little larger.
