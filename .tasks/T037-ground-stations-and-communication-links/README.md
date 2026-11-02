# T037: Ground stations and communication links

Antennas (procedural), ground stations as records on bodies, a deterministic link budget, light delay, occultation, relay.

## Scope

- Antenna sizer: dish diameter, band, power; gain from both; mass from the diameter; omni, patch, steered dish with a slew rate. Transponder.
- Station records: Goldstone, Madrid, Canberra, Honeysuckle, Parkes, Houston and so on.
- Link budget: transmit power, both gains, free-space loss, noise floor, data rate, margin; delay; hidden behind a body or not; relay through another vessel.

## Done when

- The smoke test reports acquisition and loss of signal along Apollo 11, including the loss behind the Moon and the 1.3 s delay at the Moon.

## Done (2026-11-02)

- `comms.rs`: a deterministic budget (power, both gains, free-space loss, the receiver's noise over the data rate), a line of sight (`environment::occulted_by`), light delay. Station records in `assets/core/ground_stations`. `Universe::update_links` works out each vessel's link once a second and after a jump, to the ground or through another vessel that has one.
- The `antenna` component and sizer: dish or omni, S-band. The CSM has its dishes on the service module, the CM an omni that cannot reach the ground from the Moon.
- The Saturn V's report has margin and delay charts, and findings for acquisition and loss along the flight, the loss behind the Moon and the delay at the Moon. The mission's samples are a minute apart at most on a coast, so that is how close the times are; `next_link_change` finds them to half a second where it is used.

## Still open

- S-band only, one data rate per antenna, and a dish is assumed to point where it must (no slew rate), though an omni and a dish are told apart by their gain. A transponder is part of what the sizer weighs. Plasma blackout on entry is not modelled.
- Station figures are round ones for what the real dishes were, and the sizer's 6 kg/m² and 15 kg have no oracle.

## Where it comes from

- docs/systems-plan.md (step 12, Communications)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
