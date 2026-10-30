# Modding Open Space Program

Open Space Program is data-driven: the base game is itself a mod (`assets/core`), and
everything it contains (planets, engines, tanks, craft, scenarios, the UI, the theme,
keybindings) can be added to, replaced or patched by other mods without touching code.

Mods are data-only for now. A scripting layer will plug into the same records and actions
later.

- [Quick start](#quick-start)
- [Mod structure](#mod-structure)
- [Records and collections](#records-and-collections)
- [Patches](#patches)
- [User interface](#user-interface)
- [Reference: data keys](#reference-data-keys)
- [Reference: actions](#reference-actions)
- [Troubleshooting](#troubleshooting)

## Quick start

1. Create `mods/my_mod/mod.json5`:

   ```json5
   { id: "my_mod", name: "My Mod", version: "0.1.0", dependencies: ["core"] }
   ```

2. Add files (see below), start the game, and press **F5** to reload after each edit.
3. Press **F2** to see loaded mods and any problems, and **F12** to browse every value the
   UI can display.

`mods/example_mod` shows every mechanism in a few small files.

Files use [JSON5](https://json5.org): JSON with comments, trailing commas, unquoted keys
and single quotes.

## Mod structure

```text
mods/my_mod/
  mod.json5            manifest (required)
  engines/*.json5      records; the first directory names the collection
  layouts/*.json5
  patches/*.json5      changes to other mods' records
  textures/...         any other files (overridable by path)
```

### Manifest

| Field | Meaning |
| --- | --- |
| `id` | Unique id (required). |
| `name`, `version`, `authors`, `description` | Shown in the Mods window. |
| `dependencies` | Mods that must be installed; they load first. Missing → this mod is skipped. |
| `load_after` / `load_before` | Soft ordering against mods that may or may not be installed. |
| `enabled` | `false` to keep the mod installed but inactive. |

**Load order:** `core` first, then the other mods with dependency and ordering constraints
respected, ties broken alphabetically by id. Cycles and missing dependencies are
reported, and the affected mods are skipped; the game still starts.

## Records and collections

A record file contains one object or an array of objects, each with a string `id`. Records
are grouped by collection, which is the first directory under the mod root. A later mod
that defines the same `(collection, id)` **replaces** the record; use patches to change
parts of it instead.

| Collection | Contents | Notes |
| --- | --- | --- |
| `bodies` | Planets, moons, stars | Keplerian `orbit` (J2000 ecliptic, degrees), IAU `rotation`, optional `atmosphere`, `terrain` dataset and `max_terrain_height` (m) |
| `engines` | Engines | `thrust_vac` (N), `isp_vac`/`isp_sl` (s), `min_throttle`, `response`, `ignitions`, `requires_ullage`, `gimbal_range` (degrees, 0 = fixed), `propellants: { Name: mass ratio }`, `length` and `exit_diameter` (m), `exit_pressure` (Pa), `plume`, `turbine_exhaust`, `model`, `model_scale` (draw the model this many times its size), `nodes` (see below); see [Engine response and plumes](#engine-response-and-plumes), optional `thrust_curve` ([share of propellant left, share of full thrust] points, as a solid's) |
| `plumes` | Exhaust looks | `propellants` it is the exhaust of (alternatives with `\|`), `kinds`, `fallback`, `core`, `flame` and `shock` colours (linear RGB), `luminance` (cd/m² per m at full thrust), `soot` (0..1), `temperature` (K), `smoke` (colour), `haze` (0..1); see [Engine response and plumes](#engine-response-and-plumes) |
| `resources` | Propellants and gases | `density` (kg/m³ as stored), storage `temperature` (K) and `pressure` (Pa) |
| `materials` | Structural materials | `density` (kg/m³), `yield_strength` and `elastic_modulus` (Pa) |
| `liveries` | Paint schemes for stages, which the stage converter copies into parts | `name`, `paint`; see [Stage appearance](#stage-appearance) |
| `tank_types` | How procedural tanks are built | `material`, operating `pressure` (Pa), `min_gauge` (m), `stiffening`, `buckling_factor`, `insulation` (kg/m²), `min_temperature` (K), `utilization`, `safety_factor`, `design_load` (g), optional `propellants` allowlist, `cast` for a solid motor's grain; see [Procedural tanks and fins](#procedural-tanks-and-fins) |
| `reaction_wheels` | Reaction wheels | `mass` (kg), `torque` (N·m about each axis) |
| `rcs` | RCS blocks (four nozzles each) | `mass` (kg), `thrust` (N per nozzle, vacuum), `isp_vac`/`isp_sl` (s), `propellants: { Name: mass ratio }`, `model` |
| `pods` | Crew pods (capsules, cockpits) | `mass` (kg, with crew), `crew`, `diameter` and `height` (m), optional `top_diameter` (m, a third of `diameter` by default), `model`, `nodes` (see below) |
| `craft` | Vessels | `parts`, each a `template` or a `shape` with `components`, held `on` another; optional `sequences`; see [Craft](#craft) and [Staging](#staging) |
| `launch_sites` | Launch sites | Position, `heading`, optional `pad`, `roads` and `runway`; see [Launch sites](#launch-sites) |
| `scenarios` | New-game setups | `name` and `description` (shown in the main menu, which lists scenarios in load order); `start: { type: "LaunchSite", site: "<id>" }`, `{ type: "Runway", site: "<id>" }` (at the start of the site's runway, on its wheels), `{ type: "Flight", body, latitude, longitude, altitude, heading, speed, climb }` (in the air, nose along the flight path, wings level; degrees, metres and m/s) or `{ type: "Orbit", body, periapsis_altitude, apoapsis_altitude, inclination, longitude_of_ascending_node, argument_of_periapsis, mean_anomaly }` (degrees, from the body's equator; `plane: "<body>"` instead takes the plane of a body that orbits it, like the Moon; `spin: [pitch, yaw, roll]` starts it turning, °/s); optional `others` and `target`; `OSP_SCENARIO=<id>` starts one without the menu |
| `layouts` | UI windows and panels | See [User interface](#user-interface) |
| `themes` | Style classes | All theme records merge in load order |
| `keybindings` | Key → action | All keybinding records combine |

See the files in `assets/core` for complete, commented examples of each.

### Nodes

Engines and pods can list `nodes`: where other parts join them, for when craft are built
from parts instead of stages. Each node has:

- a `name`: `top` and `bottom` for the ends of a stack, `attach` for the point a part sticks
  to a surface by
- a `position`, and a `direction` pointing out of the part, the way whatever joins there
  lies. Both are in metres in the model's frame, with +Z up: from the base centre for a pod,
  from the gimbal pivot of the unscaled model for an engine.
- a `size`: the diameter of what fits there, m.

```json5
nodes: [
  { name: "bottom", position: [0, 0, 0], direction: [0, 0, -1], size: 2.286 },
  { name: "top", position: [0, 0, 1.7385], direction: [0, 0, 1], size: 0.997 },
],
```

The Realism mod's importer takes them from the KSP configs (`node_stack_*`, `node_attach`).

### Craft

A craft is a list of plain parts. Each is a catalogue record (`template`: the id of an
engine, pod, RCS block or reaction wheel) or a `shape` with what it is made of and does
(see [Parts and components](#parts-and-components)), and every part but the root is held
`on` another, towards the root:

```json5
{
  id: "hop",
  name: "Hop",
  parts: [
    // The capsule's stage: a floor holding the capsule.
    { id: "cabin", shape: { shell: { bottom_diameter: 1.9, sections: [{ length: 0.05 }],
                                     cap_bottom: true } },
      components: [{ container: {} }],
      nodes: [{ name: "seat", position: [0, 0, 0], direction: [0, 0, 1] }] },
    { id: "capsule", on: "cabin.seat", template: "C-1 Capsule" },
    // The booster hangs under it by its top, which lets go.
    { id: "booster", on: "cabin.bottom", by: "top",
      shape: { shell: { bottom_diameter: 1.8, sections: [{ length: 12 }], cap_bottom: true } },
      structure: { mass: 600 },
      components: [{ container: {} }, { decoupler: { node: "top" } }],
      nodes: [{ name: "engine", position: [0, 0, 0], direction: [0, 0, -1] },
              { name: "tank", position: [0, 0, 0], direction: [0, 0, 1] }] },
    { id: "engine", on: "booster.engine", by: "top", template: "K-1 Booster" },
    { id: "tank", on: "booster.tank", shape: { cylinder_tank: { diameter: 1.8, length: 10 } },
      components: [{ storage: { type: "Stiffened Al 2219",
                                contents: { Kerosene: 1, LqdOxygen: 2.36 } } }] },
  ],
}
```

- `id`: unique within the craft, without dots; sequences name parts by it.
- `on: "<part>.<connector>"`: the node or surface of the part it hangs from. `by` is its
  own node or surface held there (`bottom` by default). Where one end is a surface, `at`
  says where on it: `u` around and `v` along it (each 0 to 1), `side` (`front`, or `back`
  inside a container) and `standoff` (m). `roll` turns the part about the line it is held
  along (degrees).
- `nodes`: nodes of its own besides its shape's, in its frame, where other parts are held.
- `paint`: how it looks; see [Paint](#paint).

How parts were placed (one by one, with symmetry, as a cluster) leaves no trace: two
craft with the same parts in the same places are the same craft. The physics still
gathers parts into stages: each stage is the parts between joints that let go (a
decoupler's), with one body, a container standing upright whose `bottom` node is as wide
as the stage and whose `top` node is where a stage above stands. The root's stage stands on
the vessel's axis. A stage hangs under the one holding it, on the same axis, or beside it
anywhere round it (a radial booster); engines stand upright anywhere on their stage, so a
cluster is laid out by its nodes' positions. Engines burn once armed if no stage hangs under
theirs. Each engine and RCS block draws from the tanks propellant can reach it from: through
joints at nodes (not on surfaces), and across a decoupler only if its `crossfeed` is on; the
tanks it reaches give in proportion to what they hold, so they drain together. A craft the
stages cannot hold is refused with what it lacks.

A booster beside the core, held 12 m up the core's side by a decoupler on its own node:

```json5
{ id: "core", shape: { shell: { bottom_diameter: 2.4, sections: [{ length: 22 }], cap_bottom: true } },
  components: [{ container: {} }],
  nodes: [{ name: "east", position: [1.2, 0, 12], direction: [1, 0, 0] }, ...] },
{ id: "east", on: "core.east", by: "side",
  shape: { shell: { bottom_diameter: 1.6, sections: [{ length: 14 }], cap_bottom: true } },
  components: [{ container: {} }, { decoupler: { node: "side" } }],
  nodes: [{ name: "side", position: [-0.8, 0, 12], direction: [-1, 0, 0] },
          { name: "engine", position: [0, 0, 0], direction: [0, 0, -1] }, ...] },
{ id: "east_engine", on: "east.engine", by: "top", template: "K-1 Booster" },
```

By default the first sequence lights the bottom stage with the stages beside it, and the
next releases those. The air sees boosters by their own surfaces, as it sees everything
else (see [Aerodynamics](#aerodynamics)).

An assembly (collection `assemblies`) is a group of parts written the same way, with no
sequences and a `free: "<part id>.<connector>"` where it is held when placed: a spacecraft,
a booster with its decoupler. Placing one copies its parts into a craft and leaves no trace
of it; the editor will offer them. The Realism mod's Gemini spacecraft and Titan II GLV are
assemblies (`mods/realism/assemblies/gemini.json5`), and its `titan2_gemini` craft is the
two placed together.

### Writing craft as stages

A craft can also be written as a stack of stages, as the sections below describe, and
converted into parts:

```sh
cargo run -p osp_sim --example convert_craft --release -- mods/my_mod/craft/*.json5
```

Each stage becomes a body with its parts on nodes of it where the stage puts them, its
shells stacked on top, and a decoupler on its top that lets go of the stage above. The
converter keeps the comment at the top of a file. The Realism mod's generator writes its
craft this way.

### Parts and components

A part is a shape, what its structure is made of, and components for what it does. Any
component works with any shape that has what it needs:

```json5
{
  name: "Equipment section",
  shape: { shell: { bottom_diameter: 3.05, sections: [{ length: 1.4, top_diameter: 2.3 }] } },
  structure: { material: "Al 2219-T87", thickness: 0.002 },   // or { mass: 180 }
  components: [{ container: {} }, { decoupler: { node: "top" } }],
}
```

- `structure`: a `material` with the `thickness` of walls, the `solidity` of solids (the
  share that is material, 1 by default) and the `member_area` of trusses (m²); or a total
  `mass` spread the same way. Without either it weighs nothing.
- `storage`: `type` (a tank type), `contents` (mass ratios), and the `cell` it fills when
  the shape has more than one (a common bulkhead's `lower` and `upper`). Its walls are
  sized as above.
- `engine`, `rcs`, `reaction_wheel`: `{ id }` of a catalogue record. Their mass sits at the
  part's origin.
- `battery`: a set `charge` (kJ), or a `technology` (a `batteries` record) that fills one of
  the part's solids and weighs what such a battery does.
- `crew`: `seats`.
- `decoupler`: the `node` it lets go of (which decides which side keeps the part), the
  `impulse` it pushes the two sides apart with (N·s; without one they part at 1 m/s), and
  `crossfeed`: propellant flows across it until it lets go (an asparagus stage, a booster
  feeding its core).
- `lifting_surface`: the part's loft is its planform and airfoil, with an optional
  `control` surface as on wings.
- `container`: things can mount on the inside of its walls and on its inside `nodes`.
- `gear`: a leg's `length`, `wheel_radius`, `stroke`, `load`, `mass`, `brakes` and
  `steering`, its strut out along the part's −Z.

`{ mirrored: <shape> }` reflects a shape left to right, for the left wing of a right one.
Engine, pod, RCS and reaction wheel records become parts of this kind when placed.

In a `compound` shape, a placed shape with `unseen: true` is inside the part and not
drawn, but weighs: the room its equipment takes.

#### Paint

Parts are drawn from their shapes: walls show their outside and, past an open end, their
inside; lofts, solids and trusses are drawn whole. The detail of a finish (orange peel,
weave, brushing) follows the surface. `paint` says how each piece looks:

```json5
paint: {
  color: [0.85, 0.85, 0.83],        // linear RGB; the finish's own if left out
  finish: "paint",                  // paint, composite, metal, char, foam, cork or stainless
  pieces: {                         // by element; "<shape>/<element>" in a compound
    "skin/interstage": { color: [0.03, 0.03, 0.035], finish: "composite" },
  },
  layers: [                         // over the outside, each covering those before it
    { band: { along: [18.5, 22.5], color: [0.01, 0.01, 0.01], finish: "composite" } },
    { stripe: { around: [0.0, 0.06], along: [6, 18], color: [0.01, 0.01, 0.01] } },
    { checker: { around: 4, along: [0, 3.5, 7], color: [0.01, 0.01, 0.01] } },
    { text: { text: "UNITED STATES", at: [0.125, 29], height: 1.5, vertical: true,
              color: [0.01, 0.01, 0.01] } },
    { image: { file: "decals/us_flag.svg", at: [0.125, 37], size: [3.4, 1.8] } },
    { stringers: { along: [38, 42], count: 120, depth: 0.012 } },
  ],
  inside: { finish: "composite" },  // the back of its walls; bare metal by default
},
```

Without a colour, a finish is white paint, black composite, bare aluminium, soot,
orange-brown foam, cork or bare steel. Stainless steel shows its rings' weld seams.

Layers are laid out round the part's axis: `around` in turns from its +X towards +Y (a
stripe from one turn to another runs through 0 if the second is smaller), `along` in metres
up the axis from the part's origin. A `checker` is a roll pattern of `around` sectors and
rows between the heights in `along`, every other cell painted, from the second sector of the
first row (`first: true` for the first). `text` is lettered in Barlow, its capitals `height`
m tall, centred on `at` (`[around, along]`), reading round the part or down it if `vertical`.
`image` is an SVG the mod ships (`file`, from the mod's root), `size` m wide round the part
and tall along it. `stringers` are relief only. A layer without a `finish` keeps the
finish under it. Bands, stripes, roll patterns and stringers are worked out per pixel and
stay sharp however close; lettering and pictures are textures fine enough for a close look.
A part holds up to 16 layers.

### Craft mass and control

A stage's `diameter` and `length` shape its rotation as well as its looks. Every part is a
simple shape with its own mass and inertia, placed where it sits: `structure_mass` a solid
cylinder through the whole stage, each tank its wall (a thin tube, or a spherical shell),
engines points on the stage base, RCS blocks points on its forward rim, reaction wheels a
point halfway up, the crew pod a solid frustum from `diameter` to `top_diameter` at the
top, shells their walls, fins and wings flat plates, gear legs points halfway down their
struts, and a fairing two half shells. Propellant is in the tanks: each holds its share of what is left in its
stage, a liquid as a column settled at the tank's aft end and a compressed gas filling the
tank. Together they give the centre of mass and the moments of inertia, which change as
tanks drain and stages drop. A tank low in a long stage keeps its propellant low, so place
tanks (`position`) where that mass belongs: a plane wants its propellant near the centre of
mass, so that its trim hardly changes as the tanks drain.

Control torque comes from the bottom stage's burning engines swivelling up to
`gimbal_range`, and from reaction wheels anywhere in the stack. A single engine sits on the
axis and cannot roll the vessel; two to six engines sit on a ring at half the stage
radius, and seven or more on a ring at 65% around a central engine, so clusters roll by
gimballing sideways. With the engines off, only reaction wheels and RCS can turn the
vessel, so give upper stages and payloads some.

A stage's `rcs` blocks sit evenly around its forward rim (list four for full control). Each
has four nozzles: forward and back ones pitch, yaw and push along the axis, sideways ones
roll and push sideways. They burn the propellants they name from tanks on the same stage,
so give the stage a tank of it. RCS only fires while the player has it on (R). Engines with
`requires_ullage` only light while the vessel is being pushed forward, and after a coast
the RCS forward translation (H) is what does that.

### Procedural tanks and fins

Tanks and fins are not catalogue parts: a stage describes them, and the game sizes them
from their material and what they must withstand.

```json5
tanks: [
  // Cylinders stack up the stage from its base, in order, at the stage's diameter
  // unless `diameter` is given; `position` (m above the stage base) puts one higher.
  // Length includes the √2:1 domes.
  { type: "Isogrid Al-Li", length: 9.6, design_load: 6.5,
    propellants: { Kerosene: 1, LqdOxygen: 2.36 } },          // mass ratio
  // Spheres (pressurant bottles) sit inside the stage, off the load path.
  { type: "COPV", shape: "sphere", diameter: 1.35, propellants: { Nitrogen: 1 } },
],
fins: { count: 4, root_chord: 2.4, tip_chord: 1.0, span: 1.1, sweep: 1.2,
        thickness: 0.02, material: "Al 2219-T87" },
```

A **tank** is filled to `utilization` of its internal volume in the given mass ratio, using
each resource's density. It can only hold a resource if its type is insulated down to the
resource's storage temperature (bare metal is good to 77 K: liquid oxygen yes, liquid
hydrogen no), is rated for its storage pressure (compressed gases need a COPV), and allows
it (`propellants`, if listed). Each of its walls is as thin as the material allows: the
design pressure is the type's operating `pressure` plus the hydrostatic head of the full
tank at `design_load` g (the tallest column, each resource standing at the bottom of the
tank as high as its share fills it), and every wall must keep the material
`safety_factor` below yield at that, but is never thinner than `min_gauge`. Walls are sized
by membrane theory along their curve, so a barrel's hoops carry the pressure times its
radius, a √2:1 dome only 0.71 of that and a sphere half; a flat end bends as a plate and
comes out much heavier. Mass is the walls times `stiffening` (isogrid ribs, stringers,
liners) plus `insulation`. That gives two strengths:

- **Burst:** the internal pressure at which the weakest wall yields. In flight the tank sees its
  operating pressure plus the hydrostatic head of what is left in it at the current
  acceleration, so an upper stage that rides a 6 g booster with full tanks needs a higher
  `design_load` than a first stage, which is only full at liftoff.
- **Buckling:** the axial load at which the cylinder collapses: thin-shell buckling with
  the NASA SP-8007 knockdown factor, times `buckling_factor` for grid stiffening, plus the
  load the tank's own pressure carries (which is all that holds up a balloon tank). In
  flight a tank carries everything stacked above it at the current acceleration, plus the
  drag on the nose; on the pad, the stack's weight. Tanks whose barrels stand side by side
  in a stage (Proton's outer tanks round its core) carry it together, each its share by
  strength.

A type marked `cast: true` holds a solid motor's grain. The motor's case is its engine
record's mass and, built for the chamber pressure, carries the stack, so the grain presses
on no wall and the tank is only its liner: give it no `pressure` and it adds next to
nothing.

A **fin** is a solid plate of its `material`. The air pushes on it at its own angle of
attack (see [Aerodynamics](#aerodynamics)) and its root bends; it fails when the root
reaches yield.

When any part goes past 100% of its strength it fails: fins tear off; a tank failure breaks
the vessel up (engines, RCS and controls stop, and warp halts). The Structure window shows
every part's make-up, strength and current load.

### Shells

A stage's `shells` are procedural bodies of revolution stacked on top of it, bottom first:
nose cones, size adapters between stages, plain cylinders. The game adds no nose of its
own: the stack ends where its last shell does, and without one its top is flat.

```json5
shells: [
  // From the stage's diameter down to 2.4 m over 1.5 m, under a narrower stage.
  { length: 1.5, top_diameter: 2.4 },
  // A von Kármán nose closing to a point.
  { length: 3.6, top_diameter: 0, profile: "haack", mass: 150 },
],
```

- `length` (m); `bottom_diameter` (m, by default what the shell stands on); `top_diameter`
  (m, by default its bottom diameter; 0 closes it to a point, and a top left open at the
  top of the stack is a flat face).
- `profile`: `conic` (straight, the default), `ogive` (a tangent ogive, at least as long as
  its radius changes), `elliptical` (blunt; a hemisphere when as long as its radius),
  `parabolic` or `haack` (von Kármán, the least wave drag). The curved ones leave their
  base parallel to the axis, so they meet a cylinder smoothly.
- `mass` (kg, spread over the wall; 0 by default, for when `structure_mass` already counts
  it) and an optional `name`.

Shells stand above the stage's `length` (its hull's) and add to its height. They carry
wiring but no propellant, and fall away with their stage. The air, the mass and the
drawing all go by the same outline. Under a `fairing`, the stage's shells are the
fairing's nose: its halves wrap them, their mass joins the fairing's, and they go with it
when it is jettisoned, so such a stage must be the top of the stack.

### Aerodynamics

The air works on a vessel the way Ferram Aerospace Research does in KSP. The parts'
surfaces, as they are drawn (with a model's proxy shape standing in for it), are cut
across the axis into thin sections, and each cut is filled in from outside: only the outer
skin meets the air, and whatever it encloses (a fairing's payload, engines in an
interstage, a service module's tanks) is shielded. An open skirt counts as solid across
its mouth; the gaps in a truss do not. Each section is pushed by the air it actually
meets, including the vessel's own rotation, at the middle of its area, so boosters on one
side push off the axis. Lift, drag, stability and damping all follow from the shape:

- A section whose cross-section grows along the flow (a nose, a pod, a step up to a wider
  stage) lifts towards where it points. A bare stack is therefore unstable, like a real
  launcher: the engines' gimbal or SAS has to hold it. Fins behind the centre of mass pull
  the centre of pressure back so the vessel turns into the wind by itself.
- Each fin lifts at its own angle of attack (DATCOM lift slope subsonic, Ackeret bounded by
  slender-wing theory supersonic, flat-plate crossflow beyond), at a quarter of its mean
  chord, moving to half of it supersonic.
- Drag is skin friction (by Reynolds number) plus pressure: blunt faces and steep slopes
  cost the most, especially through Mach 1, where drag rises steeply. A base or step down
  leaves a wake that pulls back, except where engines are firing out of the base.
- Blunt bodies use Newtonian flow: a capsule's heat shield takes the full stagnation
  pressure, and since a pod's mass sits low, a capsule falls heat shield first.
- Wings are worked out in strips along their span, each at its own angle of attack: lift
  slope from the wing's aspect ratio and sweep, a stall at a lift coefficient set by the
  airfoil's thickness, flat-plate flow beyond, induced drag, and downwash from the wing
  onto the tailplane behind it.

Engines sit in the wake of the base they stand on and are left out of the body, as are
lifting surfaces and gear, which have their own models; fins, wing strips and gear inside
the body feel no air. A flat face across the axis is flat to the air too: a fairing or
stage with an open or flat top meets the full stagnation pressure, so give it a nose or a
taper where the real vehicle has one.

### Wings and control surfaces

A stage can carry `wings`: wings, tailplanes and tail fins. Each is a trapezoidal panel
standing out from the stage's hull; `mirror: true` adds the matching panel on the other
side. Vessel axes: +X is the pilot's right, +Y their feet, +Z the nose, and the stage's
base is at the bottom.

```json5
wings: [
  {
    name: "Wing",
    position: 1.1,       // root leading edge, m above the stage base
    angle: 0,            // where around the hull: 0 right, 90 down, 180 left, 270 on top
    root_chord: 1.0, tip_chord: 0.45, span: 7.0,  // m; span from the hull to the tip
    sweep: 0.15,         // tip leading edge behind the root's, m
    dihedral: 3,         // degrees, tip up
    incidence: 1.5,      // degrees, leading edge up
    thickness: 0.15,     // airfoil thickness over chord (sets the stall and profile drag)
    mass: 160,           // kg, both panels
    mirror: true,
    // Ailerons on the outer 40% of the span, 22% of the chord, 15 degrees each way.
    control: { axes: ["roll"], chord: 0.22, span: [0.55, 0.95], deflection: 15 },
  },
],
```

A control surface answers the rotation commands in `axes` (`pitch`, `yaw`, `roll`; list
two for elevons or ruddervators). Which way each one moves follows from where it is: a
right aileron goes up when the left goes down, an elevator behind the centre of mass goes
up to pitch up, a canard ahead of it goes down. SAS uses them too; their authority grows
with dynamic pressure. A wing's mass counts towards the stage, and the panels set the
vessel's moments of inertia, so a winged vessel rolls, pitches and yaws differently.

The `glider` craft is a worked example: let go of its boosters and wheels it trims to glide
hands off at about 38 m/s. Wings cannot be torn off yet.

The glider also takes off from a runway (`glider_runway`). Two `S-1` solid boosters (RATO
bottles, each a radial stage on the tail boom with a `Solid Grain` tank of `HTPB`) take it
from the runway to about 70 m/s in eleven seconds, and a dolly (a slim stage held to the
cabin's belly, three gear legs on it) carries the wheels; three sequences light the
boosters, let them go and let the dolly go. Throttle up before the first press: a solid
lights with the throttle like any engine. Without the dolly a vessel has no wheels, and
touching the ground faster than 10 m/s is a crash.

A winged vessel has to be stable to fly: its centre of mass ahead of the point where the
air's push on the whole vessel acts. A long nose ahead of the wing pushes that point
forward (it lifts like a rocket's nose), and in yaw it fights the fin; the `comet` craft
moved its wing back and grew its fin until both settled.

### Landing gear

A stage's `gear` legs stand out from its hull like wings do (from the hull of a stage
beside the axis, if it hangs there: a dolly), each a sprung, damped strut with a wheel at
its end:

```json5
gear: [
  {
    name: "Main gear",
    position: 3.6,       // strut mount, m above the stage base
    angle: 50,           // around the hull: 90 straight down, less splays a right leg out
    length: 1.2,         // hull to axle, fully extended, m
    wheel_radius: 0.3,
    stroke: 0.25,        // strut travel, m
    load: 2300,          // kg it is sprung for: standing still it sits a third into its travel
    mass: 80,            // kg, both legs
    mirror: true,
    brakes: true,        // B holds the brakes
  },
  { name: "Nose gear", position: 0.8, length: 0.84, wheel_radius: 0.22, stroke: 0.2,
    load: 560, mass: 40, steering: 25 },   // steers with the yaw command, degrees
],
```

Wheels roll along the nose, grip sideways like tyres and brake up to the friction their
load allows; fixed gear adds drag in the air. Near the ground a vessel on gear is
integrated in 2 ms substeps. Anything else that touches the ground (the hull, a wing tip)
skids on it; hitting the ground with it faster than 5 m/s is a crash. Standing still on its
wheels a vessel is parked, and it rolls again once its engines push.

Mind the geometry: the main wheels go just behind the centre of mass (or it sits on its
tail), the struts' lengths decide whether it stands level, and the track must be wide
enough for the springs to hold it upright (roughly `k · track² / 2` above
`mass · g · height`, k the strut stiffness), or it rolls onto a wing.

### Engine response and plumes

Engines do not jump to whatever the throttle says. Each has a thrust level (a share of
full thrust) that follows the throttle through its `response`:

```json5
min_throttle: 0.4,          // the lowest share it holds while running
response: {
  spool_up: 1.4,            // s from ignition to full thrust...
  startup: [[0, 0], [0.08, 0.3], [0.3, 0.42], [0.65, 0.82], [1, 1]],  // ...along this curve
  spool_down: 0.6,          // s from cutoff until the thrust has tailed off to 1%
  throttle_time: 2.5,       // s to sweep the thrust from 0 to 100%
},
```

`startup` points are `[share of spool_up, share of full thrust]`, piecewise linear; a start
throttled below full follows the curve until it reaches the setting. Throttle settings
below `min_throttle` run the engine at that floor (1.0: not throttleable, any setting is
full thrust). Leaving `response` out gives an ideal engine that responds at once. Thrust and
propellant flow follow the level, so a tail-off still pushes and settles propellant; the
burn executor cuts early by what the tail-off will still give.

The plume is drawn from the engine's state: it grows out of the nozzle as the engine spools
up, and shortens and dims as it throttles down. `exit_pressure` (Pa, at full thrust; scales
with thrust) against the ambient pressure sets its shape: at or below the ambient pressure
the jet is a narrow column with shock diamonds; as the air thins it overtakes it, the cells
stretch and the plume fans out and fades into vacuum. Well below the ambient pressure (an
overexpanded sea-level engine such as an RS-25 at liftoff) or well above it, each cell ends
in a bright Mach disk. Without `exit_pressure` it is guessed from the Isp the engine loses
at sea level. The jet leaves along the gimballed nozzle and bends within a few metres onto
where the exhaust has been going over the last half second or so, leaning into the airflow
the harder the air pushes against the jet, so quick gimbal moves do not swing the whole
plume. The jets of engines that share a look merge into one plume.

The look comes from a `plumes` record: the one the engine's `plume` names, or else the one
whose `propellants` the engine all burns (the one naming the most wins), so a new hydrolox
engine looks like hydrolox without saying so. An entry of `propellants` may list alternatives
(`"Kerosene|RP-1"`), a cooled propellant counts as the plain one, and `kinds` (`solid`) picks a
look by the kind of engine instead. The look with `fallback: true` is for an engine no other
look fits, so none goes without a plume; electric engines have none. A look has the gas's own glow (`core`, `flame`
and `shock` colours and `luminance`), the particles it carries (`soot`: they hide what is
behind them, glow like a black body up to `temperature` and are smoke of colour `smoke` once
cool) and the `haze` it condenses into (the white trail in the air, the sunlit cloud in thin
air). Give an engine a look of its own when its cycle changes the flame: the stock `Kerolox
Staged` is the cleaner, brighter flame of an oxygen-rich staged-combustion engine.

An open-cycle (gas-generator) engine dumps the exhaust that drove its turbine, and
`turbine_exhaust` says where: `{ pipe: { exit: [x, y, z], diameter } }` for a pipe of its own
(the centre of its outlet in the model's axes from the gimbal pivot, nozzle down −y, in
metres; it points along the nozzle), which shows as a dark, sooty streak beside the plume
until it catches fire with the air, or `"nozzle"` where it lines the nozzle wall, a dark
collar under the bell before the plume bursts into flame (the F-1's).

### Staging

Stages and staging are separate. A stage is a section of the vessel between
joints that let go. An **activation** is one action on one part, and the player can do any
of them by hand: right-click the part and press its button in the part window (Ignite on
an engine, Decouple on a decoupler or the part under it, Jettison on a fairing panel, RCS
on). What the stage key does is only a list of **sequences** of them, fired one per press,
each activation in the order listed (a sequence that decouples before it ignites is not the
same as one that ignites first); one done by hand leaves its sequence. Each is by one part: `engines` (light an engine; it
starts with the throttle once no stage hangs under its own), `decoupler` (release the
joint a part hangs from, dropping it and everything hanging from it), `fairing` (jettison
a fairing panel, which takes its mass with it) and `rcs` (switch RCS on). The sequence
stack shows identical parts in a sequence (activations of one kind by parts of the same
name, or engines of the same record) as one icon with a count, wherever on the vessel they
are. The player rearranges them in flight: drag an icon to another sequence to move all
its parts, drag its count to move one of them (to light a cluster's centre engine first,
say), drag a sequence's number to reorder. A fired sequence leaves the stack; the engines
it lit stay in a row under it, with the fuel feeding them, until their stage falls away.

A craft can set its own; each activation is `"<kind>:<part id>"`. Naming the part that
does it activates that part alone; naming another part of a stage (its body, say)
activates the stage's: all its engines, the joint it hangs from, all its fairing's panels,
its RCS. A part named twice in a sequence is activated once.

```json5
sequences: [
  ["engines:core_engine_centre"],                 // the centre engine first...
  ["engines:core"],                               // ...then the rest of the core's
  ["decoupler:core", "rcs:upper_stage"],          // separate, settle with RCS...
  ["engines:upper_stage"],                        // ...then light the upper stage
  ["fairing:payload"],
  ["decoupler:upper_stage"],
],
```

#### Ullage

An engine with `requires_ullage` lights only while the vessel is being pushed forward.
Right after a stage falls away nothing is, so something has to push first: RCS, or solid
ullage motors on the stage (or on an interstage that stays with it) lit a sequence earlier.
The Saturn V does the second. The S-IC's separation lights the four motors in the S-II's
interstage, and the next press, a moment later, starts the J-2s while they still burn; the
S-IVB has two motors of its own. A motor is a solid engine with a `Solid Grain` tank of its
propellant on the same stage (`mods/realism/craft/saturn_v.json5`). A failed start still
uses up an ignition.

Without `sequences`: light the first stage with the stages beside it, then release those;
at each joint up the stack release the decoupler, light the stage above (and those beside
it) and switch on its RCS together (so an engine that needs ullage lights while the
propellant is still settled), then release what is beside it; jettison a fairing right
after the stage below it is lit; release a stage without engines (a payload) on its own.

An engine waits to light while a stage hangs under its own, unless the exhaust has a way
out: the stage hangs by an open part (a truss, a ring of panels with gaps between them,
found from its shape), which is hot staging, as the Soyuz's Blok I and the Titan II's
second stage do; or what hangs under it is a hollow ring with nothing across the axis (an
interstage left on after the stage under it fell away, as the Saturn V's S-IC/S-II
interstage is, let go at both ends by a decoupler on each side). An interstage that stays
with the stage under it is a part of that stage; one let go at both ends is a stage of its
own, a container with `bottom` and `top` nodes. Staging an engine that has to wait logs
which stage it waits for, and its part window reads "Blocked by the stage below". A
sounding rocket whose sustainer lights with its booster (the Aerobee, the WAC Corporal)
stands on an open joint: vented panels or a few rods, carrying the booster's decoupler. A
stage hollow down the middle (a booster skirt round the sustainer, as on the Atlas) lets
the engine above it fire through it too, and with a `crossfeed` decoupler its own engines
burn the propellant above: a stage and a half.

The fairing tool fits a fairing's panels (shells over part of a turn whose `bottom` lets
go, on one part) to what stands above their base: `convert_craft --fit-fairings <craft>`
rewrites them as a barrel as wide as the base under a tangent ogive nose 1.3 diameters
long, 0.1 m clear of everything.

What falls away flies on as **debris**, a vessel of its own named after the stage and the
vessel, e.g. `Booster (Pathfinder)`: a decoupler pushes the two sides apart with its
`impulse`, or at 1 m/s without one,
along the axis or straight out from the side a booster hung on (on the ground the spent
stage stays standing), and a fairing comes off in two halves that
swing open about their base. Debris falls through the air, coasts on rails and shows in the
Universe Manifest (which can clear it all) and the map, and it can be targeted and flown.
Unless it is being flown, debris that crashes is removed, and it never holds up time warp:
whatever of it needs physics when rails warp starts (in the air, low over the ground) is
left behind, as in KSP.

### Stage appearance

In a craft written as stages, the converter gives each stage's body a skin drawn from the
stage itself: skirts around its tanks; above them a dark interstage when the next stage's
engines hang into it (tapering if that stage is narrower), otherwise more skin; and its
shells on top. A top stage under a fairing without tanks, engines or a pod is a satellite
on an adapter. Engine stages get a dark engine section and thrust plate. `appearance` adds
the rest as paint and parts, and changes nothing in the physics:

```json5
appearance: {
  paint: [0.85, 0.85, 0.83],        // hull colour, linear RGB (default white)
  livery: "saturn_s1c",             // or a `liveries` record (below)
  interstage: [0.03, 0.03, 0.035],  // interstage colour (default black composite)
  legs: 4,                          // landing legs folded against the base
  grid_fins: 4,                     // grid fins stowed at the top
  raceway: true,                    // a cable raceway up the side of the tanks
},
```

A livery (collection `liveries`) is a paint scheme for a stage, `{ id, name, paint }`, its
layers' heights from the stage's base. The converter copies it onto the stage's body,
tanks, shells and fairing, each with its layers moved to its own height; the craft it
writes keeps the paint, not the livery. The Realism mod's historical schemes are in
`mods/realism/liveries/historical.json5`.

### Models

Engines, RCS blocks and crew pods can have a glTF model (`model`, a path relative to the mod
root, e.g. `"models/engines/my_engine.glb"`); the highest-priority mod's copy of the file
wins. Real scale in metres, +Y up (Blender's default glTF export from Z-up):

| Part | Origin | Orientation |
| --- | --- | --- |
| Engine | Gimbal pivot, top centre | Nozzle down −Y; exit at `−length` |
| RCS block | Centre of the face against the hull | +X out from the hull, +Y towards the nose |
| Pod | Centre of the base | Nose up +Y; `height` tall |

Without a model the game draws a stand-in of the right size. The stock models are built by
scripts in `tools/blender/`.

### Launch sites

A launch site's pad and roads are part of its body's terrain: the ground is reshaped for
collision and rendering alike, and the pad gets a small set of structures (launch mount,
service tower, floodlights, tanks). Layout coordinates are metres in the site frame,
`[right, forward]`, with forward along `heading` (degrees clockwise from north).

```json5
{
  id: "my_site",
  name: "My Site",
  body: "Earth",
  latitude: 34.632, longitude: -120.611,   // vessels stand here, on the deck
  heading: 0,
  pad: {
    size: [90, 70],         // deck length along the heading, width across, m
    corner_radius: 12,
    elevation: 110,         // deck height above sea level, m (default: 1 m above the ground)
    slope: 0.4,             // embankment steepness, rise over run
    mount_height: 3,        // vehicles stand this high on the launch mount, m (default 0)
  },
  roads: [ { points: [[-40, 0], [-400, -150]], width: 9 } ],
  // A level concrete runway centred on the site along its heading, with painted
  // markings: length, width (m) and height above sea level (default 1 m above the ground).
  runway: { size: [4570, 91], elevation: 4 },
}
```

Start a scenario there with `start: { type: "LaunchSite", site: "my_site" }`, or at the
near end of its runway, lined up and on its wheels, with `{ type: "Runway", site:
"my_site" }`.

### Terrain data

A body with `terrain: "terrain/<name>"` uses the baked dataset in that directory (from the
highest-priority mod that has it): elevation, colour, land cover and night lights on a
cube-sphere tile pyramid. Without one the body is a smooth sphere in its `color`. The Earth
dataset is built by `cargo run -p osp_bake --release`; see
`assets/core/terrain/README.md` for the format and the data sources. Set
`max_terrain_height` to the highest elevation so the simulation integrates vessels before
they can reach the ground.

## Patches

Patch files live in `patches/` and contain a list of operations. Each operation targets
records in one collection:

```json5
[
  // Deep-merge fields (objects merge key by key; null deletes a key).
  { op: "merge", collection: "engines", target: "K-1 Booster", value: { ignitions: 5 } },

  // Set a value at a path; missing objects are created. "-" appends to an array.
  { op: "set", collection: "bodies", target: "Earth", path: "atmosphere.height", value: 150000 },

  // Remove a whole record (no path) or something inside it.
  { op: "remove", collection: "craft", target: "pathfinder", path: "parts.id=upper_stage_rcs_4" },

  // Insert into an array (append when index is omitted; negative counts from the end).
  { op: "insert", collection: "layouts", target: "toolbar", path: "#toolbar.children", index: 0,
    value: { widget: "button", text: "Hi", action: "ui.toggle:my_window" } },

  // Duplicate a record under a new id, merging changes into the copy.
  { op: "copy", collection: "engines", target: "H-10", new_id: "H-10B", value: { thrust_vac: 130000 } },
]
```

| Field | Meaning |
| --- | --- |
| `target` | Record id; `*` wildcards allowed (`"K-1*"`, `"*"`). |
| `where` | Only records containing these fields/values, e.g. `{ requires_ullage: true }`. |
| `path` | Dot path inside the record (see below). Empty means the whole record. |
| `pass` | `"first"`, `"default"` or `"final"`. All mods' first-pass patches run before default, and default before final. |
| `optional` | `true` suppresses the "matched nothing" warning (for patches targeting optional mods). |

**Path segments:** `key` · array index `2` / `-1` · `field=value` (the array element whose
`field` equals `value`) · `#id` (the first object *below* the current one whose `id`
matches; this is how to address UI nodes) · `-` (append, for `set`). Wrap a segment in
brackets when it contains dots: `bindings[action=vessel.stage].key`.

**Patch order:** pass, then mod load order, then file name, then position in the file.
Every change is recorded, and errors about a record name the mods that patched it.

## User interface

The UI is built from `layouts` records using `bevy_ui`. A layout is a tree of nodes:

```json5
{
  id: "my_window",          // toggle with the action ui.toggle:my_window
  visible: true,            // shown at startup (a `flight` window the player opened or closed
                            // keeps that choice between runs, in `windows.json` beside the settings)
  group: "flight",          // hidden with the flight UI (F3); optional
  dock: "right",            // stack into the right-hand dock; optional (see Docks)
  dock_order: 40,           // place in the dock, lower first
  root: {
    widget: "window", title: "Orbit", style: { left: 20, top: 300, width: 260 },
    children: [
      { widget: "label", text: "Ap {orbit.apoapsis|distance}" },
      { widget: "bar", value: "vessel.throttle" },
      { widget: "button", text: "Stage", action: "vessel.stage" },
      { widget: "column", repeat: "vessel.stages", children: [
        { widget: "label", text: "{item.name}: {item.delta_v|speed}" },
      ]},
    ],
  },
}
```

### Nodes

| Field | Meaning |
| --- | --- |
| `widget` | `column` (default), `row`, `panel`, `spacer`, `label`, `button`, `bar`, `gauge`, `window`, `dock`, `image`, `navball`. Unknown names act as containers. |
| `image` | `image` only: a PNG or SVG relative to the mod root (`"icons/engine.svg"`), from the highest-priority mod that has it, tinted with the style's `color` (`active_color` while its `active` condition holds); size it with `width` and `height`. SVGs are rasterised at load, well above their size and with mipmaps, so they stay sharp at any size; draw what the theme should tint in white. Text in an SVG is not drawn: convert it to paths. |
| `icon` | `window` only: an image shown before the title (class `window_icon`). |
| `pin` | `window` only: a pin button in the title bar (classes `window_pin`, `window_pin_icon`). Popup windows have one unless `pin: false`. |
| `drag`, `drop` | Drag and drop: a node with `drag` can be picked up and carries that text (a template); dropping it on a node with `drop` fires the `drop` action with `$drag` replaced by what it carried, e.g. `drop: "staging.drop:$drag:{item.index}"`. |
| `tooltip` | Text (a template) shown beside the node while the pointer is over it, styled by the `tooltip` class; icon buttons use it for their name. |
| `id` | Optional; lets patches address the node with `#id`. |
| `class` | Space-separated theme classes. |
| `style` | Inline style (overrides classes). |
| `text` | Text template (`label`, `button`). |
| `title` | Window title template. |
| `action` | Action fired on click (`button`); may contain `{bindings}`. |
| `value` | Data key for the fill (`bar`), 0..1 unless `min` / `max` say otherwise. |
| `min`, `max`, `vertical`, `color` | `bar` only: the value range (with `min` below zero the bar fills either way from zero, for −1..1 inputs), filling upwards instead of across, and a data key holding the fill colour (`"#rrggbb"`; empty keeps the theme's). |
| `min`, `max`, `start`, `end` | `gauge` only: an arc of a ring from `start` to `end` degrees (0 at the top, clockwise; an `end` below `start` runs anticlockwise), filled from `start` by `value` between `min` and `max`. The node is the ring's square: its `border` is the thickness, `border_color` the empty track and `color` the fill. |
| `visible` | Condition (below); the node shows while it holds. |
| `active` | Condition for the node's active look (`active_background`, `active_color`, `active_border_color`): a selected mode, a toggle that is on, an open window. |
| `repeat` | List key; the children are created once per item, and `item.` refers to the current item. A `repeat` inside one takes a list from its item (`repeat: "item.items"`), and its children's `item.` is an item of that list. |
| `children` | Child nodes. |

Windows have a title bar that can be dragged to move the window, and a close button. A
floating window comes to the front when it opens or is clicked. `navball` shows the
attitude indicator; give it a `width` and `height`.

#### Docks

A `dock` node is a column that layouts stack into: a layout with `dock: "<node id>"` is
placed in the `dock` node with that id, in `dock_order`, instead of by its own position,
so docked windows line up and never overlap. In a dock a window's `width` becomes a
minimum, so the column takes the widest one. Empty dock space lets clicks through to the
scene. Dragging a docked window by its title pops it out to float; closing it puts it back.
A dock styled `scroll: true` (the `dock_scroll` class) scrolls with the mouse wheel when
its windows do not fit, with a thin indicator at its edge.

The base game's docks are in `layouts/docks.json5`: `left_top` (clocks), `left` (Flight,
Vessel, Target), `left_bottom` (the stage stack) and `right` (Orbit, Flight plan, Attitude),
beside the launcher. Windows that are opened now and then and are large (Structure, Log,
Manifest, Transfer planner, Mods, Help) float in the middle. To put a mod's window in a
dock, give its layout a `dock`; to move a base window, patch its `dock` or `dock_order`.

A layout's `group` lets the player hide and show several layouts at once without losing
which of them were open: `ui.toggle_group:flight` (F3) does that for the flight UI. Every
layout publishes `ui.shown.<id>`, so a button can light up while its window is open:
`{ widget: "button", text: "Orbit", action: "ui.toggle:orbit_info", active: "ui.shown.orbit_info" }`.

#### Popups and per-key windows

A layout with `popup: true` opens beside the pointer and closes when the player clicks
anywhere outside it (a press and release in place, so dragging the camera does not count),
unless it is pinned. Its window gets a pin button (`ui.pin:<id>`), and `ui.pinned.<id>`
says whether it is pinned.

A layout with `instanced: true` is a template: instead of opening once, it opens a copy
per key, `ui.show:<id>:<key>` (and `ui.hide`, `ui.toggle`). In the copy, `self.` in any
binding reads that key's data, `<id>.<key>.`, lists included
(`repeat: "self.resources"`). The data goes away when the copy closes. The part windows
(`part_info`) are both: the game opens a copy per right-clicked part and publishes its
data.

### Templates and formatters

`{key}` inserts a value; `{key|formatter}` or `{key|formatter:arg}` formats it; `{{` and `}}`
produce literal braces.

| Formatter | Example output |
| --- | --- |
| `fixed:N` | `12.35` |
| `int` | `12` |
| `percent[:N]` | `45%` (from 0.45) |
| `distance` | `950 m`, `12.35 km`, `384400 km`, `1.000 AU` |
| `speed` | `216.9 m/s`, `11.20 km/s` |
| `mass` | `431.14 t` |
| `force` | `7.7 MN` |
| `torque` | `17.6 MN m` |
| `pressure` | `20.18 kPa` |
| `duration` | `1d 01:01:01` |
| `countdown` | `T-00:05:00` before, `T+00:00:12` after (from a time-to in seconds) |
| `degrees[:N]` | `28.61 deg` |
| `date`, `clock` | `2000-01-01 18:00:39`, `18:00:39` (from `time.ut`) |
| `heading` | `090` |
| `sci[:N]`, `upper`, `lower`, `yesno` | |

**Conditions** (for `visible` and `active`): a key holds when its value is truthy, and `!key`
inverts it. `false`, `0`, an empty string, an empty list and missing keys are false.
`key == value` and `key != value` compare the value as text: `vessel.sas_mode == prograde`,
`time.warp == 1`.

### Styles and themes

Styles are resolved in layers, later wins: the class named after the widget, then each
class in `class`, then the inline `style`. Theme records (`themes` collection) define
classes; all of them merge in load order, so a mod can restyle anything with a patch like
`{ op: "set", collection: "themes", target: "default", path: "classes.panel.background", value: "#102030" }`.

| Property | Values |
| --- | --- |
| `direction` | `row`, `column`, `row_reverse`, `column_reverse` |
| `position` | `relative`, `absolute` |
| `left` `right` `top` `bottom` `width` `height` `min_*` `max_*` | number (px) or `"50%"`, `"12px"`, `"20vw"`, `"10vh"`, `"auto"` |
| `padding` `margin` `border` | `8`, `[vertical, horizontal]`, `[top, right, bottom, left]` |
| `gap` | px between children |
| `align_items` `align_self` | `start`, `end`, `center`, `stretch`, `baseline` |
| `justify_content` | `start`, `end`, `center`, `space_between`, `space_around`, `space_evenly` |
| `grow` `shrink` `wrap` `clip` | flex grow/shrink, wrapping, clip overflow |
| `scroll` | clip overflow and scroll it vertically with the mouse wheel |
| `radius` | corner radius, px |
| `background` `hover_background` `border_color` `color` | `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, `[r, g, b]`, `[r, g, b, a]` |
| `active_background` `active_border_color` `active_color` | colours while the node's `active` condition holds |
| `font_size` | px |
| `font` | font family, or a CSS-style list (`"Barlow, sans-serif"`); the files come from the theme's `fonts` |
| `font_weight` | 100 to 900 (400 regular, 600 semibold); picks the face of that weight |
| `letter_spacing` | extra px between letters |
| `uppercase` | `true` shows the text in capitals |
| `gradient` `border_gradient` | colours from top to bottom, spread evenly, for the background (over `background`) or the border (instead of `border_color`) |
| `shadow` | drop shadow `{ color, x, y, blur, spread }` (px) |
| `z_index` | stacking order of layout roots |

A theme record can list font files to load, relative to its mod's root:
`fonts: ["fonts/Barlow-Medium.ttf", "fonts/Barlow-SemiBold.ttf"]`. Every text node gets the
`text` class under its own style, so `classes.text.font` sets the font everywhere. Numbers
use tabular figures, so readouts do not jitter. UI colours blend in linear light, where a
faint white overlay looks much brighter than its alpha suggests; the default theme uses
solid surface colours for controls on panels (see the comment at the top of
`themes/default.json5`).

The flight UI is a set of modules, each its own layout in the `flight` group:

| Layout | Contents | Node ids |
| --- | --- | --- |
| `docks` | The left and right docks (`dock_left`, `dock_right`) | `#left_top`, `#left`, `#left_bottom`, `#right` |
| `toolbar` | App launcher: icon buttons down the right edge, named in tooltips | `#toolbar` |
| `time` | Warp buttons, a chevron per warp level (`time.warp_levels`: `lit`, `rails`), UT and mission time (top of the left dock) | `#time` |
| `altimeter` | Altitude, vessel and situation (top centre) | `#altimeter` |
| `messages` | Latest event, warp notices and lag warning | `#messages` |
| `navball` | Navball in a housing picture (`icons/navball_housing.svg`) with arc gauges for thrust, throttle and air pressure, the speed tab (click to change frame), RCS and SAS switches, a heading plate, and beside it the SAS mode buttons (the navball's markers as icons) and control inputs (bottom centre) | `#navball`, `#speed`, `#plate`, `#sas`, `#sas_modes`, `#inputs` |
| `staging` | Sequence stack (bottom of the left dock, F8), with the lit engines under it | `#staging`, `#sequences`, `#armed` |
| `resources` | Every propellant on board, a row per stage coloured by stage, and the RCS delta-v | `#rcs_supply`, `#list` |
| `flight_info` | Speeds, Mach, pressures | `#flight` |
| `orbit_info` | Apsides, inclination, period, warp-to buttons, predicted events | `#orbit`, `#warp_to`, `#events` |
| `flight_plan` | Planned burns, the selected burn's editor, the burn executor | `#burns`, `#editor`, `#executor` |
| `target_info` | Target distance, relative speed, closest approaches, plane crossings | `#target` |
| `manifest` | Universe Manifest (F11): every body and vessel, filtered by kind, with Follow, Target, Fly, Recover or Destroy, and Clear debris | `#items` |
| `transfer_planner` | Transfer planner (F4): porkchop plot (`widget: "porkchop"`) and the picked transfer | `#plot` |
| `vessel_info` | Thrust, TWR, mass, delta-v | `#propulsion` |
| `attitude_info` | Centre of mass, inertia, control authority, rates | `#attitude` |
| `structure_info` | Every tank and fin set: make-up, strength, current load | `#parts` |
| `part_info` | Part window (right-click a part): an instanced popup, a section per part type | `#summary`, `#engine`, `#tank`, `#resources`, `#rcs`, `#wheel`, `#pod`, `#fins`, `#wing`, `#gear`, `#shape`, `#structure` |
| `log` | Flight log | `#log` |

Outside the flight group: `main_menu` (scenarios, and the Graphics page with node id
`#graphics`) and `perf`, the performance overlay (F10, node ids `#perf`, `#systems`,
`#passes`).

To add a window to the launcher, insert a `launcher_button` into `#toolbar.children` with an
`image` child (class `launcher_icon`, a white icon on transparent from your mod, SVG or PNG;
give it the button's `active` too so it lights up) and a `tooltip` (see
`mods/example_mod/patches/ui.json5`). Window rows are `kv` rows: an optional `row_icon`
image (the base game's are in `icons/telemetry/`), a `key` label and a value.

### Keybindings

```json5
{ id: "my_keys", bindings: [ { key: "F6", action: "ui.toggle:my_window" },
                             { key: "KeyU", action: "vessel.throttle_up", hold: true } ] }
```

`key` is a Bevy `KeyCode` name (`KeyA`, `Digit1`, `F1`, `Space`, `ShiftLeft`, `Comma`, ...).
`hold: true` keeps the action active while the key is down; otherwise it fires once per press.
To rebind a base-game key, patch it:
`{ op: "set", collection: "keybindings", target: "default", path: "bindings[action=vessel.stage].key", value: "Enter" }`.

## Reference: data keys

Press **F12** in game for a live list. Lists are marked `[]`; their item fields follow.

| Key | Meaning |
| --- | --- |
| `time.ut` | Seconds since J2000 (format with `date` / `clock`) |
| `time.warp`, `time.warp_mode` | Warp rate; `"physics"` or `"rails"` |
| `time.lagging`, `time.notice` | Simulation behind real time; last warp notice |
| `view.map` | Map view active |
| `sim.last_event`, `sim.last_event_time` | Latest simulation event message and time |
| `vessel.name`, `vessel.body`, `vessel.situation`, `vessel.motion` | Identity and state (`landed`, `coasting`, `integrated`) |
| `vessel.in_space` | Out of the atmosphere (sub-orbital, orbiting or escaping); the Orbit window opens and closes with it |
| `vessel.altitude`, `vessel.vertical_speed`, `vessel.surface_speed`, `vessel.orbital_speed` | m, m/s |
| `vessel.latitude`, `vessel.longitude` | degrees |
| `vessel.mach`, `vessel.dynamic_pressure`, `vessel.static_pressure` | Pa |
| `vessel.angle_of_attack`, `vessel.sideslip` | Degrees: nose above the airflow, airflow from the right |
| `vessel.lift`, `vessel.drag` | N: the air's force across and against the airflow |
| `vessel.throttle`, `vessel.thrust`, `vessel.twr`, `vessel.mass`, `vessel.accel_g` | 0..1, N, -, kg, g |
| `vessel.engine_level`, `vessel.min_throttle` | Thrust the bottom stage's engines give as a share of full (0..1, follows the throttle through spool-up, throttling and tail-off); the highest minimum throttle among them |
| `vessel.delta_v`, `vessel.stage_count`, `vessel.landed`, `vessel.engines_running` | |
| `vessel.sas`, `vessel.sas_mode`, `vessel.sas_mode_label` | SAS on; mode (`off`, `stability`, `prograde`, `retrograde`, `normal`, `anti_normal`, `radial_out`, `radial_in`, `burn`, `target`, `anti_target`) |
| `vessel.rcs`, `vessel.has_rcs`, `vessel.rcs_fuel` | RCS on; any RCS with a propellant supply in the stack; scarcest RCS propellant left (0..1) |
| `vessel.rcs_mass`, `vessel.rcs_delta_v` | RCS propellant left (kg) and the delta-v it can still give the whole vessel in vacuum (m/s) |
| `vessel.structure_load`, `vessel.destroyed` | The most loaded part's load over its strength (1 = failing); broke up |
| `nav.frame`, `nav.speed` | `"Surface"`, `"Orbit"` or `"Target"`; speed in that frame, m/s |
| `nav.heading`, `nav.pitch`, `nav.roll` | Attitude against the local horizon, degrees (roll is empty pointing straight up or down) |
| `control.pitch`, `control.yaw`, `control.roll` | Rotation command applied (pilot or SAS), −1..1 |
| `attitude.com`, `attitude.moi_pitch`, `attitude.moi_yaw`, `attitude.moi_roll` | Centre of mass above the base (m); moments of inertia (kg·m²) |
| `attitude.torque_pitch`, `attitude.torque_roll`, `attitude.accel_pitch`, `attitude.accel_roll` | Torque available (N·m) and the angular acceleration it gives (deg/s²) |
| `attitude.rate_pitch`, `attitude.rate_yaw`, `attitude.rate_roll` | Angular velocity, deg/s |
| `attitude.gimbal`, `attitude.gimbal_range`, `attitude.wheels`, `attitude.rcs` | Current and maximum gimbal angle (deg); reaction wheel and RCS pitch torque (N·m) |
| `time.met`, `time.launched` | Seconds since liftoff |
| `screen.message` | The latest log message, for a few seconds |
| `ui.shown.<layout>` | Whether a layout is showing |
| `ui.pinned.<layout>` | Whether a popup is pinned (copies of instanced layouts by their `<layout>:<key>` id) |
| `part_info.<vessel>-<part>.*` | An open part window's part: `name`, `type` (`engine`, `tank`, `rcs`, `wheel`, `pod`, `fins`, `wing`, `gear`, `shell`, `fairing`, `hull`, `decoupler`), `kind` (its label), `stage`, `vessel`, `mass` (kg, dry), `manufacturer`, `description`. `has_actions`, `actions[]` (what can be done to the part by hand now: `label`, `do`, the `part.do` argument). Engines: `status`, `lit`, `thrust` (N), `level` (0..1), `thrust_vac`, `thrust_sl`, `isp` (s, here), `isp_vac`, `isp_sl`, `min_throttle`, `gimbal`, `gimbal_range` (deg), `ignitions`, `limited`, `propellants`. Tanks: `resources[]` (`name`, `amount`, `capacity` in kg, `fraction`), `contents_mass`, `size`, `material`, `volume` (m³), `wall` (mm), `max_pressure` (bar), `max_axial_load` (N). RCS: `on`, `firing` (0..1), `nozzle_thrust`, `isp_vac`, `isp_sl`, `propellants`. Wheels `torque`; pods `crew`, `diameter`, `height`; fins (one per part) `count`, `area`, `span`, `thickness`, `material`, `max_fin_force`; wings (per side) `span`, `area`, `sweep`, `mirrored`, `control`, `deflection`; gear `compression` (0..1), `length`, `stroke`, `rated_mass`, `brakes`, `steering`, `mirrored`; shells `profile`, `length`, `bottom_diameter`, `top_diameter`; hulls and fairing halves `diameter`, `length`. Tanks and fins also `load` (0..1+) and `mode` |
| `vessel.stages[]` | `name`, `active`, `delta_v`, `burn_time`, `fuel` (0..1), `has_fuel`, `ignitions`, `engines` |
| `vessel.sequences[]` | The activation sequences, next first: `index`, `number`, `next`, `empty`, `caption` (what firing it does), `lines[]` (`tiles[]`: `items[]` in lines of four), `items[]` (`seq`, `index`, `kind`, `is_engines` (a liquid engine), `is_vacuum`, `is_solid`, `is_decoupler`, `is_fairing`, `is_rcs`, `stage`, `count`, `multiple`, `fuel`, `has_fuel`, `color`: the stage's colour, `caption`, `tip`: the caption with thrust, Isp, burn time and fuel) |
| `vessel.next_caption`, `vessel.armed_lines[]` | What the stage key does next; the lit engines in lines of four (`tiles[]`) |
| `vessel.sequence_count`, `vessel.stage_twr` | How many sequences are left; thrust-to-weight of the bottom stage at full throttle here |
| `vessel.armed[]`, `vessel.armed_count`, `vessel.armed_number` | Stages whose engines are lit (their sequence fired), bottom first: `stage`, `count`, `multiple`, `fuel` (0..1), `has_fuel`, `color`; how many; the number of the last sequence fired |
| `vessel.resources[]`, `vessel.resource_count` | Every propellant on board, a row per stage that holds it, top of the stack first: `name`, `stage`, `color` (the stage's, `"#rrggbb"`; a stage keeps its colour as the ones below fall away), `bottom` (on the bottom stage), `amount` and `capacity` (kg), `fraction`, `engines` and `rcs` (whether the stage's engines or RCS burn it), `tag` (`"RCS"` if only the RCS does); how many rows |
| `vessel.parts[]` | Tanks and fin sets: `stage`, `name`, `tank` (bool), `size`, `material`, `mass` (kg, empty), `load` (0..1+), `mode` (`buckling`, `pressure`, `bending`); tanks also `volume` (m³), `utilization`, `contents`, `wall` (mm), `max_pressure` (bar), `max_axial_load` (N, cylinders); fins `max_fin_force` (N per fin) |
| `vessel.log[]` | `time`, `message` (newest first) |
| `orbit.apoapsis`, `orbit.periapsis` | Altitudes, m (apoapsis empty when escaping) |
| `orbit.escape`, `orbit.inclination`, `orbit.eccentricity`, `orbit.semi_major_axis`, `orbit.period` | |
| `orbit.events[]` | `label`, `time_to` (s): predicted SOI changes and atmosphere entry |
| `plan.count`, `plan.total_dv`, `plan.dv_step`, `plan.time_step` | Planned burns; their summed delta-v (m/s); editor step sizes (m/s, s) |
| `plan.burns[]` | `index`, `label` (`B1`...), `time_to` (s), `delta_v` (m/s), `duration` (s, with the bottom stage), `start_in` (s), `selected` |
| `plan.has_selection`, `plan.selected` | A burn is selected; its index |
| `burn.label`, `burn.prograde`, `burn.normal`, `burn.radial`, `burn.delta_v` | The selected burn's delta-v by axis, m/s |
| `burn.time_to`, `burn.duration`, `burn.start_in`, `burn.apoapsis`, `burn.periapsis` | When, how long, and the apsis altitudes (m) after it |
| `autoburn.engaged`, `autoburn.state`, `autoburn.remaining` | Burn executor on; `Off`, `Waiting` or `Burning`; delta-v still to give (m/s) |
| `map.hovering`, `map.hover_time_to`, `map.hover_altitude` | Map view: the trajectory point under the mouse (s, m) |
| `target.has`, `target.name` | A target is set; its name |
| `target.distance`, `target.speed`, `target.closing_speed` | Now: m, relative m/s, m/s closing (negative when opening) |
| `target.has_approach`, `target.approach_distance`, `target.approach_time_to`, `target.approach_speed` | First closest approach on the planned trajectory (m, s, m/s); `target.approach2_*` the next |
| `target.has_plane`, `target.inclination`, `target.an_time_to`, `target.dn_time_to` | Plane crossings when the target orbits the same body (deg, s) |
| `transfer.has`, `transfer.computing`, `transfer.stale`, `transfer.kind` | A plot exists; being computed; made for another target; what kind |
| `transfer.window_from`, `transfer.window_to`, `transfer.tof_min`, `transfer.tof_max`, `transfer.best_total` | The plot's departure window (s from now), flight times (s), cheapest total (m/s) |
| `transfer.picked`, `transfer.departure_in`, `transfer.tof`, `transfer.depart_dv`, `transfer.arrive_dv`, `transfer.total` | The picked transfer |
| `burn.arrival_periapsis`, `burn.targets_body` | Periapsis altitude at the targeted body after the selected burn (m) |
| `manifest.items[]` | The rows the filters let through: `name`, `kind` (`body`, `vessel`, `debris`), `is_body`, `is_debris`, `key` (action argument), `parent`, `apoapsis`, `periapsis`, `distance` and `speed` (from what the camera follows), `is_target`, `is_active`, `is_followed`, `target_body`, `target_vessel`, `can_fly`, `can_recover` (landed on a world with a launch site), `can_destroy`, `confirming` (asking whether to remove it), `confirm_text` |
| `manifest.show_bodies`, `manifest.show_vessels`, `manifest.show_debris` | The manifest's filters |
| `manifest.debris_count`, `manifest.confirm_clear`, `manifest.following` | Debris that Clear debris would remove; it is asking whether to; the name of what the camera follows instead of the active vessel (empty if nothing) |
| `mods.count`, `mods.errors`, `mods.warnings` | |
| `mods.list[]` | `id`, `name`, `version`, `authors`, `description` |
| `mods.problems[]` | `error` (bool), `text` |
| `menu.scenarios[]`, `menu.playing`, `menu.page` | Main menu: `id`, `name`, `description`; a scenario is being flown; the page showing (`scenarios`, `graphics`) |
| `graphics.preset`, `graphics.recommended`, `graphics.summary` | The preset the settings match (`low`, `medium`, `high`, `ultra` or `custom`); the one for this GPU; every setting on one line |
| `graphics.render_scale`, `graphics.antialiasing`, `graphics.shadows`, `graphics.terrain`, `graphics.trees`, `graphics.effects`, `graphics.bloom`, `graphics.vsync`, `graphics.fps_limit` | Each setting as text: percent; `off`, `fxaa`, `msaa`; a level (`off`, `low`, `medium`, `high`, `ultra`); `on` / `off`; frames per second (`0` for none) |
| `perf.fps`, `perf.frame_ms`, `perf.worst_ms` | Over the last half second: frame rate, mean and worst frame time (ms) |
| `perf.main_ms`, `perf.render_ms`, `perf.gpu_wait_ms`, `perf.bound` | Main and render thread time (ms), the render thread's wait for the GPU, and what limits the frame rate |
| `perf.gpu`, `perf.entities`, `perf.meshes`, `perf.visible_meshes`, `perf.triangles_m` | Adapter name, backend and type; entity and mesh counts; triangles in the visible meshes (millions) |
| `perf.systems_traced`, `perf.systems[]` | Built with `--features profile`: the slowest systems, `name`, `ms`, `max_ms` |
| `perf.gpu_timed`, `perf.passes[]` | Render passes timed on the GPU (`profile` build, Vulkan and DX12): `name`, `cpu_ms`, `gpu_ms` |

## Reference: actions

| Action | |
| --- | --- |
| `vessel.stage` | Fire the next activation sequence |
| `part.do:<vessel>:<kind>:<part>` | Do one activation by hand, outside the sequences: `<kind>` is `engines` (ignite), `decoupler` (release the joint the part hangs from), `fairing` (jettison a panel) or `rcs`; the part window's buttons fire it, and it leaves the sequence that listed it |
| `vessel.throttle_full`, `vessel.throttle_cut` | press |
| `staging.drop:<dragged>:<sequence>` | `<dragged>` is `item:<sequence>:<index>` (an activation, moves into that sequence) or `seq:<sequence>` (a sequence, moves to its place) |
| `staging.drop_at:<dragged>:<place>` | Put an activation in a new sequence before the one at `<place>` (the strip between two rows), or move a sequence there |
| `staging.drop_new:<dragged>`, `staging.add`, `staging.remove:<sequence>` | Give an activation a new last sequence (or move a sequence to the end); add an empty sequence at the end; remove an empty one |
| `vessel.throttle_up`, `vessel.throttle_down` | hold |
| `vessel.pitch_up`, `vessel.pitch_down`, `vessel.yaw_left`, `vessel.yaw_right`, `vessel.roll_left`, `vessel.roll_right` | hold |
| `vessel.sas_toggle`, `vessel.sas_mode:<mode>` | SAS on/off; select a mode (switches SAS on; the selected mode again goes back to `stability`) |
| `vessel.rcs_toggle` | RCS on/off |
| `vessel.translate_forward`, `vessel.translate_back`, `vessel.translate_left`, `vessel.translate_right`, `vessel.translate_up`, `vessel.translate_down` | hold; RCS translation |
| `nav.toggle_frame` | Cycle prograde and speed through surface, orbit and (with a target) target |
| `vessel.next`, `vessel.previous`, `vessel.fly:<key>` | Fly another vessel (`key` from `manifest.items`) |
| `manifest.filter:<bodies\|vessels\|debris>` | Show or hide that kind of row in the Universe Manifest |
| `manifest.follow:<key>`, `manifest.follow_body:<name>` | Point the camera at a vessel (the active one: back to it) or, in map view, a body; flying another vessel, the map closing or the vessel going ends it |
| `manifest.remove:<key>`, `manifest.clear_debris` | Ask to recover (landed on a world with a launch site) or destroy a vessel other than the active one, or to remove all debris |
| `manifest.confirm`, `manifest.cancel` | Answer that question |
| `target.body:<id>`, `target.vessel:<key>`, `target.clear` | Choose or drop the target (also Shift+click in map view) |
| `plan.match_plane` | Plan a burn at the next plane crossing that matches the target's plane |
| `warp.increase`, `warp.decrease`, `warp.stop` | |
| `warp.to:<ap|pe|soi|burn|approach>` | Warp to the next apoapsis, periapsis, SOI change, burn start or closest approach, slowing on the way in |
| `plan.add:<ap|pe>`, `plan.circularize:<ap|pe>` | Plan a burn at the next apsis (after any planned burns); one that circularises there |
| `plan.select:<index>`, `plan.zero`, `plan.delete`, `plan.clear` | Select a burn; zero, delete it; delete all |
| `plan.dv:<prograde|normal|radial>:<1|-1>`, `plan.dv_step:<m/s>` | Change the selected burn by one step |
| `plan.time:<1|-1>`, `plan.orbit:<1|-1>`, `plan.time_step:<s>` | Move it by one time step or one orbit |
| `plan.aim:<m>` | Adjust the selected burn for a periapsis this high above the targeted body |
| `plan.execute` | Engage or disengage the burn executor (throttle and SAS commands also disengage it) |
| `transfer.compute`, `transfer.best`, `transfer.create` | Plot transfers to the target; pick the cheapest; plan its departure burn |
| `view.toggle_map` | |
| `ui.toggle:<layout>`, `ui.show:<layout>`, `ui.hide:<layout>` | For an instanced layout, `<layout>:<key>` names a copy |
| `ui.pin:<layout>` | Pin or unpin a popup, so a click outside does not close it |
| `ui.toggle_group:<group>`, `ui.show_group:<group>`, `ui.hide_group:<group>` | Every layout in a group |
| `game.reload` | Reload all mods and rebuild the UI |
| `game.menu`, `game.resume`, `game.start:<scenario>`, `game.quit` | Main menu |
| `menu.page:<scenarios|graphics>` | Switch the main menu's page |
| `graphics.preset:<low|medium|high|ultra>`, `graphics.detect` | Apply a preset; apply the one recommended for this GPU (both keep vsync and the frame cap) |
| `graphics.set:<setting>:<value>` | Change one setting (`render_scale:75`, `antialiasing:fxaa`, `shadows:off`, `vsync:on`, ...); saved at once |

## Troubleshooting

- **Nothing changed?** Press F5, then F2. Syntax errors, unknown style properties, invalid
  or unmatched patches and values of the wrong type are listed there (and in the console)
  with the mod and file. A broken record is skipped; the rest of the mod still loads.
- **A patch says "matched nothing"?** Check the collection name, the target id, and whether
  the target mod loads before yours (add it to `dependencies` or `load_after`).
- **Physics data (bodies, parts, craft, launch sites) is read when a game starts.** F5
  reloads the UI immediately; restart to fly with changed parts or planets.
- **Earth is a smooth blue sphere?** Its terrain data has not been baked; F2 shows a warning.
  Run `cargo run -p osp_bake --release`.
