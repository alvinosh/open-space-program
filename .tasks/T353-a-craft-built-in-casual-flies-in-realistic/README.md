# T353: A craft built in Casual flies in Realistic

Found on 2026-11-21, when the stock craft were rebuilt in the builder (in a Casual game). Their
tanks hold Casual's Liquid Fuel and Oxidiser in Casual's one tank type, so in a Realistic game no
engine lights: Realistic engines burn kerosene, hydrogen, methane or hypergolics, hydrogen needs an
insulated tank and nitrogen a COPV. The stock craft only fly there because Realistic Fuels patches
each of their tanks by part id (54 lines, rewritten for the rebuilt craft); a player's own craft has
no such patch.

## Done when

- A craft built in one mode and flown in the other has tanks holding what its engines burn there,
  in a tank type that can hold it (as the builder already fills a tank from its engines, T209), and
  the stock craft patches in Realistic Fuels are gone.

## More than fuel

The rebuilt stock craft needed three more things before they flew in Realistic, none of which a
Casual game asks for or the builder mentions:

- An antenna: a probe core is flown by radio. Without one nothing stages on the pad.
- A battery: the probe core and antenna need power; with only solar panels the craft shed its load
  at liftoff, lost its link and could not stage again.
- Avionics rated for the craft: a probe core flies 20 t. Realistic Avionics patches each stock
  craft's probe core (`mods/realistic_avionics/patches/craft.json5`) until this task is done.
