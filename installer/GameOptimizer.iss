; Game Optimizer — per-user Inno Setup wizard (no admin required).
; Compile: iscc installer\GameOptimizer.iss
; Requires a release binary at ..\target\release\game_optimizer.exe

#define AppName "Game Optimizer"
#define AppVersion "0.5.1"
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
DisableWelcomePage=no
DisableDirPage=no
DisableProgramGroupPage=yes
DisableReadyPage=no
DisableFinishedPage=no
AlwaysShowDirOnReadyPage=yes
ShowTasksTreeLines=yes
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=GameOptimizer-{#AppVersion}-setup
Compression=lzma2
SolidCompression=yes
MinVersion=10.0
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
WizardSizePercent=120
WizardResizable=yes
WizardImageFile=wizard-sidebar.bmp
WizardSmallImageFile=wizard-small.bmp
UninstallDisplayIcon={app}\{#AppExeName}
UninstallDisplayName={#AppName}
SetupIconFile=..\assets\icon.ico
CloseApplications=yes
CloseApplicationsFilter=game_optimizer.exe
RestartApplications=no
UsedUserAreasWarning=no
LanguageDetectionMethod=uilanguage
ShowLanguageDialog=yes
SetupLogging=yes
AppMutex={#AppMutexName}Setup

[Languages]
Name: "brazilianportuguese"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"; InfoBeforeFile: "welcome-pt.txt"; LicenseFile: "license-pt.txt"
Name: "english"; MessagesFile: "compiler:Default.isl"; InfoBeforeFile: "welcome-en.txt"; LicenseFile: "license-en.txt"

[Messages]
brazilianportuguese.BeveledLabel=Game Optimizer
english.BeveledLabel=Game Optimizer
brazilianportuguese.WelcomeLabel1=Bem-vindo ao assistente de instalação do [name]
brazilianportuguese.WelcomeLabel2=Este assistente vai instalar o [name/ver] nesta conta do Windows. Não precisa de administrador.%n%nO Game Optimizer deixa seus jogos mais fluidos e nunca fecha nenhum jogo.%n%nClique em Avançar para continuar.
english.WelcomeLabel1=Welcome to the [name] Setup Wizard
english.WelcomeLabel2=This will install [name/ver] for this Windows account. Administrator rights are not required.%n%nGame Optimizer keeps your games smoother and never closes them.%n%nClick Next to continue.
brazilianportuguese.FinishedHeadingLabel=Concluiu a instalação do [name]
brazilianportuguese.FinishedLabel=O [name] está instalado. Você pode abrir o programa agora ou depois pelo Menu Iniciar.
english.FinishedHeadingLabel=Completing the [name] Setup Wizard
english.FinishedLabel=[name] is installed. You can launch it now or later from the Start Menu.

[CustomMessages]
brazilianportuguese.StartWithWindows=Iniciar com o Windows (recomendado)
english.StartWithWindows=Start with Windows (recommended)
brazilianportuguese.StartupGroup=Inicialização:
english.StartupGroup=Startup:
brazilianportuguese.FeaturesTitle=O que será instalado
english.FeaturesTitle=What will be installed
brazilianportuguese.FeaturesSub=Game Optimizer na sua conta do Windows
english.FeaturesSub=Game Optimizer for this Windows account
brazilianportuguese.FeaturesCaption=Resumo do que o assistente copia e configura:
english.FeaturesCaption=Summary of what Setup copies and configures:
brazilianportuguese.FeaturesBody=Pasta do programa (sua conta, sem admin)%nAtalho no Menu Iniciar%nAtalho na Área de Trabalho (opcional)%nInício com o Windows (opcional, recomendado)%n%nO otimizador detecta jogos abertos, ajusta prioridade e memória, e pode ir para a bandeja. Nenhum jogo é fechado.%n%nA limpeza da pasta Temp, se você usar depois no app, só atinge arquivos temporários do seu usuário.
english.FeaturesBody=Program folder (this account, no admin)%nStart Menu shortcut%nDesktop shortcut (optional)%nStart with Windows (optional, recommended)%n%nThe optimizer detects running games, adjusts priority and memory, and can sit in the tray. Games are never closed.%n%nTemp cleanup, if you use it later in the app, only touches this user's temporary files.

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
Type: files; Name: "{app}\.setup-wizard-complete"

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
var
  FeaturesPage: TOutputMsgMemoWizardPage;

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

procedure InitializeWizard;
begin
  FeaturesPage := CreateOutputMsgMemoPage(
    wpWelcome,
    CustomMessage('FeaturesTitle'),
    CustomMessage('FeaturesSub'),
    CustomMessage('FeaturesCaption'),
    CustomMessage('FeaturesBody'));
end;

procedure RemoveDuplicateAutostart;
begin
  { Old aliases + HKLM copies would start a second instance on login. }
  RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Game Optimizer');
  RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'game_optimizer');
  RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'game_optimizer.exe');
  RegDeleteValue(HKCU, 'Software\Microsoft\Windows\CurrentVersion\Run', 'GameOptimizer.exe');
  RegDeleteValue(HKLM, 'Software\Microsoft\Windows\CurrentVersion\Run', 'GameOptimizer');
  RegDeleteValue(HKLM, 'Software\Microsoft\Windows\CurrentVersion\Run', 'Game Optimizer');
  DeleteFile(ExpandConstant('{userstartup}\Game Optimizer.lnk'));
  DeleteFile(ExpandConstant('{userstartup}\GameOptimizer.lnk'));
  DeleteFile(ExpandConstant('{userstartup}\game_optimizer.lnk'));
end;

procedure CurStepChanged(CurStep: TSetupStep);
begin
  if CurStep = ssPostInstall then
  begin
    RemoveDuplicateAutostart;
    SaveStringToFile(ExpandConstant('{app}\.autostart-initialized'), '1', False);
    { Inno already ran the setup wizard; skip the in-app first-run wizard. }
    SaveStringToFile(ExpandConstant('{app}\.setup-wizard-complete'), '1', False);
  end;
end;
