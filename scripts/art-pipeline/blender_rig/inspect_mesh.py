# Blender 探针：导入 meili GLB，打印结构/尺寸，六向正交渲染供自检
# 用法: blender -b -P scripts/art-pipeline/blender_rig/inspect_mesh.py -- <input.glb> <out_dir>
import sys

import bpy

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = argv[0], argv[1]

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)

meshes = [o for o in bpy.context.scene.objects if o.type == "MESH"]
arms = [o for o in bpy.context.scene.objects if o.type == "ARMATURE"]
print(f"[inspect] meshes={len(meshes)} armatures={len(arms)}")
for o in meshes:
    print(f"[inspect] mesh={o.name} verts={len(o.data.vertices)} tris={sum(len(p.vertices)-2 for p in o.data.polygons)}")
    for i, m in enumerate(o.data.materials or []):
        print(f"[inspect]   material[{i}]={m.name if m else None}")

# 世界包围盒
import mathutils
lo = mathutils.Vector((1e9, 1e9, 1e9))
hi = mathutils.Vector((-1e9, -1e9, -1e9))
for o in meshes:
    for c in o.bound_box:
        w = o.matrix_world @ mathutils.Vector(c)
        lo = mathutils.Vector(map(min, lo, w))
        hi = mathutils.Vector(map(max, hi, w))
print(f"[inspect] bbox min={tuple(round(v,3) for v in lo)} max={tuple(round(v,3) for v in hi)}")

# 六向正交渲染（Workbench 软件渲染，无 GPU 依赖）
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.light = "STUDIO"
scene.display.shading.color_type = "RANDOM"
scene.render.resolution_x = 512
scene.render.resolution_y = 512
scene.render.film_transparent = False

cam_data = bpy.data.cameras.new("cam")
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
cam_data.type = "ORTHO"

# 让相机包住模型：对角线长度
center = (lo + hi) / 2
radius = max((hi - lo).length, 1e-3)
cam_data.ortho_scale = radius * 1.25

views = {
    "front": (0, -radius, 0),
    "back": (0, radius, 0),
    "right": (radius, 0, 0),
    "left": (-radius, 0, 0),
    "top": (0, 0, radius),
}
import os
out_dir = os.path.abspath(out_dir)
print(f"[inspect] cwd={os.getcwd()} out_dir={out_dir}")
os.makedirs(out_dir, exist_ok=True)
for name, offset in views.items():
    direction = mathutils.Vector(offset)
    cam.location = center + direction
    look = center - cam.location
    cam.rotation_euler = look.to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"view_{name}.png")
    bpy.ops.render.render(write_still=True)
    print(f"[inspect] rendered {name}")
print("[inspect] done")
