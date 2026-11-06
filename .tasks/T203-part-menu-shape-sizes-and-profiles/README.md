# T203: Part menu: shape, sizes and profiles on a click

Asked for on 2026-11-06, after the first handles: "clicking in a part should open a menu where you can
select the shape, top and bottom radius, profile", like KSP with Procedural Parts. The handles stay
for dragging; the menu is where everything about a part is set.

## Scope

- A window for the picked part, generated from a schema in the sim (no editor code per kind of part):
  its shape (cylinder, cone, sphere, nose and so on, switched with the sizes kept where they mean the
  same), every size typed or dragged (top and bottom diameter, length), a shell's profile per
  section, domes and skirts, what a tank holds and its type, a decoupler's load.
- Sizes typed exactly (the `input` widget) as well as slid.
- Changing a stage's width offers to carry the parts stacked on it along.

## Done when

- A tank put in from the list is turned into a cone, an adapter and a nose by its menu alone, and
  every number in the menu is the one the flight uses.

## Done so far

- 2026-11-06: the part window (`osp_sim::properties`, `osp_editor::part_window`, `editor_part.json5`).
  A click on a part, or putting one on, opens it beside the pointer (pinnable, Esc or a click
  elsewhere closes it). Every field is generated from the part: name; a tank's shape (cylinder,
  cone, sphere, switched with its sizes kept), length and diameters, domes, skirts, thrust plate;
  a shell's shape (tube, taper, point), profile and caps; a wing's chords, span, sweep, dihedral,
  twist, section (plate or airfoil), thickness, bevel or camber; what a tank holds (mixes the
  content has every resource of, or nothing, which makes it structure), its tank type (only those
  that can hold it; one that cannot gives way to the first that can), utilization (a new per-tank
  override of the tank type's) and the load it is built for; material and wall; every number a
  sizer asks for; how far it is turned. Numbers are typed or stepped, choices drop down. Volume,
  propellant, dry and full mass are under them. Each change goes through `Builder::set_field`.

## Still open

- A widened stage carrying the parts stacked on it along; paint; a fill level at launch (T089).
