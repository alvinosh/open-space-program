# T209: Tank contents follow the engines on the craft, at their ratio

Asked for on 2026-11-06: choosing what a tank holds "should only give you options for the fuels
that an engine in the rocket can use, with the correct fuel ratios". The part window offers a fixed
list of mixes (`MIXES` in `osp_sim::properties`) with ratios of their own, whatever engines the
craft has.

## Scope

- The choices are the propellant sets of the engines on the craft (each engine's `propellants`,
  at its own mass ratio), named once each, plus Nothing (structure). Mixes no engine burns are not
  offered; with no engine on the craft yet, the usual list is.
- A tank's current contents stay shown if they match none (as now, "custom").
- Single resources an engine burns alone (monopropellant) are offered the same way.

## Done when

- On a craft with one kerolox engine the tank offers kerolox at that engine's ratio and nothing
  else but structure; adding a hydrolox engine adds hydrolox; tests cover it.

## Done so far

- 2026-11-06: `properties::mixes(craft, catalog)` gives what the tanks of a craft can hold: the
  propellants of every engine and RCS block on it (a template's, an `engine`, `rcs` or
  `solid_motor` component, or what a sizer makes), each set once at its engine's own ratio, in the
  order the craft lists them. The same resources at two ratios are two choices (`kerolox`,
  `kerolox_2`). A sized part that holds all its engine burns (a separation motor and its grain)
  is left out. No engine on the craft gives the usual list (`MIXES`). Labels carry the ratio,
  "Kerosene and liquid oxygen, 1 : 2.27"; sets `MIXES` does not name are called by their resource
  ids, fuel first.
- `properties::fields` and `properties::set` take the mixes (`&[Mix]`); `Builder::set_field`,
  the part settings rail and the capture's `ExpectField` work them out from the draft. A tank
  whose contents match none stays as "custom", labelled with its resources and ratio. Setting a
  mix no engine burns is refused ("Holds cannot be `hydrolox`: no engine on the craft burns it").
- Tests: `properties` (one K-5 gives kerolox at 2.27 and structure, an H-25 adds hydrolox at 6,
  two kerolox ratios, no engine, S-1 and cold gas thrusters alone, separation motor left out) and
  `builder` (`set_field` on a craft with a K-0).

- 2026-11-07, after the user said "The tank is still hard to fill up with fuel, the tank should
  only show options for the engines that are attached": a tank is offered only what the engines
  and thrusters it feeds burn, those joined to it through joints that pass propellant (the same
  joints the flight plumbing uses), each label naming them ("Kerosene and liquid oxygen, 1 : 2.27
  (K-5)"). Gone: the single propellants on their own, the usual list for a craft with no engine,
  and the engines of other stages. A tank that feeds nothing offers only Nothing, and the rail
  says "Feeds: No engine yet". `properties::mixes(craft, catalog, part)`.
- A tank still as the parts list gives it (`properties::NEW_TANK`, kerosene and oxygen 1 : 2.36)
  fills itself with its engine's mix at the engine's ratio, in a tank type that holds it, when it
  or an engine goes on (`properties::fill_from_engines`, called by `Builder::put_set` and by the
  preview, so what is shown is what goes on). An engine's mix wins over thrusters'. Tanks the
  player filled, and tanks past a decoupler, are left alone. One undo takes the part and the
  filling back.
- The part settings rail works the choices out only when the craft or the picked part changes
  (they build the craft). Tests: `properties` (engine's ratio and name, past a decoupler, two
  ratios, two engines named once, solid and thrusters, filling whichever goes on last, a filled
  tank left alone) and `builder` (an engine under two new tanks fills both, undone in one step).
  `OSP_CAPTURE_EDITOR_BUILD` shows the tank with no engine (`build_tank_no_engine.png`) and with
  the K-1 Booster on (`build_tank_filled.png`).
- 2026-11-07, after "if i attach an engine radially it says no propellant": a joint on a surface
  passed no propellant (`Carries::SURFACE`), so an engine on a tank's side was fed by nothing, and
  the tank offered it nothing. Surfaces now pass propellant as nodes do; a decoupler still passes
  it only with its crossfeed on, whether on a node or on a side. No shipped craft's staging figures
  change (`craft_golden`). Test: `builder` (a K-0 on the lower tank's side fills both tanks, they
  offer what it burns, and in flight they feed it).

## Open

- On a craft with engines, contents that are not an engine's whole mix are only kept, not
  offered: the split tanks of the Saturn V's service module and LM (Aerozine50 or MON1 alone),
  fuel cell hydrogen and oxygen, helium pressurant, water and food all show as "custom".
