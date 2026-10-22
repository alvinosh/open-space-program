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
| `engines` | Engines | `thrust_vac` (N), `isp_vac`/`isp_sl` (s), `min_throttle`, `response`, `ignitions`, `requires_ullage`, `gimbal_range` (degrees, 0 = fixed), `propellants: { Name: mass ratio }`, `length` and `exit_diameter` (m), `exit_pressure` (Pa), `plume`, `model`, `model_scale` (draw the model this many times its size); see [Engine response and plumes](#engine-response-and-plumes) |
| `plumes` | Exhaust looks | `core`, `flame` and `shock` colours (linear RGB), `luminance` (cd/m² per m at full thrust), `soot` (0..1) |
| `resources` | Propellants and gases | `density` (kg/m³ as stored), storage `temperature` (K) and `pressure` (Pa) |
| `materials` | Structural materials | `density` (kg/m³), `yield_strength` and `elastic_modulus` (Pa) |
| `tank_types` | How procedural tanks are built | `material`, operating `pressure` (Pa), `min_gauge` (m), `stiffening`, `buckling_factor`, `insulation` (kg/m²), `min_temperature` (K), `utilization`, `safety_factor`, `design_load` (g), optional `propellants` allowlist; see [Procedural tanks and fins](#procedural-tanks-and-fins) |
| `reaction_wheels` | Reaction wheels | `mass` (kg), `torque` (N·m about each axis) |
| `rcs` | RCS blocks (four nozzles each) | `mass` (kg), `thrust` (N per nozzle, vacuum), `isp_vac`/`isp_sl` (s), `propellants: { Name: mass ratio }`, `model` |
| `pods` | Crew pods (capsules, cockpits) | `mass` (kg, with crew), `crew`, `diameter` and `height` (m), optional `top_diameter` (m, a third of `diameter` by default), `model` |
| `craft` | Vessels | `stages` from the bottom: `engines`, procedural `tanks` and `fins`, `wings`, `gear`, `reaction_wheels`, `rcs`, `pod`, `fairing: { mass }`, `structure_mass`, `diameter`, `length`, `appearance`; optional `sequences`; see [Staging](#staging) and [Stage appearance](#stage-appearance) |
| `launch_sites` | Launch sites | Position, `heading`, optional `pad`, `roads` and `runway`; see [Launch sites](#launch-sites) |
| `scenarios` | New-game setups | `name` and `description` (shown in the main menu, which lists scenarios in load order); `start: { type: "LaunchSite", site: "<id>" }`, `{ type: "Runway", site: "<id>" }` (at the start of the site's runway, on its wheels), `{ type: "Flight", body, latitude, longitude, altitude, heading, speed, climb }` (in the air, nose along the flight path, wings level; degrees, metres and m/s) or `{ type: "Orbit", body, periapsis_altitude, apoapsis_altitude, inclination, longitude_of_ascending_node, argument_of_periapsis, mean_anomaly }` (degrees, from the body's equator; `plane: "<body>"` instead takes the plane of a body that orbits it, like the Moon); optional `others` and `target`; `OSP_SCENARIO=<id>` starts one without the menu |
| `layouts` | UI windows and panels | See [User interface](#user-interface) |
| `themes` | Style classes | All theme records merge in load order |
| `keybindings` | Key → action | All keybinding records combine |

See the files in `assets/core` for complete, commented examples of each.

### Craft mass and control

A stage's `diameter` and `length` shape its rotation as well as its looks. Each stage is
modelled as a cylinder: tank dry mass as a thin shell along it, `structure_mass` (and
reaction wheels and RCS blocks) as a uniform solid, the crew pod as a solid frustum from
`diameter` to `top_diameter` on top, engines as point masses on its base, fins as rings of
mass at their centroid, and propellant as a solid column settled at the aft end. That gives the centre of mass and the moments of inertia, which change as
tanks drain and stages drop.

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
  // unless `diameter` is given. Length includes the √2:1 domes.
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
it (`propellants`, if listed). Its wall is as thin as the material allows: the design
pressure is the type's operating `pressure` plus the hydrostatic head of the full tank at
`design_load` g, and the wall must keep the material `safety_factor` below yield at that,
but is never thinner than `min_gauge`. Mass is that wall times `stiffening` (isogrid ribs,
stringers, liners) plus `insulation`. That gives two strengths:

- **Burst:** the internal pressure at which the wall yields. In flight the tank sees its
  operating pressure plus the hydrostatic head of what is left in it at the current
  acceleration, so an upper stage that rides a 6 g booster with full tanks needs a higher
  `design_load` than a first stage, which is only full at liftoff.
- **Buckling:** the axial load at which the cylinder collapses: thin-shell buckling with
  the NASA SP-8007 knockdown factor, times `buckling_factor` for grid stiffening, plus the
  load the tank's own pressure carries (which is all that holds up a balloon tank). In
  flight a tank carries everything stacked above it at the current acceleration, plus the
  drag on the nose; on the pad, the stack's weight.

A **fin** is a solid plate of its `material`. The air pushes on it at its own angle of
attack (see [Aerodynamics](#aerodynamics)) and its root bends; it fails when the root
reaches yield.

When any part goes past 100% of its strength it fails: fins tear off; a tank failure breaks
the vessel up (engines, RCS and controls stop, and warp halts). The Structure window shows
every part's make-up, strength and current load.

### Aerodynamics

The air works on a vessel the way Ferram Aerospace Research does in KSP: the outline of the
stack (stages, tapers between them, the nose or pod on top) is cut into thin sections, and
each section is pushed by the air it actually meets, including the vessel's own rotation.
So lift, drag, stability and damping all follow from the shape:

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

Engine bells, landing legs and grid fins are not part of the outline.

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

The `glider` craft is a worked example: it trims to glide hands off at about 38 m/s.
Wings cannot be torn off yet.

A winged vessel has to be stable to fly: its centre of mass ahead of the point where the
air's push on the whole vessel acts. A long nose ahead of the wing pushes that point
forward (it lifts like a rocket's nose), and in yaw it fights the fin; the `comet` craft
moved its wing back and grew its fin until both settled.

### Landing gear

A stage's `gear` legs stand out from its hull like wings do, each a sprung, damped strut
with a wheel at its end:

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
stretch and the plume fans out and fades into vacuum. Without `exit_pressure` it is guessed
from the Isp the engine loses at sea level. `plume` names a `plumes` record for the colours
and brightness; engines without one draw no plume. The jet leaves along the gimballed
nozzle and bends within a few metres onto where the exhaust has been going over the last
half second or so, leaning into the airflow the harder the air pushes against the jet, so
quick gimbal moves do not swing the whole plume.

### Staging

As in KSA, stages and staging are separate. A stage is a section of the vessel and its
propellant pool. What the stage key does is a list of **sequences**, fired one per press,
each holding **activations**: `engines` (light a stage's engines; they start with the
throttle once their stage is at the bottom), `decoupler` (release the joint above a stage,
dropping it and everything below), `fairing` (jettison a stage's fairing, which takes its
mass with it) and `rcs` (switch RCS on). The player rearranges them in flight in the
sequence stack: drag an icon to another sequence, drag a sequence's number to reorder.

A craft can set its own; each activation is `"<kind>:<stage name>"`:

```json5
sequences: [
  ["engines:Booster"],
  ["decoupler:Booster", "rcs:Upper Stage"],   // separate, settle with RCS...
  ["engines:Upper Stage"],                    // ...then light the upper stage
  ["fairing:Payload"],
  ["decoupler:Upper Stage"],
],
```

Without `sequences`: light the first stage; at each joint release the decoupler, light the
stage above and switch on its RCS together (so an engine that needs ullage lights while
the propellant is still settled); jettison a fairing right after the stage below it is lit;
release a stage without engines (a payload) on its own.

### Stage appearance

Hulls are drawn from the stage itself: its tanks are painted barrels with weld seams; the
structure above them is a dark interstage when the next stage's engines hang into it (and
tapers if that stage is narrower), otherwise more hull; a top stage without tanks, engines
or a pod is a payload fairing; and the top of the stack gets an ogive nose unless a pod is
there. Engine stages get a dark base ring and heat shield. `appearance` adds the rest, and
changes nothing in the physics:

```json5
appearance: {
  paint: [0.85, 0.85, 0.83],        // hull colour, linear RGB (default white)
  interstage: [0.03, 0.03, 0.035],  // interstage colour (default black composite)
  legs: 4,                          // landing legs folded against the base
  grid_fins: 4,                     // grid fins stowed at the top
  raceway: true,                    // a cable raceway up the side of the tanks
},
```

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
  { op: "remove", collection: "craft", target: "pathfinder", path: "stages.name=Payload" },

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
  visible: true,            // shown at startup
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
| `widget` | `column` (default), `row`, `panel`, `spacer`, `label`, `button`, `bar`, `window`, `dock`, `image`, `navball`. Unknown names act as containers. |
| `image` | `image` only: a picture relative to the mod root (`"icons/engine.png"`), from the highest-priority mod that has it, tinted with the style's `color`; size it with `width` and `height`. |
| `drag`, `drop` | Drag and drop: a node with `drag` can be picked up and carries that text (a template); dropping it on a node with `drop` fires the `drop` action with `$drag` replaced by what it carried, e.g. `drop: "staging.drop:$drag:{item.index}"`. |
| `id` | Optional; lets patches address the node with `#id`. |
| `class` | Space-separated theme classes. |
| `style` | Inline style (overrides classes). |
| `text` | Text template (`label`, `button`). |
| `title` | Window title template. |
| `action` | Action fired on click (`button`); may contain `{bindings}`. |
| `value` | Data key for the fill (`bar`), 0..1 unless `min` / `max` say otherwise. |
| `min`, `max`, `vertical` | `bar` only: the value range (with `min` below zero the bar fills either way from zero, for −1..1 inputs) and filling upwards instead of across. |
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
| `z_index` | stacking order of layout roots |

The flight UI is a set of modules, each its own layout in the `flight` group:

| Layout | Contents | Node ids |
| --- | --- | --- |
| `docks` | The left and right docks (`dock_left`, `dock_right`) | `#left_top`, `#left`, `#left_bottom`, `#right` |
| `toolbar` | App launcher (down the right edge) | `#toolbar` |
| `time` | Warp buttons, UT and mission time (top of the left dock) | `#time` |
| `altimeter` | Altitude, vessel and situation (top centre) | `#altimeter` |
| `messages` | Latest event, warp notices and lag warning | `#messages` |
| `navball` | Navball, speed, throttle, SAS modes, control inputs (bottom centre) | `#navball`, `#throttle`, `#ball`, `#sas`, `#inputs` |
| `staging` | Sequence stack (bottom of the left dock, F8) | `#staging`, `#sequences` |
| `flight_info` | Speeds, Mach, pressures | `#flight` |
| `orbit_info` | Apsides, inclination, period, warp-to buttons, predicted events | `#orbit`, `#warp_to`, `#events` |
| `flight_plan` | Planned burns, the selected burn's editor, the burn executor | `#burns`, `#editor`, `#executor` |
| `target_info` | Target distance, relative speed, closest approaches, plane crossings | `#target` |
| `manifest` | Universe Manifest (F11): every body and vessel, with Target and Fly | `#items` |
| `transfer_planner` | Transfer planner (F4): porkchop plot (`widget: "porkchop"`) and the picked transfer | `#plot` |
| `vessel_info` | Thrust, TWR, mass, delta-v | `#propulsion` |
| `attitude_info` | Centre of mass, inertia, control authority, rates | `#attitude` |
| `structure_info` | Every tank and fin set: make-up, strength, current load | `#parts` |
| `log` | Flight log | `#log` |

Outside the flight group: `main_menu` (scenarios, and the Graphics page with node id
`#graphics`) and `perf`, the performance overlay (F10, node ids `#perf`, `#systems`,
`#passes`).

To add a window to the launcher, insert a button into `#toolbar.children` (see
`mods/example_mod/patches/ui.json5`).

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
| `vessel.stages[]` | `name`, `active`, `delta_v`, `burn_time`, `fuel` (0..1), `has_fuel`, `ignitions`, `engines` |
| `vessel.sequences[]` | The activation sequences, next first: `index`, `number`, `next`, `empty`, `items[]` (`seq`, `index`, `kind`, `is_engines`, `is_decoupler`, `is_fairing`, `is_rcs`, `stage`, `count`, `multiple`, `fuel`, `has_fuel`) |
| `vessel.sequence_count`, `vessel.stage_twr` | How many sequences are left; thrust-to-weight of the bottom stage at full throttle here |
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
| `manifest.items[]` | `name`, `kind` (`body`, `vessel`), `key` (action argument), `parent`, `apoapsis`, `periapsis`, `distance` (from the active vessel), `is_target`, `is_active`, `target_body`, `target_vessel`, `can_fly` |
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
| `vessel.throttle_full`, `vessel.throttle_cut` | press |
| `staging.drop:<dragged>:<sequence>` | `<dragged>` is `item:<sequence>:<index>` (an activation, moves into that sequence) or `seq:<sequence>` (a sequence, moves to its place) |
| `staging.drop_new:<dragged>`, `staging.add`, `staging.remove:<sequence>` | Give an activation a new last sequence (or move a sequence to the end); add an empty sequence at the end; remove an empty one |
| `vessel.throttle_up`, `vessel.throttle_down` | hold |
| `vessel.pitch_up`, `vessel.pitch_down`, `vessel.yaw_left`, `vessel.yaw_right`, `vessel.roll_left`, `vessel.roll_right` | hold |
| `vessel.sas_toggle`, `vessel.sas_mode:<mode>` | SAS on/off; select a mode (switches SAS on) |
| `vessel.rcs_toggle` | RCS on/off |
| `vessel.translate_forward`, `vessel.translate_back`, `vessel.translate_left`, `vessel.translate_right`, `vessel.translate_up`, `vessel.translate_down` | hold; RCS translation |
| `nav.toggle_frame` | Cycle prograde and speed through surface, orbit and (with a target) target |
| `vessel.next`, `vessel.previous`, `vessel.fly:<index>` | Fly another vessel |
| `target.body:<id>`, `target.vessel:<index>`, `target.clear` | Choose or drop the target (also Shift+click in map view) |
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
| `ui.toggle:<layout>`, `ui.show:<layout>`, `ui.hide:<layout>` | |
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
