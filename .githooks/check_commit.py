#!/usr/bin/env python3
"""Checks that a commit message says what the commit is and what it touches.

    type(scope): subject
    type(scope,scope): subject

The type is what kind of change it is. The scopes are the areas of the repo the commit
touches (SCOPES below): every area with a changed file has to be named, and an area that is
named has to have a changed file. So `fix(sim): ...` cannot hide a change in the game crate.

Run by the commit-msg hook (`git config core.hooksPath .githooks`), and by CI over the
commits of a push or pull request (`--range BASE HEAD`). Standard library only.
"""
import os
import re
import subprocess
import sys

TYPES = ("feat", "fix", "perf", "refactor", "docs", "test", "chore", "build", "ci", "style", "revert")

# Scope -> the paths it covers: a directory (ends with "/") or one file. The longest match wins.
SCOPES = {
    "sim": ["crates/osp_sim/"],
    "mods": ["crates/osp_mods/"],
    "terrain": ["crates/osp_terrain/"],
    "bake": ["crates/osp_bake/"],
    "ui": ["crates/osp_ui/"],
    "game": ["crates/osp_game/"],
    "assets": ["assets/"],
    "example-mod": ["mods/"],
    "tools": ["tools/"],
    "docs": ["docs/", "README.md"],
    "tasks": [".tasks/"],
    "website": ["website/"],
    "ci": [".github/"],
    "build": ["Cargo.toml", "rust-toolchain.toml", ".cargo/"],
    "repo": [".gitignore", ".githooks/"],
}

# Changes that follow from another change; they never need a scope of their own.
INCIDENTAL = {"Cargo.lock"}

EMPTY_TREE = "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
MAX_HEADER = 72
MIN_SUBJECT = 8
HEADER = re.compile(r"^(?P<type>[a-z]+)\((?P<scopes>[^)]*)\)(?P<bang>!)?: (?P<subject>.*)$")

EXAMPLE = "feat(sim,game): add hot staging"


def check_message(message, allow_fixup=False):
    """Returns (scopes, errors) for the header and body of a message."""
    lines = message.split("\n")
    header = lines[0]
    if header.startswith(("Merge ", 'Revert "')):
        return None, []
    if allow_fixup and header.startswith(("fixup! ", "squash! ", "amend! ")):
        return None, []
    m = HEADER.match(header)
    if not m:
        return [], [f"the first line must look like `type(scope): subject`, e.g. `{EXAMPLE}`"]
    errors = []
    if m["type"] not in TYPES:
        errors.append(f"unknown type `{m['type']}` (use one of: {', '.join(TYPES)})")
    scopes = [s for s in m["scopes"].split(",")]
    if any(s == "" for s in scopes):
        errors.append("name at least one scope, separated by commas without spaces: `fix(sim,ui): ...`")
        scopes = [s for s in scopes if s]
    if len(set(scopes)) != len(scopes):
        errors.append("a scope is named twice")
    for s in scopes:
        if s not in SCOPES:
            errors.append(f"unknown scope `{s}` (use: {', '.join(SCOPES)})")
    subject = m["subject"]
    if len(header) > MAX_HEADER:
        errors.append(f"the first line is {len(header)} characters, the limit is {MAX_HEADER}")
    if len(subject) < MIN_SUBJECT:
        errors.append("the subject is too short to say what changed")
    if subject.endswith("."):
        errors.append("the subject does not end with a full stop")
    if not re.match(r"[a-z0-9`]", subject):
        errors.append("the subject starts with a lower-case letter or a digit")
    if len(lines) > 1 and lines[1].strip() != "":
        errors.append("leave a blank line between the first line and the body")
    return scopes, errors


def scope_of(path):
    """The scope that covers a path, "incidental" for follow-on files, or None."""
    if path in INCIDENTAL:
        return "incidental"
    best, best_len = None, -1
    for scope, prefixes in SCOPES.items():
        for p in prefixes:
            hit = path.startswith(p) if p.endswith("/") else path == p
            if hit and len(p) > best_len:
                best, best_len = scope, len(p)
    return best


