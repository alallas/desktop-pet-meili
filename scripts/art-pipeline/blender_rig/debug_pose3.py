# debug_pose3.py — 顶点级位移直方图：按主导骨统计 root 平移的跟随量
# 用法: blender -b -P scripts/art-pipeline/blender_rig/debug_pose3.py -- <input.glb> <out_dir>
import os, runpy, sys
import bpy
import numpy as np

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"),
                   run_name="__main__")
arm, mesh = g["arm"], g["mesh"]

bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

def eval_world():
    bpy.context.view_layer.update()
    deg = bpy.context.evaluated_depsgraph_get()
    ev = mesh.evaluated_get(deg)
    em = ev.to_mesh()
    n = len(em.vertices)
    co = np.empty(n * 3, dtype=np.float64)
    em.vertices.foreach_get("co", co)
    ev.to_mesh_clear()
    co = co.reshape(-1, 3)
    mw = np.array(ev.matrix_world)
    return (mw[:3, :3] @ co.T).T + mw[:3, 3]

groups = {gi: gr.name for gi, gr in enumerate(mesh.vertex_groups)}
dom = np.empty(len(mesh.data.vertices), dtype=object)
for vi, v in enumerate(mesh.data.vertices):
    dom[vi] = max(((s.weight, groups[s.group]) for s in v.groups))[1]

rest_w = eval_world()
arm.pose.bones["root"].location = (0, 0.12, 0)
up_w = eval_world()
disp = up_w - rest_w
print("[dbg3] displacement z: mean={:.4f} std={:.4f} min={:.4f} max={:.4f}".format(
    disp[:, 2].mean(), disp[:, 2].std(), disp[:, 2].min(), disp[:, 2].max()))
print("[dbg3] displacement x/y max abs: {:.5f} {:.5f}".format(
    np.abs(disp[:, 0]).max(), np.abs(disp[:, 1]).max()))
from collections import Counter
stat = {}
for i, b in enumerate(dom):
    s = stat.setdefault(b, [0.0, 0])
    s[0] += disp[i, 2]
    s[1] += 1
for b, (tot, n) in sorted(stat.items(), key=lambda kv: kv[1][0] / kv[1][1]):
    print(f"[dbg3]   {b:8s} mean_dz={tot / n:+.4f}  n={n}")
