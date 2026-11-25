param([IntPtr]$Handle)
$src = @"
using System;
using System.Runtime.InteropServices;
public class U32 {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
Add-Type -TypeDefinition $src
$r = New-Object U32+RECT
[U32]::GetWindowRect($Handle, [ref]$r) | Out-Null
Write-Output "$($r.Left),$($r.Top),$($r.Right),$($r.Bottom)"
