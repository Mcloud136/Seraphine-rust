param(
    [string]$Exe,
    [string]$ProcessName,
    [int]$Runs = 5,
    [int]$SettleSeconds = 4
)

$results = @()
for ($i = 1; $i -le $Runs; $i++) {
    Get-Process -Name $ProcessName -ErrorAction SilentlyContinue | Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 800
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $process = Start-Process -FilePath $Exe -PassThru
    while ($sw.Elapsed.TotalSeconds -lt 30) {
        Start-Sleep -Milliseconds 50
        $process.Refresh()
        if ($process.MainWindowHandle -ne 0) { break }
    }
    $startup = [math]::Round($sw.Elapsed.TotalMilliseconds)
    Start-Sleep -Seconds $SettleSeconds
    $process.Refresh()
    $workingSet = [math]::Round($process.WorkingSet64 / 1MB, 1)
    $results += [pscustomobject]@{ Run = $i; StartupMs = $startup; WorkingSetMB = $workingSet }
    $process | Stop-Process -Force -ErrorAction SilentlyContinue
}

$startups = $results | ForEach-Object { $_.StartupMs } | Sort-Object
$medians = [math]::Round(($startups[[int][math]::Floor($startups.Count / 2)]), 0)
$results | Format-Table -AutoSize | Out-String | Write-Output
Write-Output ("startupMedianMs={0} startupMinMs={1} startupMaxMs={2} runs={3}" -f `
    $medians, $startups[0], $startups[-1], $Runs)