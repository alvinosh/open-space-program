# Vessel construction overhaul

Plan from 2026-10-28, after the Titan II GLV's Gemini looked wrong. The goal is to build
any rocket the way KSP allows: from parts joined at nodes or on surfaces, stacked or radial,
with procedural tanks of any shape, decouplers, interstages, fairings, wings, structural
parts, and service modules that hold other parts. It should look right and stay right as
the game grows. This plan covers the open Phase 2 items of the early feature checklist (now in `.tasks/`) and lays the
ground for the Phase 3 editor, Phase 4 colliders and damage, and heating.

## Why craft look wrong today

The part tree exists (`crates/osp_sim/src/parts.rs`), but craft files still describe a
stack of stages, and `PartTree::from_craft` builds the tree from them by fixed rules.
Physics, aero and rendering all run on the stages gathered back from the tree.

1. Gemini. A stage is always laid out the same way: hull, pod at the top of the hull, shells
   above the pod. `titan2_gemini.json5` asks for an adapter, so it stands on the capsule's
   1 m nose and flares out to 2.3 m. Under the capsule is 3.8 m of 3.05 m tube with an
   open top.
2. Tank ends. `hull::stage_hull` never caps a hull's top and draws no domes, skirts or
   thrust structure; a stage is just a tube.
3. Detail on caps. `shaders/vessel.wesl` projects its detail onto a cylinder
   (`atan2(-p.z, p.x) * length(p.xz), p.y`), which breaks down on discs and domes. Meshes
   have no UVs or tangents (`meshgen.rs`).
4. Decouplers. `PartKind::Decoupler` has no mass and no geometry.
5. Radial boosters. These can't be built. Only the bottom stage's engines fire, the centre
   of mass is kept on the axis, and aero is one outline along the axis. Soyuz's four
   strap-ons are faked as one 6.1 m cylinder.
6. Special cases. Each `PartKind` has its own hardcoded shape. A top stage without parts is
   drawn as a made-up satellite (`hull::payload`, mirrored in `aero::outline`). Engine
   clusters follow `parts::engine_layout`.

The root problem is that shape, function and layout are fused into part kinds and stages.
The fix is to separate them.

## Principles

1. A craft is only parts. Symmetry, mirroring, engine clusters, subassemblies, the fairing
   tool and livery presets are build-time tools in the editor and the generator scripts.
   They write plain parts, and nothing remembers them afterwards. Nothing in the game (the
   loader, physics, staging, rendering, saves) can tell two boosters placed with symmetry
   from two placed one by one in the same spots.
2. Every part has one geometry, and everything else is derived from it: mass and inertia,
   volumes, connectors, aero, meshes, and later colliders and heating. No part kind owns a
   hardcoded shape.
3. A part is geometry plus components, like KSP's modules. Any shape
   can hold propellant, be a decoupler, a fairing panel, a lifting surface or a container,
   as long as the component can work with it. Adding a shape makes it work with every
   component, and adding a component makes it work with every shape.
4. Prefer general algorithms to per-shape formulas: numerical integration over the geometry
   for mass and volume, membrane theory for the wall of any tank of revolution, and voxels
   for aero. Today's analytic formulas become the test oracles.
5. Identity is stable. Parts and connectors have ids that survive edits, saves and
   separation.
6. There are no stopgaps. Every step builds part of the final system and keeps the game
   working.

## The part model

### Geometry

A part's geometry is a list of subparts. Each subpart is one of four kinds, and only these
four reach the rest of the code:

- Revolved: walls, each a curve in the (radius, height) half-plane made of segments (line,
  circular arc, ellipse, tangent ogive, Haack, power series, Bezier). Each wall has a
  thickness and a material, and is revolved about the part's axis, fully or over an angle
  span. Cells are the closed regions the walls enclose, and each cell is a storage volume.
  This one kind covers:
  - tanks with domes, skirts and common bulkheads; spheres; tori (a closed curve off the
    axis); conic tanks; sector tanks (a span under 360°, as in the Apollo SM)
  - decoupler rings, nose cones, adapters, interstages, heat shields
  - fairing panels (a span of 360°/n)
  - service module shells and their floors, and procedural capsules
- Lofted: a 2D section (airfoil, rectangle, ellipse, polygon, I-beam) swept through two or
  more stations, each with its own scale, offset and twist. This covers wings, fins,
  control surfaces, pylons, boxes, beams, plates, polygonal prism tanks and procedural
  batteries.
