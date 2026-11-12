#!/usr/bin/env bash
# 《潮间带》CI 六项检查（工程规范 §6）。本地跑 = CI 跑。
# 用法：source scripts/dev-shell.sh && bash scripts/ci.sh
set -euo pipefail
cd "$(dirname "$0")/.."

echo "== [1/6] cargo fmt =="
(cd src-tauri && cargo fmt --check)

echo "== [2/6] cargo clippy =="
(cd src-tauri && cargo clippy --all-targets -- -D warnings)

echo "== [3/6] cargo test =="
(cd src-tauri && cargo test)

echo "== [4/6] eslint + tsc + vitest =="
npx eslint src
npx tsc --noEmit
npx vitest run

echo "== [5/6] IPC 生成物新鲜度 =="
# bindings_fresh 测试内已覆盖（改了 ipc.rs 忘重新生成会红），这里随 [3/6] 跑过
echo "（含于 cargo test: tests::bindings_fresh）"

echo "== [6/6] 许可证 + 漏洞库 =="
# Rust 侧：cargo-deny 官方二进制（坑 26：走本机代理下载的预编译包）
(cd src-tauri && ../tools/cargo-deny-bin/*/cargo-deny.exe check)
# npm 侧：自研脚本（cargo-deny 不覆盖 node_modules）
python scripts/check-licenses.py npm

echo "全部通过。"
