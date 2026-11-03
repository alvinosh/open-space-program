# T184: Eclipses and the sun's glare

## Scope

- One body's shadow on another: the Moon's umbra and penumbra crossing Earth, the sky darkening under it, Earth's shadow on the Moon. The sim already has the geometry for power (T033); the terrain and the sky pass need it per point.
- A lens flare from the Sun that follows how much of its disc can be seen, so it dies behind a vessel or the horizon rather than popping.
- Light that planets throw on each other beyond the vessel's own probe: earthshine on the Moon.

## Done when

- Captures of an eclipse from the ground and from orbit, and of the Sun passing behind a vessel, with no popping.

## Done

`crate::eclipse` with `shaders/eclipse*.wesl` and `flare.wesl`. Earthshine and moonlight are a second directional light. The game's Moon follows simplified elements, so its eclipses don't fall on historical dates; t = 33602400 s is an annular one in the Indian Ocean and t = 159653400 s a lunar one.

## Where it comes from

- The user's request of 2026-11-03. Reference: Scatterer and the celestial shadows of EnvironmentalVisualEnhancements (T067).
