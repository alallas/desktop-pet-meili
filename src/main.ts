import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";

const debug = document.getElementById("debug")!;
const win = getCurrentWindow();

// Tauri v2 无查询接口，点击穿透状态由 JS 侧自行记录（默认值与 Rust 侧 setup 一致）
let clickThrough = true;

// ---------- Three.js 场景 ----------
const scene = new THREE.Scene();

const camera = new THREE.PerspectiveCamera(40, innerWidth / innerHeight, 0.1, 100);
camera.position.set(0, 1.1, 4.2);
camera.lookAt(0, 1.1, 0);

const renderer = new THREE.WebGLRenderer({ alpha: true, antialias: true, preserveDrawingBuffer: true });
renderer.setPixelRatio(devicePixelRatio);
renderer.setSize(innerWidth, innerHeight);
renderer.setClearColor(0x000000, 0); // 全透明，桌面透出
document.body.appendChild(renderer.domElement);

scene.add(new THREE.AmbientLight(0xffffff, 1.2));
const key = new THREE.DirectionalLight(0xfff2e0, 2.2);
key.position.set(2, 4, 3);
scene.add(key);

// ---------- 模型 / 动画状态机 ----------
// M0 测试模型：RobotExpressive（three.js 示例，MIT 授权，占位用，正式形象为 3D 水獭 meili）
type AnimName = "Idle" | "Walking" | "Dance" | "Wave";
const STATE_KEYS: Record<string, AnimName> = { "1": "Idle", "2": "Walking", "3": "Dance", "4": "Wave" };

let mixer: THREE.AnimationMixer | null = null;
const actions = new Map<AnimName, THREE.AnimationAction>();
let current: AnimName = "Idle";
let headBone: THREE.Bone | null = null;
let bboxInfo = "?";

// 桌宠锚点：模型挂在锚点下并居中，锚点负责把桌宠摆到屏幕指定 NDC 位置
// M0 调试期固定在右下角（不挡用户工作区），未来改为沿屏幕边缘游走
const PET_NDC = { x: 0.72, y: -0.62 };
const PET_HEIGHT = 1.5; // 世界单位
const petAnchor = new THREE.Group();
scene.add(petAnchor);

function layoutPet() {
  // 相机前方 z=0 平面上的可视半高/半宽
  const halfH = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * camera.position.z;
  const halfW = halfH * camera.aspect;
  const cx = PET_NDC.x * halfW;
  const cy = camera.position.y + PET_NDC.y * halfH; // 相机视线中心在 (0, 1.1, 0)
  petAnchor.position.set(cx, cy - PET_HEIGHT / 2, 0);
}

new GLTFLoader().load("/models/RobotExpressive.glb", (gltf) => {
  const model = gltf.scene;

  // 自动适配：统一缩放到 PET_HEIGHT、脚底贴锚点底、水平居中
  const box = new THREE.Box3().setFromObject(model);
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  const scale = PET_HEIGHT / size.y;
  model.scale.setScalar(scale);
  model.position.set(-center.x * scale, -box.min.y * scale, -center.z * scale);
  bboxInfo = `${size.x.toFixed(1)}x${size.y.toFixed(1)}x${size.z.toFixed(1)}`;
  petAnchor.add(model);
  layoutPet();

  mixer = new THREE.AnimationMixer(model);
  for (const clip of gltf.animations) {
    const name = clip.name as AnimName;
    if (["Idle", "Walking", "Dance", "Wave"].includes(name)) {
      actions.set(name, mixer.clipAction(clip));
    }
  }
  actions.get("Idle")?.play();

  model.traverse((o) => {
    if (!headBone && o instanceof THREE.Bone && o.name === "Head") headBone = o;
  });

  renderer.setAnimationLoop(tick);
});

function play(name: AnimName) {
  if (name === current || !mixer) return;
  const from = actions.get(current);
  const to = actions.get(name);
  if (!to) return;
  to.reset().fadeIn(0.3).play();
  from?.fadeOut(0.3);
  current = name;
}

