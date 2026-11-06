# T217: Symmetry modes: radial count and mirror

Asked for on 2026-11-06, in one list of what is wrong with the editor: "the left hand side parts
menu is just full of text and not intuitive at all, there is overlap issues, also 5 buttons at the
top, all with text, not intuitive at all, also when you click on a part a menu comes up, we should
use the left hand side parts menu and display the information there [...] i want to be able to grab
a part and move it [...] we also need to add symetry modes, mirror modes, part clipping by
different offets, like 1degree 5degreee 20 degree 45 ect. And we should use icons where we can."
That list is T207 and T212 to T218.

Today symmetry is radial only, for surface parts, set with X (1, 2, 3, 4, 6, 8; Shift+X back), and
nothing on the screen shows it. A plane needs mirror symmetry (T210).

## Scope

- A symmetry button in the bar (T214) showing the mode and count; X steps the count, another key
  (KSP uses F) switches radial and mirror.
- Mirror: one copy reflected across the craft's plane that holds the parent's axis, for wings, fins,
  engines on pylons, landing gear.
- Applies to surface parts and to parts put on parts that have copies (a tank on each of four
  boosters goes on all four).
- Still build-time only: the craft holds plain parts, copies are found by matching
  (`builder::alike`), never stored as groups.

## Done when

- In a capture, four boosters go on with radial 4, a nose cone on one goes on all four, and a pair
  of wings goes on a fuselage with mirror; each set moves and turns together (T206) and undoes as one.
