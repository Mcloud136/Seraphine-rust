param(
    [string]$Exe,
    [string]$ProcessName,
    [int]$SettleSeconds = 6
)

$sw = [System.Diagnostics.Stopwatch]::StartNew()
$process = Start-Process -FilePath $Exe -PassThru
while ($sw.Elapsed.TotalSeconds -lt 30) {
    Start-Sleep -Milliseconds 100
    $process.Refresh()
    if ($process.MainWindowHandle -ne 0) { break }
}
$startupMs = [math]::Round($sw.Elapsed.TotalMilliseconds)
Start-Sleep -Seconds $SettleSeconds

function Get-TreeMemory([int]$rootId) {
    $all = Get-CimInstance Win32_Process | Select-Object ProcessId, ParentProcessId, WorkingSetSize
    $ids = New-Object System.Collections.Generic.HashSet[int]
    [void]$ids.Add($rootId)
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
    $bytes = ($all | Where-Object { $ids.Contains([int]$_.ProcessId) } | Measure-Object WorkingSetSize -Sum).Sum
    [pscustomobject]@{ Processes = $ids.Count; WorkingSetMB = [math]::Round($bytes / 1MB, 1) }
}

$tree = Get-TreeMemory $process.Id
$process.Refresh()
Write-Output ("startupMs={0} appWorkingSetMB={1} treeProcesses={2} treeWorkingSetMB={3} pid={4}" -f `
    $startupMs, [math]::Round($process.WorkingSet64 / 1MB, 1), $tree.Processes, $tree.WorkingSetMB, $process.Id)