// ---------- 光标跟随（点击穿透下轮询 Rust 侧全局光标）----------
const cursorNDC = new THREE.Vector2(0, -0.5); // 默认看向下方（"坐着"的感觉）
const cursorTarget = new THREE.Vector3(0, 0.6, 2);
let cursorScreen: [number, number] | null = null;

async function pollCursor() {
  const [cx, cy] = (await invoke("get_cursor_position")) as [number, number];
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  cursorScreen = [cx, cy];
  // 屏幕坐标 → 窗口 NDC
  const x = ((cx - pos.x) / size.width) * 2 - 1;
  const y = -(((cy - pos.y) / size.height) * 2 - 1);
  cursorNDC.set(x, y);
}
setInterval(pollCursor, 100);

// ---------- 视线 IK：头部朝向光标，限制在正面锥形范围内 ----------
const _animQ = new THREE.Quaternion();
const _lookQ = new THREE.Quaternion();
const _m = new THREE.Matrix4();
const _headPos = new THREE.Vector3();
const _dir = new THREE.Vector3();
const UP = new THREE.Vector3(0, 1, 0);

function applyGaze() {
  if (!headBone) return;
  // 光标 NDC → 面前固定深度处的世界坐标
  cursorTarget.set(cursorNDC.x, cursorNDC.y, 0.5).unproject(camera);

  headBone.getWorldPosition(_headPos);
  _dir.copy(cursorTarget).sub(_headPos);

  // 只追踪正面 150° 锥形范围，身后不扭头
  if (_dir.z < -0.2 * _dir.length()) return;

  // 记录动画姿态，与注视姿态插值混合（避免 IK 生硬）
  _animQ.copy(headBone.quaternion);
  _m.lookAt(_headPos, cursorTarget, UP);
  _lookQ.setFromRotationMatrix(_m);
  // lookAt 得到的是世界旋转，需换算到父空间
  const parent = headBone.parent!;
  parent.updateWorldMatrix(true, false);
  _lookQ.premultiply(parent.getWorldQuaternion(new THREE.Quaternion()).invert());
  headBone.quaternion.copy(_animQ).slerp(_lookQ, 0.5);
}

// ---------- 主循环 ----------
const clock = new THREE.Clock();
let frames = 0;
let fpsTime = 0;
let fps = 0;

function tick() {
  const dt = clock.getDelta();
  mixer?.update(dt);
  applyGaze(); // 必须在 mixer.update 之后，动画每帧会覆写骨骼旋转
  renderer.render(scene, camera);

  frames++;
  fpsTime += dt;
  if (fpsTime >= 0.5) {
    fps = Math.round(frames / fpsTime);
    frames = 0;
    fpsTime = 0;
    refresh();
  }
}

// ---------- 调试面板与快捷键 ----------
async function refresh() {
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  const canvasEl = renderer.domElement;
  const dbgRect = debug.getBoundingClientRect();
  const info = [
    `潮间带 M0-2：Three.js 桌宠验证`,
    `FPS: ${fps}  三角形: ${renderer.info.render.triangles}  模型包围盒: ${bboxInfo}`,
    `动画: ${current}（1 Idle / 2 Walking / 3 Dance / 4 Wave）`,
    `光标: ${cursorScreen ? `${Math.round(cursorScreen[0])}, ${Math.round(cursorScreen[1])}` : "?"}`,
    `锚点: ${petAnchor.position.x.toFixed(2)}, ${petAnchor.position.y.toFixed(2)}  相机: z=${camera.position.z} aspect=${camera.aspect.toFixed(2)}`,
    `锚点屏幕px: ${(() => { const v = petAnchor.position.clone(); v.y += PET_HEIGHT / 2; v.project(camera); return `${Math.round((v.x + 1) / 2 * innerWidth)}, ${Math.round((1 - v.y) / 2 * innerHeight)}`; })()}`,
    `视口CSS: ${innerWidth}x${innerHeight} @${devicePixelRatio}x`,
    `画布CSS: ${canvasEl.style.width}x${canvasEl.style.height} 缓冲: ${canvasEl.width}x${canvasEl.height}`,
    `面板矩形: ${Math.round(dbgRect.left)},${Math.round(dbgRect.top)} ${Math.round(dbgRect.width)}x${Math.round(dbgRect.height)}`,
    `窗口: ${size.width}x${size.height} @ (${pos.x}, ${pos.y})  穿透: ${clickThrough}`,
    ``,
    `T 切换点击穿透 | M 事件监视器 | Q 退出`,
  ];
  debug.textContent = info.join("\n");
  invoke("debug_log", { msg: info.join(" | ") }).catch(() => {});
}

