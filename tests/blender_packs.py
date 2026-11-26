"""Run the asset-pack round trip using a real headless Blender and Workshop CLI."""
import argparse
import concurrent.futures
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile


def make_source(folder):
    import bpy

    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.context.scene.unit_settings.system = 'METRIC'
    bpy.context.scene.unit_settings.scale_length = 0.5
    mat = bpy.data.materials.new("Harbor enamel")
    if not mat.node_tree:
        mat.use_nodes = True
    shader = mat.node_tree.nodes.get("Principled BSDF")
    shader.inputs["Base Color"].default_value = (0.06, 0.3, 0.4, 1)
    texture = bpy.data.images.new("Harbor paint", 4, 4)
    texture.pixels[:] = [0.12, 0.5, 0.6, 1] * 16
    texture.filepath_raw = str(folder / "palette.png")
    texture.file_format = 'PNG'
    texture.save()
    texture = bpy.data.images.load(str(folder / "palette.png"))
    image_node = mat.node_tree.nodes.new('ShaderNodeTexImage')
    image_node.image = texture
    mat.node_tree.links.new(image_node.outputs["Color"], shader.inputs["Base Color"])
    for name, shape in [("Crate", bpy.ops.mesh.primitive_cube_add), ("Buoy", bpy.ops.mesh.primitive_uv_sphere_add)]:
        group = bpy.data.collections.new(name)
        bpy.context.scene.collection.children.link(group)
        shape()
        obj = bpy.context.object
        obj.location.z = 1
        obj.name = name + " mesh"
        for collection in list(obj.users_collection):
            collection.objects.unlink(obj)
        group.objects.link(obj)
        obj.data.materials.append(mat)
    bpy.ops.wm.save_as_mainfile(filepath=str(folder / "source.blend"))


def append_and_verify(folder):
    import bpy

    bpy.ops.wm.read_factory_settings(use_empty=True)
    catalog = json.loads((folder / "pack.json").read_text())["catalog_id"]
    assert catalog in (folder / "blender_assets.cats.txt").read_text()
    for key in ["crate", "buoy"]:
        path = folder / "assets" / key
        record = json.loads((path / "asset.json").read_text())
        assert record["meshes"] == 1 and record["triangles"] > 0
        assert record["meters_per_unit"] == 1.0 and record["dimensions"] == [1.0, 1.0, 1.0]
        assert record["materials"][0]["name"] == "Harbor enamel"
        assert record["palette"] and record["palette"][0].startswith("#")
        assert (path / "model.glb").read_bytes()[:4] == b"glTF"
        assert (path / "preview.png").read_bytes()[:8] == b"\x89PNG\r\n\x1a\n"
        with bpy.data.libraries.load(str(path / "model.blend"), link=False, assets_only=True) as (source, target):
            assert source.collections == [record["collection"]], source.collections
            target.collections = source.collections
        collection = target.collections[0]
        bpy.context.scene.collection.children.link(collection)
        assert collection.asset_data.catalog_id == catalog
        assert collection.asset_data.description == record["description"]
        assert all(size > 0 for size in collection.preview.image_size)
        meshes = [obj for obj in collection.all_objects if obj.type == 'MESH']
        assert [obj.name for obj in meshes] == [key.title() + " mesh"]
        mesh = meshes[0]
        bpy.context.view_layer.update()
        assert all(abs(d - 1.0) < 0.0001 for d in mesh.dimensions)
        nodes = mesh.active_material.node_tree.nodes
        assert next(n for n in nodes if n.type == 'TEX_IMAGE').image.packed_files
    assert len([obj for obj in bpy.context.scene.objects if obj.type == 'MESH']) == 2
    bpy.ops.wm.read_factory_settings(use_empty=True)
    bpy.ops.import_scene.gltf(filepath=str(folder / "assets" / "crate" / "model.glb"))
    mesh = next(obj for obj in bpy.context.scene.objects if obj.type == 'MESH')
    assert all(abs(d - 1.0) < 0.0001 for d in mesh.dimensions)


def round_trip(root, cli, blender):
    project = root / "project"
    project.mkdir()
    source = root / "source"
    source.mkdir()
    env = dict(os.environ, WORKSHOP_HOME=str(root / "home"), WORKSHOP_BLENDER=str(blender))

    def run(*args, success=True):
        result = subprocess.run([str(cli), "--project", str(project), "packs", *map(str, args)],
                                env=env, capture_output=True, text=True, encoding="utf-8")
        if success:
            assert result.returncode == 0, result.stderr
            return json.loads(result.stdout)
        assert result.returncode != 0, result.stdout

    def background(mode, folder):
        subprocess.run([str(blender), "--background", "--factory-startup", "--disable-autoexec",
                        "--threads", "2", "--python-exit-code", "1", "--python", str(Path(__file__).resolve()),
                        "--", mode, str(folder)], check=True)

    background("source", source)
    run("create", "harbor", "--name", "Harbor props", "--style", "Weathered teal enamel; human-scale dock fittings.")

    def publish(name, **kwargs):
        return run("add", "harbor", name.lower(), source / "source.blend", "--collection", name,
                   "--name", name, "--description", f"Teal harbor {name.lower()}.", **kwargs)

    with concurrent.futures.ThreadPoolExecutor(max_workers=2) as executor:
        list(executor.map(publish, ["Crate", "Buoy"]))
    publish("Crate")  # Exact retry is idempotent.
    publish("Missing", success=False)
    found = run("show", "harbor")
    assert sorted(item["key"] for item in found["assets"]) == ["buoy", "crate"]
    assert Path(run("show", "harbor", "crate")["preview"]).is_file()
    folder = Path(run("export", "harbor", root / "portable")["folder"])
    run("export", "harbor", root / "portable", success=False)
    source.rename(root / "unavailable-original")
    (root / "home").rename(root / "unavailable-workshop")
    background("verify", folder)
    project = root / "second-project"
    project.mkdir()
    model = folder / "assets" / "crate" / "model.glb"
    original = model.read_bytes()
    model.write_bytes(original + b"incomplete")
    run("import", folder, success=False)
    assert run("list") == []
    model.write_bytes(original)
    run("import", folder)
    assert run("show", "harbor") == found
    print(f"Verified two portable, textured Blender collections in {folder}")


if __name__ == "__main__":
    if "--" in sys.argv:
        mode, folder = sys.argv[sys.argv.index("--") + 1:]
        {"source": make_source, "verify": append_and_verify}[mode](Path(folder))
    else:
        parser = argparse.ArgumentParser(description=__doc__)
        parser.add_argument("--cli", type=Path, required=True)
        parser.add_argument("--blender", type=Path, required=True)
        parser.add_argument("--keep-in", type=Path, help="Use a new folder and retain the fixtures.")
        args = parser.parse_args()
        if args.keep_in:
            args.keep_in.mkdir()
            round_trip(args.keep_in.resolve(), args.cli.resolve(), args.blender.resolve())
        else:
            with tempfile.TemporaryDirectory() as temp:
                round_trip(Path(temp), args.cli.resolve(), args.blender.resolve())
