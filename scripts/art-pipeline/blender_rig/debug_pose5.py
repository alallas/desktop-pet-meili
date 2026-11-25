# debug_pose5.py — 验证活体网格每个顶点的权重和（理论=1）
import os, runpy, sys
import bpy
import numpy as np

argv = sys.argv[sys.argv.index("--") + 1:]
src, out_dir = os.path.abspath(argv[0]), os.path.abspath(argv[1])
os.makedirs(out_dir, exist_ok=True)

sys.argv = ["rig_meili.py", "--", "rig", src, out_dir]
g = runpy.run_path(os.path.join(os.path.dirname(os.path.abspath(__file__)), "rig_meili.py"),
                   run_name="__main__")
mesh = g["mesh"]

sums = np.zeros(len(mesh.data.vertices))
ngroups = np.zeros(len(mesh.data.vertices), dtype=int)
for gi, vg in enumerate(mesh.vertex_groups):
    for v in mesh.data.vertices:
        for s in v.groups:
            if s.group == gi:
                sums[v.index] += s.weight
                ngroups[v.index] += 1
print("[dbg5] weight sums: min={:.4f} max={:.4f} mean={:.4f}".format(sums.min(), sums.max(), sums.mean()))
print("[dbg5] verts with sum<0.99:", int((sums < 0.99).sum()), " sum>1.01:", int((sums > 1.01).sum()))
print("[dbg5] groups-per-vert histogram:", {int(n): int((ngroups == n).sum()) for n in np.unique(ngroups)})

# 抽查 5 个低和顶点的详情
bad = np.where(sums < 0.99)[0][:5]
groups = {gi: gr.name for gi, gr in enumerate(mesh.vertex_groups)}
for vi in bad:
    w = sorted(((round(s.weight, 3), groups[s.group]) for s in mesh.data.vertices[vi].groups), reverse=True)
    print(f"[dbg5]   v{vi} sum={sums[vi]:.3f} weights={w}")
