# T022: Recipes, and the Saturn V's

A recipe is a record: a craft id and a list of editor commands. A tool turns any craft file into a recipe, which starts the Saturn V's and keeps it honest.

## Scope

- `examples/craft_recipe.rs`: craft file to recipe.
- The Saturn V recipe: S-IC (a tank, five F-1s, four fins by a pattern), the interstage, the S-II and S-IVB, the ullage motors and the APS (by patterns), the adapter, the `apollo_csm` assembly, the lunar module drum, and the sequences, all from palette entries.
- Flat masses and hand-shaped parts are allowed in this first build, but flagged.

## Done when

- The recipe-built craft is within 1% of the hand-built one in mass, thrust, delta-v per sequence, centre of mass and inertia.
- Every difference is listed, and every flat mass or hand-shaped part is flagged.

## Where it comes from

- docs/build-editor-plan.md (E10)
