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
| `area` | The part of the game: `editor`, `builder`, `physics`, `propulsion`, `power`, `avionics`, `comms`, `docking`, `landing`, `thermal`, `recovery`, `crew`, `structure`, `environment`, `content`, `aero`, `geometry`, `sequences`, `orbit`, `rendering`, `audio`, `ui`, `modding`, `process`, `test`. |
| `size` | Relative effort: `S` (days), `M` (a week or two), `L` (several weeks), `XL` (months); `null` for an epic. |
| `parent` | The epic it belongs to, or `null`. Grouping only. |
| `depends_on` | Ids of tasks that must be done before this one can start. This is the graph. |
| `refs` | Where the plan, the code or the old lists said more. |
| `created`, `updated` | `YYYY-MM-DD`. |
| `done` | The date it was finished, `null` until then. |

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
```

To add a task: make a folder `T0nn-short-slug` with the next free id, copy a `task.json` and a
`README.md` from a similar task, set `depends_on` honestly, and run `check`. A task is done when
its "Done when" list is true, its tests pass, `docs/modding.md` describes anything a modder sees,
and the smoke test finding it came from now passes (not "known" or "not modelled"; a known finding
names its task by id). Then `set <id> done`. A bug found in play is a task of kind `bug`.

Work one task at a time, as the rest of the project does: `ready` lists the candidates.

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
