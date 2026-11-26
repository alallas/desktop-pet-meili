# remesh_meili.py — 阶段③⑤重做第一步：Tripo 非流形网格 → Voxel Remesh 流形化
# 用法: blender -b -P scripts/art-pipeline/blender_rig/remesh_meili.py -- <input.glb> <out_dir>
# 产出: <out_dir>/meili_clean.glb（流形网格，无骨无动画，供绑骨用）
#       <out_dir>/clean_<view>.png 白模预览（正视=脸方向 +X，侧视）
# 设计：
#   - 体素尺寸 = 包围盒高 / 250（约 4000 万体素上限内取细），保闭眼缝/眉/嘴线/贝壳
#   - remesh_voxel_adaptivity=0（均匀体素，薄件不塌）；壳/绳若与躯干分离会各自成壳保留
#   - 重网格后 smooth shading + 轻量 Smooth 修正（0.25 迭代 2），抹体素台阶
#   - 拓扑体检：bmesh 统计开放边/非流形边，打印验证
import math
import os
import sys

import bmesh
import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)

mesh = next(o for o in bpy.context.scene.objects if o.type == "MESH")
# 导入 glTF 带 -90°X 根旋转（z-up 惯例），应用掉，让包围盒/体素在统一空间算
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

# 源网格健康度（重网格前）
bm = bmesh.new()
bm.from_mesh(mesh.data)
edges = {}
for e in bm.edges:
    edges.setdefault(len(e.link_faces), []).append(e)
print(f"[remesh] 源网格: 顶点{len(bm.verts)} 面{len(bm.faces)} "
      f"开放边{len(edges.get(1, []))} 非流形边{len(edges.get(3, [])) + sum(len(v) for k, v in edges.items() if k > 3)}")
bm.free()

bbox = [mesh.matrix_world @ Vector(c) for c in mesh.bound_box]
zs = [v.z for v in bbox]
height = max(zs) - min(zs)
voxel = height / 250.0
print(f"[remesh] 包围盒高 {height:.3f} → 体素 {voxel:.5f}")

bpy.context.view_layer.objects.active = mesh
bpy.ops.object.mode_set(mode="OBJECT")
mesh.data.remesh_voxel_size = voxel
mesh.data.remesh_voxel_adaptivity = 0.0
bpy.ops.object.voxel_remesh()

# 轻量平滑去台阶（不影响大形）
sm = mesh.modifiers.new("smooth", "SMOOTH")
sm.factor = 0.25
sm.iterations = 2
bpy.context.view_layer.objects.active = mesh
bpy.ops.object.modifier_apply(modifier="smooth")

# 平滑着色（角度阈值，保硬边如贝壳棱）
for p in mesh.data.polygons:
    p.use_smooth = True

# 重网格后体检
bm = bmesh.new()
bm.from_mesh(mesh.data)
edges = {}
for e in bm.edges:
    edges.setdefault(len(e.link_faces), []).append(e)
open_b = len(edges.get(1, []))
nonman = sum(len(v) for k, v in edges.items() if k >= 3)
print(f"[remesh] 干净网格: 顶点{len(bm.verts)} 面{len(bm.faces)} 开放边{open_b} 非流形边{nonman}")
bm.free()
assert open_b == 0 and nonman == 0, "重网格后仍存在开放/非流形边，体素尺寸需调"

# 出白模预览（正/侧/背）
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.light = "STUDIO"
scene.display.shading.color_type = "MATERIAL"
scene.render.resolution_x = 640
scene.render.resolution_y = 640
cam_data = bpy.data.cameras.new("cam")
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
cam_data.type = "ORTHO"
cam_data.ortho_scale = 1.4
center = Vector((0, 0, height / 2 + min(zs)))
for name, off in [("front", (1.4, 0, 0)), ("side", (0, 1.4, 0)), ("back", (-1.4, 0, 0))]:
    cam.location = center + Vector(off)
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"clean_{name}.png")
    bpy.ops.render.render(write_still=True)

# 导出干净网格（+Y up，不带相机灯光）
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
out_glb = os.path.join(out_dir, "meili_clean.glb")
bpy.ops.export_scene.gltf(filepath=out_glb, use_selection=True, export_yup=True,
                          export_animations=False, export_skins=False)
print(f"[remesh] exported {out_glb}")
print("[remesh] done")
