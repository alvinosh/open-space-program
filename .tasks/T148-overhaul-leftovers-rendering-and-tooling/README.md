# T148: Overhaul leftovers: rendering and tooling

Rendering and tooling items the overhaul left open.

## Scope

- Meshes are one entity per piece and look, not yet merged per vessel and material.
- Paint layers follow the part's axis, so on a wing they are bands across it; decals on lofts are not done.
- A material holds up to 16 layers and a checker up to five rows; weld rings are 1.83 m apart on every stainless part.
- Each tank needs the stage's lettering laid on it too (the livery tool does that for the stage's body).
- The converter keeps a file's header comment only; the importer cannot read the nodes of eight parts that model transforms place; node sizes other than a pod's base and top come from KSP's size classes, so they are approximate.

## Done when

- Each item is done or dropped with a reason.

## Where it comes from

- vessel overhaul plan, carry-over notes (docs/vessel-overhaul.md)
