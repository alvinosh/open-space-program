# T208: Parts list: filters by function, size and propellant

Asked for on 2026-11-06: "the parts selector is very bad, there should be more filters". The list
has four tabs (Procedural, Pods, Engines, Control), a search box, and a name and one line per row.

## Scope

- Filters as KSP's: by function (pods, tanks, engines, structure, aero, control, power, utility),
  by size (the widths the craft has, or a chosen one), by propellant (what engines burn, what
  tanks can hold), by mod.
- Rows that say what matters for the kind of part: thrust and Isp for engines, crew for pods.
- Sorting (name, mass, thrust).

## Done when

- An engine that burns hydrolox at 3 to 4 m is found in two clicks, and each filter is shown to
  narrow the list in a capture.

## Done so far

- 2026-11-06, with the grid of pictures (T215): tabs by function (pods, tanks, engines, structure,
  aero, control; power and utility have no parts yet), and two filters under them as rows of chips
  behind an icon, each shown only where the tab has something to narrow: width (under 1 m, 1 to 2,
  2 to 3, 3 to 4, over 4 m: a pod's base, an engine's nozzle exit) and propellant (kerolox,
  hydrolox, methalox, hypergolic, solid, monoprop, cold gas, electric, nuclear, other; only the
  kinds the tab has). A click on a chip puts its filter on, another click takes it off. Procedural
  parts, which take the width they go on and hold what they are set to, are never filtered out.
  The search goes over every tab and also finds a part by what it burns.
- `palette::Entry` says each part's `width`, `propellant` (`palette::Propellant::of` sorts what an
  engine or RCS block burns) and `about`, and its `detail` has the figures that matter.
- A hydrolox engine 3 to 4 m across is two clicks once the engines are shown (three from another
  tab); `OSP_CAPTURE_EDITOR_PARTS` captures each filter narrowing the engines.
- Left: filtering by mod, the widths the craft has, and sorting by name, mass or thrust.
