/**
 * 与 Rust 通信的唯一入口（工程规范 §1 TS 规则：业务代码禁止直接 invoke）。
 * 换协议时只改这里。
 *
 * M1-1 骨架：包装 M0 遗产命令。
 * TODO(M1-1 协议定版)：specta 生成的 types.ts 接入后，这里的方法签名改用生成类型。
 */

import { invoke } from "@tauri-apps/api/core";

export const ipc = {
  setClickThrough(through: boolean): Promise<void> {
    return invoke("set_click_through", { through });
  },
  getCursorPosition(): Promise<[number, number] | null> {
    return invoke("get_cursor_position");
  },
  debugLog(msg: string): Promise<void> {
    return invoke("debug_log", { msg });
  },
};