- Lattice: struts laid out in a pattern (bays, X or Warren bracing) on a revolved or lofted
  surface. This covers truss interstages (Soyuz Blok I), truss adapters and thrust frames.
- Model: a glTF mesh with declared metadata (mass, centre, inertia, nodes), plus a simple
  proxy geometry of the other kinds for aero and collisions. Engines and capsules use it.

Presets are parameter forms that compile to these kinds at load time: `cylinder_tank`
(diameter, length, dome ratio, skirts, common bulkhead), `sphere`, `torus`, `cone`, `nose`
(profile), `ring`, `fairing_panel`, `wing` (root and tip chord, span, sweep, airfoil),
`box`, `beam`, `truss`. The editor can offer any number of presets without the physics
changing. Writing the walls out directly is always possible as a fallback.

Each kind derives these properties once, then caches them:

- Mass, centre and inertia: walls by area, thickness and density; solids by volume and
  density; models from their metadata. Revolved walls are integrated along the curve with
  Gauss quadrature, and lofts and lattices over their closed meshes (divergence theorem).
  The analytic shapes in `mass.rs` (sphere, tube, frustum and the rest) check the results.
- Cell volumes, and the volume below a plane at any level: liquid settles under
  acceleration, so this gives the moving centre of mass for any tank shape, not just
  cylinders.
- A surface parameterisation, (u, v) to point, normal and frame, on the outside and, for
  hollow parts, the inside. Surface attachments are stored in it, so they follow the part
  when it is resized.
- Natural connectors (below).
- A tessellation with UVs and tangents for rendering, voxels for aero, and later colliders.

### Components

Components add behaviour to a part and read its geometry:

- Storage: a cell holds resources. Its tank type (material, pressure, insulation, allowed
  resources) is today's `TankTypeDef`. Wall thickness comes from the membrane theory of
  shells of revolution: meridional and hoop stress along the curve, from its two radii of
  curvature. That one method covers cylinders, domes, spheres, tori and cones, and
  replaces the cylinder-only sizing in `structure.rs`. Buckling stays. Lofted cells (box
  tanks, wet wings) hold low-pressure contents and are sized as plates.
- Engine (a model, or a procedural solid motor: grain in a cell plus a nozzle), RCS,
  reaction wheel, battery (capacity from volume and tech), crew, avionics, solar panel.
- Decoupler: makes one of the part's connectors releasable, with an ejection impulse and
  direction and a crossfeed switch. Which connector it opens decides which side keeps the
  part. A decoupler at both ends of an interstage gives dual-plane separation (the Saturn
  V's S-IC/S-II ring), and on a fairing panel it is the jettison.
- Lifting surface: reads a lofted geometry's planform and airfoil (today's `aero::Panel`
  maths). A control surface is a separate lofted part on a hinge joint, driven by the
  pitch, yaw and roll commands.
- Container: marks a hollow part's inside surfaces and inside nodes as places to mount
  things. Shielding needs no flag: voxel aero sees only the outer skin, and heating and
  rendering use the same test of what is enclosed.
- Gear, and later parachutes and docking ports.

### Connectors and joints

- A connector has an id, a kind (a node, or the inside or outside surface), a position and
  direction (for a node), a size, and what it passes (fluid, power, data). Geometry
  provides the natural ones:
  - revolved parts: a node at each end, sized to its rim, plus the outside surface and,
    if hollow, the inside surface
  - lofted parts: root and tip nodes plus the surface
  - models: the connectors they declare, which the importer takes from KSP's
    `node_stack_*` and `attachRules`

  Templates can add named nodes, such as a capsule's docking node.
- A joint connects two connectors. Node to node: the positions meet, plus a roll about the
  node's axis. Surface: (u, v) on the surface, a standoff and an orientation. Joints are
  undirected: the root decides which way the tree points, so rerooting changes no joint,
  and when a joint opens, the side with the root stays the vessel. Joints are rigid,
  releasable (from a decoupler), hinged (control surfaces, the Apollo SLA panels, later
  robotics) or sprung (gear).
- Links are connections outside the tree, for struts and fuel lines: a part and a surface
  point at each end. A link is dropped when either end leaves the vessel. The tree stays
  the load path and the structure that splits.

### Craft and assembly files

A craft is a list of parts and its sequences. Each part has:
- an id
- `on`, the joint towards the root (the root has none)
- either a `template` (a catalogue part such as an engine or capsule) or a `shape` and
  `components`
