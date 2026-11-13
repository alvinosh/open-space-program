# T293: Career money: funds earned from missions and contracts, kept per save

Chosen by the user on 2026-11-12: launch pad and builder upgrades are paid with money, so a career
needs funds. A career starts with some, earns them by meeting missions' goals (T165) and contracts
(T295), and keeps them in its save. What craft and parts cost is not in this task.

## Done when

- A met goal pays its reward into the career's funds, once, and a save keeps them (test).
- The funds show in the hangar and the editor.

## Done (2026-11-13)

- `Career::funds` (saved), starting at the tree's `funds` (`tech_trees`: Casual 50,000, Realistic
  500,000; guesses until T294 prices the facilities), `Career::earn` and `Career::pay` (refuses what
  it cannot pay, saying by how much), `career::money` for showing it.
- Goals take a `reward`; a goal met in a career pays it into the funds once
  (`Universe::observe_mission`), and the news says so ("Goal complete: Reach orbit (+12,500 funds)").
- The editor's panel shows Funds and Science at its top in a career, with a vessel on show or not
  (`career.funds`).
- Tests: `mission::tests::a_met_goal_pays_its_reward_into_a_career_once` (paid once over three jumps,
  back from a save), `career::tests::a_career_spends_only_the_funds_it_has`, `tests/save.rs` keeps the
  funds, the game's goal news with a reward. Capture: `OSP_CAPTURE_CAREER`'s editor shows "Funds
  50,000".

## Left out

- A career has no missions of its own yet, so nothing pays until contracts (T295) offer goals; the
  scenarios' goals have no rewards.
- Careers saved before have no funds (0).
- What craft and parts cost (not in this task).
