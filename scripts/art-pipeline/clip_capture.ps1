param(
  [Parameter(Mandatory = $true)][string]$WsUrl,
  [Parameter(Mandatory = $true)][string]$OutDir
)
# 逐 clip 触发 + 每 clip 抓 3 帧（0.35s 间隔），抓的是一次性动作的中段
$keys = @("1","2","3","4","5","6","7","8","9","0")
New-Item -ItemType Directory -Force -Path $OutDir | Out-Null
foreach ($k in $keys) {
  & "$PSScriptRoot\..\cdp-eval.ps1" -WsUrl $WsUrl -Expr "document.dispatchEvent(new KeyboardEvent('keydown',{key:'$k'}))" | Out-Null
  Start-Sleep -Milliseconds 300
  python "$PSScriptRoot\..\screenshot_dxgi.py" "$OutDir\clip_${k}_a.png" | Out-Null
  Start-Sleep -Milliseconds 350
  python "$PSScriptRoot\..\screenshot_dxgi.py" "$OutDir\clip_${k}_b.png" | Out-Null
  Start-Sleep -Milliseconds 350
  python "$PSScriptRoot\..\screenshot_dxgi.py" "$OutDir\clip_${k}_c.png" | Out-Null
  Write-Output "clip $k done"
}
