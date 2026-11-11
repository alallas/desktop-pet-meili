Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Win32 {
  [DllImport("user32.dll")] public static extern int GetWindowLong(IntPtr h, int i);
  [DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
}
"@
$p = Get-Process chaojiandai -ErrorAction SilentlyContinue
if (-not $p) { Write-Host "PROCESS_NOT_FOUND"; exit 1 }
Write-Host ("pid=" + $p.Id)
foreach ($proc in $p) {
  $h = $proc.MainWindowHandle
  Write-Host ("MainWindowHandle=0x" + ('{0:X}' -f [Int64]$h))
  if ($h -ne 0) {
    $ex = [Win32]::GetWindowLong($h, -20)
    $vis = [Win32]::IsWindowVisible($h)
    Write-Host ("GWL_EXSTYLE=0x" + ('{0:X8}' -f $ex) + " visible=" + $vis)
    $flags = @{
      'WS_EX_LAYERED'    = 0x80000
      'WS_EX_TRANSPARENT'= 0x20
      'WS_EX_TOPMOST'    = 0x8
      'WS_EX_TOOLWINDOW' = 0x80
      'WS_EX_NOACTIVATE' = 0x8000000
    }
    foreach ($k in $flags.Keys) {
      Write-Host ("  " + $k + "=" + [bool]($ex -band $flags[$k]))
    }
  }
}
