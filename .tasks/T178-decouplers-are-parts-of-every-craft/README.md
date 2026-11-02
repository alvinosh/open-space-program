# T178: Decouplers are parts of every craft

Raised in play on 2026-11-01: there are no decouplers on a craft, and it is odd that the Decouple action is on a hull part.

Today the stage converter (`staged.rs`) puts the `Decoupler` component on the top part of each stage that lets go of the one above: its hull, or an interstage shell on it. The part window of that part offers Decouple, and the craft has no part called a decoupler to see, select, size or place. A real craft has a separation ring (or a clamp band, or a set of bolts) at every such joint.

## Scope

- The stage converter writes a separation ring on top of every stage that lets go of the one above: a part of its own, with the `Decoupler` component on its top node, a ring at the upper stage's diameter. The hull and the shells lose the component.
- For now its height and mass come from a simple rule (a short aluminium ring, about 4% of its diameter tall), with its mass taken out of the stage's structure mass so that the stage totals stay as they were. T175 sizes it from the load it carries and adds separation motors.
- The part window, the staging tile and the Decouple button belong to the ring; sequences and `decoupler:<stage>` keep working.
- Hand-written craft that carry a decoupler as a component of another part (the glider's dolly and boosters, the Comet, the sounder, the part-built Saturn V) get a ring part of their own.
- The golden figures are rerecorded: the stages' heights, centres of mass and inertias move a little; their masses and thrusts do not.
- `docs/modding.md` says a decoupler is a part (what it is made of, what `node` means).

## Done when

- Every shipped craft has a part named Decoupler at each joint that lets go, and no other part carries a decoupler component. Selecting it shows Decouple (captured).
- The Saturn V and the first-orbit smoke flights pass with the rerecorded figures, and the placement matrix's decoupler row stays as it is.

## Notes

- A first try with a ring of zero height was refused ("a ring's height must be positive") and would not have been a decoupler: it has to have a size.
- This is the representation; the physics of the joint (shock, break load, separation impulse) is T175, and the joint strength T051.

## Done (2026-11-02)

- Every shipped craft and assembly has a part named Decoupler at each joint that lets go (116 rings; radial boosters get a small solid pad). A test holds them to it (`every_joint_that_lets_go_is_a_decoupler_part`); fairing panels keep their own hinge.
- `PartKind::Decoupler` is that part in the window (`is_decoupler`, with its size) and on its section's books. Staging releases the ring, and it leaves with the stage under it.
- A ring that only lets go hands on the question of whether engines can fire through a joint to what stands on it (`Propulsion::vented`), so a truss under it still vents.
- The converter writes the ring too, and `from_staged` lays it out the same way (`SeparationRing`).
- Mass: `3.39 d³` kg (thin aluminium), out of the part's own structure mass or the heaviest in its stage, and a tenth of that where the rule asks for more than half of what there is. Where a part has no structure mass to take it from (the Delta II's PAM-D and GEMs, Ariane 5's and the Titan III's strap-ons) it adds up to 5 kg.
- The goldens moved by the rings' height, as expected; masses and delta-v did not but for those few kilos (1.7e-3 at most, the PAM-D). The stack heights grow by 4% of each joint's diameter.

## Where it comes from

- The user's feedback of 2026-11-01.
