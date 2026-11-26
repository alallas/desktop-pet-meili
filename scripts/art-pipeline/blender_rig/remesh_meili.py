# remesh_meili.py — 阶段③⑤重做第一步：Tripo 碎片网格 → 干净流形 meili
# 用法: blender -b -P scripts/art-pipeline/blender_rig/remesh_meili.py -- <input.glb> <out_dir>
# 产出: <out_dir>/meili_clean.glb（供绑骨）+ clean_<view>.png 白模预览
#
# 管线（每步都经过多轮实证，踩坑记录见 docs/踩坑与环境档案.md）：
#   1. 体素 remesh（体素=高/250）：碎片汤 → 实体流形网格，面部细节保住
#      【不要焊接】源"三角汤"顶点间隙亚毫米~毫米级；焊接成密闭壳后 remesh
#      会把亚体素壁厚解析成噪声（指纹摩尔纹实证）；不焊接反而外壳光滑。
#   2. 连通域剔除：remesh 后碎饼尾是独立壳，按质心区域（x<-0.10, z<-0.10）
#      整体删除，主体是独立实体不受影响（删后仍 0 开放边，断言保证）。
#      【不要试图删源网格的尾巴】汤网格删面=拆背壳→内腔外露→摩尔纹+灌腔（实证）。
#   3. 程序化尾（竖尾柄+贴地扁铲）放样，join 但不再 remesh（布尔交集线
#      藏在尾根/臀部阴影处，白模阶段可接受）。
import math
import os
import sys
from collections import deque

import bmesh
import bpy
from mathutils import Matrix, Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)

mesh = next(o for o in bpy.context.scene.objects if o.type == "MESH")
bpy.ops.object.select_all(action="SELECT")
bpy.ops.object.transform_apply(location=False, rotation=True, scale=True)

bbox = [mesh.matrix_world @ Vector(c) for c in mesh.bound_box]
zs = [v.z for v in bbox]
height = max(zs) - min(zs)
voxel = height / 250.0
print(f"[remesh] 包围盒 z {min(zs):.3f}..{max(zs):.3f} 高 {height:.3f} → 体素 {voxel:.5f}")

# ---- 1. 体素 remesh（不焊接）----
bpy.context.view_layer.objects.active = mesh
bpy.ops.object.mode_set(mode="OBJECT")
mesh.data.remesh_voxel_size = voxel
mesh.data.remesh_voxel_adaptivity = 0.0
bpy.ops.object.voxel_remesh()

sm = mesh.modifiers.new("smooth", "SMOOTH")
sm.factor = 0.25
sm.iterations = 2
bpy.context.view_layer.objects.active = mesh
bpy.ops.object.modifier_apply(modifier="sm")

# ---- 2. 单平面切割封口（edit-mode bisect use_fill + 尖刺溶解兜底）----
# 实证史：bmesh bisect_plane 只切不填；Boolean EXACT 差切在本网格上行为退化
# （无差别删 97% 体面，翻面切刀同样——坑 67）；edit-mode mesh.bisect use_fill
# 主体干净，只在切面切线接触处留近退化狭缝（2 边链，拉弦报 edge exists）。
# 解法：fill 后溶解狭链中间的近共线顶点（尖刺两侧的缝随之消失）。
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
bpy.context.view_layer.objects.active = mesh
bpy.ops.object.mode_set(mode="EDIT")
bpy.ops.mesh.select_all(action="SELECT")
bpy.ops.mesh.bisect(plane_co=(-0.12, 0.0, 0.0), plane_no=(1.0, 0.0, 0.0),
                    clear_inner=True, use_fill=True)
bpy.ops.object.mode_set(mode="OBJECT")
print("[remesh] 平面切 x=-0.12 已封口")

