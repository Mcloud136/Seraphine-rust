param(
    [string]$OutputCsv = "D:\tmp\game-frames.csv",
    [int]$Seconds = 60,
    [string]$TargetProcess = "League of Legends.exe",
    [string]$SessionName = "GameBench"
)

$script = Join-Path $PSScriptRoot "bench_game_frames.py"
python $script --output $OutputCsv --seconds $Seconds --process $TargetProcess --session $SessionName
if ($LASTEXITCODE -ne 0) { throw "PresentMon capture failed (exit $LASTEXITCODE)" }
$stats = python (Join-Path $PSScriptRoot "analyze_game_frames.py") $OutputCsv $TargetProcess
Write-Output $stats