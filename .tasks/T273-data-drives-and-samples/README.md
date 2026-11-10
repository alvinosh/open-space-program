# T273: Data drives and samples

Drives on parts, sized in bytes, and samples with mass in containers. An experiment with no room
pauses. Data can be moved between drives on a vessel and deleted, and goes with its part when the
vessel splits. A sample cannot be sent; it counts when its capsule is recovered.

## Done when

- A full drive pauses an experiment, and freeing room starts it again (test).
- A sample cannot be sent, and counts when it is recovered (test).
- Drives and samples come back from a save.
