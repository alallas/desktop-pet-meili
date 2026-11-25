# animate_meili.py — meili 程序化动画（复用 rig_meili.py 绑皮，同进程打关键帧）
# 用法: blender -b -P scripts/art-pipeline/blender_rig/animate_meili.py -- <input.glb> <out_dir>
# 产出: <out_dir>/meili_animated.glb（NLA_TRACKS 导出，每 clip 一条 track）
#       <out_dir>/anim_<clip>_<frame>_<view>.png 预览自检图
#
# 设计依据（deform 测试实证，见 rig_meili.py POSES）：
#   扭头 head(0,∓25°,0)；点头 head(±20°,0,0)；歪头 head(0,0,15°)+spine(0,∓10°,0)
#   挥手 aim:arm.L(0.18,0.38,0.02)+chest(0,∓12°,0)；更大角度线性蒙皮必撕，不越界。
# root 骨平移禁用（踩坑 59）：全骨矩阵/权重和均验证无误，但顶点位移仍按骨组合部分
# 跟随（疑似 5.3 armature modifier 平移通道 bug）；跳跃高度改由应用层抛物线实现。
import math
import os
import runpy
import sys

import bpy
from mathutils import Euler, Vector

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

# ---- 复用绑骨管线（bones→蒙皮→deform 自检→导出 rigged.glb），返回其全局命名空间 ----
sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"),
                   run_name="__main__")
arm, mesh, cam, scene, center = g["arm"], g["mesh"], g["cam"], g["scene"], g["center"]
aim_bone = g["aim_bone"]
Vector = Vector

D = math.radians
scene.render.fps = 30
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
for pb in arm.pose.bones:
    pb.rotation_mode = "QUATERNION"

# 姿态值约定：("q", (rx,ry,rz)) 局部欧拉(度)→四元数；("aim", (x,y,z)) 骨 Y 轴瞄准世界点；
#            ("loc", (x,y,z)) 骨骼局部位移（root 用，Y=世界高度）
def apply_spec(spec):
    for bone, (kind, val) in spec.items():
        pb = arm.pose.bones[bone]
        if kind == "q":
            pb.rotation_quaternion = Euler(tuple(D(a) for a in val), "XYZ").to_quaternion()
        elif kind == "aim":
            aim_bone(bone, val)
        elif kind == "loc":
            pb.location = val

def key_spec(frame, spec):
    apply_spec(spec)
    for bone in spec:
        pb = arm.pose.bones[bone]
        pb.keyframe_insert("rotation_quaternion", frame=frame)
        pb.keyframe_insert("location", frame=frame)

def reset_pose():
    for pb in arm.pose.bones:
        pb.rotation_quaternion = (1, 0, 0, 0)
        pb.location = (0, 0, 0)

def render_preview(clip, frame, views=("face",)):
    scene.frame_set(frame)
    for v in views:
        off = (1.2, 0, 0) if v == "face" else (0, 1.2, 0)
        cam.location = center + Vector(off)
        cam.rotation_euler = (center - cam.location).to_track_quat("-Z", "Y").to_euler()
        scene.render.filepath = os.path.join(out_dir, f"anim_{clip}_{frame:03d}_{v}.png")
        bpy.ops.render.render(write_still=True)

# 尾尖三连（左扫/右扫）——aim 目标沿 Y 摆动，幅度小不撕
TAIL_L = {"tail.1": ("aim", (-0.28, -0.10, -0.42)), "tail.2": ("aim", (-0.40, -0.12, -0.44)),
          "tail.3": ("aim", (-0.48, -0.12, -0.45))}
TAIL_R = {"tail.1": ("aim", (-0.28, 0.08, -0.42)), "tail.2": ("aim", (-0.40, 0.10, -0.44)),
          "tail.3": ("aim", (-0.48, 0.10, -0.45))}
TAIL_C = {"tail.1": ("aim", (-0.28, -0.02, -0.42)), "tail.2": ("aim", (-0.40, -0.02, -0.44)),
          "tail.3": ("aim", (-0.48, -0.02, -0.45))}
