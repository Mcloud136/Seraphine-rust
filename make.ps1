<#
.SYNOPSIS
    This script is used to package "Seraphine".

.PARAMETER dest
    The target output path. Default is the current directory.

.PARAMETER dbg
    Whether to enable debug mode. If enabled, the `.\dist` directory will not be deleted and no 7z file will be created.

.EXAMPLE
    .\make.ps1 -dbg
#>

param(
    [Parameter()]
    [String]$dest = ".",
    [Switch]$dbg
)

$ErrorActionPreference = "Stop"
if (-not [System.IO.Path]::IsPathRooted($dest)) {
    $dest = Join-Path (Get-Location).Path $dest
}
Push-Location $PSScriptRoot
try {
    if (-not $dbg -and (Test-Path .\dist)) {
        Remove-Item -Recurse -Force .\dist
    }

    $pythonExe = (Get-Command python).Source
    $envRoot = Split-Path $pythonExe -Parent
    $env:PATH = "$envRoot;$envRoot\Library\bin;$envRoot\Scripts;$env:PATH"

    cargo build --release --manifest-path .\rust\Cargo.toml -p seraphine-sidecar --bins
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

    pyinstaller -w -i .\app\resource\images\logo.ico main.py
    if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
    Remove-Item -Recurse -Force .\build -ErrorAction SilentlyContinue
    Remove-Item -Force .\main.spec -ErrorAction SilentlyContinue
    Rename-Item -Path .\dist\main -NewName Seraphine
    Rename-Item -Path .\dist\Seraphine\main.exe -NewName Seraphine.exe

    Copy-Item .\app -Destination .\dist\Seraphine\_internal -Recurse
    Copy-Item .\rust\target\release\seraphine-sidecar.exe -Destination .\dist\Seraphine\_internal
    Copy-Item .\rust\target\release\seraphine-updater.exe -Destination .\dist\Seraphine\_internal
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\common
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\components
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\lol
    Remove-Item -Path .\dist\Seraphine\_internal\app\resource\game* -Recurse -ErrorAction SilentlyContinue
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\resource\i18n\Seraphine.zh_CN.ts
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\resource\bin\fix_lcu_window.c
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\resource\bin\readme.md
    Remove-Item -Recurse -Force .\dist\Seraphine\_internal\app\view

    $packageRoot = (Resolve-Path ".\dist\Seraphine").Path
    $files = @(Get-ChildItem -Path "$packageRoot\*" -Recurse -File | Sort-Object FullName |
        ForEach-Object { $_.FullName.Substring($packageRoot.Length + 1).Replace("\", "/") })
    $utf8NoBom = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllLines("$packageRoot\filelist.txt", $files, $utf8NoBom)

    $versionLine = (Select-String -Path ".\app\common\config.py" -Pattern '^VERSION\s*=').Line
    $version = ($versionLine -replace '^VERSION\s*=\s*"([^"]+)".*', '$1')
    $manifestFiles = @($files | ForEach-Object {
        $fullPath = Join-Path $packageRoot $_
        [ordered]@{
            path = $_
            size = (Get-Item $fullPath).Length
            sha256 = (Get-FileHash -Algorithm SHA256 -LiteralPath $fullPath).Hash.ToLowerInvariant()
        }
    })
    $manifest = [ordered]@{ version = $version; files = $manifestFiles } | ConvertTo-Json -Depth 4
    [System.IO.File]::WriteAllText("$packageRoot\update-manifest.json", $manifest, $utf8NoBom)

    if (-not $dbg) {
        if (-not (Test-Path $dest)) {
            New-Item -ItemType Directory -Path $dest -Force | Out-Null
        }
        $archive = Join-Path $dest "Seraphine.7z"
        if (Test-Path $archive) {
            Remove-Item -Force $archive
        }
        7z a $archive .\dist\Seraphine\* -r
        if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }
        Remove-Item -Recurse -Force .\dist
    }
}
finally {
    Pop-Location
}
