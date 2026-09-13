# Install Game Optimizer on Windows
#
# Default: copies release build to LocalAppData, enables Start with Windows,
# optional Desktop shortcut, optional Start Menu entry.
#
# Usage (from repo root or game_optimizer):
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -NoAutostart
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -SkipBuild

[CmdletBinding()]
param(
    [switch]$SkipBuild,
    [switch]$NoAutostart,
    [switch]$NoDesktopShortcut,
    [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"
$Root = Split-Path -Parent $PSScriptRoot
if (-not (Test-Path (Join-Path $Root "Cargo.toml"))) {
    $Root = $PSScriptRoot
    if (-not (Test-Path (Join-Path $Root "Cargo.toml"))) {
        throw "Could not locate Cargo.toml near installer script."
    }
}

Set-Location $Root

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "GameOptimizer"
}

$ExeName = "game_optimizer.exe"
$ReleaseExe = Join-Path $Root "target\release\$ExeName"

if (-not $SkipBuild) {
    Write-Host "Building release..."
    cargo build --release
    if ($LASTEXITCODE -ne 0) {
        throw "cargo build --release failed"
    }
}

if (-not (Test-Path $ReleaseExe)) {
    throw "Missing $ReleaseExe — build the project first or omit -SkipBuild only after a successful build."
}

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
Copy-Item -Force $ReleaseExe (Join-Path $InstallDir $ExeName)

$ConfigSrc = Join-Path $Root "games.toml"
if (Test-Path $ConfigSrc) {
    Copy-Item -Force $ConfigSrc (Join-Path $InstallDir "games.toml")
}

$InstalledExe = Join-Path $InstallDir $ExeName
Write-Host "Installed to $InstalledExe"

# Start with Windows (default ON) via HKCU Run — matches auto-launch crate.
$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
if (-not $NoAutostart) {
    Set-ItemProperty -Path $RunKey -Name "GameOptimizer" -Value "`"$InstalledExe`" --gui"
    Write-Host "Auto-start enabled (Run key GameOptimizer)."
} else {
    Remove-ItemProperty -Path $RunKey -Name "GameOptimizer" -ErrorAction SilentlyContinue
    Write-Host "Auto-start skipped (-NoAutostart)."
}

# Start Menu shortcut
$StartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs"
New-Item -ItemType Directory -Force -Path $StartMenu | Out-Null
$Wsh = New-Object -ComObject WScript.Shell
$StartShortcut = $Wsh.CreateShortcut((Join-Path $StartMenu "Game Optimizer.lnk"))
$StartShortcut.TargetPath = $InstalledExe
$StartShortcut.Arguments = "--gui"
$StartShortcut.WorkingDirectory = $InstallDir
$StartShortcut.Description = "Game Optimizer"
$StartShortcut.Save()

if (-not $NoDesktopShortcut) {
    $Desktop = [Environment]::GetFolderPath("Desktop")
    $DeskShortcut = $Wsh.CreateShortcut((Join-Path $Desktop "Game Optimizer.lnk"))
    $DeskShortcut.TargetPath = $InstalledExe
    $DeskShortcut.Arguments = "--gui"
    $DeskShortcut.WorkingDirectory = $InstallDir
    $DeskShortcut.Description = "Game Optimizer"
    $DeskShortcut.Save()
    Write-Host "Desktop shortcut created."
}

Write-Host ""
Write-Host "Done. Launch with:"
Write-Host "  & `"$InstalledExe`""
Write-Host "Uninstall: .\installer\uninstall.ps1"
