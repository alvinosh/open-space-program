# T330: Editor leftovers from finished tasks

When the tracker was cleaned up on 2026-11-20, T018, T203, T208 and T209 were finished by their
"Done when" lists but each still listed smaller things no other task holds. They are kept here so
they are not lost. Split one out into its own task when it is picked up.

## What is left

- Orthographic front, side and top views, and a greybox mode (parts shown plain). From T018.
- Staging goes back to the default whenever a part is added or taken off (`hangar.rs`). From T018.
- A widened part does not carry the stack along: the decoupler and interstage next to it keep their
  widths (only width matching exists, `shaping.rs`). From T018 and T203.
- After a resize, surface-mounted parts keep their (u, v) rather than their distance from an end,
  and paint layers stay where they were in metres. From T018.
- Paint. From T203.
- Parts list: filtering by mod, by the widths the craft has, and sorting by name, mass or thrust.
  From T208.
- Tank contents that are not an engine's whole mix are only kept, never offered: the Saturn V's
  split service module and LM tanks (Aerozine50 or MON1 alone), fuel cell hydrogen and oxygen,
  helium pressurant, water and food all show as "custom". From T209.

## Done when

- Each item above is built, or moved to a task of its own, or dropped by the user.
