# T034: Networks: power and data over joints

The `power` capability on connectors exists and nothing uses it. Power and data become networks over the joints that carry them, solved per tick and in closed form on rails: sources, storage, loads with priorities, brownout shedding the lowest first.

## Done when

- A test craft with a source, a battery and two loads sheds the low priority one first.

## Done (2026-11-02)

- `Systems` keeps its networks: the parts with systems that the joints' `power` capability connects (`PartTree::power_groups`), worked out again when parts come off. A system takes part through `System::power` (`Power::Source`, `Store`, `Load` with a priority), `charge` and `powered`.
- Each step what is made and stored goes to the loads, the highest priority first; a load the energy does not reach is shed; what is over charges the stores by the room they have. Exact for any step up to `next_event`, which says when the stores run out, so time warp stops there.
- A stage let go takes its network with it. Tests: `a_network_runs_on_its_source_and_its_battery`, `the_lowest_priority_is_shed_first`, `a_source_charges_its_battery_and_stops_at_full`, `a_stage_let_go_takes_its_network_with_it`.
- Data shares the same joints and has no solver of its own yet (T036, T037 use it). Real sources and loads are T035's.

## Where it comes from

- docs/systems-plan.md (step 9)
