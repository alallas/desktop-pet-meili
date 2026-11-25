# rig_meili.py — meili 程序化绑骨（路线A：自定义骨架 + 自动权重）
# 用法: blender -b -P scripts/art-pipeline/blender_rig/rig_meili.py -- <mode> <input.glb> <out_dir>
#   mode = bones   只建骨架+渲染骨位自检图（迭代骨位用）
#   mode = rig     建骨架+自动权重+deform 测试渲染+导出 rigged.glb（不含动画）
import math
import os
import sys

import bpy
import numpy as np
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
mode, src, out_dir = argv[0], os.path.abspath(argv[1]), os.path.abspath(argv[2])
os.makedirs(out_dir, exist_ok=True)

bpy.ops.wm.read_factory_settings(use_empty=True)
bpy.ops.import_scene.gltf(filepath=src)

mesh = next(o for o in bpy.context.scene.objects if o.type == "MESH")
mesh.name = "meili"
# 应用全部变换，让网格干净落地（glTF -90°X 旋转等）
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
bpy.context.view_layer.objects.active = mesh
bpy.ops.object.transform_apply(location=True, rotation=True, scale=True)
print(f"[rig] mesh verts={len(mesh.data.vertices)} dims={tuple(round(v,3) for v in mesh.dimensions)}")

# ---- 骨架定义（Blender z-up，脸朝 +X，模型 z ∈ [-0.5, 0.5]）----
# (名字, 首端, 末端, 父)  模型以原点为中心、高 1.0
BONES = [
    ("root",   (0.00, 0.00, -0.50), (0.00, 0.00, -0.38), None),
    ("spine",  (0.04, 0.00, -0.38), (0.07, 0.00, -0.20), "root"),
    ("chest",  (0.07, 0.00, -0.20), (0.10, 0.00,  0.00), "spine"),
    ("neck",   (0.10, 0.00,  0.00), (0.13, 0.00,  0.10), "chest"),
    ("head",   (0.12, 0.00,  0.10), (0.22, 0.00,  0.32), "neck"),
    ("jaw",    (0.20, 0.00,  0.05), (0.32, 0.00,  0.02), "head"),  # 下颌：下吻部
    ("arm.L",  (0.03, 0.30, 0.06),  (0.08, 0.35, -0.18), "chest"),
    ("hand.L", (0.08, 0.35, -0.18), (0.13, 0.36, -0.27), "arm.L"),
    ("arm.R",  (0.03, -0.30, 0.06), (0.08, -0.35, -0.18), "chest"),
    ("hand.R", (0.08, -0.35, -0.18), (0.13, -0.36, -0.27), "arm.R"),
    ("thigh.L",  (0.02, 0.14, -0.32), (0.05, 0.15, -0.42), "root"),
    ("foot.L",   (0.05, 0.15, -0.42), (0.12, 0.15, -0.48), "thigh.L"),
    ("thigh.R",  (0.02, -0.14, -0.32), (0.05, -0.15, -0.42), "root"),
    ("foot.R",   (0.05, -0.15, -0.42), (0.12, -0.15, -0.48), "thigh.R"),
    # 尾巴：实际拖在体后 -X 侧（渲染实证，非 -Y），三节贴地
    ("tail.1", (-0.12, -0.02, -0.40), (-0.26, -0.02, -0.44), "root"),
    ("tail.2", (-0.26, -0.02, -0.44), (-0.37, -0.02, -0.46), "tail.1"),
    ("tail.3", (-0.37, -0.02, -0.46), (-0.44, -0.02, -0.45), "tail.2"),
    ("ear.L",  (0.10, 0.30, 0.36),  (0.12, 0.34, 0.44), "head"),
    ("ear.R",  (0.10, -0.30, 0.36), (0.12, -0.34, 0.44), "head"),
]

arm_data = bpy.data.armatures.new("meili_rig")
arm = bpy.data.objects.new("meili_rig", arm_data)
bpy.context.scene.collection.objects.link(arm)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")

for name, head, tail, parent in BONES:
    eb = arm_data.edit_bones.new(name)
    eb.head = head
    eb.tail = tail
    eb.use_deform = not name.startswith("root")  # root 纯控制，不参与蒙皮
    # 包络参数调大（Tripo 网格热权重必失败，走包络蒙皮）
    eb.envelope_distance = 0.14
    eb.envelope_weight = 1.0
    eb.head_radius = 0.10
    eb.tail_radius = 0.08
    eb.use_envelope_multiply = True
    if parent:
        eb.parent = arm_data.edit_bones[parent]
        eb.use_connect = False  # 短粗角色，允许骨间空隙

bpy.ops.object.mode_set(mode="OBJECT")

