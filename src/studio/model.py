"""Build or import a game asset in a background Blender process."""
import json
import math
import re
from pathlib import Path

import bpy
from mathutils import Vector


def material(name, color):
    mat = bpy.data.materials.new(name)
    mat.diffuse_color = (*color, 1)
    if not mat.node_tree:
        mat.use_nodes = True
    shader = next(n for n in mat.node_tree.nodes if n.type == "BSDF_PRINCIPLED")
    shader.inputs["Base Color"].default_value = (*color, 1)
    return mat


def build_design(design):
    parts = design["parts"]
    if not isinstance(parts, list) or not 1 <= len(parts) <= 64:
        raise ValueError("A design must contain between 1 and 64 mesh parts")
    materials = {}
    shapes = {
        "box": (bpy.ops.mesh.primitive_cube_add, {}),
        "ellipsoid": (bpy.ops.mesh.primitive_uv_sphere_add, {"segments":16, "ring_count":8}),
        "cone": (bpy.ops.mesh.primitive_cone_add, {"vertices":16, "end_fill_type":"TRIFAN"}),
        "cylinder": (bpy.ops.mesh.primitive_cylinder_add, {"vertices":16, "end_fill_type":"TRIFAN"}),
        "torus": (bpy.ops.mesh.primitive_torus_add, {"major_segments":24, "minor_segments":8}),
    }
    for part in parts:
        for field in ("size", "center", "rotation"):
            v = part[field]
            if len(v) != 3 or any(not math.isfinite(n) or abs(n)>10000 for n in v):
                raise ValueError(f"Invalid {field} in model design")
        if min(part["size"]) < .0001:
            raise ValueError("Mesh dimensions must be positive")
        hex_color = part["color"].lstrip("#").lower()
        if not re.fullmatch("[0-9a-f]{6}", hex_color):
            raise ValueError("Invalid material color")
        if hex_color not in materials:
            srgb = [int(hex_color[i:i+2],16)/255 for i in (0,2,4)]
            linear = [n/12.92 if n<=.04045 else ((n+.055)/1.055)**2.4 for n in srgb]
            materials[hex_color] = material(hex_color, linear)
        add, options = shapes[part["shape"]]
        add(**options)
        obj = bpy.context.object
        obj.name = part["name"][:60]
        obj.dimensions = part["size"]
        bpy.ops.object.transform_apply(location=False, rotation=False, scale=True)
        obj.location = part["center"]
        obj.rotation_euler = [math.radians(n) for n in part["rotation"]]
        obj.data.materials.append(materials[hex_color])