async function toggle() {
  clickThrough = !clickThrough;
  await invoke("set_click_through", { through: clickThrough });
  await refresh();
}

document.addEventListener("keydown", async (e) => {
  if (e.key === "t" || e.key === "T") await toggle();
  if (e.key === "q" || e.key === "Q") await win.close();
  const anim = STATE_KEYS[e.key];
  if (anim) play(anim);
});

addEventListener("resize", () => {
  camera.aspect = innerWidth / innerHeight;
  camera.updateProjectionMatrix();
  renderer.setSize(innerWidth, innerHeight);
  layoutPet();
});

await refresh();

// ---------- M1-1 IPC ping-pong 样例 ----------
// Booted 事件（Rust→WebView）+ ping 命令（WebView→Rust）双通道验证。
// 结果写 %TEMP%\chaojiandai-debug.log；M1-2 事件监视器调试页上线后移除本段。
import { commands, events } from "./ipc/client";
void events.booted.listen((e) => {
  void commands
    .debugLog(
      `[ipc-sample] Booted received: kind=${e.payload.kind} payload=${JSON.stringify(e.payload.payload)}`
    )
    .then(() => commands.ping("hello from webview"))
    .then((pong) => commands.debugLog(`[ipc-sample] ping -> ${pong}`))
    .catch((err) => commands.debugLog(`[ipc-sample] ERROR ${String(err)}`));
});

// ---------- M1-2 事件监视器调试页 ----------
// 按 M 开关：固定定位 DOM 覆盖层，滚动显示事件总线原始流（最近 30 条，内存留 200）。
// 贯穿 M1 全部阶段的调试工具：采集器对不对，看它最直接。
const monitor = document.createElement("pre");
monitor.style.cssText =
  "position:fixed;left:8px;top:8px;width:380px;max-height:32vh;overflow-y:auto;margin:0;" +
  "padding:5px 7px;background:rgba(0,0,0,.6);color:#9fe8c0;font:10px/1.4 monospace;" +
  "pointer-events:none;white-space:pre-wrap;word-break:break-all;display:none;z-index:99;" +
  "border-radius:6px";
document.body.appendChild(monitor);

type StreamEvent = { ts: number; source: string; kind: string; payload: unknown };
const eventLog: StreamEvent[] = [];

function renderMonitor() {
  // 连续重复（拖动/前台动画的 LOCATIONCHANGE 洪峰）折叠为一条 + 计数，只看变化
  const folded: string[] = [];
  let runKey = "";
  let runCount = 0;
  for (const e of eventLog) {
    const t = new Date(e.ts * 1000).toLocaleTimeString("zh-CN", { hour12: false });
    const line = `${t}  ${e.source}/${e.kind}  ${JSON.stringify(e.payload)}`;
    const key = `${e.source}/${e.kind} ${JSON.stringify(e.payload)}`;
    if (key === runKey) {
      runCount++;
      folded[folded.length - 1] = `${line} ×${runCount + 1}`;
    } else {
      runKey = key;
      runCount = 0;
      folded.push(line);
    }
  }
  monitor.textContent = folded.slice(-12).join("\n");
  monitor.scrollTop = monitor.scrollHeight;
}

void events.eventStream.listen((e) => {
  const s = e.payload;
  eventLog.push({ ts: s.ts, source: String(s.source), kind: String(s.kind), payload: s.payload });
  if (eventLog.length > 200) eventLog.splice(0, eventLog.length - 200);
  if (monitor.style.display !== "none") renderMonitor();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "m" || e.key === "M") {
    monitor.style.display = monitor.style.display === "none" ? "block" : "none";
    if (monitor.style.display === "block") renderMonitor();
  }
});
