# Tasks

Everything that is planned or open for this project, one folder per task. It started as the
plans in `docs/build-editor-plan.md` and `docs/systems-plan.md`, and took over an early
feature checklist, the issue list (`docs/issues.md`) and the
carry-over notes of `docs/vessel-overhaul.md`, which are gone as lists (git history has them).
From now on this folder is the place to see what is done, what is next and what depends on what.
The plan documents say why and how; the tasks say what is left.

```
.tasks/
  README.md                  this file
  tasks.py                   checks and queries (standard library only)
  viewer.html                the browser viewer `tasks.py serve` shows
  T013-build-smoke-tests-first-version-placement-matrix/
    task.json                the data a tool reads
    README.md                what the task is, its scope, and when it is done
```

## task.json

| Field | Meaning |
| --- | --- |
| `id` | `T` and three digits, never reused. The folder is `<id>-<slug>`. |
| `title` | One line. |
| `kind` | `epic` (groups tasks, has no work of its own), `feature`, `bug`, `chore`, `test` (a smoke or build test to write), `milestone` (a gate that passes when other tasks are done), `decision` (a choice that blocks others; done when it is written down). |
| `status` | `todo`, `in_progress`, `blocked`, `done`, `dropped`, `backlog` (an idea nobody has planned; `ready` leaves it out). |
| `area` | The part of the game: `editor`, `builder`, `physics`, `propulsion`, `power`, `avionics`, `comms`, `docking`, `landing`, `thermal`, `recovery`, `crew`, `structure`, `environment`, `content`, `aero`, `geometry`, `sequences`, `orbit`, `rendering`, `audio`, `ui`, `modding`, `process`, `science`, `resources`, `test`. |
| `size` | Relative effort: `S` (days), `M` (a week or two), `L` (several weeks), `XL` (months); `null` for an epic. |
| `parent` | The epic it belongs to, or `null`. Grouping only. |
| `depends_on` | Ids of tasks that must be done before this one can start. This is the graph. |
| `refs` | Where the plan, the code or the old lists said more. |
| `created`, `updated` | `YYYY-MM-DD`. |
| `done` | The date it was finished, `null` until then. |
| `milestone` | Where it sits in the plan: `M0` to `M8`, `later` or `shelved` (below). Every open task has one. |
| `priority` | A unique number across the open tasks, lower first. Never before one of its dependencies; `check` enforces it. Epics have a milestone but no priority. |

What `depends_on` means: an edge is there when the task needs the other to exist, or when the plan
says it must not start before it (the systems tasks wait for the editor's gate, T025, for that
reason). The README of a task says which, where it is not obvious. What a task blocks is not
stored; it is every task that lists it, which the tool or a viewer works out.

`parent` and `depends_on` are different on purpose. A viewer can draw the epics as groups and the
dependencies as arrows.

Most of the imported checklist tasks have no `depends_on` yet: they were never planned in detail. Add
the edges when one of them is planned.

## Working with it

```sh
python3 .tasks/tasks.py check          # ids, folders, statuses, missing or circular dependencies
python3 .tasks/tasks.py ready          # what can be started now
python3 .tasks/tasks.py blocked        # what waits for what
python3 .tasks/tasks.py list --status todo --area editor
python3 .tasks/tasks.py set T026 done  # status, `updated`, and `done` together
python3 .tasks/tasks.py next           # the ready tasks with the lowest priority (started ones first)
python3 .tasks/tasks.py plan M3        # a milestone's open tasks in order; without M3, all of them
python3 .tasks/tasks.py serve          # the viewer in the browser: board, milestones, list, each task's README
```

The viewer (`viewer.html`, served on http://127.0.0.1:8765) reads the folders again on every reload.
Click a task to see its README, what it depends on and what it blocks; `/` searches.

To add a task: make a folder `T0nn-short-slug` with the next free id, copy a `task.json` and a
`README.md` from a similar task, set `depends_on` honestly, and run `check`. A task is done when
its "Done when" list is true, its tests pass, `docs/modding.md` describes anything a modder sees,
and the smoke test finding it came from now passes (not "known" or "not modelled"; a known finding
names its task by id). Then `set <id> done`. A bug found in play is a task of kind `bug`.

Work one task at a time, as the rest of the project does: `ready` lists the candidates.

## The plan: milestones

The goal for now is a complete core game, not more simulation depth: fly well, hear it, be taught,
build, save, keep the ship alive, and scenarios worth playing, each with a tutorial and a craft that
is fun to fly. The crew are people. The game stays free (T176). `plan` shows the order; the
dependency graph decides where a milestone cannot start earlier.

| Milestone | What it delivers |
| --- | --- |
| `M0` | Decisions and licences, in days: contact solver, scripting, real Solar System only, how the project earns, licence files. |
| `M1` | Flying feels good: the in-progress work, the visible bugs, sound, and menus that make sense (T185). T189 moved to M3 on 2026-11-04, when the Apollo work went first. |
| `M2` | Ship parts and systems, before the editor: the build core (physics on any part, open components, sizers, surface layers, environment), decouplers and separation motors, batteries, solar arrays, fuel cells, avionics, antennas, ground stations, link policy. Craft files can hold the new parts before a player can place them. |
| `M3` | Build and save: the editor chain to its gate (T025), save and load, the start menu. Since 2026-11-10 also ships that live (T260): life support, cabins, radiation, failures, science, resources and automation, each with a Casual and a Realistic setting. |
| `M4` | Flight planning, then the first scenarios and their tutorials: first launch, rendezvous, the Mars orbiter probe, the build tutorial. Missions and the tutorial layer (T165, T166) are done. |
| `M5` | The Moon flyby: the systems tutorial, the crewed flyby (life support went to T260 in M3 on 2026-11-10). Heat, entry and parachutes went to M2 for the Apollo gate (T180) on 2026-11-04. |
| `M6` | Landings: contact physics, docking, the Moon's terrain, the lander, Mars relief, the Moon landing and the Mars lander. |
| `M7` | Polish and depth: trailer-grade visuals, the Saturn V from procedural parts only. Pull a task forward if a scenario needs it. |
| `M8` | Together: multiplayer, and the paid servers that depend on it. |
| `later` | Content drops and breadth, in the order they unblock each other. |
| `shelved` | Not dropped, not for 1.0: the XL realism-depth tasks (thermodynamic engine model, interstellar, ray-traced IVA, EVA, scripting). Revisit when players ask. |

The systems come before the editor (decided 2026-11-01): T028, T031 and T033 wait for the build core
(T014), not for the Saturn V gate (T025), which now only gates the editor chain and T052. One plan rule
is still an open question, written in T049: life support waits for joining vessels.

## What is in it

- The build editor and the Saturn V built in it (T013 to T025), and the systems that follow (T026
  to T059 and what they need), from `docs/build-editor-plan.md` and `docs/systems-plan.md`.
- The staging window leftovers (T060), the open parts of the old issue list, and the carry-over
  notes of the vessel overhaul (the `Vessel overhaul leftovers` epic).
- Every unchecked item of the early feature checklist, one task each (grouped by its phase), or a
  line in the task that already covers it: the editor, the parachutes, docking ports, oceans,
  crash tolerance and electricity are in the plans' tasks. Items already done were not imported.
  The three "decisions to make" are `decision` tasks. Ideas nobody has planned are
  `backlog`.
