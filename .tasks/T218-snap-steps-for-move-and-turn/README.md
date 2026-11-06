# T218: Snap steps for move and turn

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

The request was "part clipping by different offets, like 1degree 5degreee 20 degree 45 ect". Read
here as snap steps: today a turn goes 15° a step (Shift 5°) and a slide goes in steps that suit the
view, and the player cannot choose either.

## Open question

"Part clipping" in KSP means letting parts overlap. Ask whether that is wanted too (a toggle that
lets a part be placed or slid inside another, which the builder refuses now).

## Scope

- A snap control in the bar (T214): angle steps off, 1°, 5°, 15°, 22.5°, 45°, 90°; slide steps off,
  1 cm, 5 cm, 10 cm, 25 cm, 50 cm, 1 m. Shift for finer, as now.
- Used by the move and turn tools, Q and E, and placing on a surface (the position round a tank).
- The label while dragging shows the step.

## Done when

- A fin turned with each angle step lands on multiples of it, a slide with each length step lands
  on multiples of it, and both are shown in a capture.
