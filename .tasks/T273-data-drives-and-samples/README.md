# T273: Data drives and samples

Drives on parts, sized in bytes, and samples with mass in containers. An experiment with no room
pauses. Data can be moved between drives on a vessel and deleted, and goes with its part when the
vessel splits. A sample cannot be sent; it counts when its capsule is recovered.

## Done when

- A full drive pauses an experiment, and freeing room starts it again (test).
- A sample cannot be sent, and counts when it is recovered (test).
- Drives and samples come back from a save.

## Done (2026-11-13)

- A `drive` component, `{ data: <bytes>, samples: <kg> }`, and its system (`systems::Drive`): what it
  holds by subject, data and samples apart, each with what a whole set of it comes to. Its window
  shows its room and what it holds, and offers Move data here, Delete data and Dump samples.
- Experiments write what they make onto the drives their part is wired to (the power network), the
  biggest first, so a capsule's or a probe core's drive fills before an instrument's own. With no room
  left an experiment pauses ("no room on the drives") and wants no power; freeing room starts it
  again. Time warp stops where the drives fill, and whatever a step makes that finds no room the
  experiment has not made, so what it says it has collected is what the drives got, at any warp.
- Drives on content: a pod has its record's `drive`, or 2 MB and 5 kg a seat (every Realism capsule
  too); the probe core 2 MB; an instrument room for `holds` whole sets of its own experiments (1, the
  thermometer and barometer 3), in whatever sizes the mode's experiments have.
- A sample is never offered to be sent (`Drive::sendable` lists data only, for T274).
- Recovery: `Universe::recover_vessel` takes a vessel home, and in a career what its drives held
  counts in `Career::returned` (share of a whole set per subject, for T284). The manifest's Recover
  uses it and says what came home ("Recovered Probe with 0.1 kB of data and 0.50 kg of samples").
- Fixed on the way: an experiment's record could not be read back from a save (its own `description`
  and its catalogue info's were both written), so a save with an instrument on it failed to load.
  Experiment and instrument descriptions are the catalogue info's now.
- Tests (`systems::drive`): a full drive pauses an experiment and freeing room starts it again, the
  same at the highest warp as seven seconds at a time; data goes to the biggest drive and moves
  between drives; a sample cannot be sent and counts when it is recovered; drives and samples come
  back from a save. Capture: `OSP_CAPTURE_SCIENCE` now ends on the capsule's window, with the two
  readings on its drive.

## Left out

- A sample's mass is not added to its part's (under 2 kg in either mode).
- The vessel being flown cannot be recovered yet, only another from the manifest (T304).
- Deleting data does not let an experiment do that place again: it remembers what it has done.
- Moving data to another vessel without docking, and a drive failing and losing what it holds.
