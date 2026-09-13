# Uninstall Game Optimizer
#
# Removes LocalAppData install, Run key, and shortcuts created by install.ps1.

[CmdletBinding()]
param(
    [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "GameOptimizer"
}

$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
Remove-ItemProperty -Path $RunKey -Name "GameOptimizer" -ErrorAction SilentlyContinue

$StartMenu = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Game Optimizer.lnk"
$Desktop = Join-Path ([Environment]::GetFolderPath("Desktop")) "Game Optimizer.lnk"
Remove-Item -Force $StartMenu -ErrorAction SilentlyContinue
Remove-Item -Force $Desktop -ErrorAction SilentlyContinue

if (Test-Path $InstallDir) {
    Remove-Item -Recurse -Force $InstallDir
    Write-Host "Removed $InstallDir"
} else {
    Write-Host "Install directory not found: $InstallDir"
}

Write-Host "Auto-start Run key and shortcuts cleared."