# 开放边处理：先打印定位，再 triangle_fill 兜底补一次，最后断言。
# （use_fill 偶发留小环，端点全在切面平面上时 fill 必成功——实证 diag8。）
bm = bmesh.new()
bm.from_mesh(mesh.data)
open_edges = [e for e in bm.edges if e.is_boundary]
print(f"[remesh] 开放边 {len(open_edges)}")
if open_edges:
    # 轮 1：triangle_fill 补闭合环
    r = bmesh.ops.triangle_fill(bm, edges=open_edges,
                                normal=Vector((1.0, 0.0, 0.0)))
    print(f"[remesh] triangle_fill 补面 {len(r.get('faces', []))}")
    # 轮 2..N：溶解残余狭链上的近共线顶点（尖刺中段），缝随顶点消失。
    # 全在切面平面上，溶解只影响盖面局部。最多 8 轮。
    for rnd in range(8):
        oe = [e for e in bm.edges if e.is_boundary]
        if not oe:
            break
        from collections import Counter
        cnt = Counter()
        for e in oe:
            for v in e.verts:
                cnt[v] += 1
        # 候选：在切面平面上、恰好两条开放边相交的顶点
        spikes = [v for v, c in cnt.items() if c == 2
                  and abs(v.co.x + 0.12) < 1e-4]
        if not spikes:
            break
        bmesh.ops.dissolve_verts(bm, verts=spikes,
                                 use_face_split=False, use_boundary_tear=False)
        print(f"[remesh] 溶解尖刺顶点 {len(spikes)}（轮 {rnd + 1}）")
    open_edges = [e for e in bm.edges if e.is_boundary]
    print(f"[remesh] 补后开放边 {len(open_edges)}")
    for e in open_edges:
        for v in e.verts:
            print(f"[remesh] 残边端点 ({v.co.x:.4f},{v.co.y:.4f},{v.co.z:.4f}) vid={v.index}")
assert len(open_edges) == 0, f"封口失败：{len(open_edges)} 开放边"
bm.to_mesh(mesh.data)
bm.free()

# ---- 2b. 连通域剔除：删除切后残留的小碎块（多孔壳屑/盖面碎片）----
# 实证：切后主体一个域（35 万面），碎壳屑散成 60~134 面的小域粘在切面与
# 尾根之间（俯视图可见的"蕾丝"条带来源）。主体 >> 碎块，按 5000 面阈值
# 剔除安全（尾巴此时尚未加入）。该步骤不改主体拓扑，无指纹噪声风险。
bm = bmesh.new()
bm.from_mesh(mesh.data)
tag = {}
keep_faces = []
removed = 0
for v in bm.verts:
    if v.index in tag:
        continue
    stack = [v]
    tag[v.index] = 1
    comp_vs = []
    while stack:
        x = stack.pop()
        comp_vs.append(x)
        for e in x.link_edges:
            for vv in e.verts:
                if vv.index not in tag:
                    tag[vv.index] = 1
                    stack.append(vv)
    comp_faces = set()
    for x in comp_vs:
        for f in x.link_faces:
            comp_faces.add(f.index)
    if len(comp_faces) < 5000:
        removed += len(comp_faces)
        bmesh.ops.delete(bm, geom=[f for f in bm.faces if f.index in comp_faces],
                         context="FACES")
    else:
        keep_faces.append(len(comp_faces))
print(f"[remesh] 连通域剔除小碎块 {removed} 面，主体域 {keep_faces}")
bm.to_mesh(mesh.data)
bm.free()

# ---- 3. 臀罩 + 程序化尾（join 不再 remesh；罩体盖住切面拱洞与尾根碎壳）----
# 实证：体素 remesh 把臀下空气口袋封成了"环形隧道"，平面切后拱洞暴露
# （0 开放边断言抓不到隧道）；罩体从外侧封住整个臀截面，尾巴从罩下流出。
bm = bmesh.new()
bmesh.ops.create_uvsphere(bm, u_segments=32, v_segments=24, radius=1.0)
butt_m = (Matrix.Translation((-0.02, 0.0, -0.24))
          @ Matrix.Diagonal((0.16, 0.40, 0.32, 1.0)))
