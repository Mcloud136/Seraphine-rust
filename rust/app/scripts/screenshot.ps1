param(
    [string]$ProcessName = "seraphine-app",
    [string]$Output = "D:\wxmuma\Seraphine-rust\rust\app\screenshot.png"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class NativeWindow {
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr hWnd, IntPtr hdcBlt, uint nFlags);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$process = Get-Process -Name $ProcessName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $process) { throw "No window found for $ProcessName" }
$handle = $process.MainWindowHandle

$rect = New-Object NativeWindow+RECT
[NativeWindow]::GetWindowRect($handle, [ref]$rect) | Out-Null
$width = $rect.Right - $rect.Left
$height = $rect.Bottom - $rect.Top
Add-Type -AssemblyName System.Drawing
$bitmap = New-Object System.Drawing.Bitmap $width, $height
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$hdc = $graphics.GetHdc()
$flags = 2 -bor 1
$ok = [NativeWindow]::PrintWindow($handle, $hdc, $flags)
$graphics.ReleaseHdc($hdc)
$graphics.Dispose()
$bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)
$bitmap.Dispose()
Write-Output "saved $Output ($width x $height) printWindow=$ok"