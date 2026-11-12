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
#   m2w64-headers-git-5.0.0.4636.c0ad18a-2.tar.bz2 → tools/m2w64-headers
#   m2w64-winpthreads-git-5.0.0.4634.697f757-2.tar.bz2 → tools/m2w64-winpthreads
#     （gcc 编译 C 源码用：gcc 自带 stdarg.h 是 include_next 转发壳，必须搭配
#       mingw-w64 运行时头文件；time.h 还需要 winpthreads 的 pthread_time.h。
#       不装则 bundled SQLite 等 C 依赖编译报 "stdarg.h: No such file or
#       directory"，见踩坑档案坑 25）

PROJECT_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"

export PATH="$HOME/.cargo/bin:$PROJECT_ROOT/tools/m2w64-gcc/Library/mingw-w64/bin:$PROJECT_ROOT/tools/m2w64/Library/mingw-w64/bin:$PATH"

# 本机有常驻代理（Clash Verge，127.0.0.1:7897，实测 github.com 200）。
# 凡走 GitHub/海外源的下载（tauri 打包器拉 NSIS、cargo install 的 git 依赖等）
# 需要代理；国内镜像走代理反而慢/不通，列进 NO_PROXY。
if netstat -an | grep -q "127.0.0.1:7897.*LISTENING"; then
  export HTTPS_PROXY="http://127.0.0.1:7897"
  export HTTP_PROXY="http://127.0.0.1:7897"
  export NO_PROXY="localhost,127.0.0.1,.tsinghua.edu.cn,.ustc.edu.cn,.nju.edu.cn,.sjtu.edu.cn,.aliyun.com,.tencent.com,.huaweicloud.com,.bfsu.edu.cn,modelscope.cn,mirrors.cloud.tencent.com"
fi
export C_INCLUDE_PATH="$PROJECT_ROOT/tools/m2w64-headers/Library/mingw-w64/x86_64-w64-mingw32/include:$PROJECT_ROOT/tools/m2w64-winpthreads/Library/mingw-w64/x86_64-w64-mingw32/include${C_INCLUDE_PATH:+:$C_INCLUDE_PATH}"

echo "dev-shell ready. cargo: $(command -v cargo || echo MISSING), windres: $(command -v windres || echo MISSING)"
echo "构建: cd src-tauri && cargo build"
echo "运行: powershell -NoProfile -Command \"Start-Process '$PROJECT_ROOT/src-tauri/target/debug/chaojiandai.exe'\""
echo "验证窗口: powershell -NoProfile -ExecutionPolicy Bypass -File '$PROJECT_ROOT/scripts/check-window.ps1'"
