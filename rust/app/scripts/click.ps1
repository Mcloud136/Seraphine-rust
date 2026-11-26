param(
    [int]$X,
    [int]$Y,
    [string]$Text = "",
    [switch]$Enter,
    [string]$ProcessName = "seraphine-app"
)

Add-Type @"
using System;
using System.Runtime.InteropServices;
public class NativeInput {
    [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
    [DllImport("user32.dll")] public static extern void mouse_event(uint dwFlags, uint dx, uint dy, uint dwData, IntPtr dwExtraInfo);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr hWnd, out RECT rect);
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr hWnd);
    [DllImport("user32.dll")] public static extern bool SetWindowPos(IntPtr hWnd, IntPtr hWndInsertAfter, int X, int Y, int cx, int cy, uint uFlags);
    public struct RECT { public int Left, Top, Right, Bottom; }
}
"@
Add-Type -AssemblyName System.Windows.Forms

$process = Get-Process -Name $ProcessName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $process) { throw "No window found for $ProcessName" }
$handle = $process.MainWindowHandle
$flags = 0x0001 -bor 0x0002 -bor 0x0040
[NativeInput]::SetWindowPos($handle, [IntPtr](-1), 0, 0, 0, 0, $flags) | Out-Null
[NativeInput]::SetWindowPos($handle, [IntPtr](-2), 0, 0, 0, 0, $flags) | Out-Null
[NativeInput]::SetForegroundWindow($handle) | Out-Null
Start-Sleep -Milliseconds 500

$rect = New-Object NativeInput+RECT
[NativeInput]::GetWindowRect($handle, [ref]$rect) | Out-Null
$screenX = $rect.Left + $X
$screenY = $rect.Top + $Y
[NativeInput]::SetCursorPos($screenX, $screenY) | Out-Null
Start-Sleep -Milliseconds 120
[NativeInput]::mouse_event(0x0002, 0, 0, 0, [IntPtr]::Zero)
[NativeInput]::mouse_event(0x0004, 0, 0, 0, [IntPtr]::Zero)
Start-Sleep -Milliseconds 400

if ($Text -ne "") {
    Set-Clipboard -Value $Text
    Start-Sleep -Milliseconds 120
    [System.Windows.Forms.SendKeys]::SendWait("^v")
    Start-Sleep -Milliseconds 250
}
if ($Enter) {
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
}
Write-Output "clicked $screenX,$screenY"