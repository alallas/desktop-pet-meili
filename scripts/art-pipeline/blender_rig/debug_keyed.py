# debug_keyed.py — 同姿势两路径对照：手动摆 pose（deform 测试同款）vs action 关键帧求值
# 用法: blender -b -P scripts/art-pipeline/blender_rig/debug_keyed.py -- <input.glb> <out_dir>
import math, os, runpy, sys
import bpy
from mathutils import Vector, Euler

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

D = math.radians
SPEC = {  # jump f6 同款
    "arm.L": ("aim", (0.14, 0.34, 0.14)), "arm.R": ("aim", (0.14, -0.34, 0.14)),
    "spine": ("aim", (0.12, 0, -0.24)), "head": ("q", (-6, 0, 0)),
}

def apply(spec):
    for bone, (kind, val) in spec.items():
        pb = arm.pose.bones[bone]
        if kind == "q":
            pb.rotation_quaternion = Euler(tuple(D(a) for a in val), "XYZ").to_quaternion()
        else:
            aim_bone(bone, val)

def shot(name):
    scene.frame_set(1)
    cam.location = center + Vector((1.2, 0, 0))
    cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
    scene.render.filepath = os.path.join(out_dir, f"keyed_{name}.png")
    bpy.ops.render.render(write_still=True)
    print(f"[keyed] {name} rendered")

# A: 手动摆姿势，无 action（deform 测试同款路径）
apply(SPEC)
shot("manual")

# B: 清空，走 action 关键帧路径
for pb in arm.pose.bones:
    pb.rotation_quaternion = (1, 0, 0, 0)
act = bpy.data.actions.new("t")
arm.animation_data_create()
arm.animation_data.action = act
apply(SPEC)
for bone in SPEC:
    pb = arm.pose.bones[bone]
    pb.keyframe_insert("rotation_quaternion", frame=6)
    pb.keyframe_insert("location", frame=6)
for pb in arm.pose.bones:
    pb.rotation_quaternion = (1, 0, 0, 0)
scene.frame_set(6)  # 求值应还原 SPEC 姿势
shot("action_f6")
