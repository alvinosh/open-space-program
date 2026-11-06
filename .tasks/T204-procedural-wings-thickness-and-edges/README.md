# T204: Procedural wings: root and tip thickness, edge profiles

Asked for on 2026-11-06, as in KSP's procedural wings: "on wings you should be able to set tip width,
root width, the heights for both, the profile of the leading and trailing edge".

## Scope

- `WingDef` gains a thickness at the root and at the tip, and a profile for the leading and the
  trailing edge (rounded, sharp, square) with how much of the chord each takes; the airfoil today is
  one NACA or plate section for the whole wing.
- The lifting surface's aero reads them (thickness and edges change drag and lift), the mass follows
  the volume, and old craft keep their shapes.
- Handles for the new sizes, and the menu (T203).

## Done when

- A wing set to a thin sharp-edged tip and a thick round-nosed root is drawn so, weighs so, and its
  drag differs from a plate's in the aero tests.