REST = {}  # 全回中（reset_pose 已保证）

# (clip, loop, [(frame, spec), ...], [预览帧...])
# 安全包络（deform 测试实证）：扭头≤25°、点头≤20°、歪头≤15°+spine≤12°、
# 挥臂 aim≈(0.18,0.38,0.02)+chest≤12°、抬腿 aim≈(0.10,0.28,-0.20)、tailup 三节抬高。
# 超包络必撕（armpit/腹股沟 candy-wrapper）：腿骨不能折叠、手臂不能高过肩，
# 所有 aim 目标都压在验证值附近，竖直位移一律不做（踩坑 59，由应用层弹跳补足）。
ARMS_UP = {"arm.L": ("aim", (0.15, 0.34, 0.06)), "arm.R": ("aim", (0.15, -0.34, 0.06))}
ARMS_MID = {"arm.L": ("aim", (0.14, 0.32, 0.02)), "arm.R": ("aim", (0.14, -0.32, 0.02))}
ARMS_LOW = {"arm.L": ("aim", (0.12, 0.31, -0.04)), "arm.R": ("aim", (0.12, -0.31, -0.04))}
CROUCH = {"thigh.L": ("aim", (0.10, 0.16, -0.36)), "thigh.R": ("aim", (0.10, -0.16, -0.36))}

CLIPS = [
    ("idle_a", True, [
        (1,  dict(TAIL_L, chest=("q", (0, 0, 0)), head=("q", (0, 0, 0)))),
        (30, dict(TAIL_L, chest=("q", (3, 0, 0)), head=("q", (0, 0, 5)))),
        (45, dict(TAIL_R)),
        (60, dict(TAIL_R, chest=("q", (0, 0, 0)), head=("q", (0, 0, 0)))),
        (75, dict(TAIL_C, chest=("q", (3, 0, 0)), head=("q", (0, -8, 0)))),
        (90, dict(TAIL_L, chest=("q", (0, 0, 0)), head=("q", (0, 0, 0)))),
    ], [30, 45]),
    ("wave", False, [
        (1,  REST),
        (6,  {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (14, {"arm.L": ("aim", (0.14, 0.32, -0.02)), "chest": ("q", (0, -4, 0))}),
        (22, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (30, {"arm.L": ("aim", (0.14, 0.32, -0.02)), "chest": ("q", (0, -4, 0))}),
        (38, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (46, REST),
    ], [6, 14]),
    ("jump", False, [  # 兴奋连挥（完全复用 wave 验证姿势，不镜像）+ 应用层抛物线=跳跃感
        (1,  REST),
        (4,  {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (10, REST),
        (16, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (22, REST),
        (28, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (34, REST),
    ], [4, 16]),
    ("sit", False, [  # 坐歇=抬头静听姿态（无手臂/无腿弯/无spine aim，全部验证安全骨）
        (1,  REST),
        (20, {"chest": ("q", (-4, 0, 0)), "head": ("q", (0, -12, 0)), **TAIL_C}),
        (60, {"chest": ("q", (-4, 0, 0)), "head": ("q", (0, -12, 0)), **TAIL_C}),
    ], [30]),
    ("sleep", True, [  # 头侧垂≤15°（歪头验证值）+尾卷+慢呼吸
        (1,  {"head": ("q", (3, 0, 14)), "spine": ("aim", (0.05, 0, -0.31)),
              "tail.1": ("aim", (-0.30, -0.04, -0.36))}),
        (45, {"head": ("q", (6, 0, 15)), "spine": ("aim", (0.05, 0, -0.31)),
              "tail.1": ("aim", (-0.30, -0.04, -0.36)), "chest": ("q", (2, 0, 0))}),
        (90, {"head": ("q", (3, 0, 14)), "spine": ("aim", (0.05, 0, -0.31)),
              "tail.1": ("aim", (-0.30, -0.04, -0.36))}),
    ], [45]),
    ("petted", True, [
        (1,  {"head": ("q", (0, 0, 14))}),
        (15, {"head": ("q", (3, 0, 15)), "ear.L": ("q", (0, 0, 8))}),
        (30, {"head": ("q", (0, 0, 14)), "ear.R": ("q", (0, 0, 6))}),
        (45, {"head": ("q", (3, 0, 15))}),
        (60, {"head": ("q", (0, 0, 14))}),
    ], [15]),
    ("listen", False, [  # 歪头 15°+spine-10°（tilt 验证值）+微点头
        (1,  REST),
        (12, {"head": ("q", (0, 0, 15)), "spine": ("q", (0, -10, 0))}),
        (30, {"head": ("q", (10, 0, 15)), "spine": ("q", (0, -10, 0))}),
        (40, {"head": ("q", (2, 0, 15)), "spine": ("q", (0, -10, 0))}),
        (52, {"head": ("q", (10, 0, 15)), "spine": ("q", (0, -10, 0))}),
        (75, REST),
    ], [12, 30]),
    ("special_rub_face", False, [  # 蹭脸：wave 验证姿势 + 歪头（臂值必须精确=(0.18,0.38,0.02)，邻近值必撕）
        (1,  REST),
        (10, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0)), "head": ("q", (0, 0, 10))}),
        (30, {"head": ("q", (0, 0, -10)), "ear.R": ("q", (0, 0, -8))}),
        (50, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0)), "head": ("q", (0, 0, 10))}),
        (60, REST),
    ], [10, 30]),
    ("special_gift_shell", False, [  # 外向抬爪递出+抬头（内向压缩必撕，弃）
        (1,  REST),
        (12, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "head": ("q", (-8, 0, 0))}),
        (48, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "head": ("q", (-8, 0, 0))}),
        (62, REST),
    ], [12]),
    ("special_explain", True, [  # 完全复用 jump/wave 已验证姿势序列 + 一档 jaw 开合
        (1,  REST),
        (4,  {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (10, REST),
        (16, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0)), "jaw": ("q", (0, -10, 0))}),
        (22, REST),
        (28, {"arm.L": ("aim", (0.18, 0.38, 0.02)), "chest": ("q", (0, -12, 0))}),
        (34, REST),
    ], [4, 16]),
]

