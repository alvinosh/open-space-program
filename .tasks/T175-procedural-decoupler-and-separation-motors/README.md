# T175: Procedural decoupler and separation motors

A decoupler today is a component on a part with a node, a separation impulse and a crossfeed flag. It has no ring size, no strength and no shock, and a player cannot place one because there is no editor palette yet. Make it a sized part like the tanks, with the small motors that make a stage separation clean.

## Scope

- A decoupler sized from the ring diameter and the load it carries (mass above, preload, bending): its mass, its break load, and its separation impulse from springs or a pyrotechnic charge.
- Axial and radial (for boosters), and one that releases an inside joint (the lander in its adapter).
- Separation motors as small solid motors the sizer picks: retro motors that pull a spent stage away and ullage motors that settle propellant (T150 needs the second).
- The shock at release as a load on both neighbours (feeds the joint strengths of T051), and the interplay with hot staging (T159).
- A palette entry with sliders for the editor (T017).

## Done when

- A decoupler placed at a ring of a given diameter carrying the Saturn V's loads is the sized number, not a flat mass, and breaks over its limit.
- The Saturn V's stage separations use sized retro and ullage motors.

## Where it comes from

- Planning of 2026-11-01: raised as missing from the part list.
