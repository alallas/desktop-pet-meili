# Thread-level CPU profiler for chaojiandai.exe (#37)
# Ranks per-thread CPU deltas over N seconds using .NET Process.Threads.
param([int]$Seconds = 10, [string]$ProcessName = "chaojiandai")

$ErrorActionPreference = 'SilentlyContinue'
$proc = Get-Process -Name $ProcessName
if (-not $proc) { Write-Output "process $ProcessName not found"; exit 1 }
$targetPid = $proc.Id

function ThreadSnapshot($targetPid) {
    $snap = @{}
    foreach ($t in (Get-Process -Id $targetPid).Threads) {
        $snap[[int]$t.Id] = $t.TotalProcessorTime.TotalMilliseconds
    }
    return $snap
}

$s1 = ThreadSnapshot $targetPid
Start-Sleep -Seconds $Seconds
$s2 = ThreadSnapshot $targetPid

$rows = @()
foreach ($tid in $s2.Keys) {
    $d = 0.0
    if ($s1.ContainsKey($tid)) { $d = $s2[$tid] - $s1[$tid] }
    $pct = [math]::Round($d / ($Seconds * 1000.0) * 100.0, 2)
    if ($pct -gt 0.05) { $rows += [PSCustomObject]@{ Tid = $tid; Cpu = $pct } }
}
$rows | Sort-Object Cpu -Descending | Format-Table -AutoSize
$sum = ($rows | Measure-Object Cpu -Sum).Sum
Write-Output ("threads={0} hot={1} sampled-CPU-total={2}% (per-core)" -f $s2.Count, $rows.Count, [math]::Round($sum, 2))
