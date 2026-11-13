# T307: Editor speed: no stalls while building, even on big craft

Said by the user on 2026-11-13, among what makes the editor better: "better optimization".

What is known: the Saturn V takes about 85 ms to rebuild (T018), which is why shaping and previews
build on threads of their own. Each pointer move over a surface works out a new plan; each placement
rebuilds the craft, its rules check (overlaps sample up to 600 points a part pair) and what is shown.
Nothing has been measured in the editor since the craft grew service modules, voxel aero and
interstages.

## Scope

- Measure first: frame times and the longest frames (`OSP_PROFILE`, see the perf notes) while
  building a small rocket from nothing and while editing the Saturn V: holding a part over it,
  placing, undo, shaping a tank, moving a part, opening part settings.
- Fix what stalls the frame, biggest first. Likely suspects to check, not assume: work done on the
  main thread per pointer move, rebuilding what has not changed, the rules check on every preview,
  meshes and materials made again for parts that did not change.

## Done when

- A report of what each editor action costs on the Saturn V and on a small rocket, before and after.
- No frame over 33 ms on the M5 while holding, placing, shaping or undoing on the Saturn V.
