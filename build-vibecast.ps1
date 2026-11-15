param([string]$OutputDirectory = (Join-Path $PSScriptRoot 'release'))
$ErrorActionPreference = 'Stop'
$projectRoot = $PSScriptRoot
$destination = [IO.Path]::GetFullPath($OutputDirectory)
$cargoTool = (Get-Command cargo -ErrorAction SilentlyContinue).Source
if (!$cargoTool) { $cargoTool = Join-Path $env:USERPROFILE '.cargo\bin\cargo.exe' }
if (!(Test-Path -LiteralPath $cargoTool)) { throw 'Install Rust with the MSVC toolchain first.' }
New-Item -ItemType Directory -Path $destination -Force | Out-Null
Push-Location (Join-Path $projectRoot 'windows')
try {
    & $cargoTool build --release --bin Snowhush
    if ($LASTEXITCODE -ne 0) { throw 'Application build failed' }
    $env:SNOWHUSH_APP = (Resolve-Path 'target\release\Snowhush.exe').Path
    & $cargoTool build --release --features setup --bin Snowhush-Setup
    if ($LASTEXITCODE -ne 0) { throw 'Installer build failed' }
    Copy-Item -LiteralPath $env:SNOWHUSH_APP -Destination (Join-Path $destination 'VibeCast.exe') -Force
    Copy-Item -LiteralPath 'target\release\Snowhush-Setup.exe' -Destination (Join-Path $destination 'VibeCast-Setup.exe') -Force
    Get-FileHash -Algorithm SHA256 -LiteralPath (Join-Path $destination 'VibeCast.exe'),(Join-Path $destination 'VibeCast-Setup.exe') |
        ForEach-Object { '{0}  {1}' -f $_.Hash,[IO.Path]::GetFileName($_.Path) } |
        Set-Content -LiteralPath (Join-Path $destination 'SHA256SUMS.txt') -Encoding ascii
} finally { Pop-Location }
