"""Convert the ten selected KSP MU engine models to Bevy-compatible GLB files.

Run with Blender 3.x/4.x/5.x:
    blender -b --factory-startup --python tools/blender/convert_imported_mu.py
"""

from __future__ import annotations

import sys
from pathlib import Path

import bpy


ROOT = Path(__file__).resolve().parents[2]
ADDON = ROOT / "tools" / "blender" / "vendor"
SOURCE = ROOT / "assets" / "core" / "imported" / "source_models"
OUTPUT = ROOT / "assets" / "core" / "models" / "engines" / "imported"
MODEL_IDS = [
    "bluedog_AgenaA",
    "bluedog_AgenaD",
    "bluedog_AtlasV_RD180",
    "bluedog_Delta2_RS27",
    "bluedog_DeltaK_AJ10",
    "bluedog_LR87_mod1",
    "bluedog_LR91_mod1",
    "bluedog_Titan_TranstageEngine",
    "bluedog_atlasBooster",
    "bluedog_atlasSustainer",
]


def clear_scene() -> None:
    bpy.ops.object.select_all(action="SELECT")
    bpy.ops.object.delete(use_global=False)
    for collection in list(bpy.data.collections):
        if collection.name != "Collection":
            bpy.data.collections.remove(collection)


def convert(model_id: str) -> None:
    source = SOURCE / f"{model_id}.mu"
    output = OUTPUT / f"{model_id}.glb"
    if not source.is_file():
        raise FileNotFoundError(source)
    clear_scene()
    from io_object_mu.import_mu import import_mu

    collection = bpy.context.scene.collection
    import_mu(collection, str(source), True, False, False)
    meshes = [obj for obj in bpy.context.scene.objects if obj.type == "MESH"]
    if not meshes:
        raise RuntimeError(f"{source}: importer produced no mesh objects")
    bpy.ops.object.select_all(action="DESELECT")
    for obj in meshes:
        obj.select_set(True)
    bpy.context.view_layer.objects.active = meshes[0]
    output.parent.mkdir(parents=True, exist_ok=True)
    bpy.ops.export_scene.gltf(
        filepath=str(output),
        export_format="GLB",
        use_selection=True,
        export_apply=True,
    )
    if not output.is_file() or output.stat().st_size == 0:
        raise RuntimeError(f"{output}: GLB export did not produce a file")
    print(f"converted {model_id}: {output} ({output.stat().st_size} bytes)")


def main() -> None:
    sys.path.insert(0, str(ADDON))
    import io_object_mu

    io_object_mu.register()
    for model_id in MODEL_IDS:
        convert(model_id)


if __name__ == "__main__":
    main()