def check_files(scopes, files):
    """Errors for scopes that do not match the changed files."""
    touched = {}
    errors = []
    for f in files:
        s = scope_of(f)
        if s is None:
            errors.append(f"no scope covers `{f}`; add its directory to SCOPES in .githooks/check_commit.py")
        elif s != "incidental":
            touched.setdefault(s, []).append(f)
    if errors or not touched:
        return errors
    for s, fs in touched.items():
        if s not in scopes:
            more = f" and {len(fs) - 1} more" if len(fs) > 1 else ""
            errors.append(f"the commit changes `{fs[0]}`{more}, which is scope `{s}`; name it or leave those changes out")
    for s in scopes:
        if s in SCOPES and s not in touched:
            errors.append(f"scope `{s}` is named but no file in it changes")
    return errors


def git(*args):
    r = subprocess.run(["git", *args], capture_output=True, text=True)
    return r.returncode, r.stdout


def staged_files():
    """The files the commit being made changes, or None when that cannot be told."""
    rc, _ = git("rev-parse", "--verify", "-q", "HEAD")
    if rc != 0:
        return None  # the first commit touches everything
    base = "HEAD"
    if "--amend" in parent_command():
        rc, _ = git("rev-parse", "--verify", "-q", "HEAD~1")
        if rc != 0:
            return None
        base = "HEAD~1"
    _, out = git("diff", "--cached", "--name-only", "--no-renames", "-z", base)
    return [f for f in out.split("\0") if f]


def parent_command():
    try:
        r = subprocess.run(["ps", "-o", "command=", "-p", str(os.getppid())], capture_output=True, text=True)
        return r.stdout
    except OSError:
        return ""


def report(header, errors):
    print(f"\ncommit message rejected: {header!r}", file=sys.stderr)
    for e in errors:
        print(f"  - {e}", file=sys.stderr)
    print(f"\nFormat: type(scope): subject, e.g. `{EXAMPLE}`", file=sys.stderr)
    print(f"Types:  {', '.join(TYPES)}", file=sys.stderr)
    print(f"Scopes: {', '.join(SCOPES)}\n", file=sys.stderr)


def run_hook(path):
    with open(path, encoding="utf-8") as f:
        text = "".join(l for l in f if not l.startswith("#"))
    message = text.strip("\n")
    scopes, errors = check_message(message, allow_fixup=True)
    if scopes is not None and not errors:
        files = staged_files()
        if files:
            errors = check_files(scopes, files)
    if errors:
        report(message.split("\n")[0], errors)
        return 1
    return 0


def run_range(base, head):
    """CI: every commit in base..head; just head when base is unknown (a new branch, a rewrite)."""
    ok = base and set(base) != {"0"} and git("cat-file", "-e", base + "^{commit}")[0] == 0
    revs = git("rev-list", "--no-merges", "--reverse", f"{base}..{head}")[1].split() if ok else [head]
    failed = 0
    for rev in revs:
        message = git("show", "-s", "--format=%B", rev)[1].strip("\n")
        scopes, errors = check_message(message)
        if scopes is not None and not errors:
            files = git("diff-tree", "-r", "--root", "--no-commit-id", "--no-renames", "--name-only", "-z", rev)[1]
            files = [f for f in files.split("\0") if f]
            if git("rev-parse", "--verify", "-q", rev + "^")[0] == 0:
                errors = check_files(scopes, files)
        if errors:
            print(f"{rev[:10]} {message.split(chr(10))[0]!r}", file=sys.stderr)
            for e in errors:
                print(f"  - {e}", file=sys.stderr)
            failed += 1
    print(f"checked {len(revs)} commit(s), {failed} rejected")
    return 1 if failed else 0


def main(argv):
    if len(argv) == 3 and argv[1] == "--hook":
        return run_hook(argv[2])
    if len(argv) == 4 and argv[1] == "--range":
        return run_range(argv[2], argv[3])
    print("usage: check_commit.py --hook MESSAGE_FILE | --range BASE HEAD", file=sys.stderr)
    return 2


if __name__ == "__main__":
    sys.exit(main(sys.argv))
