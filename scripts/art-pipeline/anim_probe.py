import json, struct, sys
import numpy as np
p = r"D:\Repos\desktop-pet-meili\public\models\meili.glb"
d = open(p, 'rb').read()
ln = struct.unpack('<I', d[12:16])[0]
g = json.loads(d[20:20+ln])
binofs = 20 + ln

def acc(i):
    a = g['accessors'][i]
    bv = g['bufferViews'][a['bufferView']]
    ofs = binofs + bv.get('byteOffset', 0) + a.get('byteOffset', 0)
    n = a['count']
    t = {5121: np.uint8, 5123: np.uint16, 5125: np.uint32, 5126: np.float32}[a['componentType']]
    comp = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4}[a['type']]
    arr = np.frombuffer(d, dtype=t, count=n*comp, offset=ofs)
    return arr.reshape(n, comp) if comp > 1 else arr

nodes = g['nodes']
skin = g['skins'][0]
joint_names = [nodes[j].get('name', f'#{j}') for j in skin['joints']]

def anim_stats(name):
    a = next(x for x in g['animations'] if x['name'] == name)
    rows = []
    for ch in a['channels']:
        tgt = ch['target']
        node = nodes[tgt['node']]
        s = a['samplers'][ch['sampler']]
        times = acc(s['input']).astype(float)
        vals = acc(s['output']).astype(float)
        if len(vals) == 0: continue
        path = tgt['path']
        if path == 'translation':
            rng = np.abs(vals.max(0) - vals.min(0)).max()
        elif path == 'rotation':
            # 四元数逐帧相对首帧的最大角度
            q0 = vals[0] / (np.linalg.norm(vals[0]) + 1e-12)
            dots = np.abs((vals / (np.linalg.norm(vals, axis=1, keepdims=True) + 1e-12)) @ q0)
            rng = np.degrees(2 * np.arccos(np.clip(dots.min(), -1, 1)))
        else:
            rng = float(np.abs(vals.max() - vals.min()))
        if rng > 1e-3:
            rows.append((node.get('name', '?'), path, round(float(rng), 3), f"{times[0]:.2f}-{times[-1]:.2f}s"))
    rows.sort(key=lambda r: -r[2])
    print(f"== {name} ==")
    for r in rows[:12]:
        print(f"  {r[0]:20s} {r[1]:12s} 幅度 {r[2]:>8}  时间 {r[3]}")
    if not rows:
        print("  （全部通道恒定，无运动）")

for nm in sys.argv[1:] or ['wave', 'jump', 'idle_a']:
    anim_stats(nm)
