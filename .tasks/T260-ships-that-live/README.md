# T260: Ships that live: life support, cabins, radiation, science, failures, resources, automation

Said on 2026-11-10: the next big project, with different settings for Casual and Realistic. The brief
is `spec.md` in this folder, a breakdown of the KSP mod Kerbalism: a ship is a set of parts that keep
running, each needing something and doing something, with a cost when it cannot. It is a design
reference only; no code or data is copied, and its numbers are not used.

## What the game already has

- Every vessel lives in the one `Universe` and is stepped whether it is flown or not, exactly over any
  step up to `next_event`, so warp gives the same result as real time and stops where something
  runs out (`universe.rs`, `System::step`). That is the spec's background simulation (§1.3, §14.3).
  New systems keep that contract instead of adding a scheduler.
- Power networks over joints with sources, stores, loads by priority and brownout (T034, T035), and
  ground power on the pad (T201). Power stays its own solver; processes are loads on it.
- Comms: link budget, line of sight, light delay, relays over many hops, a crew or a link to
  command (T037, T038, T256). No data rate yet.
- Heat per part with conduction and burn-off (T257).
- Life support: oxygen, water and food by the day; without oxygen for five minutes every person dies at
  once. Crew are a count, not people (T049, `systems/crew.rs`).
- A `rules` record mods patch (`assets/core/rules`), per-save mods (T245), and Casual and Realistic
  as sets of mods on the New game screen (T246).
- Avionics triggers, which do part actions when a time, a propellant level or an acceleration
  comes (`TriggerDef`): the start of automation.
- What each system can be told to do, offered with no code of its own (`System::actions`).

## Casual and Realistic

Every system has a Casual setting in the base game and a Realistic one from a mod: the table is in
T250, confirmed by the user on 2026-11-10, when this became the priority. Science buys the tech tree
as in KSP with Kerbalism (T271), spacewalks will be done (T104, T283), and in Casual a probe core has a
small radio of its own (T282).

## Order

1. T250 decided; T261 the rules each system reads; T282 a probe core's own radio.
2. T262 processes and one resource solver per vessel; T263 crew who are people.
3. T258 life support; T264 cabins; T265 recyclers and greenhouses; T266 comfort and stress.
4. T267 radiation; T268 seeded chance; T269 solar storms.
5. T050 failures; T270 repairs inside; T283 outside, on a spacewalk (later, after EVA).
6. T271 decided; T272 experiments; T273 drives and samples; T274 sending data; T275 laboratories;
   T284 science buys the tech tree.
7. T276 where resources are; T277 drills and ISRU.
8. T278 automation; T279 the vessel monitor; T280 the planner.
9. T281 the long-stay test that closes it.

One task at a time, as always.

## Different from the spec

- No scheduler of its own and no "unloaded" vessels: see above.
- No scripting language (T075): automation is a fixed set of events and actions.
- Realistic needs come from the figures pods use today (0.84 kg oxygen, 3.0 kg water, 0.6 kg food a
  person a day), not the spec's.
- Crew are people, never kittens.

## Done when

- T281 passes under Casual and under Realistic, and the Saturn V still flies under both.
