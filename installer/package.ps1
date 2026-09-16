# Package Game Optimizer for Windows distribution.
#
# Builds a release binary, copies it into dist\GameOptimizer\, and zips it.
# Then prints how to compile the Inno Setup installer (iscc).
#
# Usage (from repo root or installer\):
#   powershell -ExecutionPolicy Bypass -File .\installer\package.ps1

[CmdletBinding()]
param()

$ErrorActionPreference = "Stop"

$ScriptDir = $PSScriptRoot
if (-not $ScriptDir -and $PSCommandPath) {
    $ScriptDir = Split-Path -Parent $PSCommandPath
}
if (-not $ScriptDir) {
    $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
}
$Root = Split-Path -Parent $ScriptDir
if (-not $Root) { $Root = $ScriptDir }

$Version = "0.5.1"
$ExeName = "game_optimizer.exe"
$CargoToml = Join-Path $Root "Cargo.toml"
if (-not (Test-Path -LiteralPath $CargoToml)) {
    throw "Cargo.toml not found at $CargoToml"
}

Write-Host "Building release ($Version)..."
Push-Location $Root
try {
    & cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$ReleaseExe = Join-Path $Root (Join-Path "target\release" $ExeName)
if (-not (Test-Path -LiteralPath $ReleaseExe)) {
    throw "Missing $ReleaseExe after cargo build --release"
}

$DistRoot = Join-Path $Root "dist"
$StageDir = Join-Path $DistRoot "GameOptimizer"
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

Copy-Item -LiteralPath $ReleaseExe -Destination (Join-Path $StageDir $ExeName) -Force

$GamesToml = Join-Path $Root "games.toml"
if (Test-Path -LiteralPath $GamesToml) {
    Copy-Item -LiteralPath $GamesToml -Destination (Join-Path $StageDir "games.toml") -Force
} else {
    Write-Host "Warning: games.toml not found at $GamesToml"
}

$UninstallSrc = Join-Path $ScriptDir "uninstall.ps1"
if (Test-Path -LiteralPath $UninstallSrc) {
    Copy-Item -LiteralPath $UninstallSrc -Destination (Join-Path $StageDir "uninstall.ps1") -Force
} else {
    throw "uninstall.ps1 not found at $UninstallSrc"
}

$InstallSrc = Join-Path $ScriptDir "install.ps1"
if (Test-Path -LiteralPath $InstallSrc) {
    Copy-Item -LiteralPath $InstallSrc -Destination (Join-Path $StageDir "install.ps1") -Force
}

$IconSrc = Join-Path $Root (Join-Path "assets" "icon.ico")
if (Test-Path -LiteralPath $IconSrc) {
    Copy-Item -LiteralPath $IconSrc -Destination (Join-Path $StageDir "icon.ico") -Force
}

Write-Host "Staged portable folder:"
Write-Host "  $StageDir"

$ZipPath = Join-Path $DistRoot "GameOptimizer-$Version-windows.zip"
$zipCreated = $false
if (Get-Command Compress-Archive -ErrorAction SilentlyContinue) {
    if (Test-Path -LiteralPath $ZipPath) {
        Remove-Item -LiteralPath $ZipPath -Force
    }
    Compress-Archive -Path $StageDir -DestinationPath $ZipPath -Force
    $zipCreated = $true
    Write-Host "Zip criado:"
    Write-Host "  $ZipPath"
} else {
    Write-Host "Compress-Archive não está disponível; zip omitido."
    Write-Host "Compress-Archive is not available; skipped zip creation."
}

Write-Host ""
Write-Host "Next steps — compile the Inno Setup installer (Inno Setup 6.3+):"
Write-Host "  iscc `"$ScriptDir\GameOptimizer.iss`""
$pf86 = ${env:ProgramFiles(x86)}
if (-not $pf86) {
    $pf86 = "C:\Program Files (x86)"
}
$isccGuess = Join-Path $pf86 "Inno Setup 6\ISCC.exe"
Write-Host "  or:"
Write-Host "  & `"$isccGuess`" `"$ScriptDir\GameOptimizer.iss`""
Write-Host "Output (after iscc):"
Write-Host "  $(Join-Path $DistRoot "GameOptimizer-$Version-setup.exe")"
if ($zipCreated) {
    Write-Host "Portable zip:"
    Write-Host "  $ZipPath"
}
