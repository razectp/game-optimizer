# Uninstall Game Optimizer
#
# Removes the LocalAppData install, HKCU Run key, Start Menu / Desktop
# shortcuts, and Add/Remove Programs registration.
#
# If this script lives inside the install directory, it copies itself to TEMP
# and re-runs from there so the install folder can be deleted.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File .\uninstall.ps1
#   powershell -ExecutionPolicy Bypass -File .\installer\uninstall.ps1

[CmdletBinding()]
param(
    [string]$InstallDir = "",
    [switch]$FromTemp
)

$ErrorActionPreference = "Continue"

$scriptPath = $PSCommandPath
if (-not $scriptPath -and $MyInvocation.MyCommand.Path) {
    $scriptPath = $MyInvocation.MyCommand.Path
}
$scriptDir = $PSScriptRoot
if (-not $scriptDir -and $scriptPath) {
    $scriptDir = Split-Path -Parent $scriptPath
}
if (-not $scriptDir) {
    $scriptDir = (Get-Location).Path
}

function Test-IsUnderDirectory {
    param([string]$Child, [string]$Parent)
    if (-not $Child -or -not $Parent) { return $false }
    try {
        $c = [IO.Path]::GetFullPath($Child).TrimEnd('\')
        $p = [IO.Path]::GetFullPath($Parent).TrimEnd('\')
    } catch {
        return $false
    }
    if ($c -eq $p) { return $true }
    $prefix = $p + [IO.Path]::DirectorySeparatorChar
    return $c.StartsWith($prefix, [StringComparison]::OrdinalIgnoreCase)
}

function Stop-GameOptimizerProcess {
    Get-Process -Name "game_optimizer" -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

function Remove-ItemIfExists {
    param([string]$Path, [switch]$Recurse)
    if (-not $Path) { return }
    if (Test-Path -LiteralPath $Path) {
        if ($Recurse) {
            Remove-Item -LiteralPath $Path -Recurse -Force -ErrorAction SilentlyContinue
        } else {
            Remove-Item -LiteralPath $Path -Force -ErrorAction SilentlyContinue
        }
    }
}

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "GameOptimizer"
}
$InstallDir = $InstallDir.TrimEnd('\')

if (-not $FromTemp) {
    if ($scriptDir -and (Test-IsUnderDirectory -Child $scriptDir -Parent $InstallDir)) {
        Write-Host "Copiando o desinstalador para uma pasta temporária para poder apagar a instalação..."
        $tempScript = Join-Path $env:TEMP ("GameOptimizer-uninstall-" + [guid]::NewGuid().ToString("N") + ".ps1")
        Copy-Item -LiteralPath $scriptPath -Destination $tempScript -Force
        Set-Location $env:TEMP
        $psExe = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
        $argList = @(
            "-NoProfile",
            "-ExecutionPolicy", "Bypass",
            "-File", $tempScript,
            "-InstallDir", $InstallDir,
            "-FromTemp"
        )
        $proc = Start-Process -FilePath $psExe -ArgumentList $argList -Wait -PassThru
        if ($proc.ExitCode -ne 0) {
            Write-Host "O desinstalador temporário terminou com código $($proc.ExitCode)."
        }
        exit $proc.ExitCode
    }
}

Write-Host "Removendo Game Optimizer..."

Set-Location $env:TEMP
Stop-GameOptimizerProcess

$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
foreach ($name in @("GameOptimizer", "Game Optimizer", "game_optimizer", "game_optimizer.exe", "GameOptimizer.exe")) {
    Remove-ItemProperty -Path $RunKey -Name $name -ErrorAction SilentlyContinue
}
$LmRun = "HKLM:\Software\Microsoft\Windows\CurrentVersion\Run"
if (Test-Path -LiteralPath $LmRun) {
    foreach ($name in @("GameOptimizer", "Game Optimizer", "game_optimizer", "game_optimizer.exe", "GameOptimizer.exe")) {
        Remove-ItemProperty -Path $LmRun -Name $name -ErrorAction SilentlyContinue
    }
}
$StartupDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Startup"
foreach ($lnk in @("Game Optimizer.lnk", "GameOptimizer.lnk", "game_optimizer.lnk")) {
    Remove-ItemIfExists -Path (Join-Path $StartupDir $lnk)
}
Write-Host "Chave de início automático removida (se existia)."

$ArpKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\GameOptimizer"
Remove-Item -LiteralPath $ArpKey -Recurse -Force -ErrorAction SilentlyContinue
Write-Host "Registro de Aplicativos e recursos removido (se existia)."

$StartMenuFolder = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Game Optimizer"
$StartMenuLegacy = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Game Optimizer.lnk"
$Desktop = Join-Path ([Environment]::GetFolderPath("Desktop")) "Game Optimizer.lnk"
Remove-ItemIfExists -Path $StartMenuFolder -Recurse
Remove-ItemIfExists -Path $StartMenuLegacy
Remove-ItemIfExists -Path $Desktop
Write-Host "Atalhos removidos (se existiam)."

if (Test-Path -LiteralPath $InstallDir) {
    Remove-ItemIfExists -Path $InstallDir -Recurse
    if (Test-Path -LiteralPath $InstallDir) {
        Write-Host "Não foi possível apagar completamente: $InstallDir"
        Write-Host "Feche o Game Optimizer e apague a pasta manualmente se ainda existir."
        Write-Host "Could not fully delete the install folder. Close the app and remove it manually if needed."
    } else {
        Write-Host "Pasta de instalação removida: $InstallDir"
    }
} else {
    Write-Host "Pasta de instalação não encontrada (ok): $InstallDir"
}

if ($FromTemp) {
    $self = $PSCommandPath
    if (-not $self) { $self = $scriptPath }
    if ($self -and (Test-Path -LiteralPath $self)) {
        $cmd = Join-Path $env:SystemRoot "System32\cmd.exe"
        Start-Process -FilePath $cmd -ArgumentList "/c ping 127.0.0.1 -n 2 > nul & del /f /q `"$self`"" -WindowStyle Hidden
    }
}

Write-Host ""
Write-Host "Game Optimizer foi desinstalado."
Write-Host "Obrigado por usar o aplicativo."
