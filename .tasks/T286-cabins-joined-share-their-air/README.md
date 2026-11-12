# T286: Cabins joined by a tunnel share their air, a hatch shuts one off, and a cabin's volume comes from its shape

Left over from T264 (2026-11-12). Every cabin keeps its own air today, even docked to another one.

- Cabins joined directly, or through docked ports with a tunnel (the way `board` moves people), share
  their air: one volume, one pressure, the regulators of both topping it up. Docking two cabins at
  different pressures evens them out.
- A hatch between them can be shut, which takes that cabin's volume, and its people's room, out of the
  shared air, and opened again.
- A cabin's volume comes from its part's shape (the inside of its shell, less what equipment takes),
  not the rules' room per seat, unless the part says.

## Done when

- Two docked cabins, one let down, come to one pressure when the hatch is opened, by their volumes (test).
- A leak in one of two joined cabins is held off from the other by shutting the hatch (test).
- A capsule's cabin volume follows its shell's size (test).
