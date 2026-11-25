# debug_pose2.py — 撕裂定位 v2：rest 对照 + 放宽阈值 + 分姿势统计肇事顶点
# 用法: blender -b -P scripts/art-pipeline/blender_rig/debug_pose2.py -- <input.glb> <out_dir>
import math, os, runpy, sys
import bpy
from mathutils import Vector, Euler

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"),
                   run_name="__main__")
arm, cam, scene, center, aim_bone, mesh = g["arm"], g["cam"], g["scene"], g["center"], g["aim_bone"], g["mesh"]

bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

def shot(name):
    scene.frame_set(1)
    cam.location = center + Vector((1.2, 0, 0))
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"dbg2_{name}_face.png")
    bpy.ops.render.render(write_still=True)
    print(f"[dbg2] {name} rendered")

def flyaway_report(tag, thresh=0.45):
    bpy.context.view_layer.update()
    import numpy as np
    deg = bpy.context.evaluated_depsgraph_get()
    ev = mesh.evaluated_get(deg)
    em = ev.to_mesh()
    n = len(em.vertices)
    co = np.empty(n * 3, dtype=np.float64)
    em.vertices.foreach_get("co", co)
    co = co.reshape(-1, 3)
    mw = np.array(ev.matrix_world)
    world = (mw[:3, :3] @ co.T).T + mw[:3, 3]
    groups = {gi: gr.name for gi, gr in enumerate(mesh.vertex_groups)}
    from collections import Counter

    def dom(vi):
        return max(((s.weight, groups[s.group]) for s in mesh.data.vertices[vi].groups))[1]

    # 帆区精查：rest 模型 |y|≤0.42，y>0.42 的顶点必属手臂区异常
    sail = np.where(world[:, 1] > 0.42)[0]
    cnt = Counter(dom(vi) for vi in sail)
    print(f"[dbg2] {tag}: y>0.42 verts = {len(sail)}; dominant: {cnt.most_common(6)}")
    # 打印最飞出的 5 个顶点详情
    if len(sail):
        top = sail[np.argsort(world[sail, 1])[-5:][::-1]]
        for vi in top:
            w = sorted(((round(s.weight, 2), groups[s.group]) for s in mesh.data.vertices[vi].groups),
                       reverse=True)[:4]
            print(f"[dbg2]   v{vi} world=({world[vi,0]:.2f},{world[vi,1]:.2f},{world[vi,2]:.2f}) weights={w}")
    ev.to_mesh_clear()
    return len(sail)

shot("rest")
flyaway_report("rest")

arm.pose.bones["root"].location = (0, 0.12, 0)
shot("root_up")
flyaway_report("root_up")
arm.pose.bones["root"].location = (0, 0, 0)

aim_bone("arm.L", (0.18, 0.47, 0.08))
arm.pose.bones["chest"].rotation_quaternion = Euler((0, math.radians(-11), 0), "XYZ").to_quaternion()
shot("arm_flutter")
flyaway_report("arm_flutter")
