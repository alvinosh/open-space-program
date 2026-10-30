# T032: Sizers: requirement in, design out

The `procedural` field in craft files, the sizer trait that turns a requirement and a tech level into dimensions, mass and performance, the oracle harness, a palette entry for each sizer, and an inventory of every flat mass in the shipped craft.

## Scope

- A craft stores the requirement, not the sized result, so a data fix improves every craft.
- A requirement that cannot be met is a build error with the reason.
- Each sizer has an oracle: a real part it must reproduce within a stated tolerance.

## Done when

- The battery sizer reproduces the CM's 21 390 kJ within 10% in mass, and the editor's palette lists it with a slider.

## Where it comes from

- docs/systems-plan.md (step 7, Principles)
