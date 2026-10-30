# Smoke tests

Whole-mission flights, start to finish, to find what the simulation cannot do yet. The first is
the Saturn V on Apollo 11: pad, parking orbit, translunar injection, lunar orbit, the return and
the entry. Dragon on a Falcon 9, the Shuttle, Soyuz and the rest will be more missions in the same
harness.

```sh
cargo smoke                    # every mission, about 3 s each
cargo smoke saturn_v           # one
OSP_SMOKE_OPEN=1 cargo smoke   # and open the report
OSP_SMOKE_TRACE=1 cargo smoke  # print a line of telemetry every 10 s, for calibrating a script
```

Reports go to `target/smoke/`: `index.html` lists every mission that has been flown, and each
`<mission>/index.html` is one self-contained page (no network). `report.json` beside it has the
same data for scripts.

## Reading a report

- The timeline is the mission's phases. Click one to zoom the charts and findings to it. Phases
  the script never reached are greyed, and a banner says if it stopped early.
- Telemetry: altitude, speed, acceleration, thrust, mass and propellant, the orbit's apsides,
  dynamic pressure, attitude, distance to the Earth and the Moon, structure load.
- Systems: one card per area (propulsion, power, comms, ...) saying whether the sim modelled it.
- Findings: every check, with the real mission's figure next to what the sim did.
- The craft, part by part: what the sim runs for each part, which are shaped by hand in the craft
  file, which carry a flat mass instead of being built from tanks and structure.
- The event log: the vessel's own log and the script's marks.

Each finding has one result:

| Result | Meaning | Fails the test |
| --- | --- | --- |
| Pass | The sim did what the real vehicle did, within the range given | no |
| Fail | It did not: a bug | yes |
| Known issue | It did not, and a task in `.tasks/` covers it (its id is in the finding) | no |
| Not modelled | The sim has nothing for this yet | no |
| Bridged | The script stepped over something the sim cannot do to go on with the rest | no |
| Info | A measurement | no |

## How a mission flies

It loads every mod and starts a scenario through `Universe::new_game`, as the game does. After
that it only does what a player can: throttle, the stage key, SAS modes, RCS, targets, the transfer
planner, planned burns and the burn executor, time warp. Nothing is placed, teleported or patched
in. Steering uses the game's own SAS control law (`tests/smoke/pilot.rs`), and the ascent is a
pitch program with an angle-of-attack limit, then a levelling-off law, which any player could fly.
Small corrections are made with an RCS push (`planning::rcs_trim`) because the executor is not
accurate enough (T027).

Where the sim cannot do what the mission did, the script says so with `known`, `gap` or `bridged`,
and goes on if it can. A `bridged` finding is the only place the script may do something no
player could, and it must say what and why. The Saturn V throws its lunar module drum away with the
adapter (T044), waits out the surface stay in lunar orbit, and goes home as a CSM alone.

## Adding a mission

1. A new file in `crates/osp_sim/tests/smoke/` with a `pub static MISSION: Mission` (id, title,
   summary, scenario id from the mods, the list of phases, and `fly`), and a `#[test]` in
   `main.rs` that runs it. Copy `saturn_v.rs`.
2. Write the phases the real mission had, in order, each with what it did (`PhaseSpec::goal`).
   Phases the script cannot reach still show in the report as not reached.
3. In `fly`, call `w.phase("id")` at the start of each, drive the vessel, and check against the
   real figures with `w.within(area, title, value, range, unit, "real mission: ...")`. Use `must`
   for a check the script cannot go on without; it stops the flight.
4. Run `audit::inspect` and `audit::report` on the pad. They build the craft from its part list
   and write what is made of what. Add the names of parts that do something in the real vehicle
   and nothing here to `FUNCTIONS` in `audit.rs`.
5. Keep the real figures and their source in the check, not in a comment.

`world.rs` is the flight (ticks, rails, sampling, findings), `pilot.rs` the hands (pointing, the
ascent), `planning.rs` the map (transfers, aiming, burns), `report.rs` and `report.html` the page.

