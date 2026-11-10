# T265: Recyclers and greenhouses

Processes and parts that close the loop: water recovered from waste water, CO2 and hydrogen to water
and methane, electrolysis, and a greenhouse that grows food and oxygen from water, CO2 and light,
slower in shadow unless lit at a power cost. Each loses something, so nothing makes more than it
takes. In the parts list.

Casual: a greenhouse makes supplies from power and light. Realistic: the chains.

## Done when

- A craft with water recovery lasts longer by the share its record recovers (test).
- A greenhouse stops without water, and grows slower in shadow with its lights off (test).
- Mass is kept across every chain in the stock records (test).

## 2026-11-10

T258 built the LiOH scrubber and water recovery (`mods/realistic_life_support/processes`), and the
solver runs a step's lots together, so a recycler's output is there to use in the same step. Left
here: Sabatier, electrolysis for air, the greenhouse, and the casual greenhouse.
