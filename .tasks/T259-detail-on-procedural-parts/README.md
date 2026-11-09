# T259: Detail on the resizable parts: tanks, fins, nose cones, structural

Said on 2026-11-09: the placeholder parts look bad. The fixed-size ones got models (reaction wheels,
RCS, fuel line fitting, landing wheel; `tools/blender/hardware.py`). Tanks, fins, wings, nose cones and
structural parts change size in the editor, so a fixed model cannot replace them: they are plain
smooth shapes. They need detail that follows their size, made in the code: panel lines, end rings,
weld seams, a default paint scheme. The look should be our own.

The list picture of a landing wheel still shows only its strut (`part_pictures::draw` draws no gear).

## Done when

- A tank in the hangar reads as a tank at every size, and resizing keeps the detail in proportion.

## Done so far

Built 2026-11-09: `paint.trim` (`trim_layers` in `osp_sim::component`) and a `panels` paint layer in the
vessel shader. Tanks, shells, rings and wings in the parts list have it. The stock craft do not yet:
T249 rebuilds them. The blocky dev version draws none of it.
