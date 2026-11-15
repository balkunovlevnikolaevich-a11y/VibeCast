param(
    [Parameter(Mandatory=$true)][string]$CurrentVersion,
    [ValidatePattern('^VibeCast(-Free|-Pro)?-Setup\.exe$')][string]$AssetName='VibeCast-Setup.exe',
    [switch]$CheckOnly
)
$ErrorActionPreference='Stop'
$repository='balkunovlevnikolaevich-a11y/VibeCast'
$dataRoot=Join-Path $env:LOCALAPPDATA 'VibeCast'
$updateRoot=Join-Path $dataRoot 'updates'
New-Item -ItemType Directory -Path $updateRoot -Force | Out-Null
$lockPath=Join-Path $updateRoot 'update.lock'
$lock=$null
try {
    try { $lock=[IO.File]::Open($lockPath,[IO.FileMode]::OpenOrCreate,[IO.FileAccess]::ReadWrite,[IO.FileShare]::None) }
    catch { exit 0 }
    [Net.ServicePointManager]::SecurityProtocol=[Net.SecurityProtocolType]::Tls12
    $headers=@{ 'User-Agent'='VibeCast-Updater'; 'Accept'='application/vnd.github+json' }
    $release=Invoke-RestMethod -Uri "https://api.github.com/repos/$repository/releases/latest" -Headers $headers -TimeoutSec 20
    if ($release.draft -or $release.prerelease) { exit 0 }
    $tag=[string]$release.tag_name
    if ($tag -notmatch '^v?(\d+\.\d+\.\d+)$') { throw 'Unsupported release version' }
    $latest=[version]$Matches[1]
    $current=[version]($CurrentVersion -replace '^v','')
    if ($latest -le $current) { exit 0 }
    $assets=@($release.assets | Where-Object { $_.name -ceq $AssetName -and $_.state -eq 'uploaded' })
    if ($assets.Count -ne 1) { throw 'Release has no unique installer for this edition' }
    $asset=$assets[0]
    if ($asset.digest -notmatch '^sha256:([a-fA-F0-9]{64})$') { throw 'Release installer has no SHA-256 digest' }
    $expectedHash=$Matches[1]
    $expectedUrl="https://github.com/$repository/releases/download/$tag/$AssetName"
    if ([string]$asset.browser_download_url -cne $expectedUrl) { throw 'Installer URL does not belong to this release' }
    if ([long]$asset.size -le 0 -or [long]$asset.size -gt 100MB) { throw 'Unexpected installer size' }
    if ($CheckOnly) { Write-Output ("Available: {0}; asset: {1}" -f $latest,$AssetName); exit 0 }
    $destination=Join-Path $updateRoot ($latest.ToString()+'-'+$AssetName)
    $partial=$destination+'.partial'
    Invoke-WebRequest -Uri $expectedUrl -Headers $headers -OutFile $partial -UseBasicParsing -TimeoutSec 120
    if ((Get-Item -LiteralPath $partial).Length -ne [long]$asset.size) { throw 'Installer size mismatch' }
    if ((Get-FileHash -LiteralPath $partial -Algorithm SHA256).Hash -ine $expectedHash) { throw 'Installer SHA-256 mismatch' }
    Move-Item -LiteralPath $partial -Destination $destination -Force
    $installer=Start-Process -FilePath $destination -ArgumentList '--silent' -WindowStyle Hidden -PassThru
    $installer.WaitForExit()
    if ($installer.ExitCode -ne 0) { throw 'Installer did not complete successfully' }
    Write-Output ("Updated to {0}" -f $latest)
} catch {
    # Keep a failed update from interrupting the running weather overlay.
    Add-Content -LiteralPath (Join-Path $updateRoot 'update.log') -Value ((Get-Date -Format o)+' '+$_.Exception.Message)
    exit 1
} finally {
    if ($lock) { $lock.Dispose() }
}
