; Game Optimizer — per-user Inno Setup installer (no admin required).
; Compile: iscc installer\GameOptimizer.iss
; Requires a release binary at ..\target\release\game_optimizer.exe

#define AppName "Game Optimizer"
#define AppVersion "0.4.0"
#define AppPublisher "Game Optimizer"
#define AppExeName "game_optimizer.exe"
#define AppMutexName "GameOptimizer"

[Setup]
AppId={{E8C3A91F-6B2D-4F70-9A15-3C7E8D4B2A10}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher={#AppPublisher}
AppCopyright={#AppPublisher}
VersionInfoVersion={#AppVersion}
VersionInfoCompany={#AppPublisher}
VersionInfoProductName={#AppName}
DefaultDirName={localappdata}\GameOptimizer
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=GameOptimizer-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
SetupIconFile=..\assets\icon.ico
CloseApplications=yes
CloseApplicationsFilter=game_optimizer.exe
RestartApplications=no
UsedUserAreasWarning=no
LanguageDetectionMethod=uilanguage
ShowLanguageDialog=auto
AppMutex={#AppMutexName}Setup

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[CustomMessages]
brazilianportuguese.StartWithWindows=Iniciar com o Windows (recomendado)
english.StartWithWindows=Start with Windows (recommended)
brazilianportuguese.StartupGroup=Inicialização:
english.StartupGroup=Startup:

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: checkedonce
Name: "autostart"; Description: "{cm:StartWithWindows}"; GroupDescription: "{cm:StartupGroup}"; Flags: checkedonce

[Files]
Source: "..\target\release\game_optimizer.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\games.toml"; DestDir: "{app}"; Flags: ignoreversion onlyifdoesntexist
Source: "uninstall.ps1"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\assets\icon.ico"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[UninstallDelete]
Type: files; Name: "{app}\.autostart-initialized"

[Icons]
Name: "{group}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--gui"; WorkingDir: "{app}"; Comment: "{#AppName}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExeName}"; Parameters: "--gui"; WorkingDir: "{app}"; Comment: "{#AppName}"; Tasks: desktopicon

[Registry]
; Start with Windows (HKCU Run). uninsdeletevalue removes it on uninstall.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "GameOptimizer"; ValueData: """{app}\{#AppExeName}"" --gui"; Flags: uninsdeletevalue; Tasks: autostart
; If the user unchecks autostart on upgrade, drop a leftover Run value.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "GameOptimizer"; Flags: deletevalue; Tasks: not autostart

[Run]
Filename: "{app}\{#AppExeName}"; Parameters: "--gui"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[Code]
procedure CloseAppIfRunning;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM game_optimizer.exe /F /T',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function InitializeSetup(): Boolean;
begin
  CloseAppIfRunning;
  Result := True;
end;

function InitializeUninstall(): Boolean;
begin
  CloseAppIfRunning;
  Result := True;
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
    SaveStringToFile(ExpandConstant('{app}\.autostart-initialized'), '1', False);
end;