# ---- 自研"最近骨段距离"蒙皮 ----
# Tripo 网格含悬浮件（贝壳/绳/尾尖），骨热权重的测地距离在孤岛失效 → 远距离乱指派（噪声补丁）；
# 包络/热权重在短粗比例下互相撕扯。改用对每个顶点算所有 deform 骨段的 3D 距离，
# Top-4 高斯平滑归一——与拓扑无关（不怕非流形/孤岛），贝壳/绳天然跟随最近的颈/胸骨。
mesh_n = len(mesh.data.vertices)
verts = np.empty(mesh_n * 3, dtype=np.float64)
mesh.data.vertices.foreach_get("co", verts)
verts = verts.reshape(-1, 3)

deform_bones = [(b.name, np.array(b.head_local), np.array(b.tail_local)) for b in arm.data.bones if b.use_deform]
print(f"[rig] custom skinning over {mesh_n} verts x {len(deform_bones)} bones")

R_SMOOTH = 0.10  # 平滑半径：距离差在此范围内产生混合权重
# 解剖区域门：欧氏最近骨在短粗比例下会跨区抢顶点（手臂骨抢侧腹 → 挥手大三角帆）。
# 给四肢/尾/耳/下颌加解析门槛，只允许它们领自己解剖区的顶点；
# 全部不满足时回退无门槛距离（中央骨兜底）
GATES = {
    "arm.L": lambda p: (p[:, 1] > 0.20) & (p[:, 2] > -0.30),
    "hand.L": lambda p: (p[:, 1] > 0.20) & (p[:, 2] > -0.30),
    "arm.R": lambda p: (p[:, 1] < -0.20) & (p[:, 2] > -0.30),
    "hand.R": lambda p: (p[:, 1] < -0.20) & (p[:, 2] > -0.30),
    "thigh.L": lambda p: (p[:, 0] > -0.03) & (p[:, 1] > 0.05) & (p[:, 1] < 0.24) & (p[:, 2] < -0.24),
    "foot.L": lambda p: (p[:, 0] > -0.03) & (p[:, 1] > 0.05) & (p[:, 1] < 0.24) & (p[:, 2] < -0.24),
    "thigh.R": lambda p: (p[:, 0] > -0.03) & (p[:, 1] < -0.05) & (p[:, 1] > -0.24) & (p[:, 2] < -0.24),
    "foot.R": lambda p: (p[:, 0] > -0.03) & (p[:, 1] < -0.05) & (p[:, 1] > -0.24) & (p[:, 2] < -0.24),
    "tail.1": lambda p: (p[:, 0] < -0.12) & (p[:, 2] < -0.30),
    "tail.2": lambda p: (p[:, 0] < -0.12) & (p[:, 2] < -0.30),
    "tail.3": lambda p: (p[:, 0] < -0.12) & (p[:, 2] < -0.30),
    "ear.L": lambda p: (p[:, 1] > 0.20) & (p[:, 2] > 0.28),
    "ear.R": lambda p: (p[:, 1] < -0.20) & (p[:, 2] > 0.28),
    # 颈：中央小带（侧向收紧，躯干侧壁归胸骨），只到下颌线以下
    "neck": lambda p: (p[:, 2] < 0.12) & (np.abs(p[:, 1]) < 0.18),
    # 下颌只管嘴下方那片：前突下吻部，别再吃掉整片肚皮和贝壳
    "jaw": lambda p: (p[:, 0] > 0.22) & (p[:, 2] > 0.0) & (p[:, 2] < 0.14),
}
INF = np.float32(1e9)
all_d = np.empty((mesh_n, len(deform_bones)), dtype=np.float32)
gate_mask = np.empty((mesh_n, len(deform_bones)), dtype=bool)
for bi, (bn, bh, bt) in enumerate(deform_bones):
    ab = bt - bh
    ab_len2 = float(ab @ ab)
    ap = verts - bh
    t = np.clip(ap @ ab / max(ab_len2, 1e-12), 0.0, 1.0)
    proj = bh + t[:, None] * ab
    d = np.linalg.norm(verts - proj, axis=1)
    gate = GATES.get(bn)
    ok = gate(verts) if gate else np.ones(mesh_n, dtype=bool)
    all_d[:, bi] = np.where(ok, d, INF)
    gate_mask[:, bi] = ok
# 顶点级兜底：若全部骨被门槛拒绝，回退到无门槛距离（中央骨在所以不会发生，保险）
no_gate = ~gate_mask.any(axis=1)
if no_gate.any():
    for bi, (bn, bh, bt) in enumerate(deform_bones):
        ab = bt - bh
        ab_len2 = float(ab @ ab)
        ap = verts[no_gate] - bh
        t = np.clip(ap @ ab / max(ab_len2, 1e-12), 0.0, 1.0)
        proj = bh + t[:, None] * ab
        all_d[no_gate, bi] = np.linalg.norm(verts[no_gate] - proj, axis=1)

