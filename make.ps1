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

if ($dbg -and (Test-Path .\dist)) {
    rm -r -Force .\dist
}

# Garante que o PyInstaller resolva as DLLs (libssl/libcrypto) do ambiente
# Python ATIVO, e nao de outro ambiente conda que apareca antes no PATH -- By Claude
# (isso causava "RuntimeError: SSL is not supported" no executavel empacotado)
$pythonExe = (Get-Command python).Source
$envRoot = Split-Path $pythonExe -Parent
$env:PATH = "$envRoot;$envRoot\Library\bin;$envRoot\Scripts;$env:PATH"

cargo build --release --manifest-path .\rust\Cargo.toml -p seraphine-sidecar --bins
if ($LASTEXITCODE -ne 0) { exit $LASTEXITCODE }

# Usa o layout padrao do PyInstaller 6 (tudo dentro de `_internal`, deixando
# so o Seraphine.exe visivel na raiz da pasta) -- By Claude
pyinstaller -w -i .\app\resource\images\logo.ico main.py
rm -r -fo .\build
rm -r -fo .\main.spec
rni -path .\dist\main -newName Seraphine
rni -path .\dist\Seraphine\main.exe -newName Seraphine.exe

# app/resource precisa ficar dentro de `_internal`: e' para la' que o
# os.chdir(dirname(__file__)) do main.py aponta dentro do executavel
# empacotado, entao e' onde os caminhos relativos "app/resource/..." resolvem -- By Claude
cpi .\app -destination .\dist\Seraphine\_internal -recurse
cpi .\rust\target\release\seraphine-sidecar.exe -destination .\dist\Seraphine\_internal
cpi .\rust\target\release\seraphine-updater.exe -destination .\dist\Seraphine\_internal
rm -r .\dist\Seraphine\_internal\app\common
rm -r .\dist\Seraphine\_internal\app\components
rm -r .\dist\Seraphine\_internal\app\lol
rm -Path .\dist\Seraphine\_internal\app\resource\game* -r
rm -r .\dist\Seraphine\_internal\app\resource\i18n\Seraphine.zh_CN.ts
rm -r .\dist\Seraphine\_internal\app\resource\bin\fix_lcu_window.c
rm -r .\dist\Seraphine\_internal\app\resource\bin\readme.md
rm -r .\dist\Seraphine\_internal\app\view

$packageRoot = (Resolve-Path ".\dist\Seraphine").Path
$files = Get-ChildItem -Path "$packageRoot\*" -Recurse -File |
    ForEach-Object { $_.FullName.Substring($packageRoot.Length + 1).Replace("\", "/") }
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

if (! $dbg) {
    7z a $dest\Seraphine.7z .\dist\Seraphine\* -r
    rm -r .\dist
}
