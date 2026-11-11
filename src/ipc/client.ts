/**
 * 与 Rust 通信的唯一入口（工程规范 §1 TS 规则：业务代码禁止直接 invoke）。
 * 换协议时只改这里。
 *
 * types.ts 由 specta 从 Rust 生成（不手改）；本文件只做 re-export，
 * 让业务代码的 import 面只有 client.ts 一处。
 */

export { commands, events } from "./types";
export type {
  Booted,
  EventKind,
  EventSource,
  PrivacyTier,
  StdEvent,
} from "./types";
