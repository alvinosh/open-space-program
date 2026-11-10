# T278: Automation: rules a vessel follows by itself

For each vessel, a list of rules: when an event happens (landed, in the air, in space, sunlight,
shadow, power above or below a share, link gained or lost; radiation and full or empty drives once
T267 and T273 are in), and its conditions hold, do actions the parts already offer
(`System::actions`). They run whether the vessel is flown or not, and a crossing is caught at its
time at any warp. A window to write them. A fixed set of events and actions, no scripting language
(T075). It grows from the avionics triggers (`TriggerDef`). Both modes.

## Done when

- "In shadow and below 20 % power, shut the experiment" happens at the crossing at real time and at
  the highest warp (test).
- Rules come back from a save.
