# T155: Staging window you can read

The stack shows a number and a row of identical icons: every engine is the same icon, the decoupler another, and nothing says what a row does, which engine it lights or what is lit now. You have to know.

## Scope

- Icons per kind: liquid engine, solid motor, vacuum engine, decoupler, fairing, RCS, parachute; from data, not from code.
- A one-line caption on the row: what it does ("Light S-IC, release pad"), and the stage's name.
- The next sequence to fire is unmistakable; lit engines and spent stages are dimmed or moved out of the list.
- A hover tooltip: the part's name, thrust, Isp, burn time left.
- Fuel bars with a number (burn time or percent).

## Done when

- Someone who has not seen the game can say what Space does next from the window alone (check with a captured screenshot, `OSP_CAPTURE_STAGING=1`).

## Done

Icons for liquid, vacuum and solid engines (`icons/vacuum_engine.svg`, `icons/solid_motor.svg`; kind from the engine record: `kind: solid`, or sea-level Isp under 60% of vacuum Isp), decoupler, fairing and RCS. Hover an icon: what it does with thrust, Isp, burn time left and fuel; hover a row's number: the whole row. The line under the stack says what Space does next ("Space: Release Booster · Light K-1 Vacuum"). `Propulsion::burn_time_left` is new. Lit and spent stages are not dimmed (the stack already holds only what has not fired; the lit engines sit under it in the amber row). Parachutes: no activation exists yet. Not checked by hand: the tooltips (a still cannot show them; they use the same `{item.x}` binding as the Resources window's).

The caption line under the stack was dropped in the redesign (T160); the row's number tooltip carries it.
