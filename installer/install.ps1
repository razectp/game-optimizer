# Install Game Optimizer on Windows (end-user, no Rust required).
#
# Default: copy a prebuilt game_optimizer.exe into %LOCALAPPDATA%\GameOptimizer,
# create Start Menu / Desktop shortcuts, enable Start with Windows, and register
# Add/Remove Programs. Does NOT run cargo unless -Build is passed.
#
# Usage:
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -NoAutostart
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -NoDesktopShortcut
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -Launch
#   powershell -ExecutionPolicy Bypass -File .\installer\install.ps1 -Build
#
# -SkipBuild is accepted for compatibility with older docs and is the default.

[CmdletBinding()]
param(
    [switch]$Build,
    [switch]$SkipBuild,
    [switch]$NoAutostart,
    [switch]$NoDesktopShortcut,
    [switch]$Launch,
    [string]$InstallDir = ""
)

$ErrorActionPreference = "Stop"

# -SkipBuild is the default behavior (kept so old command lines still parse).
$null = $SkipBuild

$ScriptDir = $PSScriptRoot
if (-not $ScriptDir -and $PSCommandPath) {
    $ScriptDir = Split-Path -Parent $PSCommandPath
}
if (-not $ScriptDir) {
    $ScriptDir = Split-Path -Parent $MyInvocation.MyCommand.Path
}

function Stop-GameOptimizerProcess {
    Get-Process -Name "game_optimizer" -ErrorAction SilentlyContinue |
        Stop-Process -Force -ErrorAction SilentlyContinue
    Start-Sleep -Milliseconds 400
}

function Find-PrebuiltExe {
    param(
        [string]$RootDir,
        [string]$ScriptDir,
        [string]$ExeName
    )
    $candidates = @(
        (Join-Path $RootDir (Join-Path "target\release" $ExeName)),
        (Join-Path $RootDir (Join-Path "dist" $ExeName)),
        (Join-Path $RootDir (Join-Path "dist\GameOptimizer" $ExeName)),
        (Join-Path $ScriptDir $ExeName)
    )
    foreach ($path in $candidates) {
        if ($path -and (Test-Path -LiteralPath $path)) {
            return (Resolve-Path -LiteralPath $path).Path
        }
    }
    return $null
}

function Find-GamesToml {
    param(
        [string]$RootDir,
        [string]$ScriptDir,
        [string]$ExePath
    )
    $candidates = @()
    if ($ExePath) {
        $candidates += (Join-Path (Split-Path -Parent $ExePath) "games.toml")
    }
    $candidates += (Join-Path $RootDir "games.toml")
    $candidates += (Join-Path $ScriptDir "games.toml")
    foreach ($path in $candidates) {
        if ($path -and (Test-Path -LiteralPath $path)) {
            return (Resolve-Path -LiteralPath $path).Path
        }
    }
    return $null
}

function Set-RegString {
    param([string]$Path, [string]$Name, [string]$Value)
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType String -Force | Out-Null
}

function Set-RegDword {
    param([string]$Path, [string]$Name, [int]$Value)
    New-ItemProperty -Path $Path -Name $Name -Value $Value -PropertyType DWord -Force | Out-Null
}

function New-AppShortcut {
    param(
        [object]$Wsh,
        [string]$LinkPath,
        [string]$TargetPath,
        [string]$WorkDir,
        [string]$Description
    )
    $parent = Split-Path -Parent $LinkPath
    if ($parent) {
        New-Item -ItemType Directory -Force -Path $parent | Out-Null
    }
    $sc = $Wsh.CreateShortcut($LinkPath)
    $sc.TargetPath = $TargetPath
    $sc.Arguments = "--gui"
    $sc.WorkingDirectory = $WorkDir
    $sc.Description = $Description
    $sc.IconLocation = "$TargetPath,0"
    $sc.Save()
}

$Root = Split-Path -Parent $ScriptDir
if (-not $Root) { $Root = $ScriptDir }

if (-not $InstallDir) {
    $InstallDir = Join-Path $env:LOCALAPPDATA "GameOptimizer"
}

$ExeName = "game_optimizer.exe"
$Version = "0.5.0"
$Publisher = "Game Optimizer"

if ($Build) {
    $cargoToml = Join-Path $Root "Cargo.toml"
    if (-not (Test-Path -LiteralPath $cargoToml)) {
        throw @"
Cargo.toml não encontrado em $Root.
-Build exige o código-fonte e o Rust (cargo).

Cargo.toml was not found under $Root.
-Build requires the source tree and the Rust toolchain.
"@
    }
    Write-Host "Compilando release (cargo build --release)..."
    Push-Location $Root
    try {
        & cargo build --release
        if ($LASTEXITCODE -ne 0) {
            throw "cargo build --release failed with exit code $LASTEXITCODE"
        }
    } finally {
        Pop-Location
    }
}

$SourceExe = Find-PrebuiltExe -RootDir $Root -ScriptDir $ScriptDir -ExeName $ExeName
if (-not $SourceExe) {
    throw @"
Não foi encontrado game_optimizer.exe.
Use o pacote de release (zip ou instalador) que já inclui o executável,
ou execute com -Build se tiver o Rust instalado.

game_optimizer.exe was not found.
Looked in:
  $Root\target\release\$ExeName
  $Root\dist\$ExeName
  $Root\dist\GameOptimizer\$ExeName
  $ScriptDir\$ExeName
Use a packaged release (zip or setup) that includes the executable,
or run with -Build if you have the Rust toolchain.
"@
}

$UninstallSrc = Join-Path $ScriptDir "uninstall.ps1"
if (-not (Test-Path -LiteralPath $UninstallSrc)) {
    throw "uninstall.ps1 not found next to install.ps1: $UninstallSrc"
}

