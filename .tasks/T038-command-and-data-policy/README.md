# T038: Command and data policy

What a link changes about control: an uncrewed vessel is controllable only with a link or with its own avionics running a plan it was given; a crewed one has autonomy. Burn plans are uploaded and arrive late.

## Scope

- A comms window with link margin and the next acquisition and loss.
- Warp can stop at loss and acquisition.

## Done when

- An uncrewed test vessel cannot be commanded without a link, and a plan uploaded to it arrives after the light delay.

## Done (2026-11-02)

- `Vessel::commandable`: a crew, or a link. Without it (and with a radio) the throttle, the stage key and steering by hand do not answer. A vessel with no radio and no crew flies as it always has (T087).
- `Vessel::upload_burn`: a crew takes a burn at once; otherwise it joins the plan after the link's delay, and without a link it does not go. A vessel keeps flying the plan it was given.
- The comms window (the launcher's Communications button): station, margin, delay, whether commands are answered.
- `Universe.warp_stops_at_links` makes `advance_rails` stop at the focused vessel's next link change. Nothing in the game's UI sets it yet.

## Still open

- The window looks an hour ahead every five seconds for the next change, on a coast. No game code uses `upload_burn` yet: burns the player plans still go straight into the plan, which needs a look at the flight planning UI.

## Where it comes from

- docs/systems-plan.md (step 13)
