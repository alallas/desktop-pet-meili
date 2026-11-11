#!/usr/bin/env bash
# 潮间带开发环境 shell（Git Bash）
# 用法：source scripts/dev-shell.sh
#
# 背景：本机无 MSVC（缺 Windows SDK 库文件），默认 windows-gnu 工具链的 ld 过旧，
# 链接统一走 rust 自带 rust-lld（见仓库根 .cargo/config.toml），此处只负责：
#   1. 把 cargo 加入 PATH
#   2. 提供 windres（资源编译）：m2w64-binutils + m2w64-gcc（windres 预处理要调 gcc）
# 来源：清华 anaconda 镜像
#   https://mirrors.tuna.tsinghua.edu.cn/anaconda/pkgs/msys2/win-64/
#   m2w64-binutils-2.25.1-5.tar.bz2 → tools/m2w64
#   m2w64-gcc-5.3.0-6.tar.bz2      → tools/m2w64-gcc

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export PATH="$HOME/.cargo/bin:$PROJECT_ROOT/tools/m2w64-gcc/Library/mingw-w64/bin:$PROJECT_ROOT/tools/m2w64/Library/mingw-w64/bin:$PATH"

echo "dev-shell ready. cargo: $(command -v cargo || echo MISSING), windres: $(command -v windres || echo MISSING)"
echo "构建: cd src-tauri && cargo build"
echo "运行: powershell -NoProfile -Command \"Start-Process '$PROJECT_ROOT/src-tauri/target/debug/chaojiandai.exe'\""
echo "验证窗口: powershell -NoProfile -ExecutionPolicy Bypass -File '$PROJECT_ROOT/scripts/check-window.ps1'"
