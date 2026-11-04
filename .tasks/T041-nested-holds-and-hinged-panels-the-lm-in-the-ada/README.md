# T041: Nested holds and hinged panels (the LM in the adapter)

The lunar module sits inside the adapter, which holds it on releasable joints, and the adapter's panels open on actuated joints. Today 'a stage cannot let go from inside another'.

## Scope

- A release that is not at the end of the stack.
- The container component already marks inside surfaces and nodes.

## Done when

- The smoke test does transposition and extraction with the real lunar module, from the S-IVB.

## Notes

- 2026-11-02: moved up for the Moon mission. The adapter panels may simply let go until actuated joints (T030) exist.

- 2026-11-02: from T039, what a plain decoupler sheds loses its sequences (`Propulsion::from_released`; only a vessel letting go of a join keeps them, by `Sequences::take_parts`). The CSM separating from the adapter sheds the S-IVB with the lander on it, so the lander's own sequences must survive that. A release in a stack that a join turned round already goes the right way and leaves in its own frame.

## Done

- A stage on its holder's axis above the holder's base is `carried` (`Section::carried`, `Propulsion::carried`), with whatever hangs from it. It is not under its holder, so it does not block the holder's engines, and until it lets go its engines cannot fire, its RCS blocks do not work, its legs do not count and nothing arming the bottom of the stack arms it. No part of this knows about Apollo: a test holds that the lander is the only carried stage in any craft in the game today.
- What was left to stage on the parts that leave goes with them, not only when a docked vessel leaves.
- Something shed with seats on it is a vessel and not debris, and holds its attitude, so a bump does not leave it tumbling.
- A vessel that joined and has lost the part it was rooted at goes by the part it joined by: the S-IVB's name stays with the S-IVB and the lander leaves as "Lunar Module".
- `saturn_v.json5`: the drum and the solid cone went. The adapter is 8.5 m and 1.8 t, as the real one: 2 m fixed to the S-IVB and four panels that open and leave as fairing panels (no hinges yet: T030), with the real lander inside, folded. The stack came out 2 m shorter, 109.7 m without an escape tower.
- The smoke mission flies it: panels open, CSM backs off 26 m, turns, docks, the lander is pulled out, 45.4 t goes to lunar orbit on a 332 s burn (Apollo 11: 357 s), and the lander undocks there. The bridged step "the lunar module is thrown away" and the gap "transposition, docking and extraction" are gone.
- `OSP_SCENARIO=apollo11_transposition` starts from the S-IVB in orbit to fly it by hand, and `OSP_CAPTURE_TRANSPOSITION=1` captures it.

## Left open

- Nobody has flown it by hand. The script's pilot is the docking mission's.
- Pressing the stage key after docking fires the next sequence on the CSM's own list, which is the service module's jettison. The way to pull the lander out by hand is the Decouple button on the adapter's lower ring, or moving that sequence up in the staging window. Both are there, but the key is a trap.
- The panels let go instead of hinging first (T030).
- The scenario starts with the S-IVB's solid ullage motors still burning for about 5 s, because `sequences_done` lights them.

## Later

Since this task the S-IVB's own APS does its ullage and flies the stack, with the service module's thrusters shut until it separates (`rcs_shut`), and the lander's legs fold by data and not by where their nodes point.

## Where it comes from

- docs/systems-plan.md (step 16)
- old issue list, issue 8 (docs/issues.md, removed; see git history)