## What the Saturn V found

Nothing modelled for these, listed in the report and covered by tasks in `.tasks/` (T026, T027, T035 to
T049): the lunar
module and docking, landing, electrical power (batteries are a number; the fuel cells are boxes),
avionics, communications, entry heating, life support, parachutes and the sea, capsule entry
(it tumbles and pulls 69 g), an engine cutoff in a sequence, and a burn executor that overshoots
by 0.9 m/s. What flew and matched the real mission closely: liftoff mass and thrust, max Q (35 kPa
at 83 s), S-IC and S-II burn times and cutoff altitudes, orbit insertion at 11 min, translunar
injection (3.1 km/s, 10.86 km/s at cutoff, the Moon's sphere of influence at 66 h), lunar orbit
insertion and the return burn.

Building the mission also showed the craft needed two things, now in `saturn_v.json5`: the S-IVB
had no way to turn in orbit (it now carries four APS thrusters and their propellant), and the
service module's RCS tank held a fifth of the real CSM's propellant (the Saturn IB and the
`apollo_csm` assembly changed with it).

When one of the gaps is closed, the check that says "not modelled" should become a real check.

The plan for closing these gaps, with the procedural parts each needs, is `docs/systems-plan.md`.
The build editor and its tests come first (`docs/build-editor-plan.md`): the build suite below, then a mission test that builds the Saturn V in the editor and flies Apollo 11 from it.

## The build suite

A second suite next to the missions, `crates/osp_sim/tests/build/`, for how craft are built rather than flown. It was written first, while most of it fails, so that the red cells are the measured to-do list for the editor's core.

```sh
cargo smoke build              # only this suite (and `cargo smoke` runs it with the missions)
OSP_SMOKE_OPEN=1 cargo smoke build
```

The page is `target/smoke/build/index.html`, with `report.json` beside it, and the home page links it.

### The placement matrix

Rows are the kinds of part the sim has: storage, engine, RCS block, reaction wheel, battery, crew, decoupler, lifting surface, container, gear leg, and two that the stage gatherer treats on their own, a pod and a part with two components. Columns are the ways to put one on a craft: on a top node, under a bottom node, radially on a surface at (u, v) with a standoff, in a ring of 2, 3, 4 or 6, inside a container, inside a fairing (on a deck, fitted by `CraftDef::fit_fairings`), canted 15 and 90 degrees, and as a mirrored pair. Each cell builds a 1 m host with the part placed on it (`cell.rs`) and checks (`checks.rs`):

| Check | Passes when |
| --- | --- |
| Loads | The craft builds, or is refused with a rule of building. A refusal that says "until the physics runs on parts" is a limit of the sim, not a rule, and fails. |
| Kept | Every part and component is in the physics: the tanks, engines, RCS blocks, wheels, wings, gear legs, crew seats and battery charge of the craft are all there. |
| Mass | The craft weighs what its parts and their propellant weigh, summed apart from the physics. |
| Balance | Each part sits where the assembly puts it, and the dry centre of mass and inertia are the parts' sum. |
| Aero | Outside the skin, the air sees it (the outline or the loads change); inside a container or fairing it does not. An engine is modelled apart and only its inside cells are checked. |
| Save | The craft written and read back is the same craft, in the same places. |
| Pad | Five seconds on the pad at the Cape give no NaN and no break-up, and the joint each stage hangs from carries the weight above it. |

A cell is green (all pass), red (a check fails, and the cell's rule in `expect.rs` says which and names the task that fixes it) or grey (the sim refuses with a rule, which the refusal names). The rules say what is expected; the test fails when a cell is not as its rule says. That works both ways: a new failure fails the test, and so does a red cell that now passes, until its rule is removed (the expectation flips). A red cell must name a task that is not yet done.

`build_checks_can_fail` runs each check on a craft it does not match, so a check that cannot fail does not pass for nothing.

Later tasks add to the same suite: sizer checks (each against its oracle), reference builds placed from the palette, and property tests with a seeded generator.
