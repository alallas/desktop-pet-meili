import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import * as THREE from "three";
import { GLTFLoader } from "three/examples/jsm/loaders/GLTFLoader.js";
import { commands, events, type Value } from "./ipc/client";

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

// 渲染后端诊断（#36 排查）：WebGL/WebGPU 实际后端落 debug 日志——
// 出现 SwiftShader/llvmpipe/Dawn CPU = 软渲染（CPU 烧在光栅化上）；
// AMD/NVIDIA/Intel + D3D11 = 硬件渲染正常。排查期常驻，产品化可摘。
{
  const gl = renderer.getContext();
  const dbgExt = gl.getExtension("WEBGL_debug_renderer_info");
  const glRenderer = dbgExt ? String(gl.getParameter(dbgExt.UNMASKED_RENDERER_WEBGL)) : "n/a";
  let gpuInfo = "webgpu-na";
  const navGpu = (navigator as { gpu?: { requestAdapter(): Promise<{ info?: { vendor?: string; architecture?: string } } | null> } }).gpu;
  if (navGpu) {
    void navGpu
      .requestAdapter()
      .then((a) => a?.info ?? null)
      .then((i) => {
        gpuInfo = i ? `${i.vendor}/${i.architecture}` : "no-adapter";
        return commands
          .debugLog(`[gpu] WebGL=${glRenderer} | WebGPU=${gpuInfo} | dpr=${devicePixelRatio} canvas=${renderer.domElement.width}x${renderer.domElement.height}`)
          .catch(() => {});
      })
      .catch(() => {});
  } else {
    void commands.debugLog(`[gpu] WebGL=${glRenderer} | WebGPU=unsupported`).catch(() => {});
  }
}

scene.add(new THREE.AmbientLight(0xffffff, 1.2));
const key = new THREE.DirectionalLight(0xfff2e0, 2.2);
key.position.set(2, 4, 3);
scene.add(key);

// ---------- 模型 / 动画状态机 ----------
// 当前模型：meili 水獭原型（#16 阶段③④⑤：Tripo p2-quad 生成 → fbx2glb 转换 →
// Blender 程序化绑骨+10 clip → gltf-transform 标准化 28193 三角/无纹理）。
// Soldier（Mixamo 授权占位）留 public/models/Soldier.glb 可回退。
// clip 命名见 docs/美术生产线.md 附录（Blender NLA track 名原样导出）。
type AnimName =
  | "idle_a" | "wave" | "jump" | "sit" | "sleep" | "petted" | "listen"
  | "special_rub_face" | "special_gift_shell" | "special_explain";
// 常驻循环 clip；其余为一次性动作（播完自动回 idle_a）
const LOOPING = new Set<AnimName>(["idle_a", "sleep", "petted", "special_explain"]);
const STATE_KEYS: Record<string, AnimName> = {
  "1": "idle_a", "2": "wave", "3": "jump", "4": "sit", "5": "sleep",
  "6": "petted", "7": "listen", "8": "special_rub_face",
  "9": "special_gift_shell", "0": "special_explain",
};
const STATE_KEYS_INV = new Set<string>(Object.values(STATE_KEYS));

let mixer: THREE.AnimationMixer | null = null;
const actions = new Map<AnimName, THREE.AnimationAction>();
let current: AnimName = "idle_a";
let headBone: THREE.Bone | null = null;
let bboxInfo = "?";
let petModel: THREE.Object3D | null = null;

// 桌宠锚点：模型挂在锚点下并居中，锚点负责把桌宠摆到窗口内 NDC 位置。
// 小窗形态（M1-1 定案 B 方案）：宠物居中偏下，头顶留白给气泡（窗内显示，
// Windows 透明窗内容画出窗口边界会被裁剪）。
// 屏幕级游走（巡逻/坐标题栏）= 窗口级移动，归 M1-6 表现层（Rust SetWindowPos）
const PET_NDC = { x: 0, y: -0.5 };
const PET_HEIGHT = 1.3; // 世界单位
const petAnchor = new THREE.Group();
scene.add(petAnchor);

function layoutPet() {
  // 相机前方 z=0 平面上的可视半高/半宽
  const halfH = Math.tan(THREE.MathUtils.degToRad(camera.fov / 2)) * camera.position.z;
  const halfW = halfH * camera.aspect;
  const cx = PET_NDC.x * halfW;
  const cy = camera.position.y + PET_NDC.y * halfH; // 相机视线中心在 (0, 1.1, 0)
  petAnchor.position.set(cx, cy - PET_HEIGHT / 2, 0);
  petBasePos.copy(petAnchor.position);
}
const petBasePos = new THREE.Vector3();

