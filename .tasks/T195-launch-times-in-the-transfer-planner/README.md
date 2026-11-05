# T195: Launch times in the transfer planner

The planner refused to do anything on the ground, but the launch time decides the plane of the parking orbit, and that decides how soon a cheap transfer comes.

## Scope

- From the ground, Compute lists when to launch for a target around the same body, and which way to fly: leaving now, and the best times to come.
- Warp to a launch time.

## Done when

- A launch at a time and heading it gives leads to a cheap transfer soon after reaching orbit, checked with the planner's own search.

## Done

- `osp_sim::transfer::LaunchRequest`. A far target (past 5 parking radii) is reached where it will be: any plane through the site and that point will do, and the best times are those flown most nearly due east. A near target is matched in its own plane, when the site passes under it or comes closest.
- The Cape never passes under the Moon's plane: the Moon's orbit is 20.9 deg from Earth's equator in the base game and the Cape is at 28.6 deg, which is why the first kind is needed.
- Measured for the Moon, cheapest transfer within two orbits of reaching orbit: about 3,950 m/s from a best time; 3,950 to 4,200 m/s from the heading given at any hour of the day; up to 9,900 m/s flown due east at the wrong hour. The heading matters more than the hour, and leaving off a best time costs what the rotation no longer gives (`extra`): under 30 m/s two hours off, about 400 m/s near polar.
- `OSP_SCENARIO=pad OSP_CAPTURE_LAUNCH_TIMES=1` targets the Moon, warps to the first time and lifts off. The orbits in the tests are ideal parking orbits: no ascent was flown from one to a transfer yet.

## Left out

- Flight time is taken as the Hohmann time, about 5 days to the Moon. A faster flight wants a launch time up to a couple of hours off.
- From the ground, a target around another body (another planet, or Earth from the Moon) still says to get into orbit first.
