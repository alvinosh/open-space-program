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
| `engines` | Engines | `thrust_vac` (N), `isp_vac`/`isp_sl` (s), `min_throttle`, `ignitions`, `requires_ullage`, `gimbal_range` (degrees, 0 = fixed), `propellants: { Name: mass ratio }` |
| `tanks` | Tanks | `dry_mass`, `contents: { Name: kg }` |
| `reaction_wheels` | Reaction wheels | `mass` (kg), `torque` (N·m about each axis) |
| `craft` | Vessels | `stages` in firing order: `engines`, `tanks`, `reaction_wheels`, `structure_mass`, `diameter`, `length` |
| `launch_sites` | Launch sites | Position, `heading`, optional `pad` and `roads`; see [Launch sites](#launch-sites) |
| `scenarios` | New-game setups | `start: { type: "LaunchSite", site: "<id>" }` or `{ type: "Orbit", ... }`; pick with `OSP_SCENARIO=<id>` |
| `layouts` | UI windows and panels | See [User interface](#user-interface) |
| `themes` | Style classes | All theme records merge in load order |
| `keybindings` | Key → action | All keybinding records combine |

See the files in `assets/core` for complete, commented examples of each.

### Craft mass and control

A stage's `diameter` and `length` shape its rotation as well as its looks. Each stage is
modelled as a cylinder: tank dry mass as a thin shell along it, `structure_mass` (and
reaction wheels) as a uniform solid, engines as point masses on its base, and propellant as
a solid column settled at the aft end. That gives the centre of mass and the moments of
inertia, which change as tanks drain and stages drop.

Control torque comes from the bottom stage's burning engines swivelling up to
`gimbal_range`, and from reaction wheels anywhere in the stack. A single engine sits on the
axis and cannot roll the vessel; two to six engines sit on a ring at half the stage
radius, and seven or more on a ring at 65% around a central engine, so clusters roll by
gimballing sideways. With the engines off, only reaction wheels can turn the vessel, so
give upper stages and payloads some.

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
  },
  roads: [ { points: [[-40, 0], [-400, -150]], width: 9 } ],
}
```

Start a scenario there with `start: { type: "LaunchSite", site: "my_site" }`.

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
| `widget` | `column` (default), `row`, `panel`, `spacer`, `label`, `button`, `bar`, `window`, `navball`. Unknown names act as containers. |
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
| `repeat` | List key; the children are created once per item, and `item.` refers to the current item. |
| `children` | Child nodes. |

Windows have a title bar that can be dragged to move the window, and a close button.
`navball` shows the attitude indicator; give it a `width` and `height`.

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
| `radius` | corner radius, px |
| `background` `hover_background` `border_color` `color` | `"#rgb"`, `"#rrggbb"`, `"#rrggbbaa"`, `[r, g, b]`, `[r, g, b, a]` |
| `active_background` `active_border_color` `active_color` | colours while the node's `active` condition holds |
| `font_size` | px |
| `z_index` | stacking order of layout roots |

The flight UI is a set of modules, each its own layout in the `flight` group:

| Layout | Contents | Node ids |
| --- | --- | --- |
| `toolbar` | App launcher (top right) | `#toolbar` |
| `time` | Warp buttons, UT and mission time (top left) | `#time` |
| `altimeter` | Altitude, vessel and situation (top centre) | `#altimeter` |
| `messages` | Latest event and warp notices | `#messages` |
| `navball` | Navball, speed, throttle, SAS modes, control inputs (bottom centre) | `#navball`, `#throttle`, `#ball`, `#sas`, `#inputs` |
| `staging` | Stage stack (bottom left) | `#staging`, `#stages` |
| `flight_info` | Speeds, Mach, pressures | `#flight` |
| `orbit_info` | Apsides, inclination, period, predicted events | `#orbit`, `#events` |
| `vessel_info` | Thrust, TWR, mass, delta-v | `#propulsion` |
| `attitude_info` | Centre of mass, inertia, control authority, rates | `#attitude` |
| `log` | Flight log | `#log` |

