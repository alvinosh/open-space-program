# T210: Building a plane in the editor

Asked for on 2026-11-06: "it's not intuitive how you build a plane". A plane is built standing on
its tail, like a rocket, with its belly towards +Y, and launched from the runway lying down. Nothing
says which side is the belly, wings and gear have no special help, and there is no centre of lift.

## Scope (to be planned with the user)

- Build lying down, as it will sit on the runway, when the vessel is for the runway (KSP's
  spaceplane hangar), or a turn of the whole craft.
- Mirror symmetry for wings and gear, across the plane's middle.
- Gear, intakes and control surfaces easy to find and put on (T208's filters).
- Centre of mass, of lift and of thrust shown (T205).

## Done when

- A plane is built from nothing with the pointer and takes off from the runway, in a capture.

## Done so far (2026-11-06)

The user: "it's also currently not possible to make a plane, since a command pod defaults to
pointing up and you cannot rotate it".

- New plane (`editor.new:plane`, in the list and the empty hangar's panel beside New rocket):
  the hangar shows the craft lying as it will stand on the runway (nose along the hall, belly
  down), so a pod points forward from the start. Choosing Runway or Pad turns what is on show the
  same way (`Vessel::lie_on_display`), and a craft on wheels put on show goes from the runway.
- Mirror symmetry is on for a new plane (T217): wings, gear and engines go on in pairs.
- Landing gear in the parts list: Wheel (brakes) and Nose wheel (steers), put on a surface by
  their strut, turned to roll along the craft (`builder::upright_roll` with +Y, `builder::rolls`).
- The root can be turned (T206), which needed the craft to say how its root stands:
  `points` and `roll` on the root (`craft::root_rotation`, `Assembly::with_root_pose`).
- Capture `OSP_CAPTURE_EDITOR_CRAFT=1`: capsule, tank, mirrored wings, a nose wheel and a pair of
  main wheels, launched: it stands level on its wheels on the runway (pitch 0).

## Open

- Centre of mass, lift and thrust markers (T205).
- Gear `load` is a fixed 5 t (wheel) and 3 t (nose wheel); it is not in the part settings yet.
- Not yet flown off the runway in a capture.
