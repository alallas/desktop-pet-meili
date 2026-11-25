# debug_minimal.py — 立方体+两级骨，root 平移是否均匀带动全部顶点？
import bpy
from mathutils import Vector

bpy.ops.wm.read_factory_settings(use_empty=True)
sc = bpy.context.scene

mesh = bpy.data.objects.new("cube", bpy.data.meshes.new("cube"))
bpy.context.scene.collection.objects.link(mesh)
import bmesh
bm = bmesh.new()
bmesh.ops.create_cube(bm, size=0.2)
bm.to_mesh(mesh.data)
bm.free()

ad = bpy.data.armatures.new("a")
arm = bpy.data.objects.new("a", ad)
bpy.context.scene.collection.objects.link(arm)
bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="EDIT")
b1 = ad.edit_bones.new("root")
b1.head = (0, 0, -0.3); b1.tail = (0, 0, -0.1)
b2 = ad.edit_bones.new("top")
b2.head = (0, 0, 0.0); b2.tail = (0, 0, 0.2)
b2.parent = b1
bpy.ops.object.mode_set(mode="OBJECT")

vg = mesh.vertex_groups.new(name="top")
vs = [v.index for v in mesh.data.vertices if v.co.z > 0]
vg.add(vs, 1.0, "REPLACE")
vg2 = mesh.vertex_groups.new(name="root")
vs2 = [v.index for v in mesh.data.vertices if v.co.z <= 0]
vg2.add(vs2, 1.0, "REPLACE")

mod = mesh.modifiers.new("Arm", "ARMATURE")
mod.object = arm
mesh.parent = arm

bpy.context.view_layer.objects.active = arm
bpy.ops.object.mode_set(mode="POSE")
arm.pose.bones["root"].location = (0, 0.12, 0)
bpy.context.view_layer.update()

import numpy as np
deg = bpy.context.evaluated_depsgraph_get()
ev = mesh.evaluated_get(deg)
em = ev.to_mesh()
n = len(em.vertices)
co = np.empty(n * 3)
em.vertices.foreach_get("co", co)
co = co.reshape(-1, 3)
dz = co[:, 2] - np.array([v.co.z for v in mesh.data.vertices])
print("[min] dz per vert:", np.round(dz, 4))
ev.to_mesh_clear()