k = 4
idx_part = np.argpartition(all_d, k, axis=1)[:, :k]
rows = np.arange(mesh_n)[:, None]
top_d = all_d[rows, idx_part]
w = np.exp(-(top_d / R_SMOOTH) ** 2)
w /= w.sum(axis=1, keepdims=True)

for g in list(mesh.vertex_groups):
    mesh.vertex_groups.remove(g)
vg_map = {bn: mesh.vertex_groups.new(name=bn) for bn, _, _ in deform_bones}
# VertexGroup.add 的 weight 只收标量，逐顶点写入（91k x 4 ≈ 秒级）
bi_of = {bn: bi for bi, (bn, _, _) in enumerate(deform_bones)}
for vi in range(mesh_n):
    for slot in range(k):
        bn = deform_bones[idx_part[vi, slot]][0]
        vg_map[bn].add((vi,), float(w[vi, slot]), "REPLACE")
counts = {g.name: 0 for g in mesh.vertex_groups}
for v in mesh.data.vertices:
    for gseg in v.groups:
        counts[mesh.vertex_groups[gseg.group].name] += 1
print(f"[rig] vertex_group_counts={counts}")
empties = [n for n, c in counts.items() if c == 0]
if empties:
    print(f"[rig] WARNING empty groups: {empties}")

mod = mesh.modifiers.new("Armature", "ARMATURE")
mod.object = arm
mesh.parent = arm



# aim_bone：把骨骼 Y 轴瞄准世界坐标目标点（避免欧拉角盲区瞎转）
# 正确形式：姿态基（rest 局部空间）B = R0⁻¹ @ R1，其中 R1 = Q @ R0，
# Q 为骨架空间里 rest 方向→目标方向的旋转。直接赋 Q 是错的（v10 实证：骨乱飞）。
def aim_bone(bone_name, target):
    pb = arm.pose.bones[bone_name]
    bone = arm.data.bones[bone_name]
    R0 = bone.matrix_local.to_quaternion()
    rest_dir = R0 @ Vector((0, 1, 0))
    want = (Vector(target) - pb.head).normalized()
    Q = rest_dir.rotation_difference(want)
    R1 = Q @ R0
    pb.rotation_mode = "QUATERNION"
    pb.rotation_quaternion = R0.inverted() @ R1


# ---- 渲染骨位自检图（正视=从脸方向 +X，侧视）----
scene = bpy.context.scene
scene.render.engine = "BLENDER_WORKBENCH"
scene.display.shading.light = "STUDIO"
scene.render.resolution_x = 640
scene.render.resolution_y = 640

cam_data = bpy.data.cameras.new("cam")
cam = bpy.data.objects.new("cam", cam_data)
scene.collection.objects.link(cam)
scene.camera = cam
cam_data.type = "ORTHO"
cam_data.ortho_scale = 1.4
center = Vector((0, 0, 0))

# 骨架可视化：object color + show in front
arm.show_in_front = True

for name, offset in {"face": (1.2, 0, 0), "side": (0, 1.2, 0)}.items():
    cam.location = center + Vector(offset)
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"bones_{name}.png")
    bpy.ops.render.render(write_still=True)
    print(f"[rig] rendered bones_{name}")

# ---- 权重可视化（mode=weights）：每顶点按主导骨组上色，直观看分区 ----
if mode == "weights":
    import random
    palette = {}
    rng = random.Random(42)
    for bn, _, _ in deform_bones:
        palette[bn] = (rng.random(), rng.random(), rng.random())
    print(f"[rig] palette={ {bn: tuple(round(c,2) for c in col) for bn, col in palette.items()} }")

# ---- 探针点诊断：关键解剖位置的 Top-4 权重（不看颜色猜，直接看数）----
PROBES = {
    "tail_mid": (-0.30, -0.02, -0.44),
    "tail_tip": (-0.42, -0.02, -0.45),
    "tail_old_pos": (-0.18, -0.45, -0.45),
    "belly_front": (0.25, 0.0, -0.15),
    "chest_front": (0.22, 0.0, 0.05),
    "cheek_L": (0.22, 0.15, 0.15),
    "thigh_L": (0.05, 0.15, -0.35),
    "arm_L_mid": (0.06, 0.33, -0.05),
    "ear_R": (0.11, -0.32, 0.40),
    "shell": (0.18, 0.0, -0.02),
    "crown": (0.05, 0.0, 0.45),
}
for pname, p in PROBES.items():
    d = np.linalg.norm(verts - np.array(p), axis=1)
    vi = int(np.argmin(d))
    tops = []
    for slot in range(k):
        bi = int(idx_part[vi, slot])
        tops.append(f"{deform_bones[bi][0]}={w[vi, slot]:.2f}")
    print(f"[probe] {pname} v{vi} dist={d[vi]:.3f}: {', '.join(tops)}")
