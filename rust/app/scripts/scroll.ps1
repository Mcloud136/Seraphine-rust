param(
    [int]$X = 600,
    [int]$Y = 500,
    [int]$Delta = -600,
    [int]$Times = 3,
    [string]$ProcessName = "seraphine-app"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class NativeWheel {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, int dx, int dy, int dwData, IntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@

$process = Get-Process -Name $ProcessName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$handle = $process.MainWindowHandle
$flags = 0x0001 -bor 0x0002 -bor 0x0040
[NativeWheel]::SetWindowPos($handle, [IntPtr](-1), 0, 0, 0, 0, $flags) | Out-Null
[NativeWheel]::SetWindowPos($handle, [IntPtr](-2), 0, 0, 0, 0, $flags) | Out-Null
[NativeWheel]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Milliseconds 400

$rect = New-Object NativeWheel+RECT
[NativeWheel]::GetWindowRect($handle, [ref]$rect) | Out-Null
[NativeWheel]::SetCursorPos($rect.Left + $X, $rect.Top + $Y) | Out-Null
Start-Sleep -Milliseconds 150
for ($i = 0; $i -lt $Times; $i++) {
    [NativeWheel]::mouse_event(0x0800, 0, 0, $Delta, [IntPtr]::Zero)
    Start-Sleep -Milliseconds 120
}
Write-Output "scrolled $Times x $Delta"