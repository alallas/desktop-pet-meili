# 《潮间带》TTS sidecar 依赖固化（待办 #14）：建仓库自有 venv + 锁定安装 + 打补丁 +  vendor HF 资产
# 产物：tools/tts-venv（gitignored）、models/hf-cache（gitignored，离线 HF 缓存布局）
# 用法（仓库根）：powershell -NoProfile -ExecutionPolicy Bypass -File scripts/setup-tts-venv.ps1 [-BasePython <解释器>]
# 幂等：venv 已存在则复用；补丁带版本标记，重复执行不叠加。
param(
    # venv 基底解释器（默认链：参数 > CHAOJIANDAI_PY_BASE 环境变量 > py -3.12 > python）
    [string]$BasePython = ""
)

$ErrorActionPreference = 'Stop'
$root = Resolve-Path (Join-Path $PSScriptRoot "..")
$venv = Join-Path $root "tools\tts-venv"
$venvPy = Join-Path $venv "Scripts\python.exe"
$mirror = "https://pypi.tuna.tsinghua.edu.cn/simple"

function Pick-BasePython {
    if ($BasePython) { return $BasePython }
    if ($env:CHAOJIANDAI_PY_BASE) { return $env:CHAOJIANDAI_PY_BASE }
    $py = Get-Command py -ErrorAction SilentlyContinue
    if ($py) { return "py -3.12" }
    return "python"
}

# 1) venv
if (-not (Test-Path $venvPy)) {
    $base = Pick-BasePython
    Write-Host "创建 venv（基底: $base）→ $venv"
    & $base -m venv $venv
    if (-not (Test-Path $venvPy)) { throw "venv 创建失败" }
} else {
    Write-Host "venv 已存在，复用: $venv"
}

# 2) 锁定安装（坑 16 流程固化：--no-deps 装 kokoro/misaki 绕开 spacy 地狱，依赖显式列出）
Write-Host "安装 kokoro/misaki（--no-deps，锁定 0.9.4）…"
& $venvPy -m pip install --no-deps -i $mirror "kokoro==0.9.4" "misaki==0.9.4"

Write-Host "安装 torch CPU 版…"
# CPU 轮在 pytorch.org 专属 index（PyPI/tuna 默认 CUDA 版会多拉数 GB nvidia 包）
& $venvPy -m pip install --index-url "https://download.pytorch.org/whl/cpu" "torch==2.15.0"
if ($LASTEXITCODE -ne 0) {
    Write-Host "pytorch.org 不可达，回退 tuna 默认版（体积更大）…"
    & $venvPy -m pip install -i $mirror "torch==2.15.0"
}

Write-Host "安装显式依赖（版本锁定自 2026-11-22 托管 venv 实测组合）…"
& $venvPy -m pip install -i $mirror `
    "numpy==2.4.4" "soundfile==0.14.0" "huggingface_hub==2.4.0" "loguru==0.7.3" `
    "transformers==5.22.2" "attrs==26.1.0" "addict==2.4.0" "ordered-set==4.1.0" `
    "pypinyin==0.55.0" "jieba==0.42.1" "cn2an==0.5.24" "zhon==2.1.1"

# 3) 懒加载补丁：zh-only 免 spacy/espeak-ng（坑 16）。整文件覆盖（补丁与 kokoro==0.9.4 严格同版本）
$patchSrc = Join-Path $root "scripts\assets\kokoro_pipeline_0.9.4_patched.py"
$patchDst = Join-Path $venv "Lib\site-packages\kokoro\pipeline.py"
if (-not (Test-Path $patchDst)) { throw "kokoro 未安装成功（缺 $patchDst）" }
Write-Host "应用懒加载补丁 → $patchDst"
Copy-Item $patchSrc $patchDst -Force

# 4) vendor HF 资产 → models/hf-cache 离线布局（refs/main → snapshots/local）
$srcCache = Join-Path $env:USERPROFILE ".cache\huggingface\hub\models--hexgrad--Kokoro-82M\snapshots"
$dstCache = Join-Path $root "models\hf-cache\models--hexgrad--Kokoro-82M"
if (Test-Path $srcCache) {
    $hash = (Get-ChildItem $srcCache -Directory | Select-Object -First 1).Name
    $src = Join-Path $srcCache $hash
    Write-Host "vendor HF 资产（$src → $dstCache）…"
    New-Item -ItemType Directory -Force (Join-Path $dstCache "refs") | Out-Null
    Set-Content (Join-Path $dstCache "refs\main") "local" -NoNewline
    $dstSnap = Join-Path $dstCache "snapshots\local"
    New-Item -ItemType Directory -Force $dstSnap | Out-Null
    Copy-Item (Join-Path $src "config.json") $dstSnap -Force
    New-Item -ItemType Directory -Force (Join-Path $dstSnap "voices") | Out-Null
    Copy-Item (Join-Path $src "voices\zf_xiaoxiao.pt") (Join-Path $dstSnap "voices") -Force
} else {
    Write-Warning "未找到 HF 缓存 $srcCache——跳过 vendor。其他机器请自行放置 config.json 与 voices/zf_xiaoxiao.pt"
}

Write-Host "完成。venv: $venvPy"
