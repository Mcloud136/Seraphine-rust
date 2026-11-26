param(
    [string]$ProcessName,
    [int]$Seconds = 20
)

$root = Get-Process -Name $ProcessName -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
$all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, Name
$ids = New-Object System.Collections.Generic.HashSet[int]
[void]$ids.Add([int]$root.Id)
$changed = $true
while ($changed) {
    $changed = $false
    foreach ($item in $all) {
        if ($ids.Contains([int]$item.ParentProcessId) -and -not $ids.Contains([int]$item.ProcessId)) {
            [void]$ids.Add([int]$item.ProcessId)
            $changed = $true
        }
    }
}

function Get-CpuSeconds {
    $total = 0.0
    foreach ($id in $ids) {
        $process = Get-Process -Id $id -ErrorAction SilentlyContinue
        if ($process -and $process.ProcessName -notlike '*Sogou*') {
            $total += $process.CPU
        }
    }
    return $total
}

$before = Get-CpuSeconds
Start-Sleep -Seconds $Seconds
$after = Get-CpuSeconds
$percent = [math]::Round((($after - $before) / $Seconds) * 100, 2)
Write-Output ("idleCpuPercentOfOneCore={0} processes={1} seconds={2}" -f $percent, $ids.Count, $Seconds)