def starter(name, template):
    # Ordered profile rings, radius in metres, and the material of the following band.
    profiles = {
        "Crystal": (7, [(0, .29, 0), (.22, .42, 0), (.95, .32, 0), (1.36, .055, 1)],
                    [(0.055, .30, .36), (.10, .49, .46), (.055, .22, .34)]),
        "Barrel": (14, [(0, .39, 0), (.06, .41, 1), (.12, .42, 0), (.48, .48, 0),
                        (.84, .42, 1), (.91, .41, 0), (.97, .39, 2)],
                   [(.31, .13, .055), (.095, .13, .14), (.41, .22, .09)]),
        "Planter": (12, [(0, .27, 0), (.08, .29, 0), (.55, .41, 1), (.58, .445, 1),
                         (.65, .445, 1), (.65, .39, 0), (.10, .245, 2)],
                    [(.51, .19, .11), (.76, .36, .19), (.38, .13, .07)]),
    }
    sides, rings, colors = profiles[template]
    verts, faces, slots, coords = [], [], [], []
    for level, (z, radius, _) in enumerate(rings):
        for i in range(sides):
            a = math.tau * i / sides
            lean = z * .105 if template == "Crystal" else 0
            verts.append((lean + radius*math.cos(a), radius*math.sin(a), z))
        if level == 0:
            continue
        for i in range(sides):
            j = (i+1) % sides
            base = (level-1)*sides
            faces.append((base+i, base+j, base+j+sides, base+i+sides))
            slots.append((i+level//2) % 3 if template == "Crystal" else rings[level-1][2])
            a, b = i/sides, (i+1)/sides
            low, high = .65*(level-1)/(len(rings)-1), .65*level/(len(rings)-1)
            coords.append([(a,low),(b,low),(b,high),(a,high)])
    for level in (0, len(rings)-1):
        center = len(verts)
        ring = verts[level*sides:(level+1)*sides]
        verts.append(tuple(sum(v[k] for v in ring)/sides for k in range(3)))
        for i in range(sides):
            j = (i+1) % sides
            a,b = (j,i) if level==0 else (i,j)
            faces.append((center,level*sides+a,level*sides+b))
            slots.append(rings[level][2])
            u = .25 if level == 0 else .75
            coords.append([(u,.825)] + [(u+.15*math.cos(math.tau*k/sides),.825+.15*math.sin(math.tau*k/sides)) for k in (a,b)])
    mesh = bpy.data.meshes.new(name)
    mesh.from_pydata(verts, [], faces)
    mesh.update()
    obj = bpy.data.objects.new(name, mesh)
    bpy.context.scene.collection.objects.link(obj)
    for i, color in enumerate(colors):
        mesh.materials.append(material(f"{template} {i+1}", color))
    uv = mesh.uv_layers.new()
    for face, slot, face_uv in zip(mesh.polygons, slots, coords):
        face.material_index = slot
        for loop, position in zip(face.loop_indices, face_uv):
            uv.data[loop].uv = position
    bpy.context.view_layer.objects.active = obj
    obj.select_set(True)


def preview(objects, corners, out, collection=None):
    # An isolated preview scene leaves the editable source's camera and render settings intact.
    scene = bpy.data.scenes.new("Workshop preview")
    if collection is not None:
        scene.collection.children.link(collection)
    else:
        for obj in objects:
            scene.collection.objects.link(obj)
    center = Vector([(min(v[i] for v in corners)+max(v[i] for v in corners))/2 for i in range(3)])
    radius = max(.01, max((corner-center).length for corner in corners))
    camera = bpy.data.objects.new("Preview camera", bpy.data.cameras.new("Preview camera"))
    scene.collection.objects.link(camera)
    camera.location = center + Vector((1.5,-2.2,1.35)).normalized() * (radius*3.5)
    camera.rotation_euler = (center-camera.location).to_track_quat("-Z","Y").to_euler()
    camera.data.type = 'ORTHO'
    camera.data.ortho_scale = radius * 2.22
    camera.data.clip_start = radius*.001
    camera.data.clip_end = radius*8
    scene.camera = camera
    scene.render.engine = 'BLENDER_WORKBENCH'
    scene.display.shading.color_type = 'MATERIAL'
    scene.display.shading.show_shadows = True
    scene.display.shading.show_cavity = True
    if collection is not None:
        # Render the actual shaders and textures for a reusable style reference.
        scene.render.engine = 'CYCLES'
        scene.cycles.device = 'CPU'
        scene.cycles.samples = 12
        scene.cycles.use_denoising = True
        scene.world = bpy.data.worlds.new("Preview world")
        scene.world.color = (.4, .4, .4)
        lights = []
        for offset, intensity in [((2, -3, 4), 1050), ((-3, -1, 2), 550), ((1, 3, 3), 800)]:
            data = bpy.data.lights.new("Preview softbox", 'AREA')
            data.energy = intensity * radius**2
            data.size = 3 * radius
            light = bpy.data.objects.new("Preview softbox", data)
            scene.collection.objects.link(light)
            light.location = center + Vector(offset).normalized() * radius * 4
            light.rotation_euler = (center-light.location).to_track_quat("-Z", "Y").to_euler()
            lights.append((light, data))
    scene.render.film_transparent = True
    scene.render.resolution_x = scene.render.resolution_y = 480
    scene.render.resolution_percentage = 100
    scene.render.filepath = str(out/"preview.png")
    bpy.ops.render.render(scene=scene.name, write_still=True)
    world = scene.world
    bpy.data.scenes.remove(scene)
    camera_data = camera.data
    bpy.data.objects.remove(camera, do_unlink=True)
    bpy.data.cameras.remove(camera_data)
    if collection is not None:
        bpy.data.worlds.remove(world)
        for light, data in lights:
            bpy.data.objects.remove(light, do_unlink=True)
            bpy.data.lights.remove(data)


def pack_collection(request, out):
    bpy.ops.wm.open_mainfile(filepath=request["source"], load_ui=False, use_scripts=False)
    units = bpy.context.scene.unit_settings
    scale = units.scale_length if units.system != 'NONE' else 1.0
    selected = request.get("collection")
    if selected:
        collection = bpy.data.collections.get(selected)
        if collection is None:
            raise ValueError(f"No collection named {selected!r} in the Blender file")
    else:
        collection = bpy.data.collections.new(request["name"])
        for obj in bpy.context.view_layer.objects:
            if obj.visible_get() and obj.type not in {'CAMERA', 'LIGHT'}:
                collection.objects.link(obj)
    # A library round trip keeps the collection's dependencies and removes unrelated scenes.
    library = out / "selection.blend"
    name = collection.name
    bpy.data.libraries.write(str(library), {collection}, path_remap='ABSOLUTE')
    bpy.ops.wm.read_factory_settings(use_empty=True)
    with bpy.data.libraries.load(str(library), link=False, recursive=True) as (_, data):
        data.collections = [name]
    collection = data.collections[0]
    bpy.context.scene.collection.children.link(collection)
    # Appending a collection into another scene does not carry source scene units.
    # Normalize the root once; rigs, modifiers and child transforms stay editable.
    bpy.context.scene.unit_settings.system = 'METRIC'
    bpy.context.scene.unit_settings.scale_length = 1.0
    if not math.isclose(scale, 1.0):
        bpy.context.view_layer.update()
        objects = set(collection.all_objects)
        anchor = bpy.data.objects.new("Asset scale", None)
        collection.objects.link(anchor)
        for obj in objects:
            if obj.parent not in objects:
                if obj.parent is None:
                    obj.parent = anchor
                else:
                    world = obj.matrix_world.copy()
                    obj.parent = anchor
                    obj.matrix_world = world
        anchor.scale = (scale,) * 3
        collection.instance_offset *= scale
    bpy.data.orphans_purge(do_recursive=True)
    library.unlink()
    bpy.ops.file.pack_all()
    for image in bpy.data.images:
        if image.users and image.source in {'FILE', 'TILED', 'SEQUENCE', 'MOVIE'} and not image.packed_files:
            raise ValueError(f"Pack or replace the missing or external image {image.name!r} before publishing")
    # Blender records an optional append-reuse origin even for fully local IDs.
    external = [p for p in bpy.utils.blend_paths(absolute=True, packed=False) if p and Path(p) != library]
    if any(path for path in external):
        raise ValueError(f"The collection still needs an external file: {next(path for path in external if path)}")
    collection.asset_mark()
    collection.asset_data.description = request["description"]
    collection.asset_data.catalog_id = request["catalog_id"]
    return collection


def material_colors(objects):
    result = {}
    for obj in objects:
        for slot in obj.material_slots:
            mat = slot.material
            if mat is None or mat.name in result:
                continue
            shader = next((n for n in mat.node_tree.nodes if n.type == 'BSDF_PRINCIPLED'), None) if mat.node_tree else None
            color = shader.inputs["Base Color"].default_value if shader else mat.diffuse_color
            rgb = [max(0, min(1, c)) for c in color[:3]]
            result[mat.name] = "#" + "".join(f"{round(255*(12.92*c if c <= .0031308 else 1.055*c**(1/2.4)-.055)):02X}" for c in rgb)
    return [{"name": name, "color": result[name]} for name in sorted(result)]


out = Path.cwd()
request = json.loads((out/"request.json").read_text(encoding="utf-8"))
collection = None
if request["type"] == "pack_asset":
    collection = pack_collection(request, out)
elif request["type"] == "blend":
    bpy.ops.wm.open_mainfile(filepath=request["source"], load_ui=False, use_scripts=False)
else:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    if request["type"] == "model_brief":
        build_design(json.loads((out/"design.json").read_text(encoding="utf-8")))
    else:
        starter(request["name"], request["template"])
bpy.context.view_layer.update()
graph = bpy.context.evaluated_depsgraph_get()
objects, corners, tris, mesh_count = {}, [], 0, 0
for instance in graph.object_instances:
    evaluated = instance.object
    if evaluated.type != 'MESH' or not instance.show_self:
        continue
    obj = evaluated.original
    if not instance.is_instance and not obj.visible_get():
        continue
    corners.extend(instance.matrix_world @ Vector(v) for v in evaluated.bound_box)
    mesh = evaluated.to_mesh()
    try:
        mesh.calc_loop_triangles()
        tris += len(mesh.loop_triangles)
    finally:
        evaluated.to_mesh_clear()
    objects[obj.name] = obj
    mesh_count += 1
if not tris:
    raise ValueError("The selected scene has no mesh faces")
result = bpy.ops.export_scene.gltf(filepath=str(out/"model.glb"), export_format='GLB', use_visible=True,
                                   export_cameras=False, export_lights=False)
if result != {'FINISHED'}:
    raise RuntimeError("glTF export did not finish")
preview(objects.values(), corners, out, collection)
if collection is not None:
    with bpy.context.temp_override(id=collection):
        if bpy.ops.ed.lib_id_load_custom_preview(filepath=str(out/"preview.png")) != {'FINISHED'}:
            raise RuntimeError("Could not embed the Asset Browser thumbnail")
    dimensions = [round(max(v[i] for v in corners)-min(v[i] for v in corners), 5) for i in range(3)]
    (out/"geometry.json").write_text(json.dumps(dict(collection=collection.name, meshes=mesh_count,
        triangles=tris, dimensions=dimensions, meters_per_unit=bpy.context.scene.unit_settings.scale_length,
        materials=material_colors(objects.values()))), encoding="utf-8")
bpy.ops.wm.save_as_mainfile(filepath=str(out/"model.blend"), copy=True, relative_remap=True)
(out/"summary.txt").write_text(f"GLB · {mesh_count} mesh{'es' if mesh_count!=1 else ''} · {tris:,} triangles", encoding="utf-8")
