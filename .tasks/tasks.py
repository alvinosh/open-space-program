#!/usr/bin/env python3
"""Checks and queries the task tracker in this folder. Standard library only.

    python3 .tasks/tasks.py check            validate every task; exit 1 on an error
    python3 .tasks/tasks.py list [--status todo] [--kind feature] [--parent T002] [--area editor]
    python3 .tasks/tasks.py ready            tasks you can start: todo, every dependency done
    python3 .tasks/tasks.py blocked          todo tasks and what they wait for
    python3 .tasks/tasks.py set T026 done    change a status (sets `updated`, and `done` for done)

See .tasks/README.md for the format.
"""
import datetime
import json
import os
import sys

HERE = os.path.dirname(os.path.abspath(__file__))
STATUSES = ["todo", "in_progress", "blocked", "done", "dropped", "backlog"]
KINDS = ["epic", "feature", "bug", "chore", "test", "milestone", "decision"]
SIZES = ["S", "M", "L", "XL", None]
REQUIRED = ["id", "title", "kind", "status", "area", "size", "parent", "depends_on", "refs",
            "created", "updated", "done"]


def load():
    tasks = {}
    folders = {}
    for name in sorted(os.listdir(HERE)):
        path = os.path.join(HERE, name, "task.json")
        if not os.path.isfile(path):
            continue
        with open(path) as f:
            data = json.load(f)
        data["_folder"] = name
        tasks[data.get("id", name)] = data
        folders[name] = data
    return tasks


def check(tasks):
    errors, warnings = [], []
    for id, t in tasks.items():
        where = t["_folder"]
        for key in REQUIRED:
            if key not in t:
                errors.append("%s: missing `%s`" % (where, key))
        if t.get("id") != where.split("-")[0]:
            errors.append("%s: id `%s` does not match the folder" % (where, t.get("id")))
        if t.get("kind") not in KINDS:
            errors.append("%s: kind `%s` is not one of %s" % (where, t.get("kind"), KINDS))
        if t.get("status") not in STATUSES:
            errors.append("%s: status `%s` is not one of %s" % (where, t.get("status"), STATUSES))
        if t.get("size") not in SIZES:
            errors.append("%s: size `%s` is not one of S, M, L, XL or null" % (where, t.get("size")))
        if not os.path.isfile(os.path.join(HERE, where, "README.md")):
            errors.append("%s: no README.md" % where)
        parent = t.get("parent")
        if parent is not None:
            if parent not in tasks:
                errors.append("%s: parent `%s` does not exist" % (where, parent))
            elif tasks[parent]["kind"] != "epic":
                errors.append("%s: parent `%s` is not an epic" % (where, parent))
        for d in t.get("depends_on", []):
            if d not in tasks:
                errors.append("%s: depends on `%s`, which does not exist" % (where, d))
            elif d == id:
                errors.append("%s: depends on itself" % where)
            elif t.get("status") == "done" and tasks[d]["status"] not in ("done", "dropped"):
                warnings.append("%s: done, but `%s` is not" % (where, d))
        if t.get("kind") == "epic" and t.get("depends_on"):
            warnings.append("%s: an epic groups tasks; put dependencies on its tasks" % where)
        if (t.get("status") == "done") != (t.get("done") is not None):
            errors.append("%s: `done` must be a date exactly when the status is done" % where)
    # cycles
    state = {}

    def visit(id, path):
        if state.get(id) == 2:
            return
        if state.get(id) == 1:
            errors.append("dependency cycle: " + " -> ".join(path + [id]))
            return
        state[id] = 1
        for d in tasks[id].get("depends_on", []):
            if d in tasks:
                visit(d, path + [id])
        state[id] = 2

    for id in tasks:
        visit(id, [])
    # epics
    for id, t in tasks.items():
        if t["kind"] == "epic" and t["status"] == "done":
            open_ = [c for c, ct in tasks.items() if ct.get("parent") == id and ct["status"] not in ("done", "dropped")]
            if open_:
                warnings.append("%s: epic is done but %s are not" % (t["_folder"], ", ".join(open_)))
    return errors, warnings


def row(t):
    return "%-5s %-11s %-9s %-4s %s" % (t["id"], t["status"], t["kind"], t["size"] or "-", t["title"])


def cmd_list(tasks, args):
    flt = {}
    i = 0
    while i < len(args):
        if args[i].startswith("--") and i + 1 < len(args):
            flt[args[i][2:]] = args[i + 1]
            i += 2
        else:
            i += 1
    for id in sorted(tasks):
        t = tasks[id]
        if all(str(t.get(k)) == v for k, v in flt.items()):
            print(row(t))


def finished(tasks, d):
    return tasks[d]["status"] in ("done", "dropped")


def cmd_ready(tasks):
    for id in sorted(tasks):
        t = tasks[id]
        if t["kind"] != "epic" and t["status"] == "todo" and all(finished(tasks, d) for d in t["depends_on"]):
            print(row(t))


def cmd_blocked(tasks):
    for id in sorted(tasks):
        t = tasks[id]
        if t["kind"] == "epic" or t["status"] not in ("todo", "blocked"):
            continue
        waiting = [d for d in t["depends_on"] if not finished(tasks, d)]
        if waiting:
            print("%s  waits for %s" % (row(t), ", ".join(waiting)))


def cmd_set(tasks, args):
    if len(args) != 2 or args[1] not in STATUSES or args[0] not in tasks:
        sys.exit("usage: tasks.py set <id> <%s>" % "|".join(STATUSES))
    t = tasks[args[0]]
    t["status"] = args[1]
    today = datetime.date.today().isoformat()
    t["updated"] = today
    t["done"] = today if args[1] == "done" else None
    folder = t.pop("_folder")
    with open(os.path.join(HERE, folder, "task.json"), "w") as f:
        json.dump(t, f, indent=2, ensure_ascii=False)
        f.write("\n")
    print("%s is now %s" % (args[0], args[1]))


def main():
    args = sys.argv[1:]
    if not args:
        print(__doc__)
        return
    tasks = load()
    cmd = args[0]
    if cmd == "check":
        errors, warnings = check(tasks)
        for w in warnings:
            print("warning:", w)
        for e in errors:
            print("error:", e)
        counts = {}
        for t in tasks.values():
            counts[t["status"]] = counts.get(t["status"], 0) + 1
        print("%d tasks: %s" % (len(tasks), ", ".join("%d %s" % (n, s) for s, n in sorted(counts.items()))))
        sys.exit(1 if errors else 0)
    elif cmd == "list":
        cmd_list(tasks, args[1:])
    elif cmd == "ready":
        cmd_ready(tasks)
    elif cmd == "blocked":
        cmd_blocked(tasks)
    elif cmd == "set":
        cmd_set(tasks, args[1:])
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
