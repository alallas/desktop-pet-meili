param([IntPtr]$Handle, [string]$Out)
$src = @"
using System;
using System.Runtime.InteropServices;
public class PWC {
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hwnd, IntPtr hdc, uint flags);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool GetClientRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool ClientToScreen(IntPtr h, ref POINT p);
    public struct RECT { public int Left, Top, Right, Bottom; }
    public struct POINT { public int X, Y; }
}
"@
Add-Type -TypeDefinition $src
Add-Type -AssemblyName System.Drawing
$cr = New-Object PWC+RECT
[PWC]::GetClientRect($Handle, [ref]$cr) | Out-Null
$pt = New-Object PWC+POINT
[PWC]::ClientToScreen($Handle, [ref]$pt) | Out-Null
$w = $cr.Right - $cr.Left; $h = $cr.Bottom - $cr.Top
$bmp = New-Object System.Drawing.Bitmap($w, $h)
$g = [System.Drawing.Graphics]::FromImage($bmp)
$hdc = $g.GetHdc()
# PW_RENDERFULLCONTENT=2 | PW_CLIENTONLY=1
[PWC]::PrintWindow($Handle, $hdc, 3) | Out-Null
$g.ReleaseHdc($hdc)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$g.Dispose(); $bmp.Dispose()
Write-Output "saved ${w}x${h} at ($($pt.X),$($pt.Y))"
