# T247: Liquid fuel and oxidiser in the base game, a Realistic Fuels mod

The base game gets two propellants. The kerosene, oxygen, hydrogen, methane and hypergolic records
move to a `realistic_fuels` mod, with what tanks need to hold them (insulation, pressure).

Propellant names are written into the code, not only the data: `osp_sim` (`properties.rs`,
`palette.rs`, `systems.rs`, `structure.rs` and others) and the plume colours in `osp_render`. Those
have to come from the records.

The `realism` mod depends on this one. A mod with a dependency that is off is dropped, so
`osp_mods::load` with no choices must still give the tests and `cargo smoke` the full set.

## Done when

- The base game's engines and tanks hold only the two propellants, and the plumes still look right.
- With the mod on, the fuels and the tank choices are back.
- `cargo smoke` and the Saturn V tests pass.

## Done so far

Built 2026-11-09. The base game has Liquid Fuel, Oxidiser and Solid Fuel. Every liquid engine and
thruster burns one mix at 1 : 2.36, so any tank suits any engine, and stock engines name the plume
they look like. `mods/realistic_fuels` holds the real propellants and patches the stock engines,
thrusters and craft; its special tanks are whole records, so Realism's own Solid Grain still
replaces them. With both realism mods on, every stock record is what it was.

My calls, to confirm: RCS burns the same mix in the base game (no separate gas tank), and Solid Fuel
is kept for solid motors.

Fuel cells and crew oxygen still name real resources in the code. Only Realism's craft use them; what
the base game does about them is T250.
