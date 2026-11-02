# T181: The Apollo CSM's RCS is about 8.7 times too strong

The service module's four quads use `Hypergolic Quad` from the base game (3,870 N a nozzle, 280 s). The real ones were R-4Ds, which the mod already has as `R-4D Quad` (445 N, 290 s), and the lunar module uses. Found flying the `docking` mission: on the RCS alone the CSM accelerates at 0.49 m/s², where the real one managed about 0.06 m/s².

The S-IVB's APS uses the same record, and its real thrusters were not R-4Ds either, so look at those separately.

## Done

- The CSM has `R-4D Quad` in all three places. Found to matter by the first hand test of docking (T040): with the old thrusters, one second on the key is over the capture limit.
- A turn round takes the CSM 34 s, not 11 s.
- With a real CSM the SPS's first midcourse correction comes out 0.12 m/s over on 24 m/s, because the RCS holds the nose less firmly against a thrust line off the centre of mass. The check was ±0.1 m/s, calibrated on the old thrusters, and is now 1% of the burn or 0.1 m/s. A crew would null that on the RCS; the burn executor does not.
- Only `saturn_v` and `saturn_ib` moved in the golden figures. The S-IVB's APS keeps `Hypergolic Quad`.

## Done when

- The CSM in `saturn_v`, `saturn_ib` and the `apollo_csm` assembly has `R-4D Quad`, or the reason it cannot is written here.
- The Saturn V and Apollo 7 smoke missions and `docking` still fly; ullage and turns take as long as they now do with the weaker thrusters.
- The golden figures are rerecorded, with a note on why they moved.
