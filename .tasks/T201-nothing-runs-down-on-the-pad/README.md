# T201: Nothing runs down on the pad

Reported 2026-11-06: "Vessel shouldnt drain resources on the pad". A vessel waiting on the pad drew its batteries down and burned its fuel cells' reactants and its crew's supplies, so waiting for a launch time cost it. A Saturn V left a day on the pad went from 318,625 to 138,674 kJ in its batteries and 52 kg lighter.

## Done when

- A vessel waiting on the pad, at any warp, keeps everything it carries, and time warp does not stop for a battery going flat there.
- Once it leaves, it draws on its own as before.

## Done

- Ground support, as a launch site gives: a vessel put on a pad or a runway (`Vessel::on_launch_site`, `on_runway`) is on it until it stops standing still (lifts off, or rolls away), and then for good. `Vessel::on_ground_support`.
- On it, every load has power, the batteries are kept full and the fuel cells stand idle (`Systems::supply_power`), and what the crew and the fuel cells burn is fed without coming out of the tanks. Nothing runs out, so time warp has nothing to stop for.
- `PowerState::supplied`, `vessel.power_supplied`: the Resources window says "On ground power".
- Test: `the_pad_keeps_it_topped_up` (a day on the pad leaves the Saturn V's charge and mass exactly as they were; the same stack in orbit draws). It fails with ground support off.
- `OSP_CAPTURE_POWER` now checks the lunar module holds full on the pad through days of warp, then runs down in orbit.

## Left out

- Saves from before have no ground support, so a vessel waiting on the pad in one runs down until it is launched again.
- Batteries are topped up at once, not charged at a rate.
- Landing back on a pad does not plug it in again.