- its paint

Sequences list part ids. An assembly uses the same schema with no sequences and one free
connector. Assemblies are a mod collection (`assemblies`, for example "Gemini spacecraft",
"Apollo SM" or "GEM-40 booster"), and placing one writes copies of its parts. The formats
below are drafts to settle in step 4. The numbers and some of the names (such as the retro
motor template) are only illustrative.

The Titan II GLV's spacecraft end, with two service sections that are containers:

```json5
{ id: "cabin", template: "Gemini Cabin" },
{ id: "rr", on: "cabin.top", name: "R&R section",
  shape: { nose: { length: 0.95, top_diameter: 0.79, profile: "conic" } } },
{ id: "retro", on: "cabin.bottom", name: "Retrograde section",
  shape: { frustum_shell: { length: 0.9, bottom_diameter: 2.6 } },
  components: [{ container: {} }, { decoupler: { connector: "top" } }] },
{ id: "retro_motor_1", on: "retro.floor", at: { u: 0.0, v: 0.3 },
  template: "Gemini Retro Motor" },
// ...three more retro motors, each written out
{ id: "equipment", on: "retro.bottom", name: "Equipment section",
  shape: { frustum_shell: { length: 1.4, bottom_diameter: 3.05 } },
  components: [{ container: {} }, { decoupler: { connector: "top" } }] },
{ id: "oams_fuel", on: "equipment.inside", at: { u: 0.25, v: 0.4 },
  shape: { sphere: { diameter: 0.7 } },
  components: [{ storage: { type: "HP Steel Fuselage", contents: { MMH: 1 } } }] },
{ id: "spacecraft_sep", on: "equipment.bottom",
  shape: { ring: { diameter: 3.05, height: 0.06 } },
  components: [{ decoupler: { connector: "bottom", impulse: 400 } }] },
```

Radial boosters, Soyuz style. Each booster is written out in full; whether the editor
placed them with symmetry leaves no trace:

```json5
{ id: "booster_a_sep", on: "core.outside", at: { u: 0.0, v: 0.88 },
  shape: { box: { size: [0.3, 0.4, 0.6] } },
  components: [{ decoupler: { connector: "outer", direction: "out" } }] },
{ id: "booster_a", on: "booster_a_sep.outer", at: { u: 0.5, v: 0.92 },
  shape: { cone_tank: { length: 19.6, bottom_diameter: 2.68, top_diameter: 0.6 } },
  components: [{ storage: { type: "Al Stringer Tank",
                            contents: { Kerosene: 0.29, LqdOxygen: 0.71 } } }] },
// booster_b, _c and _d at u = 0.25, 0.5 and 0.75, written out the same way
```

## Physics on parts

- No stages. A resource group is whatever propellant can reach through joints and links
  that carry it. Each engine draws through that graph under flow rules: crossfeed through
  stacks, a crossfeed switch per decoupler (asparagus), drain priority, and later fuel
  lines.
- Mass: full 3D centre of mass and inertia from the parts, with the vessel's origin at its
  centre of mass. This drops "kept on the axis" from `attitude.rs`.
- Engines: any armed engine with propellant fires. Force and torque come from its own mount
  and axis, and gimbal authority from the geometry. A booster that runs dry or fails gives
  asymmetric thrust.
- Loads: each joint carries the inertial and aerodynamic loads of the side away from the
  root. Tank buckling uses them, and later so do joint strength and breakup.
- Separation: a decoupler's impulse pushes both sides apart, and the released side flies
  on as a vessel (that part exists already).