To add a window to the launcher, insert a button into `#toolbar.children` (see
`mods/example_mod/patches/ui.json5`).

### Keybindings

```json5
{ id: "my_keys", bindings: [ { key: "F6", action: "ui.toggle:my_window" },
                             { key: "KeyR", action: "vessel.throttle_up", hold: true } ] }
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
| `vessel.throttle`, `vessel.thrust`, `vessel.twr`, `vessel.mass`, `vessel.accel_g` | 0..1, N, -, kg, g |
| `vessel.delta_v`, `vessel.stage_count`, `vessel.landed`, `vessel.engines_running` | |
| `vessel.sas`, `vessel.sas_mode`, `vessel.sas_mode_label` | SAS on; mode (`off`, `stability`, `prograde`, `retrograde`, `normal`, `anti_normal`, `radial_out`, `radial_in`) |
| `nav.frame`, `nav.speed` | `"Surface"` or `"Orbit"`; speed in that frame, m/s |
| `nav.heading`, `nav.pitch`, `nav.roll` | Attitude against the local horizon, degrees (roll is empty pointing straight up or down) |
| `control.pitch`, `control.yaw`, `control.roll` | Rotation command applied (pilot or SAS), −1..1 |
| `attitude.com`, `attitude.moi_pitch`, `attitude.moi_roll` | Centre of mass above the base (m); moments of inertia (kg·m²) |
| `attitude.torque_pitch`, `attitude.torque_roll`, `attitude.accel_pitch`, `attitude.accel_roll` | Torque available (N·m) and the angular acceleration it gives (deg/s²) |
| `attitude.rate_pitch`, `attitude.rate_yaw`, `attitude.rate_roll` | Angular velocity, deg/s |
| `attitude.gimbal`, `attitude.gimbal_range`, `attitude.wheels` | Current and maximum gimbal angle (deg); reaction wheel torque (N·m) |
| `time.met`, `time.launched` | Seconds since liftoff |
| `screen.message` | The latest log message, for a few seconds |
| `ui.shown.<layout>` | Whether a layout is showing |
| `vessel.stages[]` | `name`, `active`, `delta_v`, `burn_time`, `fuel` (0..1), `has_fuel`, `ignitions`, `engines` |
| `vessel.log[]` | `time`, `message` (newest first) |
| `orbit.apoapsis`, `orbit.periapsis` | Altitudes, m (apoapsis empty when escaping) |
| `orbit.escape`, `orbit.inclination`, `orbit.eccentricity`, `orbit.semi_major_axis`, `orbit.period` | |
| `orbit.events[]` | `label`, `time_to` (s): predicted SOI changes and atmosphere entry |
| `mods.count`, `mods.errors`, `mods.warnings` | |
| `mods.list[]` | `id`, `name`, `version`, `authors`, `description` |
| `mods.problems[]` | `error` (bool), `text` |

## Reference: actions

| Action | |
| --- | --- |
| `vessel.stage`, `vessel.throttle_full`, `vessel.throttle_cut` | press |
| `vessel.throttle_up`, `vessel.throttle_down` | hold |
| `vessel.pitch_up`, `vessel.pitch_down`, `vessel.yaw_left`, `vessel.yaw_right`, `vessel.roll_left`, `vessel.roll_right` | hold |
| `vessel.sas_toggle`, `vessel.sas_mode:<mode>` | SAS on/off; select a mode (switches SAS on) |
| `nav.toggle_frame` | Switch prograde and speed between surface and orbit |
| `warp.increase`, `warp.decrease`, `warp.stop` | |
| `view.toggle_map` | |
| `ui.toggle:<layout>`, `ui.show:<layout>`, `ui.hide:<layout>` | |
| `ui.toggle_group:<group>`, `ui.show_group:<group>`, `ui.hide_group:<group>` | Every layout in a group |
| `game.reload` | Reload all mods and rebuild the UI |

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
