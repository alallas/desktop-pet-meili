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

scene.add(new THREE.AmbientLight(0xffffff, 1.2));
const key = new THREE.DirectionalLight(0xfff2e0, 2.2);
key.position.set(2, 4, 3);
scene.add(key);

// ---------- 模型 / 动画状态机 ----------
// M1 测试模型：Soldier（three.js 示例，Mixamo 授权，占位用，正式形象为 3D 水獭 meili）。
// 换它的原因：RobotExpressive 的 Idle 自带头部摇摆，掩盖倾听反馈动作（2026-11-19 用户要求）。
type AnimName = "Idle" | "Walk" | "Run" | "TPose";
const STATE_KEYS: Record<string, AnimName> = { "1": "Idle", "2": "Walk", "3": "Run", "4": "TPose" };

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
  petBasePos.copy(petAnchor.position);
}
const petBasePos = new THREE.Vector3();

new GLTFLoader().load("/models/Soldier.glb", (gltf) => {
  const model = gltf.scene;

  // 自动适配：统一缩放到 PET_HEIGHT、脚底贴锚点底、水平居中
  const box = new THREE.Box3().setFromObject(model);
  const size = box.getSize(new THREE.Vector3());
  const center = box.getCenter(new THREE.Vector3());
  const scale = PET_HEIGHT / size.y;
  model.scale.setScalar(scale);
  model.position.set(-center.x * scale, -box.min.y * scale, -center.z * scale);
  bboxInfo = `${size.x.toFixed(1)}x${size.y.toFixed(1)}x${size.z.toFixed(1)}`;
  // Mixamo 模型正面朝 +Z，与 RobotExpressive（-Z）相反，需转 180° 面向用户
  model.rotation.y = Math.PI;
  petAnchor.add(model);
  layoutPet();

  mixer = new THREE.AnimationMixer(model);
  for (const clip of gltf.animations) {
    const name = clip.name as AnimName;
    if (["Idle", "Walk", "Run", "TPose"].includes(name)) {
      actions.set(name, mixer.clipAction(clip));
    }
  }
  actions.get("Idle")?.play();

  model.traverse((o) => {
    // GLTFLoader 的 PropertyBinding.sanitizeNodeName 会删掉冒号等保留字符，
    // 骨骼名 "mixamorig:Head" 加载后变成 "mixamorigHead"（2026-11-19 扭头失效根因）
    if (!headBone && o instanceof THREE.Bone && ["mixamorig:Head", "mixamorigHead"].includes(o.name))
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
  bubbleUntil = performance.now() + 12000;
  lookAtUser(4000);
  playDing();
  void commands
    .debugLog(
      `[wakecue] fired bubbleUntil=${bubbleUntil} body=${!!document.body} bubble=${!!bubble}`
    )
    .catch(() => {});
}

void events.eventStream.listen((e) => {
  const s = e.payload;
  if (s.kind === "WakeWord") {
    wakeCue();
  } else if (s.kind === "TranscriptReady") {
    bubbleUntil = 0; // 收到话了，气泡使命完成
  }
});

// ---------- 光标跟随（点击穿透下轮询 Rust 侧全局光标）----------
const cursorNDC = new THREE.Vector2(0, -0.5); // 默认看向下方（"坐着"的感觉）
const cursorTarget = new THREE.Vector3(0, 0.6, 2);

async function pollCursor() {
  const [cx, cy] = (await invoke("get_cursor_position")) as [number, number];
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  // 屏幕坐标 → 窗口 NDC
  const x = ((cx - pos.x) / size.width) * 2 - 1;
  const y = -(((cy - pos.y) / size.height) * 2 - 1);
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
const clock = new THREE.Clock();
let frames = 0;
let fpsTime = 0;
let fps = 0;

function tick() {
  const dt = clock.getDelta();
  mixer?.update(dt);
  applyGaze(); // 必须在 mixer.update 之后，动画每帧会覆写骨骼旋转

  // 唤醒跳跃：抛物线，落地后归零（不干扰 layoutPet 的基准位置）
  const hopT = (performance.now() - hopStart) / HOP_MS;
  petAnchor.position.y =
    petBasePos.y + (hopT >= 0 && hopT < 1 ? 4 * HOP_H * hopT * (1 - hopT) : 0);

  // "我在听"气泡：宠物头顶上方，世界坐标投影到屏幕
  const bubbleOn = performance.now() < bubbleUntil;
  if (bubbleOn) {
    _v3.set(petBasePos.x, petBasePos.y + PET_HEIGHT + 0.15, 0).project(camera);
    bubble.style.display = "block";
    bubble.style.left = `${(_v3.x * 0.5 + 0.5) * innerWidth}px`;
    bubble.style.top = `${(-_v3.y * 0.5 + 0.5) * innerHeight}px`;
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
    `FPS ${fps}  三角形 ${renderer.info.render.triangles}  ${bboxInfo}`,
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
  "position:fixed;left:8px;top:8px;width:380px;max-height:32vh;overflow-y:auto;margin:0;" +
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
  { label: "鼠标点击", kind: "mouse_clicked", payload: { button: "left", x: 960, y: 540 } },
];

const simPanel = document.createElement("div");
simPanel.style.cssText =
  "position:fixed;right:8px;top:8px;width:210px;display:none;padding:8px;" +
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

let simOpen = false;

// 面板开关权威在 Rust（全局热键），前端按事件同步显示与穿透状态
void events.simPanel.listen((e) => {
  simOpen = e.payload;
  simPanel.style.display = simOpen ? "block" : "none";
  clickThrough = !simOpen;
  simLog(simOpen ? "面板已开（穿透关）" : "");
});