- Aero, FAR style:
  - Voxelise the vessel's outer geometry in the body frame, at a resolution scaled to its
    size, then flood-fill from outside. Anything enclosed (fairing contents, engines
    inside an interstage, a service module's tanks) is shielded without a flag.
  - From the slices along the main axis: the cross-section area and its derivatives (the
    slender-body normal force, and wave drag by the area rule), the wetted perimeter
    (skin friction) and base areas (base drag).
  - Lifting surfaces stay out of the voxel body and go through the panel model.
  - The voxels are rebuilt off the main thread when the vessel changes shape (staging,
    jettison).
  - Today's `aero::Shape` maths runs on the voxel area curve, and its current results for
    plain stacks are the oracle.

  This covers radial boosters, open interstages, fairings and odd shapes with no special
  cases.
- The staging UI, recorder, autopilot and `ui_data` read parts and resource groups.
  `propulsion::Stage` goes.

A golden test runs throughout: converted craft must reproduce today's total mass and Δv
exactly, and the centre of mass, inertia and outline within a small tolerance, with each
difference explained, until a step deliberately changes them.

## Rendering

- Meshes are generated per geometry kind, with UVs and tangents. On revolved walls, u is the
  arc length around and v the arc length along the curve, in metres. On lofts u runs along
  the span and v around the section. The shader takes its detail frame from these instead
  of projecting the position.
- Walls are real geometry with an outside and an inside face, so an open aft skirt shows
  its inside, the dome and the thrust structure, with no rules about exposed ends. Meshes
  are merged per vessel and per material and rebuilt when the vessel changes. Moving parts
  (hinges, gimbals, gear) stay separate.
- Paint is per part: layers in the part's UV space, so they stay crisp at any distance. The
  layers are:
  - base colour and finish
  - roll patterns (V-2, Redstone, Saturn), bands and stripes
  - lettering and flags as decals
  - surface finishes: spray-on foam, cork, bare stainless with weld lines, and stringer
    relief

  The `liveries` collection holds presets. Image textures from mods can be another kind of
  layer.

## The editor, later

Symmetry, mirroring and clusters place copies and keep them in step while you edit. On
reopening a craft, the editor can find counterparts again by matching identical parts
whose poses differ by a rotation about the parent's axis or a mirror, purely for editing
convenience. The fairing tool writes n panel parts with a profile fitted around what they
enclose, and finds them again by their shared base and matching profile. Assemblies come
from the catalogue, or are saved from a selection.

## Steps

Each step is one turn: build it, verify it, report back. Sizes are defined in `.tasks/README.md`.

Milestone 1, the part model:

1. (L) Geometry: the four kinds and the presets; mass, inertia, volume and fill level by
   integration; surface parameterisation. Tested against the analytic shapes, including
   a torus and a sector tank. Done 2026-10-28: `osp_sim::geometry`. Nothing uses it yet.
   For step 8:
   - Liquid settling along a revolved part's own axis costs 5 to 20 µs. Off the axis it
     goes through a mesh and costs about 0.3 ms, so warm-start it from the last tick's
     level.
   - A loft is defined by its station parameters varying linearly between stations, not
     by its mesh, so a twisted wing's volume does not depend on how its mesh is
     triangulated.
2. (M) Connectors and joints: natural connectors, node and surface attachment, joint kinds,
   undirected joints with a root. The importer brings nodes from the KSP configs. Done
   2026-10-28: `osp_sim::assembly`, and geometry nodes. Pods and engines carry `nodes`;
   608 of the 899 engine configs and 31 of the 33 pods have them. For later steps:
   - Eight parts place some nodes by model transforms, which the importer can't read. They
     are listed in `scripts/realism/out/report.json`.
   - Node sizes other than a pod's base and top come from KSP's size classes, so they are
     approximate.
   - What a connector passes and whether it releases are fields set per part, for
     step 3's components to fill.
3. (M) Components: storage (with membrane wall sizing), engine, RCS, wheel, battery, crew,
   decoupler, lifting surface, container, gear. Today's part kinds are ported onto them.
   Done 2026-10-28: `osp_sim::component`, with templates for engine, pod, RCS and wheel
   records, a `mirrored` shape, and stage tanks sized by the storage component. For
   later steps:
   - Tank walls got lighter: domes take 0.71 of the barrel's wall, and the head comes from
     the real liquid level. The historical craft were recalibrated to their real dry
     masses; the stock craft are up to 2.1% lighter dry and gained 8 to 103 m/s.
   - Each of today's kinds has a component form that weighs the same (tests in
     `component/tests.rs`); step 4's converter writes those. Shells and a fairing's nose
     then spread their mass along their real curve, not a 24-point outline or the barrel.
   - A control surface is still part of its lifting surface, flying today's flap model; a
     separate hinged part needs the aero of step 9.
   - Reverse pressure on a common bulkhead and compressive hoop stress at a shallow
     dome's knuckle are sized for yield, not buckling.
