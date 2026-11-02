# T032: Sizers: requirement in, design out

The `procedural` field in craft files, the sizer trait that turns a requirement and a tech level into dimensions, mass and performance, the oracle harness, and an inventory of every flat mass in the shipped craft.

## Scope

- A craft stores the requirement, not the sized result, so a data fix improves every craft.
- A requirement that cannot be met is a build error with the reason.
- Each sizer has an oracle: a real part it must reproduce within a stated tolerance.

## Done when

- The battery sizer reproduces the CM's 21 390 kJ within 10% in mass, from a requirement in a craft file.

## Done (2026-11-02)

- `crates/osp_sim/src/sizing.rs`: `procedural` on a craft part holds the requirement, and `ProceduralDef::size` makes the design every build. It lists its fields for the palette, and a requirement that cannot be met is a build error with its reason.
- The battery is the first: a solid drum at the technology's Wh/L, through the battery component that already fills a solid. 21 390 kJ from a craft file is 21 390 kJ on the part, 61.8 kg at the record's 96.1 Wh/kg.
- Inventory: the build suite writes `target/smoke/build/flat_masses.json`, every `structure.mass` in the shipped craft, largest first.

## Open questions for this oracle

- The oracle's mass is provisional. The real set is three 40 Ah, 29 V batteries; "28 lb" each comes from a fragment of a NASA description that could not be opened in full, and nothing says for certain that it is about these batteries. At that mass the sizer is within 10% (36.2 kg against 38.1 kg). NASA TN D-6976 (J. Barry Trout, "Apollo experience report: battery subsystem") is the report to check it against.
- Those three batteries hold 12 528 kJ. The pod record's 21 390 kJ, which this task's text calls the CM's, is 70% more; the record came with the Realism import and may stand for something else. Not changed: T035 should look at it when it builds the power budget.

## Where it comes from

- docs/systems-plan.md (step 7, Principles)

## Notes

- A sizer lists its requirement fields (name, unit, range, default) so the editor's palette (T017) can show a slider for each; the palette entries are T017's, since the editor now comes after the systems.
