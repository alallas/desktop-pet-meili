# debug_pose.py — 定位撕裂元凶：root 位移 vs aim 抬臂 各渲一张对照
# 用法: blender -b -P scripts/art-pipeline/blender_rig/debug_pose.py -- <input.glb> <out_dir>
import math, os, runpy, sys
import bpy
from mathutils import Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"),
                   run_name="__main__")
arm, cam, scene, center, aim_bone = g["arm"], g["cam"], g["scene"], g["center"], g["aim_bone"]

bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

def shot(name):
    scene.frame_set(1)
    for v, off in {"face": (1.2, 0, 0), "side": (0, 1.2, 0)}.items():
        cam.location = center + Vector(off)
        cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(out_dir, f"dbg_{name}_{v}.png")
        bpy.ops.render.render(write_still=True)
    print(f"[dbg] {name} rendered")

# A: 纯 root 位移 +0.12（jump 峰值同款）
arm.pose.bones["root"].location = (0, 0.12, 0)
shot("root_up")
arm.pose.bones["root"].location = (0, 0, 0)

# B: aim 抬臂到 flutter 峰值 (0.18,0.47,0.08) + chest -11°（wave f16 同款）
aim_bone("arm.L", (0.18, 0.47, 0.08))
import mathutils
arm.pose.bones["chest"].rotation_quaternion = mathutils.Euler((0, math.radians(-11), 0), "XYZ").to_quaternion()
shot("arm_flutter")

# C: 找出变形后跑出合理范围的"肇事顶点"，打印 Top-4 权重归属
bpy.context.view_layer.update()
import numpy as np
me = g["mesh"]
deg = bpy.context.evaluated_depsgraph_get()
ev = me.evaluated_get(deg)
ev_mesh = ev.to_mesh()
n = len(ev_mesh.vertices)
co = np.empty(n * 3, dtype=np.float64)
ev_mesh.vertices.foreach_get("co", co)
co = co.reshape(-1, 3)
mw = np.array(ev.matrix_world)
world = (mw[:3, :3] @ co.T).T + mw[:3, 3]
# 脸部撕裂观察范围：|x|>0.6 或 |y|>0.6 或 z 越界即为飞出
fly = np.where((np.abs(world[:, 0]) > 0.6) | (np.abs(world[:, 1]) > 0.6) |
               (world[:, 2] > 0.6) | (world[:, 2] < -0.6))[0]
print(f"[dbg] flyaway verts: {len(fly)}")
# 变形网格顶点与原始网格按序对应，权重用原始 mesh 的 vertex_groups
groups = {gidx: g.name for gidx, g in enumerate(me.vertex_groups)}
from collections import Counter
cnt = Counter()
for vi in fly[:5000]:
    tops = sorted(((gseg.weight, groups[gseg.group]) for gseg in me.data.vertices[vi].groups),
                  reverse=True)[:4]
    cnt[tuple(b for _, b in tops)] += 1
for combo, c in cnt.most_common(10):
    print(f"[dbg]   {c:5d} verts top-bones: {combo}")
ev.to_mesh_clear()