4. (L) Craft format v2 and assemblies: a loader, a converter from stages, all 42 craft
   converted (14 in `assets/core`, 28 in `mods/realism`, with
   `scripts/realism/historical_craft.py` writing v2 from now on), and the golden test.
   Until step 8, the physics gathers stages from the tree (a stage being the parts between
   release joints), so radial craft get a clear error until then. Done 2026-10-28: craft
   are plain parts (`data::CraftDef`, loaded by `osp_sim::craft`), converted by the stage
   converter (`osp_sim::staged`, run by `examples/convert_craft.rs`), gathered into stages
   by `PartTree::from_craft` (`parts/gather.rs`), and held to `tests/craft_golden.rs`.
   `AssemblyDef` and `CraftDef::place` are ready for the editor. For later steps:
   - The conversion kept every mass, delta-v, burn time, outline and air load to 1.3e-10.
     Centres of mass and inertias moved only where tanks, fins, wings, shells and fairings
     now weigh as their shapes: most by a few percent, the planes' roll inertia by up to
     23% (a tapered wing's mass lies nearer its root than a plate's), and stacks under a
     heavy fairing the most, its nose now weighing on the nose (the Titan IIIE's last
     stage by 1.2 m and 144%). The golden figures were rerecorded after.
   - A converted stage is a body (a solid cylinder of its structure mass) holding its parts
     on nodes, faithful to the old stage but not to the real vehicle; steps 7 and 14
     rebuild craft from stacked tanks, skirts and interstages. The Titan II's adapter still
     stands on the capsule's nose until step 7.
   - The bridge needs each stage to have one such body, cylinder or sphere tanks, an even
     set of plate fins at its base, wings in mirrored pairs and pods at the stage top.
   - Sequences name parts, but act on their stages until step 10.
   - The converter keeps a file's header comment only; the stock craft's other comments
     were moved onto their parts by hand.

Milestone 2, looks:

5. (M) Meshes from geometry with UVs and tangents, shader detail from them, and a per-part
   renderer to replace `stage_hull`. This fixes the caps, domes, skirts and decouplers.
   Done 2026-10-28: `osp_sim::geometry::draw` meshes every kind (walls with a back face
   and rims, lofts with caps, solids, trusses), `part_mesh.rs` turns them into Bevy
   meshes and materials, and `vessel_render.rs` draws each part on its own; `hull.rs` is
   gone. Parts carry `paint` (colour, finish, per-element pieces and an inside), and the
   shader takes its detail frame from the UVs, keeping the old projection for glTF models.
   For later steps:
   - A converted stage's body is now a skin of weightless walls (skirts, interstage,
     separation ring, thrust plate, or a payload adapter) and an `unseen` equipment ring
     that carries the structure mass as v1's solid cylinder did, so the golden figures
     held exactly. Legs, grid fins, the raceway and a payload's satellite are parts of
     their own. Steps 7 and 14 replace these bodies with real parts.
   - A control surface is cut out of its wing's loft along the hinge and turns on a pivot;
     the wing around it is drawn in up to three pieces.
   - Meshes are one entity per piece and look, not yet merged per vessel and material.
   - The Titan II's adapter still stands on the capsule's nose, drawn as it is written.
6. (M) Paint layers, finishes and liveries, with historical schemes. Done 2026-10-28:
   `paint.layers` (bands, stripes, roll-pattern checkers, lettering, SVG pictures and
   stringer relief, laid out round the part's axis), the foam, cork and stainless finishes,
   and the `liveries` collection, which the stage converter copies into a stage's parts
   (`appearance.livery`). `paint_layers.rs` packs the layers into the material and
   rasterises lettering (Barlow) and pictures into a mipmapped texture per material; the
   shader draws the rest per pixel. Ten historical schemes in
   `mods/realism/liveries/historical.json5` (V-2, Jupiter-C, Mercury-Redstone, Atlas,
   Saturn IB and V, Delta II, Electron, Starship) and a US flag. For later steps:
   - Layers follow the part's axis, so on a wing they are bands across it; decals on lofts
     wait for the editor.
   - A material holds up to 16 layers, and a checker up to five rows.
   - Weld rings are 1.83 m apart on every stainless part.
7. (S) Titan II GLV rebuilt from plain parts, and the first assembly records. Done
   2026-10-28: `mods/realism/assemblies/gemini.json5` holds the Gemini spacecraft (heat
   shield, cabin and R&R section; the retrograde section with four Star-13E and their
   propellant; the equipment section) and the Titan II GLV (separate fuel and oxidizer
   tanks in each stage, a 5.8 m second stage, two LR87s); `titan2_gemini` is the two placed
   together, written by hand (the generator only writes its launch), and
   `tests/assemblies.rs` holds it to that. Its sequences drop the equipment section before
   retrofire and the retrograde section after, and the retro rockets give about 100 m/s.
   The adapter now flares from the cabin to the Titan's 3.05 m. Along the way, engines
   burning RealFuels' densified (`Cooled…`) propellants got their plumes back. For later
   steps:
   - Each tank needs the stage's lettering laid on it too (the livery tool does that for
     converted craft); the editor will apply liveries to plain parts.
   - Gemini's OAMS and its fuel cells are mass in the equipment section, not parts yet.

