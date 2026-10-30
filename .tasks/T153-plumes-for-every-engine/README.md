# T153: Plumes for every engine

An engine gets a plume only if all propellants of one of five looks (`Kerosene`+`LqdOxygen`, `LqdHydrogen`+`LqdOxygen`, `Aerozine50`+`NTO`, `LqdMethane`+`LqdOxygen`) are in its propellant list, else it has none and is drawn dark with no warning. The realism import uses other names, so 557 of its 899 engine configs have no plume:

- 95 `RP-1`+`LqdOxygen` (the F-1, so the Saturn V's first stage), 10 `CooledRP-1`
- 92 `NTO`+`UDMH`, 30 `UDMH`+`LqdOxygen`, 29 `AK27`+`UDMH`, 22 `MMH`+`MON3`, 8 `IRFNA`+`UDMH`
- 19 `RG-1`, 22 `CooledRG-1`, 18 `Ethanol75`, 10 `AK20`, 8 `HTP`
- all 108 solids (`HTPB`, `PBAN`, `CTPB`, `PSPC`, `PUPE`, `NGNC`): that is T154's look, but they must not be dark in the meantime
- electric (`XenonGas`) and nuclear thermal have none and may stay without, or get a faint one

## Scope

- Match a look by what the propellant is (a class: kerosene-like, hypergolic, alcohol, solid, hydrogen, methane) with a name table in data, not by exact names; or give the importer a `plume` per engine. Data, no special case in code.
- An engine whose propellants match nothing gets a generic fallback look rather than none.
- Load-time warning that names the engine when a look is missing.

## Done when

- A test loads every engine of every shipped mod and asserts each non-electric one resolves to a look.
- The Saturn V, Atlas, Soyuz, Titan II and Redstone craft show a plume at ignition (captured screenshots).

## Done

Looks match by `|` alternatives per propellant entry and by engine `kinds`; new looks Alcohol, Storable Lox, Monopropellant, Nuclear Thermal, Solid, and a `fallback` Generic (electric engines are left without). `plume::tests::every_shipped_engine_has_a_plume` loads every engine of core and the realism mod. Checked by screenshot: the Saturn V (Apollo 11 scenario) shows its F-1 plume at liftoff. The other craft in the list were not captured one by one; the test covers them by propellant. Solid motors have a plume now but are still drawn with a liquid bell: T154.
