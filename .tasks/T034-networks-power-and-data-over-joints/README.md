# T034: Networks: power and data over joints

The `power` capability on connectors exists and nothing uses it. Power and data become networks over the joints that carry them, solved per tick and in closed form on rails: sources, storage, loads with priorities, brownout shedding the lowest first.

## Done when

- A test craft with a source, a battery and two loads sheds the low priority one first.

## Where it comes from

- docs/systems-plan.md (step 9)
