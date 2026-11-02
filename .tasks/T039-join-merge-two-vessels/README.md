# T039: Join: merge two vessels

`Assembly::release` and `split` exist; nothing joins. A join takes two vessels and a pair of connectors, merges the part trees, re-roots, adds mass and inertia, conserves linear and angular momentum, merges runtime state and sequences, and removes one vessel from the universe.

## Done when

- A property test joins and releases any two craft without changing either.

## Notes

- 2026-11-02: moved up for the Moon mission. It no longer waits for the builder (T015): a join works on two vessels in flight.

## Where it comes from

- docs/systems-plan.md (step 14, Join)
