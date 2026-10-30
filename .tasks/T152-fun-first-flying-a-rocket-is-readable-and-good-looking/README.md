# T152: Fun first: flying a rocket is readable and looks right

What makes the game worth playing right now, ahead of the feature work. Raised in play on 2026-10-30: half the rockets have no exhaust plume, staging is bad, hot staging is missing, stages cannot be swapped around, the UI is too big and unclear, engines are not highlighted, and solid boosters look like liquid engines.

## Order

1. T153 plumes for every engine (557 of the 899 realism engine configs have none; the F-1 is one of them). Cheapest and the most visible.
2. T156 flight HUD smaller, fewer windows open at the start.
3. T155 staging window you can read (names, kinds, what lights next), then T160 to make it compact. Designed together, built one at a time.
4. T157 hover or select a stage icon and its parts glow on the rocket.
5. T158 reordering stages by hand that works and shows where it will land.
6. T159 hot staging.
7. T154 solid boosters that look and burn like solids.

T153 and T156 first because they need no new design. T155, T157 and T158 are one UI and go well together, but build one at a time. T159 depends on the staging window being clear enough to show a delay.

## Also relevant, not part of this epic

- T060 (staging window leftovers): T158 covers its drag and drop and reordering items, the rest stays there.
- T118 (plumes: refraction, trails, pad smoke): polish on top of T153, not the missing plumes.
- T026, T029: cutoffs and generic actions in sequences; T159 needs only a small part of that and should not wait for it.
- T084, T128: in-flight engine readouts and the HUD scale setting; T156 takes the defaults, not these.
