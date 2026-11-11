# 潮间带（desktop-pet-meili）工程守则

## 开工必读（按顺序）

1. `docs/踩坑与环境档案.md` —— 环境坑与解法汇总，**每个新任务开工前先读**，防止重复踩坑；有新坑随手追加。
2. `docs/产品设计.md` / `docs/技术架构.md` —— 已定稿的产品与技术决策，不要重开讨论，改动需经用户确认。
3. `docs/工程规范.md` —— 代码写法宪法（M1 起生效）：目录细则、Rust/TS 规范、IPC 类型同步、测试策略、AI 协作规范。所有新代码必须符合。

## 常用入口

```bash
source scripts/dev-shell.sh    # 配好 PATH（cargo + windres），Git Bash 每个新 shell 都要
cd src-tauri && cargo build    # 编译（禁止全量 clean，缓存很贵）
```

- 运行 exe：用 PowerShell `Start-Process`（Git Bash 直接执行会 Exec format error，原因见踩坑档案坑 8）
- 验证窗口：`scripts/check-window.ps1`
- 定位 build 脚本 panic：catch_unwind 写文件法（踩坑档案坑 10）

## 红线

- 不重开已定稿的产品/架构决策。
- 不执行全量 `cargo clean`；只许 `cargo clean -p chaojiandai`。
- 工具链目录 `tools/` 不进 git；不放 /tmp。
- GitHub 不通，下载走清华镜像。
- 每完成一个任务就提交一次代码（git commit，信息写清任务编号与结论）。
- 写实现细节时，若逻辑明显符合某个设计模式、或存在多种等价写法，**先提出并与用户讨论定案，不擅自选择**（详见工程规范 §5.6）。
