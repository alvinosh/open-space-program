# T309: More guesses while building

The second round of "more intelligent building" (the user, 2026-11-13), after T306 sized new parts
for what they go on. Each is a guess the editor makes once, when a new part goes on; nothing already
on the craft changes size by itself.

## Ideas, to pick from with the user

- A tank put under a pod or a tank, built top down, takes a length for the stage it starts: if the
  craft already has engines, enough for them to lift what is above at a sensible TWR; if not, shorter
  than 3x its width (a C-1 Capsule's 1.9 m tank is 5.7 m long today, 15 t that the Casual first
  engines cannot lift).
- A pod or engine put on a node much narrower or wider than itself gets a tube as an adapter between
  them, offered in the hint and put on with one key.
- An engine put on a tank's end with symmetry picks the count and radius that fits the end.
- A nose cone on a booster is as long for its width as the core's nose.

## Done when

- The ones picked are built, each with a test and in a capture.

## Seen in the playtests (2026-11-16)

`first_rocket` (capsule, then tank, then a K-1 Booster, top down as most players build) lifts off at
a thrust to weight of 4.9, rising to 29 at burnout, with the High g warning on and 90 kPa of dynamic
pressure (three times a real rocket's max-Q). The same rocket with the tank T306 would give an engine
put on first (2.8 m x 8.1 m) lifts off at 1.9 and peaks at 37 kPa. That is the first idea above;
still to pick with the user.

## Picked and built (2026-11-16)

The user picked the first idea ("Sure"). `fitting::size_for_engines`, run when parts are put on
(`Builder::put_set`): a cylinder tank still the size the parts list gave it where it is
(`as_given`: as `fit` makes a new one on that node, or the list's own size as the root), with an
engine just put under it by the engine's front, is sized as T306 sizes a tank put on an engine:
150 s of a sea-level engine's burn or 350 s of a vacuum one's, but no more than leaves the engine
lifting all the craft at a thrust to weight of 1.4 (sea level) or 0.6 (vacuum, weighed at the
surface), and within a career's size limits. It keeps its width when something stands on it. A tank
the player sized stays as it is, and so does one already as heavy as the engine can lift.

- It is not previewed while the engine is in hand: grown there, it would move the ring the engine is
  held to (the preview went round in a loop). It grows when the engine is let go, and the editor now
  redraws whenever what was put on differs from what the preview showed.
- `first_rocket`: the tank grows from 5.7 m to 16.6 m, liftoff at a thrust to weight of 1.73, all
  its hopes met (`Until("editor.height > 15")` checks the growing). `two_stages`: the upper tank
  grows as the whole rocket's would; the lower K-1 cannot lift a bigger lower tank, so that one
  stays as given (liftoff at 1.28); the second stage burns out at 7.5 km/s.
- Not done: engines put under a tank by their side with symmetry (T300's clusters) do not size it.

The other ideas above were not picked.