Milestone 3, physics on parts:

8. (L) 3D mass, the resource graph and flow rules, engines that fire anywhere with a plume
   each, separation impulse, joint loads. `Stage` deleted. Taken in two parts:
   - 8a. Done 2026-10-28, ahead of 6 and 7, so radial craft can be built by hand: stages
     hang under each other or beside each other (`parts/gather.rs`), each knowing where its
     base is, what it hangs from and whether it is beside it; every armed engine burns from
     its own mount when no stage hangs under its stage, on its stage's propellant, with a
     plume each; the centre of mass is a point (`MassProperties::center_of_mass`), and
     thrust, air, RCS and gear loads are taken about it; released boosters are pushed
     straight out; default sequences light the boosters with their core and release them
     next; the delta-v readout counts their burn on the core's row. Tests in
     `vessel/radial.rs`. For 8b and later:
     - A stage is still a group between joints that let go, with one propellant pool.
     - The air sees the stack's outline plus each side stage's cross-section (the area
       rule) until step 9. Off-axis parts that never separate (Proton's outer tanks) are
       not in it yet.
     - A stage beside another carries only itself in the structure loads; a stage carries
       the ones it hangs under and what hangs beside those. Joint loads replace this.
     - Engines still stand upright (no canted engines), and an armed engine waits while a
       stage hangs under its own (no hot staging).
     - Converted craft that mix engines in one ring (the R-7 family, Delta II, Ariane 5,
       Atlas, Starship) and the planes (tail fin) now have their centre of mass slightly
       off the axis; the golden figures were rerecorded and nothing else moved.
   - 8b. Done 2026-10-28: each tank holds its own propellant, and each engine and RCS
     block draws from the tanks propellant reaches it from (`PartTree::fluid_groups`:
     through node joints, and across decouplers with `crossfeed` on), every tank giving
     in proportion to what it holds; a decoupler pushes the two sides apart with its
     `impulse` (1 m/s apart without one); `structure::joint_loads` gives the axial load at
     the joint each stage hangs from (the thrust and ground push on everything hanging
     from it, less its inertia; drag on the root), and tank buckling builds on it. Tests
     in `vessel/radial.rs`. The golden figures did not move. For later steps:
     - Flow has no priorities yet (drain order, fuel lines).
     - Joint loads are axial and per stage joint; joints inside a stage, shear and joint
       strength come with breakup.
     - On the ground the stages standing on it share its push by their mass.
   - 8c. `Stage` deleted. Done 2026-10-28: what changes in flight lives with the parts,
     as `Propulsion`'s engines, tanks and RCS blocks (each knowing its part and section)
     and the sections armed and with their fairing jettisoned; a `Section` (the group of
     parts between joints that let go) is worked out from the parts whenever they change,
     for the sequences, the HUD and the stack-based aero. Released parts carry their
     state off with them. The renderer and part windows find engines and RCS blocks by
     part. Nothing moved in the golden figures. For later steps:
     - The aero still sees sections as a stack (step 9), and sequences name sections
       (step 10).
9. (L) Voxel aero with lifting surfaces and shielding. Done 2026-10-29: `aero::voxel` cuts
   the parts' surfaces (`Geometry::outer`: what is drawn, with models' proxies) across the
   axis, each cut sampled along rows and columns like a run-length voxel grid, exact along
   each line, and flooded from outside, so whatever the skin encloses is shielded and a
   gap narrower than a thousandth of a row lets no air through. Each cut gives its area,
   the middle of it, its widths across x and y, the wetted perimeter (Cauchy–Crofton) and
   how far it reaches; flat faces are cut just below and just above. `Shape::new` now
   takes the `Propulsion`, runs today's section maths on that area curve with each
   section's forces at its middle, leaves engines, lifting surfaces and gear out of the
   body, and drops fins, wing strips and gear legs that are inside it. The shape is
   worked out once per change of parts (`Propulsion::shape`), over several threads:
   about 12 ms for the Saturn V. Ground contact uses the cuts' rings. For later steps:
   - Against the outline its parts draw, the sounder's loads hold to 0.2% (the cut
     polygons' areas are a thousandth short of circles').
   - The golden figures were rerecorded. Craft whose stages drew tapers, buses and noses
     their parts lack moved most: flat-topped fairings (Vostok), flat shoulders (N1,
     Sputnik), hammerhead fairing bases (Delta II, Titan IIIE, Ariane 1), payload
     antennas through the nose (Scout, Vanguard), and folded legs and grid fins (Kestrel,
     Pathfinder, Falcon 9). The glider's boom and Colossus's second stage now taper as
     their stages did.
   - Fins and wings still come from the sections, as panels; a control surface is still
     part of its wing, and the cluster of fins a `FinSet`.
   - The shape is built on the sim thread when the parts change, not in the background,
     so that flights stay deterministic.
