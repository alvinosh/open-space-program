# T220: The lunar module's thrusters are its model's own, configured on the stage

The ascent stage's model (`models/pods/ROC-LEMAscentBDB.glb`) has the LM's four RCS quads built in.
The craft mounts four separate `R-4D Quad` parts on nodes placed by eye (`rcs_1` to `rcs_4`, at
±1.55 m and 1.0 m up), in `lunar_module.json5` and again for the LM in `saturn_v.json5`. So a second
set of quads is drawn over the model's own, and the thrust comes from where the guesses are, not from
the quads on the model. It should be one set: the thrusters the model shows, configured on the stage
and flown as its RCS. Seen while watching Apollo 11 in the game (2026-11-06).

What there is to start from:

- The imported pod record for the part ("Apollo LM Ascent Stage" in `mods/realism/pods/capsules.json5`,
  from ROCapsules' `LEMAscentBDB.cfg`) already has the numbers: 445 N a nozzle, Isp 278 s in vacuum and
  168 s at sea level, on Aerozine 50 and MON. The craft does not use the record. 15 pods in that file
  have an `rcs` block like it.
- The converted model is one mesh, so the KSP part's thruster transforms are gone.
  `tools/blender/convert_mu.py` keeps transforms whose names match `TRANSFORM_NAME` or are listed in a
  job's `report_transforms`; converting it again with the cfg's `thrusterTransformName` gives each
  nozzle's position and direction.
- The real LM had 16 R-4Ds in four quads, fed from two RCS tanks (A and B), with an interconnect to the
  ascent engine's propellant.

## Done when

- A part can carry RCS thrusters that are part of its own model: each nozzle's position and direction
  from the model's transforms (or the part's nodes), no model of their own, and the numbers per nozzle
  on the part.
- They can be configured: thrust, Isp, which tanks feed them, and which nozzles are on. In the craft
  file first; in the part's settings in the editor once T207 has them.
- The LM ascent stage flies on its own: the four mounted `R-4D Quad` parts are gone from both craft,
  translation and rotation come from the nozzles on the model, and the RCS puffs come out of them.
- The `lunar_module`, `docking` and `saturn_v` smoke missions still fly, and the golden figures are
  recorded again with a note on why they moved.
- The other pods with thrusters in their model are listed here and done the same way or given their
  own task. Check the command module first: its craft mounts four `R-4D Quad`s too, where the real CM
  had 12 single thrusters set in its skin.