arm.animation_data_create()
for clip_name, loop, keys, previews in CLIPS:
    reset_pose()
    act = bpy.data.actions.new(clip_name)
    arm.animation_data.action = act
    for frame, spec in keys:
        key_spec(frame, spec)
    # 预览（action 还挂在 arm 上，直接摆帧渲染）
    for pf in previews:
        views = ("face", "side") if clip_name == "special_rub_face" and pf in (30, 40) else ("face",)
        render_preview(clip_name, pf, views)
    # 推入 NLA 后立即静音：否则后续 clip 预览/求值时旧 track 会叠加污染姿势
    track = arm.animation_data.nla_tracks.new()
    track.name = clip_name
    track.strips.new(clip_name, int(keys[0][0]), act)
    track.mute = True
    arm.animation_data.action = None
    # Blender 5.x Action 走 layer/slot 结构，fcurves 不在 action 根上；打印 key 数即可
    print(f"[anim] clip {clip_name}: {len(keys)} keys")

reset_pose()
bpy.ops.object.mode_set(mode="OBJECT")

# 导出前解除全部静音（glTF NLA_TRACKS 模式跳过 muted track；导出器逐 track 隔离采样）
for tr in arm.animation_data.nla_tracks:
    tr.mute = False

# ---- 导出：NLA_TRACKS 模式，每 track 一个动画 ----
out_glb = os.path.join(out_dir, "meili_animated.glb")
bpy.ops.object.select_all(action="DESELECT")
mesh.select_set(True)
arm.select_set(True)
kwargs = dict(
    filepath=out_glb,
    use_selection=True,
    export_yup=True,
    export_animations=True,
    export_animation_mode="NLA_TRACKS",
    export_skins=True,
)
try:
    bpy.ops.export_scene.gltf(**kwargs, export_def_bones=True)
except TypeError:
    bpy.ops.export_scene.gltf(**kwargs)
print(f"[anim] exported {out_glb}")
print("[anim] done")
