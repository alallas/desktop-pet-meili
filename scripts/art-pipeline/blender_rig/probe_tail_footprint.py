# probe_tail_footprint.py — 精确测量源碎片尾的足迹（Blender z-up 空间）
import os, sys
import bpy

argv = sys.argv[sys.argv.index("--") + 1:]
src = os.path.abspath(argv[0])

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)
mesh = next(o for o in bpy.context.scene.objects if o.type == "MESH")
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

import bmesh
from mathutils import Vector
bm = bmesh.new()
bm.from_mesh(mesh.data)
# 碎片尾 = 贴地的面（z < -0.20）
tv = []
for f in bm.faces:
    if all(v.co.z < -0.20 for v in f.verts):
        tv.extend(v.co for v in f.verts)
print(f"[probe] 尾区顶点 {len(tv)}")
xs = [v.x for v in tv]; ys = [v.y for v in tv]; zs = [v.z for v in tv]
print(f"[probe] x {min(xs):.3f}..{max(xs):.3f}  y {min(ys):.3f}..{max(ys):.3f}  z {min(zs):.3f}..{max(zs):.3f}")
# 分 x 片量 y/z 跨度
for x0 in [-0.46,-0.42,-0.38,-0.34,-0.30,-0.26,-0.22,-0.18,-0.14,-0.10]:
    sl = [v for v in tv if x0 - 0.02 < v.x <= x0 + 0.02]
    if len(sl) > 3:
        print(f"[probe] x≈{x0}: n={len(sl):4d} y±={ (max(v.y for v in sl)-min(v.y for v in sl))/2:.3f} z±={(max(v.z for v in sl)-min(v.z for v in sl))/2:.3f} zmax={max(v.z for v in sl):.3f}")
bm.free()
