#!/usr/bin/env python3
"""Checks and queries the task tracker in this folder. Standard library only.

    python3 .tasks/tasks.py check            validate every task; exit 1 on an error
    python3 .tasks/tasks.py list [--status todo] [--kind feature] [--parent T002] [--area editor]
    python3 .tasks/tasks.py ready            tasks you can start: todo, every dependency done
    python3 .tasks/tasks.py blocked          todo tasks and what they wait for
    python3 .tasks/tasks.py set T026 done    change a status (sets `updated`, and `done` for done)
    python3 .tasks/tasks.py next [N]         the N (default 10) ready tasks with the lowest `priority`
    python3 .tasks/tasks.py plan [M3]        every milestone (or one) with its open tasks in priority order
    python3 .tasks/tasks.py serve [PORT]     the task viewer in the browser (default port 8765)
    python3 .tasks/tasks.py new-id           the id a new task takes (never one used before)
    python3 .tasks/tasks.py prune            remove done and dropped tasks; listed in removed.txt

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
# Where each task sits in the plan, in order; what each delivers is in README.md.
MILESTONES = ["M0", "M1", "M2", "M3", "M4", "M5", "M6", "M7", "M8", "later", "shelved"]


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


REMOVED = os.path.join(HERE, "removed.txt")


def load_removed():
    """Finished tasks whose folders were pruned: id -> (status, done date, title)."""
    removed = {}
    if os.path.isfile(REMOVED):
        with open(REMOVED) as f:
            for line in f:
                if line.startswith("T"):
                    parts = line.rstrip("\n").split(None, 3)
                    removed[parts[0]] = tuple(parts[1:] + [""] * (4 - len(parts)))
    return removed


def check(tasks):
    errors, warnings = [], []
    removed = load_removed()
    for id in sorted(set(tasks) & set(removed)):
        errors.append("%s: in removed.txt but still has a folder" % tasks[id]["_folder"])
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
            if parent in removed:
                errors.append("%s: parent `%s` is finished and removed" % (where, parent))
            elif parent not in tasks:
                errors.append("%s: parent `%s` does not exist" % (where, parent))
            elif tasks[parent]["kind"] != "epic":
                errors.append("%s: parent `%s` is not an epic" % (where, parent))
        for d in t.get("depends_on", []):
            if d in removed:
                continue  # finished and pruned: satisfied
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
        ms = t.get("milestone")
        if ms is not None and ms not in MILESTONES:
            errors.append("%s: milestone `%s` is not one of %s" % (where, ms, MILESTONES))
        if ms is None and t.get("status") not in ("done", "dropped"):
            warnings.append("%s: no milestone" % where)
        if t.get("priority") is not None:
            if ms is None:
                errors.append("%s: has a priority but no milestone" % where)
            for d in t.get("depends_on", []):
                dp = tasks.get(d, {}).get("priority")
                if dp is not None and dp >= t["priority"]:
                    errors.append("%s: priority %d is not after its dependency %s (%d)" % (where, t["priority"], d, dp))
                elif dp is not None and MILESTONES.index(tasks[d]["milestone"]) > MILESTONES.index(ms):
                    errors.append("%s: milestone %s is before its dependency %s (%s)" % (where, ms, d, tasks[d]["milestone"]))
    prios = [t["priority"] for t in tasks.values() if t.get("priority") is not None]
    if len(prios) != len(set(prios)):
        errors.append("priorities are not unique")
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
    # a dependency with no folder was pruned once finished (removed.txt)
    return d not in tasks or tasks[d]["status"] in ("done", "dropped")


def cmd_ready(tasks):
    for id in sorted(tasks):
        t = tasks[id]
        if t["kind"] != "epic" and t["status"] == "todo" and all(finished(tasks, d) for d in t["depends_on"]):
            print(row(t))


def ready_ids(tasks):
    return {i for i, t in tasks.items() if t["kind"] != "epic" and t["status"] == "todo"
            and all(finished(tasks, d) for d in t["depends_on"])}


def by_priority(tasks, ids):
    return sorted(ids, key=lambda i: (tasks[i].get("priority") is None, tasks[i].get("priority") or 0, i))


def cmd_next(tasks, args):
    n = int(args[0]) if args else 10
    ready = [i for i in by_priority(tasks, ready_ids(tasks)) if tasks[i].get("priority") is not None]
    # a started task comes before a new one
    started = [i for i in by_priority(tasks, tasks) if tasks[i]["status"] == "in_progress"]
    for i in (started + ready)[:n]:
        print("%-3s %s" % (tasks[i].get("milestone", "-"), row(tasks[i])))


def cmd_plan(tasks, args):
    ready = ready_ids(tasks)
    for ms in MILESTONES:
        if args and ms != args[0]:
            continue
        ids = [i for i, t in tasks.items() if t.get("milestone") == ms and t["status"] not in ("done", "dropped")]
        if not ids:
            continue
        print("\n== %s (%d open)" % (ms, len([i for i in ids if tasks[i]["kind"] != "epic"])))
        for i in by_priority(tasks, [i for i in ids if tasks[i]["kind"] != "epic"]):
            print(" %s %s" % ("*" if i in ready else " ", row(tasks[i])))
        epics = [i for i in ids if tasks[i]["kind"] == "epic"]
        if epics:
            print("   epics: " + ", ".join("%s %s" % (i, tasks[i]["title"]) for i in sorted(epics)))
    print("\n* = can start now")


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


def cmd_new_id(tasks):
    used = [int(i[1:]) for i in list(tasks) + list(load_removed()) if i[1:].isdigit()]
    print("T%03d" % (max(used, default=0) + 1))


def cmd_prune(tasks):
    """Removes the folders of done and dropped tasks, lists them in removed.txt, and takes them
    out of the remaining tasks' `depends_on` (a finished dependency is satisfied)."""
    import shutil
    gone = sorted(i for i, t in tasks.items() if t["status"] in ("done", "dropped"))
    for i, t in tasks.items():
        if i not in gone and t.get("parent") in gone:
            sys.exit("%s's parent %s is finished but %s is not; finish or re-parent it first"
                     % (i, t["parent"], i))
    if not gone:
        print("nothing to prune")
        return
    new = not os.path.isfile(REMOVED)
    with open(REMOVED, "a") as f:
        if new:
            f.write("# Finished tasks whose folders were removed. Their ids are never reused;\n"
                    "# git history has their READMEs. id, status, date finished, title.\n")
        for i in gone:
            t = tasks[i]
            f.write("%s %s %s %s\n" % (i, t["status"], t.get("done") or t.get("updated") or "-", t["title"]))
    for i, t in tasks.items():
        if i in gone:
            continue
        kept = [d for d in t["depends_on"] if d not in gone]
        if kept != t["depends_on"]:
            t["depends_on"] = kept
            data = {k: v for k, v in t.items() if k != "_folder"}
            with open(os.path.join(HERE, t["_folder"], "task.json"), "w") as f:
                json.dump(data, f, indent=2, ensure_ascii=False)
                f.write("\n")
    for i in gone:
        shutil.rmtree(os.path.join(HERE, tasks[i]["_folder"]))
    print("removed %d finished tasks (listed in removed.txt)" % len(gone))


