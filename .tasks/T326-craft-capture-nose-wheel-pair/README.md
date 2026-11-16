# T326: The craft capture's nose wheel goes on as a mirrored pair

`OSP_CAPTURE_EDITOR_CRAFT=1` stops with "the craft has 6 parts, wanted 5" at 28.9 s: the nose wheel
put on the plane's belly (`tank` barrel, u 0.25, v 0.85, mirror on) goes on as two. Seen on
2026-11-16; it fails the same way built from the last commit (19febc9), so it came in before that day's
work. Either the belly is no longer at u 0.25 on a plane lying on the runway, or a part on the mirror
plane is copied across it.

## Done when

- A part put on the mirror plane of a plane goes on once, and the capture runs through.
- Better: a playtest builds the plane (T325) instead of the timed capture.
