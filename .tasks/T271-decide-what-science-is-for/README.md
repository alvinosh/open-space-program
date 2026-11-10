# T271: Decide what science is for

The game has no career. The spec's science assumes KSP's, where data buys parts. Here it could unlock
the tech tree (the Realism mod has RP-1's nodes and years, `tech.rs`), be the goals of scenarios and
missions (T165), fill a record of what has been found at each place, or a mix.

## Done when

- The choice is written here, with what it means for T272 to T275, and T139 updated.

## Decided 2026-11-10

Science buys the tech tree, as in KSP's career with Kerbalism: data that reaches the ground (sent, or
recovered as samples and drives) earns science by the experiment, the place and the body, less each
time the same is done again, and science unlocks the tree's nodes and their parts. T284 builds it. What
it means for the rest:

- T272: each experiment record has a science value, and where it was done is part of its data.
- T273, T274: science is earned when data arrives, not when it is made.
- T275: lab analysis turns a sample into data worth its sample's science, no more.
- T139 keeps the money half of a career.
