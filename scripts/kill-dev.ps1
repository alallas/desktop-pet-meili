# 按铁律顺序清理开发进程：chaojiandai -> llama-server -> webview2(本项目) -> python(仅tts_sidecar)
# 用法: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/kill-dev.ps1
$ErrorActionPreference = 'SilentlyContinue'
Get-Process chaojiandai | Stop-Process -Force
Get-Process llama-server | Stop-Process -Force
Get-CimInstance Win32_Process -Filter "Name='msedgewebview2.exe'" |
  Where-Object { $_.CommandLine -like '*chaojiandai*' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Get-CimInstance Win32_Process -Filter "Name='python.exe'" |
  Where-Object { $_.CommandLine -like '*tts_sidecar*' } |
  ForEach-Object { Stop-Process -Id $_.ProcessId -Force }
Start-Sleep 2
Write-Output 'cleanup done'