10. (M) Sequences per part. The UI shows identical parts in a sequence under one icon with
    a count, which closes the cluster item of issue 3. Done 2026-10-29: an `Activation`
    names a part (an engine, a part hanging from a joint that lets go, a fairing panel, an
    RCS block) and engines are armed one by one; naming another part of a section in a
    craft file stands for the section's parts (`SectionActivation`), so no craft file
    changed. `Propulsion::groups` gathers a sequence's identical parts (same kind, same
    part name or engine record), anywhere on the vessel, and the panel moves a group by its
    icon or one part by its count; each icon's fuel bar is the tanks feeding its engines.
    The golden figures did not move. For later steps:
    - Groups are found by name, so two different parts given the same name share an icon.
    - Moving one part out of a group takes the last of it; which one is not chosen.

Milestone 4, building blocks and content:

11. (M) Radial craft: Soyuz, Delta II 7925 (six ground-lit and three air-lit GEMs), Titan
    IIIE, Ariane 5, and Proton's outer tanks (radial parts that never separate). Done
    2026-10-29: each booster is an assembly (`mods/realism/assemblies/boosters.json5`:
    `r7_strap_on`, `gem_40_ground`, `gem_40_air`, `ua1205`, `eap_p241`) hanging by an
    `attach` node on its axis from a node of the core pointing down, its roll turning its
    −X to the core; the cores now carry all their own propellant and the stages' fake
    bottom stages are gone. The R-7's strap-ons lean their noses in to the core's waist
    (a lofted cone that holds and weighs nothing). Titan IIIE lights its solids alone and
    the core in the air; Delta II drops six GEMs as the other three light. Proton's first
    stage is a central N2O4 tank with six UDMH tanks round it over the RD-253s. Masses
    match the generator's historical figures, `tests/assemblies.rs` holds the craft to
    their assemblies placed and to how they stage, and `convert_craft --format` writes
    craft and assemblies built as JSON in the house style. The generator (local only)
    marks the five `hand_built`. For later steps:
    - Tanks are still cylinders or spheres inside the skins (the stages need them), so the
      strap-ons' propellant sits low in them, under their noses.
    - The core's interstages and fairings are unchanged until step 12; the R-7's Blok I
      still sits on a solid interstage.
    - Strap-ons have no air rudders or verniers of their own.
12. (M) Interstages (solid, vented, truss, dual-plane) and fairings (panels, fitted
    profiles) as plain parts. Done 2026-10-29: a part where one stage hangs from another is
    open (`Part::open`, `aero::is_open`) when, cut across its axis, what it encloses is
    under half the disc it reaches round (a truss, a ring of panels with vents between).
    An engine fires while a stage hangs under its own if that stage hangs by an open part
    (hot staging), or is a hollow ring with nothing across the axis that is clear itself
    (`Propulsion::clear`). The Soyuz's Blok I stands on a steel truss that stays with the
    core and lights through it; the Titan II's interstage is six panels with vents between
    them, and its second stage fires in the hole; the Saturn V's S-IC/S-II interstage is a
    ring of its own: the S-IC lets go of it, the S-II fires through it, and it falls away
    with the next sequence. The fairing tool
    (`CraftDef::fit_fairings`, `convert_craft --fit-fairings`) fits a fairing's panels to
    what stands above their base: a barrel as wide as the base under a tangent ogive 1.3
    diameters long, 0.1 m clear. Tests in `tests/interstages.rs` and `craft.rs`. For later
    steps:
    - Default sequences treat a ring as a stage without engines: it is let go with the
      next stage's ignition; craft that drop it later write their sequences.
    - The air sees into an open interstage as far as its exposed faces, and gives them the
      pressure of faces in the open flow, so the Soyuz's truss and the Titan II's vents
      cost more drag than the cavities do in reality (the Titan II about four times its
      liftoff drag).
    - Vents narrower than a row of the cut are not seen at all; ports are drawn as gaps
      between panels.
    - Fairings are fitted by tool, not yet run over the craft (step 14).
