# 按铁律顺序清理开发进程：chaojiandai -> llama-server -> webview2(本项目) -> python(仅tts_sidecar)
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/kill-dev.ps1
$ErrorActionPreference = 'SilentlyContinue'
Get-Process chaojiandai | Stop-Process -Force
Get-Process llama-server | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
  Where-Object { $_.CommandLine -like '*chaojiandai*' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
# python 侧 car 清理（坑 52）：按 CommandLine 匹配会静默漏杀——个别进程的
# CIM CommandLine 取不到（null），-like 恒 false，孤儿 sidecar 可因此存活多日。
# 双条件兜底：命令行含 tts_sidecar 或 可执行文件在仓库 tts-venv 下；并打印实杀清单
$killed = Get-CimInstance Win32_Process -Filter "Name like 'python%'" |
  Where-Object {
    ($_.CommandLine -like '*tts_sidecar*') -or ($_.ExecutablePath -like '*tts-venv*')
  } |
  ForEach-Object {
    Stop-Process -Id $_.ProcessId -Force
    $_.ProcessId
  }
Write-Output ("python sidecar killed: " + (($killed | ForEach-Object { [string]$_ }) -join ', '))
Start-Sleep 2
Write-Output 'cleanup done'
