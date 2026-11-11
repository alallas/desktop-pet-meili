param([int]$X = -1, [int]$Y = -1, [string]$Out = "D:\Repos\desktop-pet-meili\target-shot.png")
Add-Type -AssemblyName System.Windows.Forms, System.Drawing
if ($X -ge 0) {
  Add-Type @"
using System;
using System.Runtime.InteropServices;
public class Cur { [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y); }
"@
  [Cur]::SetCursorPos($X, $Y) | Out-Null
  Start-Sleep -Milliseconds 800
}
$b = [System.Windows.Forms.SystemInformation]::VirtualScreen
$bmp = New-Object System.Drawing.Bitmap $b.Width, $b.Height
$g = [System.Drawing.Graphics]::FromImage($bmp)
$g.CopyFromScreen($b.Left, $b.Top, 0, 0, $bmp.Size)
$bmp.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
Write-Host "saved $Out ($($b.Width)x$($b.Height))"
