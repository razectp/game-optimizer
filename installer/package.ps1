# Package Game Optimizer for Windows distribution.
#
# Builds a release binary, copies it into dist\GameOptimizer\, and zips it.
# Then prints how to compile the admin-authorized, per-user Inno Setup installer (iscc).
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

$Version = ([regex]::Match((Get-Content -LiteralPath (Join-Path $Root "Cargo.toml") -Raw), '(?m)^version\s*=\s*"([^"]+)"').Groups[1].Value)
if (-not $Version) { throw "Could not read package version from Cargo.toml" }
$ExeName = "game_optimizer.exe"
$BuildDir = Join-Path $Root "target\package"
$ReleaseDir = Join-Path $BuildDir "release"
$CargoToml = Join-Path $Root "Cargo.toml"
if (-not (Test-Path -LiteralPath $CargoToml)) {
    throw "Cargo.toml not found at $CargoToml"
}

Write-Host "Building isolated package release ($Version)..."
$WintunSource = Join-Path $Root "wintun.dll"
if (-not (Test-Path -LiteralPath $WintunSource -PathType Leaf)) {
    throw "Required wintun.dll is missing at $WintunSource; refusing to create an incomplete package."
}
Push-Location $Root
try {
    & cargo build --locked --release --target-dir $BuildDir
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --locked --release --target-dir `"$BuildDir`" failed with exit code $LASTEXITCODE"
    }
} finally {
    Pop-Location
}

$ReleaseExe = Join-Path $ReleaseDir $ExeName
if (-not (Test-Path -LiteralPath $ReleaseExe)) {
    throw "Missing $ReleaseExe after cargo build --release"
}
$DistRoot = Join-Path $Root "dist"
$StageDir = Join-Path $DistRoot "GameOptimizer"
New-Item -ItemType Directory -Force -Path $StageDir | Out-Null

$Payload = [ordered]@{
    $ExeName = $ReleaseExe
    "wintun.dll" = $WintunSource
    "install.ps1" = Join-Path $ScriptDir "install.ps1"
    "uninstall.ps1" = Join-Path $ScriptDir "uninstall.ps1"
    "icon.ico" = Join-Path $Root "assets\icon.ico"
}
foreach ($file in $Payload.GetEnumerator()) {
    if (-not (Test-Path -LiteralPath $file.Value -PathType Leaf)) {
        throw "Required package input '$($file.Key)' is missing: $($file.Value)"
    }
    Copy-Item -LiteralPath $file.Value -Destination (Join-Path $StageDir $file.Key) -Force
}

$StagedWintun = Join-Path $StageDir "wintun.dll"
if ((Get-FileHash -LiteralPath $StagedWintun -Algorithm SHA256).Hash -ne (Get-FileHash -LiteralPath $WintunSource -Algorithm SHA256).Hash) {
    throw "Staged wintun.dll failed SHA-256 verification."
}

Write-Host "Staged portable folder:"
Write-Host "  $StageDir"

$ZipPath = Join-Path $DistRoot "GameOptimizer-$Version-windows.zip"
$zipCreated = $false
if (Test-Path -LiteralPath $ZipPath) { Remove-Item -LiteralPath $ZipPath -Force }
Add-Type -AssemblyName System.IO.Compression
Add-Type -AssemblyName System.IO.Compression.FileSystem
$archive = [System.IO.Compression.ZipFile]::Open($ZipPath, [System.IO.Compression.ZipArchiveMode]::Create)
try {
    foreach ($name in $Payload.Keys) {
        [System.IO.Compression.ZipFileExtensions]::CreateEntryFromFile(
            $archive, (Join-Path $StageDir $name), "GameOptimizer/$name",
            [System.IO.Compression.CompressionLevel]::Optimal) | Out-Null
    }
} finally {
    $archive.Dispose()
}
$zipCreated = $true
Write-Host "Zip criado (allowlist de payload): $ZipPath"

Write-Host ""
Write-Host "Next steps — compile the Inno Setup installer (Inno Setup 6.3+):"
Write-Host "  iscc `"$ScriptDir\GameOptimizer.iss`""
$pf86 = ${env:ProgramFiles(x86)}
if (-not $pf86) {
    $pf86 = "C:\Program Files (x86)"
}
$isccCandidates = foreach ($compilerRoot in @($pf86, $env:ProgramFiles, (Join-Path $env:LOCALAPPDATA "Programs"))) {
    foreach ($compilerVersion in @(6, 7)) {
        Join-Path $compilerRoot "Inno Setup $compilerVersion\ISCC.exe"
    }
}
$iscc = Get-Command ISCC.exe -ErrorAction SilentlyContinue
if ($iscc) { $isccGuess = $iscc.Source } else { $isccGuess = $isccCandidates | Where-Object { Test-Path -LiteralPath $_ } | Select-Object -First 1 }
Write-Host "  or:"
if ($isccGuess) { Write-Host "  & `"$isccGuess`" `"$ScriptDir\GameOptimizer.iss`"" } else { Write-Host "  ISCC.exe not found; install Inno Setup 6 to build the setup." }
Write-Host "Output (after iscc):"
Write-Host "  $(Join-Path $DistRoot "GameOptimizer-$Version-setup.exe")"
if ($zipCreated) {
    Write-Host "Portable zip:"
    Write-Host "  $ZipPath"
}
