; Game Optimizer — per-user Inno Setup wizard (no admin required).
; Compile: iscc installer\GameOptimizer.iss
; Requires a release binary at ..\target\release\game_optimizer.exe

#define AppName "Game Optimizer"
#define AppVersion "0.5.2"
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
DisableDirPage=auto
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
brazilianportuguese.SetupAppRunningError=O Game Optimizer está aberto. Feche-o (também o ícone na bandeja) e clique em OK para continuar, ou em Cancelar para sair.
english.SetupAppRunningError=Game Optimizer is currently running. Close it (including the tray icon), then click OK to continue, or Cancel to exit.

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
brazilianportuguese.UpgradeWelcomeTitle=Bem-vindo ao assistente de atualização do Game Optimizer
english.UpgradeWelcomeTitle=Welcome to the Game Optimizer update wizard
brazilianportuguese.UpgradeWelcomeBody=Este assistente vai atualizar o Game Optimizer %1 para %2 nesta conta do Windows. Não precisa de administrador.%n%nA pasta, atalhos e início com o Windows são mantidos. Nenhum jogo é fechado.%n%nSe o programa estiver aberto, o instalador pede para encerrá-lo antes de copiar os arquivos.%n%nClique em Avançar para continuar.
english.UpgradeWelcomeBody=This will update Game Optimizer %1 to %2 for this Windows account. Administrator rights are not required.%n%nYour folder, shortcuts, and start-with-Windows setting are kept. Games are never closed.%n%nIf the app is running, Setup will ask to close it before copying files.%n%nClick Next to continue.
brazilianportuguese.UpgradeWelcomeBodyUnknown=Este assistente vai atualizar o Game Optimizer para %1 nesta conta do Windows. Não precisa de administrador.%n%nA pasta e os atalhos são mantidos. Clique em Avançar para continuar.
english.UpgradeWelcomeBodyUnknown=This will update Game Optimizer to %1 for this Windows account. Administrator rights are not required.%n%nYour folder and shortcuts are kept. Click Next to continue.
brazilianportuguese.UpgradeFinished=O Game Optimizer foi atualizado para {#AppVersion}. Você pode abrir o programa agora ou depois pelo Menu Iniciar.
english.UpgradeFinished=Game Optimizer was updated to {#AppVersion}. You can launch it now or later from the Start Menu.
brazilianportuguese.AppRunningPrompt=O Game Optimizer está aberto (pode estar só na bandeja, no canto da barra de tarefas).%n%nFechar a janela não encerra o programa — ele vai para a bandeja. O instalador precisa encerrá-lo para atualizar os arquivos.%n%nEncerrar o Game Optimizer agora?
english.AppRunningPrompt=Game Optimizer is running (it may only be in the tray, near the clock).%n%nClosing the window does not quit — it hides to the tray. Setup must exit it to replace files.%n%nClose Game Optimizer now?
brazilianportuguese.AppStillRunning=Não foi possível encerrar o Game Optimizer. Clique com o botão direito no ícone da bandeja, escolha Sair, e execute o instalador de novo.
english.AppStillRunning=Could not close Game Optimizer. Right-click the tray icon, choose Exit, then run Setup again.
brazilianportuguese.AlreadyNewer=O Game Optimizer %1 já está instalado. Este instalador é a versão %2.%n%nInstalar mesmo assim?
english.AlreadyNewer=Game Optimizer %1 is already installed. This setup is version %2.%n%nInstall anyway?
brazilianportuguese.UpgradeFeaturesTitle=O que será atualizado
english.UpgradeFeaturesTitle=What will be updated
brazilianportuguese.UpgradeFeaturesSub=Atualização na sua conta do Windows
english.UpgradeFeaturesSub=Update for this Windows account

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

function InnoUninstallKey: String;
begin
  { Braces via Chr so ISPP does not treat the AppId as a constant. }
  Result := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\' +
    Chr(123) + 'E8C3A91F-6B2D-4F70-9A15-3C7E8D4B2A10}_is1';
end;

function PsUninstallKey: String;
begin
  Result := 'Software\Microsoft\Windows\CurrentVersion\Uninstall\GameOptimizer';
end;

function GetInstalledVersion: String;
begin
  Result := '';
  if RegQueryStringValue(HKCU, InnoUninstallKey, 'DisplayVersion', Result) then
    Exit;
  if RegQueryStringValue(HKCU, PsUninstallKey, 'DisplayVersion', Result) then
    Exit;
  Result := '';
end;

function PreviousInstallDir: String;
begin
  if RegQueryStringValue(HKCU, InnoUninstallKey, 'InstallLocation', Result) and
     (Result <> '') then
    Exit;
  if RegQueryStringValue(HKCU, PsUninstallKey, 'InstallLocation', Result) and
     (Result <> '') then
    Exit;
  Result := ExpandConstant('{localappdata}\GameOptimizer');
end;

function IsUpgrade: Boolean;
begin
  Result :=
    RegKeyExists(HKCU, InnoUninstallKey) or
    RegKeyExists(HKCU, PsUninstallKey) or
    FileExists(AddBackslash(PreviousInstallDir) + '{#AppExeName}');
end;

function InstalledIsNewerOrSame: Boolean;
var
  InstalledPacked, SetupPacked: Int64;
  Installed: String;
begin
  Result := False;
  Installed := GetInstalledVersion;
  if Installed = '' then
    Exit;
  if not StrToPackedVersion(Installed, InstalledPacked) then
    Exit;
  if not StrToPackedVersion('{#AppVersion}', SetupPacked) then
    Exit;
  Result := ComparePackedVersion(InstalledPacked, SetupPacked) >= 0;
end;

function IsAppRunning: Boolean;
var
  ResultCode: Integer;
  TmpFile: String;
  Lines: TArrayOfString;
  I: Integer;
begin
  Result :=
    CheckForMutexes('Local\GameOptimizerSingleInstance') or
    CheckForMutexes('GameOptimizerSingleInstance');
  if Result then
    Exit;

  if FindWindowByWindowName('Game Optimizer') <> 0 then
  begin
    Result := True;
    Exit;
  end;

  TmpFile := ExpandConstant('{tmp}\gpc-tasklist.txt');
  if Exec(ExpandConstant('{cmd}'),
     '/C tasklist /FI "IMAGENAME eq game_optimizer.exe" /FO CSV /NH > "' + TmpFile + '" 2>nul',
     '', SW_HIDE, ewWaitUntilTerminated, ResultCode) then
  begin
    if LoadStringsFromFile(TmpFile, Lines) then
    begin
      for I := 0 to GetArrayLength(Lines) - 1 do
      begin
        if Pos('game_optimizer.exe', LowerCase(Lines[I])) > 0 then
        begin
          Result := True;
          Exit;
        end;
      end;
    end;
  end;
end;

procedure CloseAppIfRunning;
var
  ResultCode: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/IM game_optimizer.exe /F /T',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode);
end;

function ConfirmCloseIfRunning: Boolean;
begin
  Result := True;
  if not IsAppRunning then
    Exit;

  if MsgBox(CustomMessage('AppRunningPrompt'), mbConfirmation, MB_YESNO) <> IDYES then
  begin
    Result := False;
    Exit;
  end;

  CloseAppIfRunning;
  Sleep(500);
  if IsAppRunning then
  begin
    MsgBox(CustomMessage('AppStillRunning'), mbError, MB_OK);
    Result := False;
  end;
end;

function InitializeSetup(): Boolean;
begin
  Result := True;
  if InstalledIsNewerOrSame then
  begin
    if MsgBox(FmtMessage(CustomMessage('AlreadyNewer'),
         [GetInstalledVersion, '{#AppVersion}']), mbConfirmation, MB_YESNO) <> IDYES then
    begin
      Result := False;
      Exit;
    end;
  end;
  Result := ConfirmCloseIfRunning;
end;

function InitializeUninstall(): Boolean;
begin
  Result := ConfirmCloseIfRunning;
end;

procedure ApplyWelcomeUpgradeText;
var
  Installed: String;
begin
  if not IsUpgrade then
    Exit;
  Installed := GetInstalledVersion;
  WizardForm.WelcomeLabel1.Caption := CustomMessage('UpgradeWelcomeTitle');
  if Installed <> '' then
    WizardForm.WelcomeLabel2.Caption :=
      FmtMessage(CustomMessage('UpgradeWelcomeBody'), [Installed, '{#AppVersion}'])
  else
    WizardForm.WelcomeLabel2.Caption :=
      FmtMessage(CustomMessage('UpgradeWelcomeBodyUnknown'), ['{#AppVersion}']);
end;

procedure InitializeWizard;
begin
  FeaturesPage := CreateOutputMsgMemoPage(
    wpWelcome,
    CustomMessage('FeaturesTitle'),
    CustomMessage('FeaturesSub'),
    CustomMessage('FeaturesCaption'),
    CustomMessage('FeaturesBody'));
  if IsUpgrade then
  begin
    FeaturesPage.Caption := CustomMessage('UpgradeFeaturesTitle');
    FeaturesPage.Description := CustomMessage('UpgradeFeaturesSub');
  end;
  ApplyWelcomeUpgradeText;
end;

procedure CurPageChanged(CurPageID: Integer);
begin
  if CurPageID = wpWelcome then
    ApplyWelcomeUpgradeText;
  if (CurPageID = wpFinished) and IsUpgrade then
    WizardForm.FinishedLabel.Caption := CustomMessage('UpgradeFinished');
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
  if CurStep = ssInstall then
  begin
    { Files in use: CloseApplications already asked; force-close leftovers. }
    if IsAppRunning then
      CloseAppIfRunning;
  end;
  if CurStep = ssPostInstall then
  begin
    RemoveDuplicateAutostart;
    SaveStringToFile(ExpandConstant('{app}\.autostart-initialized'), '1', False);
    { Inno already ran the setup wizard; skip the in-app first-run wizard. }
    SaveStringToFile(ExpandConstant('{app}\.setup-wizard-complete'), '1', False);
  end;
end;
