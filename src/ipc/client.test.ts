// IPC 客户端导出面的契约测试（工程规范 §4：TS 单元 vitest 范围=纯逻辑）
// types.ts 是生成物，这里只验证 client.ts 的 re-export 与事件名常量稳定。
import { describe, expect, it } from "vitest";
import { commands, events } from "./client";

describe("ipc client", () => {
  it("re-exports specta 生成的命令", () => {
    expect(typeof commands.ping).toBe("function");
    expect(typeof commands.debugLog).toBe("function");
  });

  it("事件监听通道存在且名称稳定（改名是协议变更，此测试会红）", () => {
    expect(typeof events.booted.listen).toBe("function");
  });
});
