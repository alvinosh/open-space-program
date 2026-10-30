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

## Where it comes from

- docs/systems-plan.md (step 10, Electrical power)
- old issue list, issue 13 (docs/issues.md, removed; see git history)
