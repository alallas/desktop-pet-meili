# debug_pose4.py — 逐骨打印 pose 矩阵位移量：骨架层 vs 蒙皮层二分
import os, runpy, sys
import bpy

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"),
                   run_name="__main__")
arm = g["arm"]

bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
arm.pose.bones["root"].location = (0, 0.12, 0)
bpy.context.view_layer.update()

print("[dbg4] per-bone pose-matrix translation delta (world):")
for pb in arm.pose.bones:
    rest = arm.data.bones[pb.name].matrix_local.translation
    d = pb.matrix.translation - rest
    print(f"[dbg4]   {pb.name:8s} d=({d.x:+.4f},{d.y:+.4f},{d.z:+.4f})")
