# T235: Delta-v and TWR in the vessel summary

The vessel summary shows height, mass and part count. To know whether a craft will do the job a
player needs delta-v per stage and in all, and the thrust to weight at liftoff; those show only for
the stage picked in the staging window.

## Scope

- Total delta-v and liftoff TWR in the summary, and each stage's delta-v and TWR on its row in the
  staging window.
- In a scenario, what it needs beside what the craft has (orbit needs about 9.4 km/s; with T237).

## Done when

- The summary and the staging rows show them, and they match what the flight's staging shows.

## Where it comes from

From the editor review of 2026-11-07: the user asked for everything wrong with the editor from a UX point of view, then for each finding to be filed as a task. The epic, T221, lists them all.

## 2026-11-21

Rebuilding the stock craft in the builder (`rebuild_*` playtests): the Titan, Talon, Heron and
Colossus, built with the original stages' outer lengths typed into their tanks, sat on the pad at a
thrust to weight of 0.95, 0.80, 0.71 and 0.69. Nothing in the editor said so before Launch.