def viewer_data(tasks):
    """Every task with its README, and what each milestone delivers (from README.md's table)."""
    out = []
    for id in sorted(tasks):
        t = dict(tasks[id])
        t["folder"] = t.pop("_folder")
        try:
            with open(os.path.join(HERE, t["folder"], "README.md")) as f:
                t["readme"] = f.read()
        except OSError:
            t["readme"] = ""
        out.append(t)
    milestones = {}
    with open(os.path.join(HERE, "README.md")) as f:
        for line in f:
            cells = [c.strip() for c in line.strip().strip("|").split("|")]
            if len(cells) == 2 and cells[0].strip("`") in MILESTONES:
                milestones[cells[0].strip("`")] = cells[1]
    return {"tasks": out, "milestones": milestones}


def cmd_serve(args):
    import http.server
    import webbrowser

    port = int(args[0]) if args else 8765
    fonts = os.path.join(os.path.dirname(HERE), "website", "fonts")

    class Handler(http.server.BaseHTTPRequestHandler):
        def do_GET(self):
            path = self.path.split("?")[0].split("#")[0]
            if path == "/tasks.json":
                # read fresh on every request, so a reload shows the folder as it is now
                body, kind = json.dumps(viewer_data(load())).encode(), "application/json"
            elif path in ("/", "/index.html"):
                body, kind = open(os.path.join(HERE, "viewer.html"), "rb").read(), "text/html; charset=utf-8"
            elif path.startswith("/fonts/") and os.path.basename(path) in os.listdir(fonts):
                body, kind = open(os.path.join(fonts, os.path.basename(path)), "rb").read(), "font/woff"
            else:
                self.send_error(404)
                return
            self.send_response(200)
            self.send_header("Content-Type", kind)
            self.send_header("Cache-Control", "no-store")
            self.end_headers()
            self.wfile.write(body)

        def log_message(self, *a):
            pass

    server = http.server.ThreadingHTTPServer(("127.0.0.1", port), Handler)
    url = "http://127.0.0.1:%d/" % port
    print("task viewer at %s (Ctrl-C to stop; reload the page to see changes)" % url)
    if "--no-open" not in sys.argv:
        webbrowser.open(url)
    try:
        server.serve_forever()
    except KeyboardInterrupt:
        pass


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
    elif cmd == "next":
        cmd_next(tasks, args[1:])
    elif cmd == "plan":
        cmd_plan(tasks, args[1:])
    elif cmd == "set":
        cmd_set(tasks, args[1:])
    elif cmd == "serve":
        cmd_serve([a for a in args[1:] if not a.startswith("--")])
    elif cmd == "new-id":
        cmd_new_id(tasks)
    elif cmd == "prune":
        cmd_prune(tasks)
    else:
        print(__doc__)


if __name__ == "__main__":
    main()
