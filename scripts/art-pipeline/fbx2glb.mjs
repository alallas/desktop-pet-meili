// FBX → GLB 转换器（Tripo p2-quad 产物实为 FBX，网关误标 gltf-binary，坑 54）
// 用法: node scripts/art-pipeline/fbx2glb.mjs <input.fbx> <output.glb>
import * as THREE from 'three';
import { FBXLoader } from 'three/examples/jsm/loaders/FBXLoader.js';
import { GLTFExporter } from 'three/examples/jsm/exporters/GLTFExporter.js';
import { readFileSync, writeFileSync } from 'node:fs';

// Node 无 FileReader（GLTFExporter 二进制输出依赖），用 Blob.arrayBuffer() 打补丁
globalThis.FileReader ??= class {
  readAsArrayBuffer(blob) {
    blob.arrayBuffer().then((ab) => {
      this.result = ab;
      this.onloadend?.();
      this.onload?.();
    });
  }
  readAsDataURL(blob) {
    blob.arrayBuffer().then((ab) => {
      this.result = `data:${blob.type};base64,${Buffer.from(ab).toString('base64')}`;
      this.onloadend?.();
      this.onload?.();
    });
  }
};

const [, , src, dst] = process.argv;
if (!src || !dst) {
  console.error('usage: node fbx2glb.mjs <input.fbx> <output.glb>');
  process.exit(1);
}

const buf = readFileSync(src);
const arrayBuffer = buf.buffer.slice(buf.byteOffset, buf.byteOffset + buf.byteLength);
const group = new FBXLoader().parse(arrayBuffer, '');

let tris = 0;
let meshes = 0;
group.updateMatrixWorld(true);
group.traverse((o) => {
  if (o.isMesh) {
    meshes += 1;
    const g = o.geometry;
    tris += (g.index ? g.index.count : g.attributes.position.count) / 3;
  }
});
console.log(`meshes: ${meshes}, triangles: ${tris}`);
const box = new THREE.Box3().setFromObject(group);
console.log('bounds min:', box.min.toArray().map((v) => v.toFixed(2)).join(','));
console.log('bounds max:', box.max.toArray().map((v) => v.toFixed(2)).join(','));

new GLTFExporter().parse(
  group,
  (result) => {
    writeFileSync(dst, Buffer.from(result));
    console.log('saved:', dst);
  },
  (err) => {
    console.error('export failed:', err);
    process.exit(1);
  },
  { binary: true },
);
