# T262: Processes, and one resource solver per vessel

A part can turn inputs into outputs: a `process` component naming a record in a new `processes`
collection, with inputs and outputs in kg/s, power in W, and conditions (in sunlight, landed, in an
atmosphere, cabin pressure once T264 has it). It can be switched on and off by the player and later by
automation (T278).

Today `Propulsion::step_systems` feeds each consumer in turn, so the first one served wins and a
process that makes something cannot hand it on in the same step. This replaces that with one solver
per vessel each step: every consumer and process together, what each can reach (`fluid_groups`),
shared by priority and then in proportion, nothing taken twice, outputs held back by tank room, and
the time the first input runs out or the first output fills, so time warp stops there. The crew's
needs and the fuel cell move onto it. Power stays on its own network; a process is a load there.

## Done when

- Two processes after the same kilogram get it by priority, and nothing is taken twice (test).
- A process whose output tank is full slows to what fits (test).
- One step of a day and 86 400 steps of a second leave the same tanks, to rounding (test).
- Electrolysis, added by the example mod as a record, runs with no code of its own.
- The Saturn V smoke flight reports the same consumables as before.

## Done (2026-11-10)

- `processes` records (`ProcessDef`: `inputs`, `outputs` in kg/s, `dump`, `power` W; refused if the
  outputs weigh more than the inputs) and a `process` component (`process`, `scale`, `priority` 0-255
  default 100, `on`), run by `systems/process.rs`, with Start and Stop on its part window. The example
  mod has electrolysis (`mods/example_mod/processes/electrolysis.json5`), which needs Realistic Fuels and
  Realistic Life Support for its resources.
- `propulsion/supply.rs`: one account per vessel. Each step, before the power networks, every system
  that uses the tanks says the most it wants (`System::wants`) and the tanks as they stand set its
  share (`plan`): the highest priority first, each resource shared in proportion to what is wanted,
  output held back by tank room unless dumped, and each lot never takes more than the lots before it
  left. A process then asks its network for the power of that share only, and a fuel cell offers no
  more than its tanks can feed (it plans at its rated power). After the networks, each takes and gives
  what it used (`uses`) the same way and is told its share (`exchanged`). People come first (255), then
  fuel cells (250).
- `next_system_event` stops time warp where a set of tanks someone draws on runs dry or one someone
  fills is full, with everything drawing on it counted together (`System::exchanging`).
- The crew (one lot per need, so short of water they still breathe) and the fuel cell are on it. The
  old `take_consumption`, `consuming`, `fed`, `fed_for` and `can_go_without` are gone.
- Tests (`propulsion/supply/tests.rs`): a scarce kilogram goes to the higher priority and is split once;
  equals share in proportion to what they want; a process with a nearly full output tank slows to what
  fits; a process without power does not run and one without inputs asks no power; a day in one step
  leaves the same tanks as 86 400 one-second steps (electrolysis feeding the fuel cell that powers it,
  with a crew breathing beside it); warp stops where an input runs dry and where an output fills; a
  stopped process does nothing. 769 tests pass across the sim, game, mods and smoke crates.
- `cargo smoke saturn_v`: 143 passed, 0 gaps, every sample and finding the same as before (43 kg
  oxygen, 112 kg water, 19 kg food left).

## Left out

- Conditions on a process (sunlight, landed, in an atmosphere, cabin pressure): each comes with the
  task that needs it (T265 greenhouses, T277 drills, T264 cabins).
- Warp stops count lots on the same tanks together; lots on different parts whose tanks only partly
  overlap are counted apart, so their stop can come late.
- On ground power a process stands still: it shows running, but nothing goes in or out of its tanks,
  as nothing does for anything on the pad (T201).
- A process takes power for its planned share; if a fuel cell burns less than it planned for, a
  process sharing its oxygen could have had a little more that step.
