# debug_anim_apply.py — 二分定位动画失效环节：REST 基线 / 直接 aim / keyframe 求值
# 用法: blender -b -P scripts/art-pipeline/blender_rig/debug_anim_apply.py -- <input.glb> <out_dir>
import math, os, sys, runpy
import bpy
from mathutils import Euler, Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"), run_name="__main__")
arm, mesh, cam, scene, center = g["arm"], g["mesh"], g["cam"], g["scene"], g["center"]
aim_bone = g["aim_bone"]

# 关掉管线自带的 deform 渲染（我们只要三张自己的对比图）
for h in list(bpy.app.handlers.render_post):
    bpy.app.handlers.render_post.remove(h)

bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

def aim_arm():
    aim_bone("arm.L", (0.18, 0.38, 0.02))

def spec_rest():
    for pb in arm.pose.bones:
        pb.rotation_quaternion = (1, 0, 0, 0)

def render(name):
    scene.frame_set(1)  # 无 action 时摆好的姿势不会被覆盖
    scene.render.filepath = os.path.join(out_dir, name)
    bpy.ops.render.render(write_still=True)

def arm_dir():
    q = arm.pose.bones["arm.L"].matrix.to_quaternion()
    return tuple(round(v, 3) for v in (q @ Vector((0, 1, 0))))

# 0) REST 基线
spec_rest()
print(f"[dbg] REST     arm.L Y轴: {arm_dir()}", flush=True)
render("dbg_0_rest.png")

# 1) 直接 aim（重新计算，不读残留 pose）
spec_rest()
aim_arm()
print(f"[dbg] DIRECT   arm.L Y轴: {arm_dir()}", flush=True)
render("dbg_1_direct.png")

# 2) keyframe 链路：rest 打帧(帧1) → aim 打帧(帧6) → frame_set(6) 求值
spec_rest()
act = bpy.data.actions.new("wave_test")
arm.animation_data_create()
arm.animation_data.action = act
spec_rest()
arm.pose.bones["arm.L"].keyframe_insert("rotation_quaternion", frame=1)
aim_arm()
arm.pose.bones["arm.L"].keyframe_insert("rotation_quaternion", frame=6)
scene.frame_set(6)
print(f"[dbg] KEYED(6) arm.L Y轴: {arm_dir()}   slots={len(act.slots)}", flush=True)
scene.render.filepath = os.path.join(out_dir, "dbg_2_keyed.png")
bpy.ops.render.render(write_still=True)
print("[dbg] done")