new GLTFLoader().load("/models/meili.glb", (gltf) => {
  const model = gltf.scene;

  // 朝向定案（2026-11-25 v2 网格）：探针确认脸朝 +X（Tripo multiview 惯例），
  // 转 -90° 面向相机（+Z）。旋转必须在量 bbox 之前设置并 updateMatrixWorld，
  // 否则旋转后包围盒错位、宠物偏出锚点。
  model.rotation.y = -Math.PI / 2;
  model.updateMatrixWorld(true);

  // 自动适配：统一缩放到 PET_HEIGHT、脚底贴锚点底、水平居中
  const box = new THREE.Box3().setFromObject(model);
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  const scale = PET_HEIGHT / size.y;
  model.scale.setScalar(scale);
  model.position.set(-center.x * scale, -box.min.y * scale, -center.z * scale);
  bboxInfo = `${size.x.toFixed(1)}x${size.y.toFixed(1)}x${size.z.toFixed(1)}`;
  petAnchor.add(model);
  petModel = model;
  layoutPet();
  reportHitRect(); // 首报后立即可拖

  mixer = new THREE.AnimationMixer(model);
  for (const clip of gltf.animations) {
    const name = clip.name as AnimName;
    // 阶段④重做期（2026-11-26 判决，坑 64-65）：9 个 clip 导出数据损坏全部禁用，
    // 只留 idle_a 保底；新资产过数值验收后恢复（每 clip 独立导出+独立校验的新管线）
    if (name === "idle_a") {
      const action = mixer.clipAction(clip);
      actions.set(name, action);
    }
  }
  actions.get("idle_a")?.play();
  // 一次性动作播完回常驻 idle
  mixer.addEventListener("finished", () => play("idle_a"));

  model.traverse((o) => {
    // GLTFLoader 的 PropertyBinding.sanitizeNodeName 会删掉冒号等保留字符，
    // 骨骼名 "mixamorig:Head" 加载后变成 "mixamorigHead"（2026-11-19 扭头失效根因）；
    // meili 自有骨架（#16 阶段④）头骨就叫 "head"，骨表见 rig_meili.py BONES。
    if (!headBone && o instanceof THREE.Bone &&
        ["head", "mixamorig:Head", "mixamorigHead"].includes(o.name))
      headBone = o;
  });
  void commands.debugLog(`[gaze] headBone=${headBone?.name ?? "NOT FOUND"}`).catch(() => {});

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

// ---------- 拖拽命中矩形上报（M1-6）：只有点在宠物身上才"拎起" ----------
// Rust 侧 WH_MOUSE_LL 钩子按下时按此矩形 hit test；透明区点击必须穿透到
// 桌面（2026-11-22 用户实机投诉：点窗口透明部分下面的字先触发拖拽）。
// 用绑定姿势投影 bbox 近似（蒙皮瞬时姿态误差由 6px 容差吸收），500ms 重算、
// 变化 >4px 才上报，避免 invoke 噪音。
const HIT_PADDING = 6;
let lastHitRect = { x: -1, y: -1, w: 0, h: 0 };

function petScreenBBox(): { x: number; y: number; w: number; h: number } | null {
  if (!petModel) return null;
  petModel.updateWorldMatrix(true, true);
  const box = new THREE.Box3().setFromObject(petModel);
  if (box.isEmpty()) return null;
  let minX = Infinity,
    minY = Infinity,
    maxX = -Infinity,
    maxY = -Infinity;
  const v = new THREE.Vector3();
  for (const xi of [box.min.x, box.max.x])
    for (const yi of [box.min.y, box.max.y])
      for (const zi of [box.min.z, box.max.z]) {
        v.set(xi, yi, zi).project(camera);
        if (v.z < -1 || v.z > 1) continue; // 相机前后裁剪外的角点不参与
        const sx = ((v.x + 1) / 2) * innerWidth;
        const sy = ((1 - v.y) / 2) * innerHeight;
        minX = Math.min(minX, sx);
        minY = Math.min(minY, sy);
        maxX = Math.max(maxX, sx);
        maxY = Math.max(maxY, sy);
      }
  if (minX === Infinity) return null;
  const x = Math.max(0, minX - HIT_PADDING);
  const y = Math.max(0, minY - HIT_PADDING);
  const w = Math.min(innerWidth, maxX + HIT_PADDING) - x;
  const h = Math.min(innerHeight, maxY + HIT_PADDING) - y;
  return { x, y, w, h };
}

function reportHitRect() {
  const r = petScreenBBox();
  if (!r) return;
  if (Math.abs(r.x - lastHitRect.x) < 4 && Math.abs(r.y - lastHitRect.y) < 4 &&
      Math.abs(r.w - lastHitRect.w) < 4 && Math.abs(r.h - lastHitRect.h) < 4) return;
  lastHitRect = r;
  void commands.setPetHitRect(r.x, r.y, r.w, r.h).catch(() => {});
}

setInterval(reportHitRect, 500);

// ---------- 唤醒反馈（M1-3.5b 简化版）：跳一下 + "我在听"气泡 ----------
// 占位模型动画表现力不足（Wave 未设 LoopOnce 会循环成"一直挥手"，且与 Idle 区分
// 度低），测试期只保留"跳跃 + 气泡"一个明确信号；正式反馈等美丽 3D 形象（#16）。
const USER_GAZE = new THREE.Vector3(0, 1.4, 4.4); // 相机后方 = 用户脸的方向
let gazeOverrideUntil = 0; // performance.now() 截止戳；期间无视光标强制看向用户

function lookAtUser(ms: number) {
  gazeOverrideUntil = performance.now() + ms;
}

const bubble = document.createElement("div");
bubble.textContent = "我在听~";
bubble.style.cssText =
  "position:fixed;display:none;padding:8px 16px;background:#fff;color:#111;" +
  "border:2px solid #4a9eff;border-radius:14px;font:bold 16px sans-serif;" +
  "box-shadow:0 3px 12px rgba(0,0,0,.3);" +
  "transform:translate(-50%,-130%);white-space:nowrap;z-index:98;pointer-events:none";
document.body.appendChild(bubble);

// ---------- M1-6 前哨：说话气泡（她出声时的视觉反馈）----------
// 行为评论与对话回复共用 ReplyReady 通道（Rust 侧行为评论复用该事件）；
// SpeakEnded（播完/被打断）即收。已知限制：对话回复是流式边想边播，
// ReplyReady 在整句生成完才发，气泡比开腔略晚（行为评论是整句台词无此问题）。
const speechBubble = document.createElement("div");
speechBubble.style.cssText =
  "position:fixed;display:none;max-width:270px;padding:8px 14px;background:#fff;color:#111;" +
  "border:2px solid #ff9e4a;border-radius:14px;font:15px/1.5 sans-serif;" +
  "box-shadow:0 3px 12px rgba(0,0,0,.3);" +
  "transform:translate(-50%,-130%);z-index:98;pointer-events:none;white-space:normal";
document.body.appendChild(speechBubble);

let speechText = "";
let speechUntil = 0; // performance.now() 截止戳；SpeakEnded 或时长估完即收
/// 台词念完时长估计：~0.28s/字 + 1.2s 收尾余量，封顶 15s（SpeakEnded 会提前收）
function estimateSpeechMs(text: string) {
  return Math.min(15000, 1200 + [...text].length * 280);
}

// 唤醒提示音：Web Audio 现场合成双音"叮"（零音频文件零授权问题；
// 小爱/Siri 都有唤醒确认音，是"她听到了"的不可忽略信号——2026-11-19 用户反馈
// "喊 10 次都不出来"实为反馈太弱：KWS 10/10 命中但跳一下+小气泡注意不到）
let audioCtx: AudioContext | null = null;
function playDing() {
  try {
    audioCtx ??= new AudioContext();
    // 自动播放策略可能挂起：任何一次用户手势后恢复（下次唤醒即可发声）
    if (audioCtx.state === "suspended") void audioCtx.resume();
    const t0 = audioCtx.currentTime;
    for (const [freq, delay, dur] of [
      [880, 0, 0.12],
      [1318, 0.1, 0.22],
    ] as const) {
      const osc = audioCtx.createOscillator();
      const gain = audioCtx.createGain();
      osc.frequency.value = freq;
      gain.gain.setValueAtTime(0.0001, t0 + delay);
      gain.gain.exponentialRampToValueAtTime(0.25, t0 + delay + 0.02);
      gain.gain.exponentialRampToValueAtTime(0.0001, t0 + delay + dur);
      osc.connect(gain).connect(audioCtx.destination);
      osc.start(t0 + delay);
      osc.stop(t0 + delay + dur + 0.05);
    }
  } catch {
    /* 音频不可用不阻塞唤醒反馈 */
  }
}

let bubbleUntil = 0; // 气泡显示截止戳（与引擎 MAX_LISTENING 12s 对齐）
let bubbleShownLogged = false; // 气泡显示日志去抖（transition 才记）
let hopStart = Number.NEGATIVE_INFINITY; // 跳跃动画起点
const HOP_MS = 500;
const HOP_H = 0.35; // 跳跃高度（世界单位），从 0.22 加高——跳一下必须看得见

function wakeCue() {
  hopStart = performance.now();
  play("jump"); // meili 原地产臂跳跃姿势 + 下方抛物线 = 完整跳跃
  bubbleUntil = performance.now() + 12000;
  lookAtUser(4000);
  playDing();
  void commands
    .debugLog(
      `[wakecue] fired bubbleUntil=${bubbleUntil} body=${!!document.body} bubble=${!!bubble}`
    )
    .catch(() => {});
}

// ---------- 表现层小动作（map.toml anim 字段；M1-6） ----------
// 闭集消费：未知值忽略。值即 meili clip 名（#16 阶段④已入库），播完自动回 idle_a。
const KNOWN_REACTIONS = new Set(["petted"]);
function playReaction(anim: string) {
  if (!KNOWN_REACTIONS.has(anim)) return;
  play(anim as AnimName); // 摸头：歪头+耳抖（petted clip）
  lookAtUser(2500);
}

void events.eventStream.listen((e) => {
  const s = e.payload;
  if (s.kind === "WakeWord") {
    wakeCue();
    speechUntil = 0; // 打断正在播的话：说话气泡立刻收
  } else if (s.kind === "TranscriptReady") {
    bubbleUntil = 0; // 收到话了，气泡使命完成
  } else if (s.kind === "ReplyReady") {
    const text = (s.payload as { text?: string }).text ?? "";
    if (text) {
      speechText = text;
      speechUntil = performance.now() + estimateSpeechMs(text);
    }
  } else if (s.kind === "Gesture") {
    // 表现层小动作（map.toml anim）：与台词解耦的即时反馈（摸头等）
    const anim = (s.payload as { anim?: string }).anim;
    if (anim) playReaction(anim);
  } else if (s.kind === "SpeakEnded") {
    speechUntil = 0; // 播完或被唤醒打断（barge-in）
  }
});

// ---------- 光标跟随（点击穿透下轮询 Rust 侧全局光标）----------
const cursorNDC = new THREE.Vector2(0, -0.5); // 默认看向下方（"坐着"的感觉）
const cursorTarget = new THREE.Vector3(0, 0.6, 2);

let lastCursorActive = 0; // #36：光标在动 = 活跃，恢复满帧（performance.now() 截止戳）
const _prevNdc = new THREE.Vector2(0, 0);

async function pollCursor() {
  const [cx, cy] = (await invoke("get_cursor_position")) as [number, number];
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  // 屏幕坐标 → 窗口 NDC
  const x = ((cx - pos.x) / size.width) * 2 - 1;
  const y = -(((cy - pos.y) / size.height) * 2 - 1);
  // 降帧依据：光标移动即活跃（静止悬停不算——盯人低频刷新足够跟手）
  if (Math.abs(x - _prevNdc.x) > 0.005 || Math.abs(y - _prevNdc.y) > 0.005) {
    lastCursorActive = performance.now();
    _prevNdc.set(x, y);
  }
  cursorNDC.set(x, y);
}
setInterval(pollCursor, 100);

// ---------- 视线 IK：头部朝向光标，限制在正面锥形范围内 ----------
const _v3 = new THREE.Vector3();
const _animQ = new THREE.Quaternion();
const _lookQ = new THREE.Quaternion();
const _m = new THREE.Matrix4();
const _headPos = new THREE.Vector3();
const _dir = new THREE.Vector3();
const UP = new THREE.Vector3(0, 1, 0);

function applyGaze() {
  if (!headBone) return;
  // 注视目标：优先"看向用户"（被喊到/说话时），否则跟随光标
  let target: THREE.Vector3;
  if (performance.now() < gazeOverrideUntil) {
    target = USER_GAZE;
  } else {
    // 光标 NDC → 面前固定深度处的世界坐标
    cursorTarget.set(cursorNDC.x, cursorNDC.y, 0.5).unproject(camera);
    target = cursorTarget;
  }

  headBone.getWorldPosition(_headPos);
  _dir.copy(target).sub(_headPos);

  // 只追踪正面 150° 锥形范围，身后不扭头
  if (_dir.z < -0.2 * _dir.length()) return;

  // 记录动画姿态，与注视姿态插值混合（避免 IK 生硬）。
  // 唤醒/说话覆盖期用全权重——必须明显区别于平时的摇摆跟随
  const gazeWeight = performance.now() < gazeOverrideUntil ? 1.0 : 0.5;
  _animQ.copy(headBone.quaternion);
  // 关键：Matrix4.lookAt 是相机约定（+Z 指向 eye），骨骼"脸"是 +Z——
  // 参数必须是 (target, headPos) 才能让脸朝向目标；写成 (headPos, target)
  // 会得到"后脑勺对目标"（M0 起光标跟随一直是反的，M1-3.5b 实机才看穿）
  _m.lookAt(target, _headPos, UP);
  _lookQ.setFromRotationMatrix(_m);
  // lookAt 得到的是世界旋转，需换算到父空间
  const parent = headBone.parent!;
  parent.updateWorldMatrix(true, false);
  _lookQ.premultiply(parent.getWorldQuaternion(new THREE.Quaternion()).invert());
  headBone.quaternion.copy(_animQ).slerp(_lookQ, gazeWeight);
}

// ---------- 主循环 ----------
// #36 渲染降帧：满帧 120FPS 实测烧 ~46% 单核（§11 #20 采样），静默常驻功耗
// 红线（<1%）要求空闲时压帧。活跃态（唤醒反馈/说话/面板/光标移动/非常驻
// 动画）回 60FPS 保手感；其余 15FPS——Idle 动画略降采样但占位模型阶段可接受。
const IDLE_FPS = 15;
const ACTIVE_FPS = 60;
let lastRenderAt = 0;
let simOpen = false; // 模拟器面板开关（事件驱动，Rust 热键为权威）

function renderActive(now: number): boolean {
  return (
    now < bubbleUntil || // "我在听"气泡
    now < speechUntil || // 说话气泡
    now < gazeOverrideUntil || // 强制注视（唤醒反馈）
    now - hopStart < HOP_MS || // 跳跃进行中
    simOpen || // 模拟器面板开
    now - lastCursorActive < 2000 || // 光标最近在动
    current !== "idle_a" // wave/jump 等非常驻动画
  );
}

const clock = new THREE.Clock();
let frames = 0;
let fpsTime = 0;
let fps = 0;

function tick() {
  const now = performance.now();
  const minMs = 1000 / (renderActive(now) ? ACTIVE_FPS : IDLE_FPS);
  if (now - lastRenderAt < minMs) return; // 降帧帧：整帧跳过（动画 dt 自然累计）
  lastRenderAt = now;
  const dt = clock.getDelta();
  mixer?.update(dt);
  applyGaze(); // 必须在 mixer.update 之后，动画每帧会覆写骨骼旋转

  // 唤醒跳跃：抛物线，落地后归零（不干扰 layoutPet 的基准位置）
  const hopT = (performance.now() - hopStart) / HOP_MS;
  petAnchor.position.y =
    petBasePos.y + (hopT >= 0 && hopT < 1 ? 4 * HOP_H * hopT * (1 - hopT) : 0);

  // 头顶气泡：说话气泡优先于"我在听"气泡，同一锚点投影
  _v3.set(petBasePos.x, petBasePos.y + PET_HEIGHT + 0.15, 0).project(camera);
  const headLeft = `${(_v3.x * 0.5 + 0.5) * innerWidth}px`;
  const headTop = `${(-_v3.y * 0.5 + 0.5) * innerHeight}px`;
  const speechOn = performance.now() < speechUntil;
  if (speechOn) {
    speechBubble.textContent = speechText;
    speechBubble.style.display = "block";
    speechBubble.style.left = headLeft;
    speechBubble.style.top = headTop;
    bubble.style.display = "none";
    bubbleShownLogged = false;
  } else {
    speechBubble.style.display = "none";
    // "我在听"气泡：宠物头顶上方，世界坐标投影到屏幕
    const bubbleOn = performance.now() < bubbleUntil;
    if (bubbleOn) {
      bubble.style.display = "block";
      bubble.style.left = headLeft;
      bubble.style.top = headTop;
      if (!bubbleShownLogged) {
        bubbleShownLogged = true;
        void commands
          .debugLog(`[wakecue] bubble shown at ${bubble.style.left},${bubble.style.top}`)
          .catch(() => {});
      }
    } else {
      bubble.style.display = "none";
      bubbleShownLogged = false;
    }
  }

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
  const info = [
    `FPS ${fps}  三角形 ${renderer.info.render.triangles}  ${bboxInfo}  build:meili-rebuild`,
    `窗口 ${size.width}×${size.height} @(${pos.x},${pos.y})  穿透:${clickThrough}`,
    `Ctrl+Alt+N 模拟器  |  T穿透 M监视 Q退出(需窗口聚焦)`,
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
  "position:fixed;left:8px;top:8px;width:284px;max-height:32vh;overflow-y:auto;margin:0;" +
  "padding:5px 7px;background:rgba(0,0,0,.6);color:#9fe8c0;font:10px/1.4 monospace;" +
  "pointer-events:none;white-space:pre-wrap;word-break:break-all;display:none;z-index:99;" +
  "border-radius:6px";
document.body.appendChild(monitor);

type StreamEvent = { ts: number; source: string; kind: string; payload: unknown };
const eventLog: StreamEvent[] = [];

function renderMonitor() {
  // 折叠策略：拖动/动画的连续事件只留最新一条 + 计数。
  // ForegroundMoved 按 app 折叠（rect 抖动不算新信息）；其余按完整 payload 折叠。
  const folded: string[] = [];
  let runKey = "";
  let runCount = 0;
  for (const e of eventLog) {
    const t = new Date(e.ts * 1000).toLocaleTimeString("zh-CN", { hour12: false });
    const line = `${t}  ${e.source}/${e.kind}  ${JSON.stringify(e.payload)}`;
    const payload = e.payload as { app?: string };
    const key =
      e.kind === "ForegroundMoved"
        ? `${e.source}/${e.kind} ${payload.app ?? ""}`
        : `${e.source}/${e.kind} ${JSON.stringify(e.payload)}`;
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

// 渲染节流：事件洪峰（前台窗口动画）下最多 400ms 刷一帧，不闪屏
let monitorRenderTimer: ReturnType<typeof setTimeout> | null = null;
function scheduleMonitorRender() {
  if (monitorRenderTimer) return;
  monitorRenderTimer = setTimeout(() => {
    monitorRenderTimer = null;
    renderMonitor();
  }, 400);
}

void events.eventStream.listen((e) => {
  const s = e.payload;
  eventLog.push({ ts: s.ts, source: String(s.source), kind: String(s.kind), payload: s.payload });
  if (eventLog.length > 200) eventLog.splice(0, eventLog.length - 200);
  if (monitor.style.display !== "none") scheduleMonitorRender();
});

document.addEventListener("keydown", (e) => {
  if (e.key === "m" || e.key === "M") {
    monitor.style.display = monitor.style.display === "none" ? "block" : "none";
    if (monitor.style.display === "block") renderMonitor();
  }
});

// ---------- M1-5 场景模拟器 ----------
// 全局热键 Ctrl+Alt+N 呼出/关闭（Rust 侧 dev_hotkey 注册；主窗口 skipTaskbar +
// 不聚焦 + 点击穿透，键盘焦点永远到不了 WebView，前端按键快捷键全部不可达）。
// 面板开关的权威状态在 Rust，这里只按 SimPanel 事件同步显示。
// 场景按钮把合成事件以 System 源注入总线，走完整四道闸（测的就是真实链路；
// 不用 Test 源是因为行为引擎跳过它）。配"重置冷却"按钮，连续测试不用等 30min。
// 面板可点击 = 热键触发时 Rust 已关点击穿透。
const SCENARIOS: { label: string; kind: string; payload: Value }[] = [
  { label: "启动", kind: "booted", payload: { version: "0.0.1" } },
  { label: "锁屏", kind: "lock_state_changed", payload: { locked: true } },
  { label: "解锁", kind: "lock_state_changed", payload: { locked: false } },
  { label: "切到 VS Code", kind: "foreground_switched", payload: { app: "Code", title: "meili.map.toml" } },
  { label: "切到浏览器", kind: "foreground_switched", payload: { app: "chrome", title: "GitHub - 潮间带" } },
  { label: "切到游戏", kind: "foreground_switched", payload: { app: "somegame", title: "Some Game" } },
  { label: "进入空闲", kind: "idle_state_changed", payload: { idle_seconds: 300, is_idle: true } },
  { label: "结束空闲", kind: "idle_state_changed", payload: { idle_seconds: 0, is_idle: false } },
  { label: "开始放歌", kind: "media_playback_changed", payload: { app: "cloudmusic", title: "晴天", artist: "周杰伦", state: "playing" } },
  { label: "暂停音乐", kind: "media_playback_changed", payload: { state: "paused" } },
  { label: "插上电源", kind: "power_state_changed", payload: { ac_online: true, charging: true, battery_percent: 42 } },
  { label: "拔掉电源", kind: "power_state_changed", payload: { ac_online: false, charging: false, battery_percent: 41 } },
  { label: "断网", kind: "network_changed", payload: { connected: false } },
  { label: "网络恢复", kind: "network_changed", payload: { connected: true } },
  { label: "点外面", kind: "mouse_clicked", payload: { button: "left", x: 960, y: 540, on_pet: false } },
  { label: "摸头", kind: "mouse_clicked", payload: { button: "left", x: 960, y: 540, on_pet: true } },
];

const simPanel = document.createElement("div");
simPanel.style.cssText =
  "position:fixed;right:8px;top:8px;width:238px;display:none;padding:8px;" +
  "max-height:calc(100vh - 24px);overflow-y:auto;" + // 窗高 460 装不下全部门控，必须可滚
  "background:rgba(20,24,32,.85);border:1px solid #4a9eff;border-radius:8px;z-index:100;" +
  "font:12px/1.5 sans-serif;color:#dde";
document.body.appendChild(simPanel);

const simTitle = document.createElement("div");
simTitle.textContent = "场景模拟器 (Ctrl+Alt+N 关闭)";
simTitle.style.cssText = "font-weight:bold;margin-bottom:6px;color:#9fe8c0";
simPanel.appendChild(simTitle);

const simStatus = document.createElement("div");
simStatus.style.cssText = "margin-top:6px;color:#ffb;min-height:1.5em;white-space:pre-wrap";
simPanel.appendChild(simStatus);

function simLog(msg: string) {
  simStatus.textContent = msg;
  void commands.debugLog(`[sim] ${msg}`).catch(() => {});
}

for (const s of SCENARIOS) {
  const btn = document.createElement("button");
  btn.textContent = s.label;
  btn.style.cssText =
    "display:block;width:100%;margin:3px 0;padding:5px 8px;text-align:left;" +
    "background:#2a3a4d;color:#dde;border:1px solid #456;border-radius:5px;cursor:pointer";
  btn.onmouseenter = () => (btn.style.background = "#3a4f68");
  btn.onmouseleave = () => (btn.style.background = "#2a3a4d");
  btn.onclick = () => {
    // 先重置冷却再注入：让这句立刻走闸①③④判定（冷却清零会把队列也清掉，
    // 顺序反了刚注入的台词会被抹掉）
    void commands
      .debugResetBehavior()
      .then((r) => {
        if (r.status === "error") {
          simLog(`重置冷却失败: ${r.error}`);
          return undefined;
        }
        return commands.debugEmitEvent(s.kind, s.payload);
      })
      .then((r) => {
        if (!r) return;
        simLog(r.status === "ok" ? `已注入 ${s.kind}` : `注入失败: ${r.error}`);
      })
      .catch((err) => simLog(`异常: ${String(err)}`));
  };
  simPanel.appendChild(btn);
}

// ---------- 动画 clip 测试区（#16 阶段④验收配套） ----------
// 点按钮直触表现层（不过行为引擎），用于逐个 clip 目检姿态/循环/回 idle。
// 与数字键的差异：① 同 clip 再点 = 重播（数字键 play() 对 current 同值直接 return）；
// ② 勾选"单次循环"可把 LoopOnce 动作改成 LoopRepeat 反复观察动作全程。
const animDivider = document.createElement("div");
animDivider.textContent = "── 动画 clip 直测 ──";
animDivider.style.cssText = "margin:8px 0 4px;color:#9fe8c0;font-weight:bold;border-top:1px solid #456;padding-top:6px";
simPanel.appendChild(animDivider);

let repeatOneShots = false;
const repeatBox = document.createElement("input");
repeatBox.type = "checkbox";
repeatBox.id = "sim-repeat";
repeatBox.onchange = () => {
  repeatOneShots = repeatBox.checked;
  // 立即对全部一次性 clip 生效；切回时恢复 LoopOnce + 定格
  for (const [name, action] of actions) {
    if (!LOOPING.has(name)) {
      if (repeatOneShots) {
        action.setLoop(THREE.LoopRepeat, Infinity);
        action.clampWhenFinished = false;
      } else {
        action.setLoop(THREE.LoopOnce, 1);
        action.clampWhenFinished = true;
      }
    }
  }
  simLog(repeatOneShots ? "单次动作已改循环（观察模式）" : "已恢复单次播放");
};
const repeatLabel = document.createElement("label");
repeatLabel.htmlFor = "sim-repeat";
repeatLabel.textContent = " 单次动作循环（便于观察全程）";
repeatLabel.style.cssText = "display:inline;margin:2px 0 4px;cursor:pointer;font-size:11px";
simPanel.appendChild(repeatBox);
simPanel.appendChild(repeatLabel);

function testPlay(name: AnimName) {
  const action = actions.get(name);
  if (!action) {
    simLog(`clip ${name} 未加载`);
    return;
  }
  if (name === current) {
    // 同 clip 重播：手动 reset，绕开 play() 的 current 早退
    action.reset().fadeIn(0.1).play();
    if (!LOOPING.has(name) && !repeatOneShots) {
      action.setLoop(THREE.LoopOnce, 1);
      action.clampWhenFinished = true;
    }
  } else {
    play(name);
  }
  // 循环 clip 保持循环；单次 clip 若在观察模式下也保持（onchange 已统一设置）
  simLog(`▶ ${name}${LOOPING.has(name) ? "（循环）" : repeatOneShots ? "（循环·观察）" : "（单次→回 idle_a）"}`);
}

const clipGrid = document.createElement("div");
clipGrid.style.cssText = "display:grid;grid-template-columns:1fr 1fr;gap:3px";
simPanel.appendChild(clipGrid);

const KEY_BY_ANIM: Record<string, string> = Object.fromEntries(
  Object.entries(STATE_KEYS).map(([k, v]) => [v, k])
);
for (const name of Object.values(STATE_KEYS)) {
  const btn = document.createElement("button");
  btn.textContent = `${KEY_BY_ANIM[name]} ${name}`;
  btn.style.cssText =
    "padding:4px 6px;text-align:left;font-size:11px;" +
    "background:#2a3a4d;color:#dde;border:1px solid #456;border-radius:5px;cursor:pointer";
  btn.onmouseenter = () => (btn.style.background = "#3a4f68");
  btn.onmouseleave = () => (btn.style.background = "#2a3a4d");
  btn.onclick = () => testPlay(name);
  clipGrid.appendChild(btn);
}

// 交互组合按钮：走真实反馈函数（含气泡/叮/注视/抛物线），验收"被喊到"完整效果
const cueDivider = document.createElement("div");
cueDivider.textContent = "── 交互反馈 ──";
cueDivider.style.cssText = "margin:8px 0 4px;color:#9fe8c0;font-weight:bold;border-top:1px solid #456;padding-top:6px";
simPanel.appendChild(cueDivider);

const CUE_BUTTONS: { label: string; fn: () => void }[] = [
  { label: "🔔 唤醒反馈（跳+气泡+叮）", fn: () => { wakeCue(); simLog("🔔 唤醒反馈"); } },
  { label: "✋ 摸头反应（petted+注视）", fn: () => { playReaction("petted"); simLog("✋ 摸头反应"); } },
];
for (const c of CUE_BUTTONS) {
  const btn = document.createElement("button");
  btn.textContent = c.label;
  btn.style.cssText =
    "display:block;width:100%;margin:3px 0;padding:5px 8px;text-align:left;" +
    "background:#2d4a3a;color:#dfd;border:1px solid #465;border-radius:5px;cursor:pointer";
  btn.onmouseenter = () => (btn.style.background = "#3a5f48");
  btn.onmouseleave = () => (btn.style.background = "#2d4a3a");
  btn.onclick = c.fn;
  simPanel.appendChild(btn);
}

const resetBtn = document.createElement("button");
resetBtn.textContent = "↺ 重置行为冷却";
resetBtn.style.cssText =
  "display:block;width:100%;margin:6px 0 0;padding:5px 8px;font-weight:bold;" +
  "background:#4d3a2a;color:#ffd;border:1px solid #654;border-radius:5px;cursor:pointer";
resetBtn.onclick = () => {
  void commands
    .debugResetBehavior()
    .then((r) => simLog(r.status === "ok" ? "冷却已重置（全局/条目/队列）" : `重置失败: ${r.error}`))
    .catch((err) => simLog(`异常: ${String(err)}`));
};
simPanel.appendChild(resetBtn);

// 面板开关权威在 Rust（全局热键），前端按事件同步显示与穿透状态
void events.simPanel.listen((e) => {
  simOpen = e.payload;
  simPanel.style.display = simOpen ? "block" : "none";
  clickThrough = !simOpen;
  simLog(simOpen ? "面板已开（穿透关）" : "");
});
// 注册成功标记：排障时确认监听器真的挂上（模块中途抛异常会静默跳过这段）
void commands.debugLog("[sim] simPanel listener registered").catch(() => {});
