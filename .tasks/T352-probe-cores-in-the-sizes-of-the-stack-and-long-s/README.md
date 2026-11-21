# T352: Probe cores in the sizes of the stack, and long solar arrays

Found on 2026-11-21 rebuilding the stock craft in the builder, as a player would (the playtests `rebuild_*`, crates/osp_game/src/playtest/rebuilds.rs), after the user said "I still hate the builder, i cant even mirror fairings into a payload".

In Casual the only probe core is a 0.6 m puck (the Avionics ring, which takes a diameter, is in
Realistic only), so an uncrewed payload on a 3.7 m stack has its probe core stuck on its side. KSP
has 1.25 m and 2.5 m cores. Solar panels are square, sized by their power, so the satellite's two
long 6 m by 1.6 m arrays came out as two 3 m squares.

## Done when

- A probe core takes the stack's width, in Casual too; a solar array's length and width can be set.