13. (M) Service modules with contents: Gemini's sections, Apollo's SM with sector tanks,
    Soyuz's PAO. Done 2026-10-29: Gemini's equipment section holds its OAMS propellant
    (MMH and N2O4 spheres) and two fuel cells; the Apollo SM (`apollo_csm`, in
    `mods/realism/assemblies/apollo.json5`, carried by the Saturn V and IB) is a shell
    whose six bays, parted by radial beams, hold the SPS's oxidizer and fuel sump and
    storage tanks, three fuel cells and the RCS propellant round two helium spheres, with
    four RCS quads round it and the SPS under it, 6.2 t dry on 18.4 t; the Soyuz 7K-OK
    (`soyuz_7k_ok`, `soyuz.json5`) is its instrument-assembly module with the KTDU-35, its
    UDMH and AK-27 and its solar panels folded for launch, the descent module (a shape with
    a `crew` component, which the stages now take as structure) and the orbital module,
    under a fairing the fairing tool fitted round it. Tests in `tests/assemblies.rs`. For
    later steps:
    - The SM's bays hold cylinder tanks, as the real one's do; tanks of other shapes
      (sector tanks proper) wait for the physics to run on parts, since the stages take
      only cylinders and spheres.
    - Fuel cells are mass; there is no power system yet.
    - The SM's RCS uses the stock hypergolic quad, much stronger than Apollo's.
14. (M) A pass over every historical craft with the new parts. Done 2026-10-29:
    - The whole R-7 family is radial: Sputnik (8K71PS) and Vostok (8K72K) are the Soyuz's
      core and leaning strap-ons with their own engines and figures
      (`r7_strap_on_8k71ps`, `r7_strap_on_8k72k`), Vostok's Blok E on a truss it lights
      through; Sputnik's core tapers to its payload instead of a flat shoulder.
    - The Mercury-Atlas is a stage and a half: the sustainer holds all 113 t and burns from
      liftoff through the booster skirt, whose LR89s draw on it across a crossfeed
      decoupler (a stage now counts as hollow by what lies across the axis, whatever
      engines it carries). The delta-v readout burns such a fed stage with the one feeding
      it for its engines' rated burn time.
    - Every historical fairing was fitted round its payload by the fairing tool, which
      closed Vostok's flat-topped cylinder (widened to the instrument module's 2.43 m, as
      the real one was) and the antennas through the Scout's and Vanguard's noses. A pod
      need no longer sit at the top of its stage.
    - The N1's Blok V tapers to the L3 instead of a flat shoulder.
    The golden figures were rerecorded after each step. Left for later:
    - The generator's other craft (Saturn IB's clustered tanks drawn as one, Juno, the
      sounding rockets) are still converted stages, faithful to their stage figures.
    - Capsules keep their flat docking-hatch tops; the escape towers are not built.

After these: the Phase 3 editor, links (struts and fuel lines), Phase 4 colliders from the
geometry kinds, and heating with shielding.

## Choices made, and why

- Plain parts over stored symmetry: two identical layouts must be the same craft, and
  saves, splits and debris never have to keep groups consistent.
- Four geometry kinds plus presets over a shape enum per part type: new shapes reach
  mass, aero, rendering and storage for free, and spheres, tori, sectors and truss
  structures need no new code paths.
- Components over part kinds: service modules, wet wings, solid motors in any case, and
  structural parts that carry things are combinations, not new types.
- Voxel aero over per-part drag: it is the only approach here that handles radial
  boosters, fairings, open interstages and arbitrary structures without special cases.
- Procedural paint layers over imported texture sets: crisp at any distance, no licence
  questions, and image layers can still be added.
- No stopgap fix to `stage_hull`: the renderer in step 5 replaces it.

Sources: FAR's voxel model:
https://forum.kerbalspaceprogram.com/topic/19321-131-ferram-aerospace-research-v01591-liepmann-4218.
