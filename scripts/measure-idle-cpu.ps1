# Silent CPU/memory measurement (tech-arch §7 red line: CPU <1%, engine+WebView <300MB)
# Scope: chaojiandai.exe process tree (incl. WebView2 children) + resident sidecars.
# CPU% is per-core (100% = one full core).
# Usage: powershell -NoProfile -ExecutionPolicy Bypass -File scripts/measure-idle-cpu.ps1 [-Seconds 10]
param([int]$Seconds = 10)

$ErrorActionPreference = 'SilentlyContinue'

# One-shot CIM snapshot of all processes (pid, name, parent, cmdline)
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name, CommandLine
$byParent = @{}
foreach ($p in $all) {
    $key = [int]$p.ParentProcessId
    if (-not $byParent.ContainsKey($key)) { $byParent[$key] = @() }
    $byParent[$key] += [int]$p.ProcessId
}

$ids = @{}
# resident sidecars matched by command line
foreach ($p in $all) {
    if ($p.CommandLine -match 'llama-server' -or $p.CommandLine -match 'tts_sidecar') {
        $ids[[int]$p.ProcessId] = [string]$p.Name
    }
}
# BFS from every chaojiandai.exe root
$queue = @()
foreach ($p in $all) { if ($p.Name -eq 'chaojiandai.exe') { $queue += [int]$p.ProcessId } }
while ($queue.Count -gt 0) {
    $curId = $queue[0]
    if ($queue.Count -gt 1) { $queue = $queue[1..($queue.Count-1)] } else { $queue = @() }
    if ($ids.ContainsKey($curId)) { continue }
    $info = $all | Where-Object { $_.ProcessId -eq $curId }
    if (-not $info) { continue }
    $ids[$curId] = [string]$info.Name
    if ($byParent.ContainsKey($curId)) { $queue += $byParent[$curId] }
}

if ($ids.Count -eq 0) { Write-Output 'chaojiandai process tree not found'; exit 1 }

function Snapshot($ids) {
    $snap = @{}
    foreach ($procId in $ids.Keys) {
        $proc = Get-Process -Id $procId
        if ($proc) { $snap[$procId] = @{ Cpu = $proc.CPU; WsMB = [math]::Round($proc.WorkingSet64 / 1MB) } }
    }
    return $snap
}

$s1 = Snapshot $ids
Write-Output ("sampling {0}s over {1} processes..." -f $Seconds, $ids.Count)
Start-Sleep -Seconds $Seconds
$s2 = Snapshot $ids

$totalCpuPct = 0.0
$totalWs = 0
$rows = @()
foreach ($procId in $s2.Keys) {
    $dCpu = 0.0
    if ($s1.ContainsKey($procId)) { $dCpu = $s2[$procId].Cpu - $s1[$procId].Cpu }
    $pct = [math]::Round($dCpu / $Seconds * 100, 2)
    $totalCpuPct += $pct
    $totalWs += $s2[$procId].WsMB
    $rows += "{0,-28} pid={1,-7} CPU={2,7}%  WS={3,6}MB" -f $ids[$procId], $procId, $pct, $s2[$procId].WsMB
}
$rows | Sort-Object
Write-Output ("-" * 60)
Write-Output ("TOTAL: CPU={0}% (per-core)  MEM={1}MB" -f [math]::Round($totalCpuPct, 2), $totalWs)
Write-Output ("RED LINE: CPU <1%  engine+WebView MEM <300MB (sidecars counted separately)")