if mode == "weights":
    mat = bpy.data.materials.new("wviz")
    mat.use_nodes = False
    mesh.data.materials.clear()
    mesh.data.materials.append(mat)
    mesh.color = (1, 1, 1, 1)
    # 顶点色：按 argmax 组
    dom = idx_part[:, 0]
    col = mesh.data.vertex_colors.new(name="dominant")
    flat = col.data
    dom = idx_part[:, 0]
    for li, lc in enumerate(flat):
        vidx = mesh.data.loops[li].vertex_index
        c = palette[deform_bones[dom[vidx]][0]]
        lc.color = (c[0], c[1], c[2], 1.0)
    mat.use_nodes = True
    bsdf = mat.node_tree.nodes["Principled BSDF"]
    vcol = mat.node_tree.nodes.new("ShaderNodeVertexColor")
    mat.node_tree.links.new(vcol.outputs["Color"], bsdf.inputs["Base Color"])
    scene.display.shading.light = "FLAT"
    scene.display.shading.color_type = "VERTEX"
    for vname, offset in {"face": (1.2, 0, 0), "side": (0, 1.2, 0)}.items():
        cam.location = center + Vector(offset)
        cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(out_dir, f"weights_{vname}.png")
        bpy.ops.render.render(write_still=True)
    # 叠 wave 姿势再渲两张：帆的颜色 = 肇事骨
    bpy.context.view_layer.objects.active = arm
    bpy.ops.object.mode_set(mode="POSE")
    aim_bone("arm.L", (0.20, 0.40, 0.10))
    for vname, offset in {"face": (1.2, 0, 0), "side": (0, 1.2, 0)}.items():
        cam.location = center + Vector(offset)
        cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(out_dir, f"weights_wave_{vname}.png")
        bpy.ops.render.render(write_still=True)
    bpy.ops.object.mode_set(mode="OBJECT")
    print("[rig] weights mode done")
    sys.exit(0)

if mode == "bones":
    # 存 blend 方便人工打开检查
    bpy.ops.wm.save_as_mainfile(filepath=os.path.join(out_dir, "meili_bones.blend"))
    print("[rig] bones mode done")
    sys.exit(0)

# ---- deform 验证：摆姿势渲染 ----
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
POSES = {
    "look":   {"head": (0, math.radians(-25), 0)},                                   # 扭头看人（25°，再大必然穿插）
    "nod":    {"head": (math.radians(20), 0, 0)},                                    # 点头
    "tilt":   {"spine": (0, math.radians(-12), 0), "head": (0, 0, math.radians(15))},  # 歪头（倾听）
    "wave":   {"aim:arm.L": (0.18, 0.38, 0.02), "chest": (0, math.radians(-12), 0)},  # 挥手：臂30°+胸协同12°，缝合带随躯干走
    "leglift": {"aim:thigh.L": (0.10, 0.28, -0.20)},                                 # 抬腿：走/跳前置验证
    "tailup": {"aim:tail.1": (-0.32, -0.02, -0.18), "aim:tail.2": (-0.42, -0.02, -0.08), "aim:tail.3": (-0.46, -0.02, -0.02)},
}

for pose_name, rots in POSES.items():
    bpy.ops.pose.select_all(action="DESELECT")
    for bone, rot in rots.items():
        if bone.startswith("aim:"):
            aim_bone(bone[4:], rot)
            continue
        pb = arm.pose.bones[bone]
        pb.rotation_mode = "XYZ"
        pb.rotation_euler = rot
    for view, offset in {"face": (1.2, 0, 0), "side": (0, 1.2, 0)}.items():
        cam.location = center + Vector(offset)
        cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(out_dir, f"deform_{pose_name}_{view}.png")
        bpy.ops.render.render(write_still=True)
    # 回中
    for bone in rots:
        pb = arm.pose.bones[bone[4:] if bone.startswith("aim:") else bone]
        pb.rotation_euler = (0, 0, 0)
        pb.rotation_quaternion = (1, 0, 0, 0)
    print(f"[rig] deform test {pose_name} rendered")

bpy.ops.object.mode_set(mode="OBJECT")

# ---- 导出（不带动画，骨名保持 L/R；导出选项兼容 5.x）----
out_glb = os.path.join(out_dir, "meili_rigged.glb")
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
arm.select_set(True)
kwargs = dict(
    filepath=out_glb,
    use_selection=True,
    export_yup=True,
    export_animations=False,
    export_skins=True,
)
try:
    bpy.ops.export_scene.gltf(**kwargs, export_def_bones=True)
except TypeError:
    bpy.ops.export_scene.gltf(**kwargs)
print(f"[rig] exported {out_glb}")
print("[rig] done")
