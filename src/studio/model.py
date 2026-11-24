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


def preview(objects, corners, out):
    # An isolated preview scene leaves the editable source's camera and render settings intact.
    scene = bpy.data.scenes.new("Workshop preview")
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
    scene.render.film_transparent = True
    scene.render.resolution_x = scene.render.resolution_y = 480
    scene.render.resolution_percentage = 100
    scene.render.filepath = str(out/"preview.png")
    bpy.ops.render.render(scene=scene.name, write_still=True)


out = Path.cwd()
request = json.loads((out/"request.json").read_text(encoding="utf-8"))
if request["type"] == "blend":
    bpy.ops.wm.open_mainfile(filepath=request["source"], load_ui=False, use_scripts=False)
else:
    bpy.ops.wm.read_factory_settings(use_empty=True)
    if request["type"] == "model_brief":
        build_design(json.loads((out/"design.json").read_text(encoding="utf-8")))
    else:
        starter(request["name"], request["template"])
bpy.context.view_layer.update()
objects = [o for o in bpy.context.view_layer.objects if o.type == 'MESH' and o.visible_get()]
if not objects:
    raise ValueError("The scene has no visible meshes to export")
graph = bpy.context.evaluated_depsgraph_get()
corners, tris = [], 0
for obj in objects:
    evaluated = obj.evaluated_get(graph)
    corners.extend(evaluated.matrix_world @ Vector(v) for v in evaluated.bound_box)
    mesh = evaluated.to_mesh()
    try:
        mesh.calc_loop_triangles()
        tris += len(mesh.loop_triangles)
    finally:
        evaluated.to_mesh_clear()
if not tris:
    raise ValueError("The selected scene has no mesh faces")
bpy.ops.wm.save_as_mainfile(filepath=str(out/"model.blend"), copy=True, relative_remap=True)
result = bpy.ops.export_scene.gltf(filepath=str(out/"model.glb"), export_format='GLB', use_visible=True,
                                   export_cameras=False, export_lights=False)
if result != {'FINISHED'}:
    raise RuntimeError("glTF export did not finish")
preview(objects, corners, out)
(out/"summary.txt").write_text(f"GLB · {len(objects)} mesh{'es' if len(objects)!=1 else ''} · {tris:,} triangles", encoding="utf-8")
