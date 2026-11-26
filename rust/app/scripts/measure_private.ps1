param(
    [string]$ProcessName,
    [int]$SettleSeconds = 6
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
Start-Sleep -Seconds $SettleSeconds
$rows = @()
$total = 0.0
$ws = 0.0
foreach ($id in $ids) {
    $process = Get-Process -Id $id -ErrorAction SilentlyContinue
    if (-not $process) { continue }
    if ($process.ProcessName -like '*Sogou*') { continue }
    $rows += [pscustomobject]@{
        Name = $process.ProcessName
        Pid = $process.Id
        PrivateMB = [math]::Round($process.PrivateMemorySize64 / 1MB, 1)
        WorkingSetMB = [math]::Round($process.WorkingSet64 / 1MB, 1)
    }
    $total += $process.PrivateMemorySize64
    $ws += $process.WorkingSet64
}
$rows | Sort-Object PrivateMB -Descending | Format-Table -AutoSize | Out-String | Write-Output
Write-Output ("treePrivateMB={0} treeWorkingSetMB={1} processes={2}" -f `
    [math]::Round($total / 1MB, 1), [math]::Round($ws / 1MB, 1), $rows.Count)