Write-Host "Instalando Game Optimizer $Version..."
Write-Host "  Origem: $SourceExe"
Write-Host "  Destino: $InstallDir"

Stop-GameOptimizerProcess

New-Item -ItemType Directory -Force -Path $InstallDir | Out-Null
$InstalledExe = Join-Path $InstallDir $ExeName
Copy-Item -LiteralPath $SourceExe -Destination $InstalledExe -Force

$ConfigSrc = Find-GamesToml -RootDir $Root -ScriptDir $ScriptDir -ExePath $SourceExe
$ConfigDest = Join-Path $InstallDir "games.toml"
if ($ConfigSrc) {
    if (Test-Path -LiteralPath $ConfigDest) {
        Write-Host "Mantendo games.toml existente do usuário (não substituído)."
    } else {
        Copy-Item -LiteralPath $ConfigSrc -Destination $ConfigDest
        Write-Host "Copiado games.toml."
    }
}

Copy-Item -LiteralPath $UninstallSrc -Destination (Join-Path $InstallDir "uninstall.ps1") -Force

# Remember that install already chose autostart on/off so the GUI does not
# re-enable it on the next launch.
Set-Content -LiteralPath (Join-Path $InstallDir ".autostart-initialized") -Value "1" -Encoding ASCII

$RunKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Run"
$autostartOn = -not $NoAutostart
if ($autostartOn) {
    Set-ItemProperty -Path $RunKey -Name "GameOptimizer" -Value "`"$InstalledExe`" --gui"
    Write-Host "Início automático ativado (HKCU Run)."
} else {
    Remove-ItemProperty -Path $RunKey -Name "GameOptimizer" -ErrorAction SilentlyContinue
    Write-Host "Início automático ignorado (-NoAutostart)."
}

$Wsh = New-Object -ComObject WScript.Shell
$StartMenuDir = Join-Path $env:APPDATA "Microsoft\Windows\Start Menu\Programs\Game Optimizer"
$StartLink = Join-Path $StartMenuDir "Game Optimizer.lnk"
New-AppShortcut -Wsh $Wsh -LinkPath $StartLink -TargetPath $InstalledExe -WorkDir $InstallDir -Description "Game Optimizer"
Write-Host "Atalho do Menu Iniciar criado."

$desktopCreated = $false
if (-not $NoDesktopShortcut) {
    $Desktop = [Environment]::GetFolderPath("Desktop")
    $DeskLink = Join-Path $Desktop "Game Optimizer.lnk"
    New-AppShortcut -Wsh $Wsh -LinkPath $DeskLink -TargetPath $InstalledExe -WorkDir $InstallDir -Description "Game Optimizer"
    $desktopCreated = $true
    Write-Host "Atalho da Área de Trabalho criado."
}

$ArpKey = "HKCU:\Software\Microsoft\Windows\CurrentVersion\Uninstall\GameOptimizer"
New-Item -Path $ArpKey -Force | Out-Null

$psExe = Join-Path $env:SystemRoot "System32\WindowsPowerShell\v1.0\powershell.exe"
$uninstallPs1 = Join-Path $InstallDir "uninstall.ps1"
$uninstallString = "`"$psExe`" -NoProfile -ExecutionPolicy Bypass -File `"$uninstallPs1`""

Set-RegString -Path $ArpKey -Name "DisplayName" -Value "Game Optimizer"
Set-RegString -Path $ArpKey -Name "DisplayVersion" -Value $Version
Set-RegString -Path $ArpKey -Name "Publisher" -Value $Publisher
Set-RegString -Path $ArpKey -Name "InstallLocation" -Value $InstallDir
Set-RegString -Path $ArpKey -Name "DisplayIcon" -Value $InstalledExe
Set-RegString -Path $ArpKey -Name "UninstallString" -Value $uninstallString
Set-RegString -Path $ArpKey -Name "QuietUninstallString" -Value $uninstallString
Set-RegString -Path $ArpKey -Name "InstallDate" -Value ((Get-Date).ToString("yyyyMMdd"))
Set-RegDword -Path $ArpKey -Name "NoModify" -Value 1
Set-RegDword -Path $ArpKey -Name "NoRepair" -Value 1
Set-RegDword -Path $ArpKey -Name "EstimatedSize" -Value 0

$sizeBytes = 0
Get-ChildItem -LiteralPath $InstallDir -Recurse -File -ErrorAction SilentlyContinue | ForEach-Object {
    $sizeBytes += $_.Length
}
if ($sizeBytes -gt 0) {
    $sizeKb = [int][Math]::Ceiling($sizeBytes / 1KB)
    Set-RegDword -Path $ArpKey -Name "EstimatedSize" -Value $sizeKb
}

if ($Launch) {
    Start-Process -FilePath $InstalledExe -ArgumentList "--gui" -WorkingDirectory $InstallDir
    Write-Host "Aplicativo iniciado."
}

Write-Host ""
Write-Host "Pronto. Game Optimizer $Version instalado em:"
Write-Host "  $InstallDir"
if ($desktopCreated) {
    Write-Host "Atalhos: Menu Iniciar e Área de Trabalho."
} else {
    Write-Host "Atalho: Menu Iniciar (Área de Trabalho omitida)."
}
if ($autostartOn) {
    Write-Host "Inicia com o Windows (pode desativar em Aplicativos e recursos / -NoAutostart)."
} else {
    Write-Host "Não foi adicionado ao início do Windows."
}
Write-Host "Para desinstalar, use Aplicativos e recursos ou:"
Write-Host "  powershell -ExecutionPolicy Bypass -File `"$uninstallPs1`""
