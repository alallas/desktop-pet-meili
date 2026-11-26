# debug_tail.py — 检查 meili_clean.glb 尾巴区域实际形状
import os, sys
import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src = os.path.abspath(argv[0])
out_dir = os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
mesh = next(o for o in bpy.context.scene.objects if o.type == "MESH")
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

import bmesh
bm = bmesh.new()
bm.from_mesh(mesh.data)
tailv = [v for v in bm.verts if v.co.x < -0.05]
print(f"[tail] x<-0.05 顶点 {len(tailv)}")
if tailv:
    xs = [v.co.x for v in tailv]; ys = [v.co.y for v in tailv]; zs = [v.co.z for v in tailv]
    print(f"[tail] x {min(xs):.3f}..{max(xs):.3f}  y {min(ys):.3f}..{max(ys):.3f}  z {min(zs):.3f}..{max(zs):.3f}")
bm.free()

scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.light = "STUDIO"
scene.display.shading.color_type = "OBJECT"
scene.render.resolution_x = 640
scene.render.resolution_y = 640
cam_data = bpy.data.cameras.new("cam")
cam_data.type = "ORTHO"
cam_data.ortho_scale = 1.4
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
center = Vector((0, 0, 0.5))
for name, off in {"side": (0, 1.4, 0), "back": (-1.4, 0, 0)}.items():
    cam.location = center + Vector(off)
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"taildbg_{name}.png")
    bpy.ops.render.render(write_still=True)
print("[tail] done")
