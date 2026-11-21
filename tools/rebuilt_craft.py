#!/usr/bin/env python3
"""Turns what a rebuild playtest launched into the craft file the game ships.

Each stock craft is built in the builder by a playtest (`rebuild_<id>` in
crates/osp_game/src/playtest/rebuilds.rs), as a player builds it from the parts list. The
playtest writes the craft it launched to target/playtest/rebuild_<id>/craft.json; this names it
and writes it to assets/core/craft/<id>.json5. Standard library only.

    OSP_PLAYTESTS=rebuild_sounder cargo playtest
    python3 tools/rebuilt_craft.py sounder "Sounder"
"""
import json
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent


def main():
    if len(sys.argv) != 3:
        sys.exit(__doc__)
    craft_id, name = sys.argv[1], sys.argv[2]
    out = ROOT / "target" / "playtest" / f"rebuild_{craft_id}"
    craft = json.loads((out / "craft.json").read_text())
    report = json.loads((out / "report.json").read_text())
    craft["id"] = craft_id
    craft["name"] = name
    head = [
        f"// {report['about']}",
        f"// Built in the builder by the playtest rebuild_{craft_id}",
        "// (crates/osp_game/src/playtest/rebuilds.rs) and written by tools/rebuilt_craft.py: change",
        "// the playtest and build it again rather than this file.",
    ]
    path = ROOT / "assets" / "core" / "craft" / f"{craft_id}.json5"
    path.write_text("\n".join(head) + "\n" + json.dumps(craft, indent=2) + "\n")
    print(f"{path.relative_to(ROOT)}: {len(craft['parts'])} parts")


if __name__ == "__main__":
    main()
