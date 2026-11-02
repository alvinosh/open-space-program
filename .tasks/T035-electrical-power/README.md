# T035: Electrical power

ElectricCharge as energy, and the parts that make and store it, all procedural, with loads and brownout.

## Scope

- Battery (volume and tech give capacity and mass; primary or rechargeable; depth of discharge; cold derate).
- Fuel cell (rated power gives mass and reactant flow; makes water). The service module gains its hydrogen and oxygen tanks, which the craft lacks today.
- Solar array (area, tracking, degradation; power from distance and incidence); RTG (isotope mass gives power, decaying); converter and bus.
- Loads: avionics, transmitters, cabin, heaters, engine ignition (`ignition_resource`), gimbals, RCS valves, reaction wheels.
- Brownout: no ignition, no SAS, no link.

## Done when

- The Saturn V smoke test reports a power budget from launch to splashdown with a margin at the end, with the fuel cells burning real hydrogen.

## Notes

- Circuits (wiring, breakers, fuses) are a separate later task on top of the network.

## Done (2026-11-02)

- Fuel cell (burns what its part's tanks can give, only what the network asks, 42 g of hydrogen and 336 g of oxygen per kWh), solar array (flux, area, efficiency, facing, degradation, nothing in a shadow), RTG (decaying by its half life), electric load (priority, essential). All in `systems/electric.rs`, with sizers in `sizing.rs`: the CSM's cell (111 kg for 1.42 kW) and Voyager's RTG (37.7 kg for 158 W) are their oracles. A solar array's 3 kg/m² has no oracle yet.
- Brownout: an essential load with no power turns SAS off, and no engine lights.
- Time warp stops where a network's stores run out, where a consumer's tanks run dry and, for a vessel with arrays, at a shadow's edge.
- Apollo: three fuel cells by requirement, 4 cryogenic spheres on the service module (the part's flat mass came down by what they weigh, so no stage figure moved), 1.2 kW on the command module, 0.6 kW on the service module. The smoke flight reports 18.9 kg of hydrogen and 151 kg of oxygen burned over 10.4 days, 10.2 kg of hydrogen left, and 94% of the entry batteries at the ground (the charts come by themselves).

## Still open

- The loads are round figures for the CSM's budget, about 1.8 kW; they do not follow what the vessel is doing. Heaters, transmitters, gimbals and RCS valves as loads of their own come with T036, T037 and T045.
- The stack is powered on its own batteries nowhere but the CM: the S-IC, S-II, S-IVB and instrument unit have no electricity (T036). The lander has none, and the surface stay is still a gap.
- The CM's battery figure of 21 390 kJ is the record's (see T032).

## Where it comes from

- docs/systems-plan.md (step 10, Electrical power)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
