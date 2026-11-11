import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";

const debug = document.getElementById("debug")!;
const win = getCurrentWindow();

// Tauri v2 无查询接口，点击穿透状态由 JS 侧自行记录（默认值与 Rust 侧 setup 一致）
let clickThrough = true;

async function refresh() {
  const pos = await win.outerPosition();
  const size = await win.outerSize();
  debug.textContent = [
    `潮间带 M0 窗口验证`,
    `屏幕: ${screen.width}x${screen.height} @${devicePixelRatio}x`,
    `窗口: ${size.width}x${size.height} @ (${pos.x}, ${pos.y})`,
    `点击穿透: ${clickThrough}`,
    ``,
    `按 T 切换点击穿透`,
    `按 Q 退出`,
  ].join("\n");
}

async function toggle() {
  clickThrough = !clickThrough;
  await invoke("set_click_through", { through: clickThrough });
  await refresh();
}

document.addEventListener("keydown", async (e) => {
  if (e.key === "t" || e.key === "T") await toggle();
  if (e.key === "q" || e.key === "Q") await win.close();
});

document.getElementById("pet")!.addEventListener("dblclick", toggle);

await refresh();
