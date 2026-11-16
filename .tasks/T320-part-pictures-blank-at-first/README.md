# T320: Part pictures are blank when a group is first shown

Seen in a playtest's picture (2026-11-16): just after the Engines group opens, its tiles are empty
dark squares with only the names; the pictures come a moment later. Opening the Pods group also gave a
154 ms frame once.

## Done when

- A group's tiles show pictures (or a placeholder in the part's outline) the frame the group opens,
  and opening a group gives no frame over 50 ms.
