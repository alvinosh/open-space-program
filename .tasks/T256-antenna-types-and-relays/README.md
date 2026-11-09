# T256: Antenna types and relays for unmanned probes

A catalogue of antennas of different sizes, bands and power use: omnis, dishes that must point, and
relays that pass on others' signal. Several hops. A probe out of contact does not answer.

## Done when

- A relay at the Moon gives a probe behind it a link, through more than one hop, in a test.
- A bigger dish reaches further and draws more, and a shut-down antenna gives no link.

## Done so far

Built 2026-11-09. Only an antenna made for it relays (`relay: true`). Signals go through as many relays as
it takes, each hop checked both ways (`comms::spread_links`). The parts list has a whip, three dishes and
two relay dishes, and says how far each reaches. A dish draws more the bigger it is. An antenna can be
shut down from its part window.

Not built: more than one radio band, and dishes that have to be pointed (they count as steerable).
`Universe::next_link_change` only looks for direct links. A probe with no antenna at all can still be
commanded (T087), which T250 should settle.