bmesh.ops.transform(bm, matrix=butt_m, verts=bm.verts)
butt_mesh = bpy.data.meshes.new("butt_plate")
bm.to_mesh(butt_mesh)
bm.free()
butt_obj = bpy.data.objects.new("butt_plate", butt_mesh)
bpy.context.scene.collection.objects.link(butt_obj)
bm = bmesh.new()
verts_ring = []
PROF = [  # (cx, rw 半宽y, cz 中心z, rh 半高z)
    ( 0.15, 0.280, -0.300, 0.170),  # t=0 根：罩住切口盖面(x=-0.12)与臀下碎壳
    ( 0.02, 0.310, -0.325, 0.150),  # 顶面抬到 z=-0.175，盖住碎壳顶(z<-0.20)
    (-0.06, 0.330, -0.355, 0.130),
    (-0.12, 0.345, -0.380, 0.115),
    (-0.15, 0.350, -0.400, 0.095),   # 过渡到扁铲
    (-0.16, 0.350, -0.432, 0.062),   # 铲：cz-rh≈-0.494 贴地
    (-0.20, 0.392, -0.436, 0.060),
    (-0.26, 0.408, -0.438, 0.058),
    (-0.32, 0.388, -0.439, 0.057),
    (-0.37, 0.332, -0.440, 0.056),
    (-0.41, 0.250, -0.441, 0.055),
    (-0.44, 0.155, -0.442, 0.052),
    (-0.46, 0.060, -0.442, 0.048),
    (-0.47, 0.015, -0.442, 0.030),   # 圆钝尖
]
for cx, rw, cz, rh in PROF:
    seg = 16
    ring = []
    for s in range(seg):
        a = 2 * math.pi * s / seg
        ring.append(bm.verts.new((cx, rw * math.cos(a), cz + rh * math.sin(a))))
    verts_ring.append(ring)
for i in range(len(PROF) - 1):
    for s in range(16):
        s2 = (s + 1) % 16
        bm.faces.new((verts_ring[i][s], verts_ring[i][s2],
                      verts_ring[i + 1][s2], verts_ring[i + 1][s]))
bm.faces.new(reversed(verts_ring[0]))
bm.faces.new(verts_ring[-1])
bm.normal_update()
tail_mesh = bpy.data.meshes.new("tail_proc")
bm.to_mesh(tail_mesh)
bm.free()
tail_obj = bpy.data.objects.new("tail_proc", tail_mesh)
bpy.context.scene.collection.objects.link(tail_obj)

bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
tail_obj.select_set(True)
butt_obj.select_set(True)
bpy.context.view_layer.objects.active = mesh
bpy.ops.object.join()
for p in mesh.data.polygons:
    p.use_smooth = True
print("[remesh] 塞体+程序化尾巴已并入")

# ---- 预览与导出 ----
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.light = "STUDIO"
scene.display.shading.color_type = "OBJECT"
scene.render.resolution_x = 640
scene.render.resolution_y = 640
cam_data = bpy.data.cameras.new("cam")
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
cam_data.type = "ORTHO"
cam_data.ortho_scale = 1.4
center = Vector((0, 0, height / 2 + min(zs)))
for name, off in [("front", (1.4, 0, 0)), ("side", (0, 1.4, 0)), ("back", (-1.4, 0, 0)),
                  ("top", (0, 0, 1.4)), ("34", (1.1, 1.1, 0.7))]:
    cam.location = center + Vector(off)
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"clean_{name}.png")
    bpy.ops.render.render(write_still=True)

bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
out_glb = os.path.join(out_dir, "meili_clean.glb")
bpy.ops.export_scene.gltf(filepath=out_glb, use_selection=True, export_yup=True,
                          export_animations=False, export_skins=False)
print(f"[remesh] exported {out_glb}")
print("[remesh] done")
