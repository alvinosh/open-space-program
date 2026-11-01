# T138: Multiplayer

Not planned in detail yet. Do it after the scenarios, the systems and the Moon (milestone M8), because a multiplayer game needs a game first; it is also what the paid server product (T176) depends on.

## A shape that fits

- `osp_sim` has no Bevy dependency, so the same crate can run as a headless authoritative server.
- A shared clock with the server as the authority; time warp only when nobody is in the physics range.
- Make the save format (T090) the network snapshot format and the simulation an input log, so the netcode is a thin layer. Craft are plain data, so sharing one is cheap.
- A mission-control role (a ground-station player with comms and the flight plan, a flyer in the capsule), which the comms work (T037) already points at. A multiplayer mission is a T165 mission with two players.

## Tasks to add when it starts

- The scope; a headless server binary on `osp_sim`; the network protocol (snapshots, commands, vessel authority); a server list; managed hosting. Multi-vessel physics (T088) and the console (T133) are prerequisites or parts.

## Done when

- Decide the scope first.

## Where it comes from

- Early feature checklist (since removed), Backlog
- Planning of 2026-11-01
