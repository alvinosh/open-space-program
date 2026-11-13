# T295: Contracts: goals offered for money in a career

A career needs work to earn money between scenarios: contracts offered a few at a time (reach space,
orbit, a flyby of the Moon, a landing, a satellite in a given orbit), each a mission goal (T165) with
a reward and a deadline, harder ones offered as the career gets further. Accepted from the hangar,
met by any craft, paid on completion.

## Done when

- A contract to reach orbit is offered, accepted, met by a flight and paid (test).
- What is offered grows with what has been done.

## 2026-11-12

Pacing asked for by the user (T301): Casual's contracts go sounding rockets, suborbital tests, first
orbit, then the Moon; Realistic's offer many sounding rockets and uncrewed probes (altitude records,
first satellite, probes to the Moon) before any crewed flight.

## Done (2026-11-13)

- `contracts` records (`osp_sim::contract::ContractDef`): `title`, `description`, `when` (a mission
  condition), `hold`, `reward`, `days`, `after`, `tree`. Three offered at a time, a career's tree's,
  the first in order whose `after` are done (`Career::offers`), so what is offered grows with what is
  done. Taken on (`Career::accept`), one is a goal for whatever vessel is flown, checked after every
  tick and jump on rails (`Universe::observe_mission`); met, it pays and is done; past its deadline
  it lapses and is offered again. Its clock starts when the game first looks at it, so a new
  career's are taken on in the hangar before its first launch. Dropped with `career.drop`.
- New conditions for them, missions and tutorials too: `Visit` (a body's sphere of influence),
  `Crew` (people aboard; `max: 0` uncrewed), `Reached` (has been this high), `All`.
- Content: Casual's 13 (`assets/core/contracts`: first launch, 10 km, 30 km, space, a person to space
  and back, orbit, a crew in orbit, a high orbit, Moon flyby, orbit and landing, a crew on the Moon,
  a Mars flyby; 8,000 to 1,200,000). Realistic's 17 (`mods/realism/contracts`: 1, 5, 20, 50, 100,
  200 and 1,000 km, back from space, the first satellite, one in space high, a probe to the Moon, a
  person to space and back, a person in orbit, a Moon orbiter and lander, a crew round the Moon and
  on it; 20,000 to 8,000,000). Rewards are guesses until T301.
- Game: the Contracts window (`contracts` layout, from the editor bar and the toolbar in a career),
  with Take on and Drop; a contract met or lapsed is a notice ("Contract complete: First launch
  (+8,000 funds)").
- Tests: `contract::tests::a_contract_to_reach_orbit_is_taken_on_met_and_paid` (offered, taken on,
  met on rails, paid once, the Moon offered after), `a_contract_past_its_deadline_lapses` (and back
  from a save), `tests/tech_trees.rs` `contracts_are_offered_in_turn_until_all_have_been` (both
  modes). Capture: `OSP_CAPTURE_CONTRACTS` (a new Casual career takes on First launch in the hangar,
  a C-1 on a short tank and a K-0 flies past a kilometre, the funds go from 50,000 to 58,000).

## Left out

- Contracts are checked against the vessel being flown only.
- Found on the way: a Casual career's first rocket is hard to build. Its first node's engines are a
  K-0 (26 kN) and the S-1 (3.3 kN), and several engines cannot go under one tank yet (T300), so a
  C-1 lifts off only on a 1.2 m tank. T300 matters for Casual's first flights.
