# glb_orient_probe.py — 无依赖 GLB 顶点探针（只有 numpy + PIL）
# 目的：从多方向深度图判断模型脸朝哪个世界方向（解决 Tripo 导出偏航角问题）
# 用法: python scripts/art-pipeline/glb_orient_probe.py <input.glb> <out_prefix>
import json
import struct
import sys

import numpy as np
from PIL import Image

COMP = {5120: ("b", 1), 5121: ("B", 1), 5122: ("h", 2), 5123: ("H", 2), 5125: ("I", 4), 5126: ("f", 4)}
NCOMP = {"SCALAR": 1, "VEC2": 2, "VEC3": 3, "VEC4": 4, "MAT4": 16}


def load_glb(path):
    with open(path, "rb") as f:
        data = f.read()
    assert data[:4] == b"glTF", "not a glb"
    off = 12
    gltf, bin_ = None, b""
    while off < len(data):
        clen, ctype = struct.unpack_from("<II", data, off)
        chunk = data[off + 8: off + 8 + clen]
        if ctype == 0x4E4F534A:
            gltf = json.loads(chunk)
        elif ctype == 0x004E4942:
            bin_ = chunk
        off += 8 + clen
    return gltf, bin_


def read_accessor(gltf, bin_, idx):
    acc = gltf["accessors"][idx]
    bv = gltf["bufferViews"][acc["bufferView"]]
    fmt, size = COMP[acc["componentType"]]
    n = NCOMP[acc["type"]]
    count = acc["count"]
    start = bv.get("byteOffset", 0) + acc.get("byteOffset", 0)
    stride = bv.get("byteStride", size * n)
    out = np.zeros((count, n), dtype=np.float32)
    for i in range(count):
        vals = struct.unpack_from("<" + fmt * n, bin_, start + i * stride)
        out[i] = vals
    if acc.get("normalized"):
        out /= float(struct.unpack("<" + fmt, struct.pack("<" + fmt, -1 if fmt.islower() else 255))[0])
    return out


def quat_to_mat(q):
    x, y, z, w = q
    return np.array([
        [1 - 2 * (y * y + z * z), 2 * (x * y - z * w), 2 * (x * z + y * w)],
        [2 * (x * y + z * w), 1 - 2 * (x * x + z * z), 2 * (y * z - x * w)],
        [2 * (x * z - y * w), 2 * (y * z + x * w), 1 - 2 * (x * x + y * y)],
    ])


def node_world_matrices(gltf):
    nodes = gltf["nodes"]
    world = [None] * len(nodes)

    def local(nd):
        if "matrix" in nd:
            return np.array(nd["matrix"], dtype=np.float64).reshape(4, 4).T
        m = np.eye(4)
        if "rotation" in nd:
            m[:3, :3] = quat_to_mat(nd["rotation"])
        if "scale" in nd:
            m[:3, :3] = m[:3, :3] @ np.diag(nd["scale"])
        if "translation" in nd:
            m[:3, 3] = nd["translation"]
        return m

    def walk(i, parent):
        w = parent @ local(nodes[i])
        world[i] = w
        for c in nodes[i].get("children", []):
            walk(c, w)

    scene = gltf["scenes"][gltf.get("scene", 0)]
    for r in scene["nodes"]:
        walk(r, np.eye(4))
    return world


def world_vertices(gltf, bin_):
    world = node_world_matrices(gltf)
    verts = []
    for i, nd in enumerate(gltf["nodes"]):
        if "mesh" not in nd or world[i] is None:
            continue
        for prim in gltf["meshes"][nd["mesh"]]["primitives"]:
            pos = read_accessor(gltf, bin_, prim["attributes"]["POSITION"]).astype(np.float64)
            hom = np.hstack([pos, np.ones((len(pos), 1))])
            verts.append((hom @ world[i].T)[:, :3])
    return np.vstack(verts)


def depth_render(pts, axis, sign, res=256):
    """沿 axis(0=x,1=y,2=z) 方向看（sign=+1 从正轴看向原点），返回深度灰度图"""
    if axis == 0:
        # 侧视：屏幕横轴 = 模型 Z，屏幕纵轴 = 模型 Y（up）
        u, v = pts[:, 2], pts[:, 1]
    elif axis == 1:
        u, v = pts[:, 0], pts[:, 2]
    else:
        u, v = pts[:, 0], pts[:, 1]
    d = pts[:, axis] * sign  # 越大越近
    umin, umax = u.min(), u.max()
    vmin, vmax = v.min(), v.max()
    span = max(umax - umin, vmax - vmin)
    img = np.full((res, res), 255, dtype=np.float32)
    px = ((u - umin) / span * (res - 1))
    py = ((vmax - v) / span * (res - 1))  # v 大在上
    dnorm = (d - d.min()) / (d.max() - d.min() + 1e-9)
    order = np.argsort(-dnorm)  # 远的先画，近的后画覆盖
    img = Image.new("L", (res, res), 255)
    from PIL import ImageDraw
    dr = ImageDraw.Draw(img)
    r = 2
    for i in order:
        g = int(255 - 230 * dnorm[i])
        dr.ellipse([px[i] - r, py[i] - r, px[i] + r, py[i] + r], fill=g)
    return img


def main():
    glb, prefix = sys.argv[1], sys.argv[2]
    gltf, bin_ = load_glb(glb)
    pts = world_vertices(gltf, bin_)
    print(f"world bbox min={pts.min(0).round(3)} max={pts.max(0).round(3)}")
    # 头/身分界估计：顶部 45% 为头部区域
    ymin, ymax = pts[:, 1].min(), pts[:, 1].max()
    head = pts[pts[:, 1] > ymin + 0.55 * (ymax - ymin)]
    print(f"head pts={len(head)}")
    views = [("front+Z", 2, 1), ("back-Z", 2, -1), ("right+X", 0, 1), ("left-X", 0, -1), ("top+Y", 1, 1)]
    tiles = []
    for name, ax, sg in views:
        im = depth_render(pts, ax, sg)
        tiles.append((name, im))
    # 拼 5 联图
    w, h = tiles[0][1].size
    mont = Image.new("L", (w * len(tiles), h + 20), 255)
    for i, (nm, im) in enumerate(tiles):
        mont.paste(im, (i * w, 20))
    mont.save(f"{prefix}_montage.png")
    for nm, im in tiles:
        im.save(f"{prefix}_{nm.replace('+','p').replace('-','m')}.png")
    print("saved", f"{prefix}_montage.png")


if __name__ == "__main__":
    main()
