# T033: Environment: sun, eclipse, albedo, occultation

At a vessel's position and time: the direction and strength of sunlight, eclipse by any body (umbra and penumbra), planet albedo and infrared, and whether another point is hidden behind a body. Cached each tick, evented on rails.

## Done when

- The eclipse in the 185 km parking orbit matches the geometry (at most about 35 of 88 minutes).

## Done (2026-11-02)

- `crates/osp_sim/src/environment.rs`: `environment::at` gives the Sun's direction and distance, the share of its disc that shows (umbra, penumbra, by any body), the flux with that share, and the albedo and infrared flux of the body the vessel is near. `BodyDef.albedo` is new, filled in for the bodies whose Bond albedo is well known. `occulted_by` says which body stops a line of sight, for links (T037).
- `next_light_change` finds a coast's shadow edges, and `Universe::advance_rails` stops there for a vessel with a system that `uses_light`; a vessel with none pays nothing for it (`Environment::DARK`).
- The longest eclipse in the 185 km parking orbit comes to 37.3 of 88 minutes, which is the geometry (2 asin(R / r)): the task's "about 35" was a round figure, and any tilt to the Sun shortens it (a test holds both).

## Where it comes from

- docs/systems-plan.md (step 8)
