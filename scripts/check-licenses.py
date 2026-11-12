#!/usr/bin/env python3
"""许可证白名单扫描（工程规范 §6：cargo-deny + license-checker 的本机替代版）。

为什么不是 cargo-deny：本机 gcc 5.3 编译 cargo-deny 的 C 依赖（zstd-sys）必 ICE
（踩坑档案坑 25b），装不了官方二进制。本脚本用 cargo metadata / node_modules
的权威元数据做等价检查，零编译。

白名单口径同工程规范 §6：MIT/Apache 及变体、宽松许可放行；
GPL/AGPL/CC-BY-NC 等出现即拦（exit 1）。
"""
import json
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent

# 放行：宽松许可（规范：MIT/Apache 及变体；BSD/ISC/CC0/Zlib 同为宽松系）
ALLOW = {
    "MIT", "MIT-0", "Apache-2.0", "Apache-2.0 WITH LLVM-exception",
    "BSD-2-Clause", "BSD-3-Clause", "ISC", "CC0-1.0", "Zlib",
    "Unicode-DFS-2016", "Unicode-3.0", "0BSD", "Unlicense",
}
# 即拦：传染/非商业（规范 §6 点名）
DENY = {"GPL-2.0", "GPL-3.0", "AGPL-3.0", "CC-BY-NC-4.0", "CC-BY-NC-SA-4.0",
        "LGPL-2.1", "LGPL-3.0", "SSPL-1.0"}
# 个案豁免：非白名单但已确认可接受（写入理由，审计可查）
OVERRIDES = {
    "MPL-2.0": "弱传染（文件级），Tauri 生态依赖（cssparser/selectors 等）不可避免；与闭源分发兼容",
    "BlueOak-1.0.0": "OSI 批准的宽松许可（npm minimatch 传递依赖）",
}

problems: list[str] = []


def _split(expr: str, op: str) -> list[str]:
    """按顶层 op 切分。cargo metadata 的 SPDX 表达式操作符总是空格分隔，够用。"""
    return [p.strip() for p in expr.split(f" {op} ")]


def _token_ok(token: str) -> bool:
    token = token.strip("() ")
    if token in ALLOW or token in OVERRIDES:
        return True
    # cargo 生态惯用 "MIT/Apache-2.0"（无空格斜杠）
    if "/" in token:
        return all(_token_ok(t) for t in token.split("/"))
    return False


def check(name: str, version: str, license_expr: str | None) -> None:
    if not license_expr or license_expr in {"NOASSERTION", "UNLICENSED"}:
        problems.append(f"[待确认] {name}@{version}: 无许可证声明")
        return
    expr = license_expr.strip()
    # AND：所有分支都必须放行；OR：任一分支放行即可（SPDX 语义，如
    # "MIT OR Apache-2.0 OR LGPL-2.1-or-later" 取 MIT 即合规）
    and_clauses = _split(expr, "AND")
    if len(and_clauses) > 1:
        ok = all(any(_token_ok(t) for t in _split(c, "OR")) for c in and_clauses)
    else:
        ok = any(_token_ok(t) for t in _split(expr, "OR"))
    # 显式黑名单优先：表达式里直接出现 DENY（OR 也不豁免用户可见的强传染）
    flat = {t.strip("() ") for c in and_clauses for t in c.replace("/", " ").split()}
    hits_deny = flat & DENY
    if hits_deny:
        problems.append(f"[拦截] {name}@{version}: {license_expr}（命中 {sorted(hits_deny)}）")
    elif not ok:
        problems.append(f"[待确认] {name}@{version}: {license_expr}")


def check_rust() -> None:
    meta = json.loads(subprocess.run(
        ["cargo", "metadata", "--format-version", "1", "--locked"],
        cwd=ROOT / "src-tauri", capture_output=True, check=True,
        encoding="utf-8", errors="replace",
    ).stdout)
    for pkg in meta["packages"]:
        check(f"{pkg['name']}(rust)", pkg["version"], pkg.get("license"))


def check_npm() -> None:
    seen: set[str] = set()
    for pkg_json in sorted((ROOT / "node_modules").rglob("package.json")):
        rel = pkg_json.relative_to(ROOT / "node_modules")
        if rel.parts[0].startswith(".") or "node_modules" in rel.parts[:-1]:
            continue  # .bin 等 / 依赖内嵌套的 node_modules
        try:
            info = json.loads(pkg_json.read_text(encoding="utf-8"))
        except (OSError, json.JSONDecodeError):
            continue
        if not info.get("name"):
            continue  # 子路径 stub（dist/commonjs/package.json 等），许可证在父包
        key = f"{info.get('name', '?')}@{info.get('version', '?')}"
        if key in seen:
            continue
        seen.add(key)
        # license 字段优先，其次旧式 licenses 数组
        lic = info.get("license")
        if not lic and info.get("licenses"):
            lic = info["licenses"][0].get("type")
        check(f"{info.get('name', '?')}(npm)", info.get("version", "?"), lic)


def main() -> int:
    scope = sys.argv[1] if len(sys.argv) > 1 else "all"
    if scope in ("all", "rust"):
        check_rust()
    if scope in ("all", "npm"):
        check_npm()
    if problems:
        print(f"许可证检查未过（{len(problems)} 项）：")
        for p in problems:
            print(" ", p)
        return 1
    print("许可证检查通过：Rust + npm 依赖全部在白名单内")
    return 0


if __name__ == "__main__":
    sys.exit(main())
