# T317: Nothing goes under an engine, so stages cannot be stacked

Found by the `two_stages` playtest (2026-11-16). Built from the top down as in KSP: capsule, decoupler,
tank, K-1 Booster, then a decoupler under the engine for the first stage. The engine has no free
node under it, so no ring shows and the decoupler cannot go on. The hand-made craft (Pathfinder) hang
the next stage's decoupler on the upper tank's `bottom` node with the engine on a node of its own, which
a player building in the editor cannot do: the engine takes the tank's `bottom`.

## Done when

- A player can put a decoupler (or an interstage) under an engine and carry on stacking, and the
  upper engine sits inside the interstage as on the real rockets.
- `two_stages` plays through: take out its `stuck_until`.

## Done (2026-11-16)

As under a KSP engine, and with the body kept whole for the air:

- Every engine has a `bottom` node at its nozzle's exit (unless its record names one), facing down.
  The builder lets an engine's back hold any width, as it does its front.
- A part put there is as wide as the stack the engine hangs from (`fitting::below_engine`, the width
  of what holds the engine's front and how far that is above the nozzle's exit).
- A decoupler put there gets an interstage that tall (`DecouplerRequirement::interstage`, also a
  field in its settings): one tube from the ring up round the engine to the base of the stack above,
  its `top` node still at the ring (sizers can give nodes now, `Design::nodes`). It stays with the
  stage below when it lets go; the engine slides out of it, as collisions treat an open interstage
  as a ring of panels.
- `two_stages` plays through with all its hopes met: one press of Space drops the first stage and
  lights the second, which flies on straight to 68 km at 2.65 km/s. Its `stuck_until` is gone.
