# anim_lbs_truth.py — 不依赖 three.js/Blender，用 GLB 自身数据做 LBS 数值真值：
# 采样 wave 动画各时刻的骨骼世界矩阵，算 hand.L 区域顶点实际位移。
# 顶点动 = three.js 也会动（同数学）；顶点不动 = 导出就是坏的。
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
    comp = {'SCALAR': 1, 'VEC2': 2, 'VEC3': 3, 'VEC4': 4, 'MAT4': 16}[a['type']]
    arr = np.frombuffer(d, dtype=t, count=n*comp, offset=ofs)
    return arr.reshape(n, comp) if comp > 1 else arr

nodes = g['nodes']
skin = g['skins'][0]
joints = skin['joints']
jidx = {j: i for i, j in enumerate(joints)}
IBM = acc(skin['inverseBindMatrices']).reshape(-1, 4, 4).transpose(0, 2, 1)  # glTF 列主序→行主序

# 节点父子
parent = {}
for i, nd in enumerate(nodes):
    for c in nd.get('children', []):
        parent[c] = i

def trs_matrix(nd):
    m = np.eye(4)
    if 'matrix' in nd:
        return np.array(nd['matrix']).reshape(4, 4).T
    t = nd.get('translation', [0, 0, 0])
    r = nd.get('rotation', [0, 0, 0, 1])
    s = nd.get('scale', [1, 1, 1])
    x, y, z, w = r
    R = np.array([
        [1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)],
        [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)],
        [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]])
    m[:3, :3] = R * np.array(s)
    m[:3, 3] = t
    return m

def sample_anim(name, t):
    a = next(x for x in g['animations'] if x['name'] == name)
    overrides = {}
    for ch in a['channels']:
        nd_i = ch['target']['node']
        path = ch['target']['path']
        s = a['samplers'][ch['sampler']]
        times = acc(s['input']).astype(float)
        vals = acc(s['output']).astype(float)
        if t <= times[0]:
            v = vals[0]
        elif t >= times[-1]:
            v = vals[-1]
        else:
            i = np.searchsorted(times, t) - 1
            f = (t - times[i]) / max(times[i+1] - times[i], 1e-9)
            v = vals[i] * (1 - f) + vals[i+1] * f  # 线性插值（旋转近似，判动静足够）
        overrides.setdefault(nd_i, {})[path] = v
    def local(nd_i):
        nd = nodes[nd_i]
        o = overrides.get(nd_i, {})
        m = np.eye(4)
        t_ = o.get('translation', nd.get('translation', [0, 0, 0]))
        r_ = o.get('rotation', nd.get('rotation', [0, 0, 0, 1]))
        s_ = o.get('scale', nd.get('scale', [1, 1, 1]))
        x, y, z, w = r_
        R = np.array([
            [1-2*(y*y+z*z), 2*(x*y-z*w), 2*(x*z+y*w)],
            [2*(x*y+z*w), 1-2*(x*x+z*z), 2*(y*z-x*w)],
            [2*(x*z-y*w), 2*(y*z+x*w), 1-2*(x*x+y*y)]])
        m[:3, :3] = R * np.array(s_)
        m[:3, 3] = t_
        return m
    world = {}
    def world_of(i):
        if i in world:
            return world[i]
        m = local(i)
        if i in parent:
            m = world_of(parent[i]) @ m
        world[i] = m
        return m
    return {j: world_of(j) for j in joints}

# 顶点数据（取第一个蒙皮 primitive）
pr = g['meshes'][0]['primitives'][0]
pos = acc(pr['attributes']['POSITION']).astype(float)
ji = acc(pr['attributes']['JOINTS_0']).astype(int)
wt = acc(pr['attributes']['WEIGHTS_0']).astype(float)
# 骨架引用可能不是全关节表（def_bones 导出），按实际 joints 表映射
ji = np.array([np.where(joints == v)[0][0] if v in joints else 0 for v in ji.flat]).reshape(ji.shape)
nm = nodes[joints[0]].get('name')

hand_j = None
root_j = None
for j in joints:
    n = nodes[j].get('name', '')
    if n == 'hand.L':
        hand_j = j
    if n == 'root':
        root_j = j
print(f"hand.L joint idx={jidx[hand_j]}, root idx={jidx[root_j]}")

# 找受 hand.L 主导的顶点
mask = (ji == jidx[hand_j]).any(1) & (wt[np.arange(len(wt)), np.argmax(ji == jidx[hand_j], axis=1)] > 0.3)
sel = np.where(mask)[0]
if len(sel) == 0:
    sel = np.where((ji == jidx[hand_j]).any(1))[0]
print(f"hand.L 主导顶点数: {len(sel)}")

def skinned(t, anim):
    W = sample_anim(anim, t)
    JM = {j: W[j] @ IBM[jidx[j]] for j in joints}
    out = np.zeros((len(sel), 3))
    for k, vi in enumerate(sel):
        acc_ = np.zeros(3)
        for c in range(4):
            j = joints[ji[vi, c]]
            m = JM[j]
            v4 = np.append(pos[vi], 1)
            acc_ += wt[vi, c] * (m @ v4)[:3]
        out[k] = acc_
    return out

for anim in sys.argv[1:] or ['wave']:
    base = skinned(0.001, anim)
    print(f"== {anim} ==")
    for t in [0.2, 0.4, 0.6, 0.8, 1.0]:
        p1 = skinned(t, anim)
        delta = np.linalg.norm(p1 - base, axis=1)
        print(f"  t={t:.1f}s  相对t=0位移: mean={delta.mean():.4f} max={delta.max():.4f}")
