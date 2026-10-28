//! Game-only Windows performance optimizer (GPC).
//!
//! Applies a Game Performance Contract to detected games — never terminates
//! them. May reclaim working sets of **browsers** when configured.

#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod autostart;
pub mod cache;
pub mod config;
pub mod contract;
pub mod detect;
pub mod gui;
pub mod macros;
pub mod optimize;
pub mod report;
pub mod setup_wizard;
pub mod tunnel;
pub mod update;
pub mod win_window;

#[cfg(windows)]
pub mod win_process;

#[cfg(not(windows))]
pub mod win_process {
    //! Stub for non-Windows hosts (compile-only messaging).
    use crate::optimize::{ActionResult, OptimizeAction};

    /// No-op outside Windows.
    pub fn enable_debug_privilege() {}

    /// No CPU set data outside Windows.
    pub fn list_performance_cpu_sets() -> Vec<u32> {
        Vec::new()
    }

    /// Stub logical CPU count.
    pub fn logical_processor_count() -> usize {
        1
    }

    /// Always fails: this tool targets Windows.
    pub fn apply_action(_action: &OptimizeAction) -> ActionResult {
        ActionResult {
            ok: false,
            detail: "game_optimizer runs on Windows only".to_string(),
        }
    }
}

pub use config::{AppConfig, GameCatalog};
pub use contract::{ContractRole, GameContract};
pub use detect::{classify_process, ProcessClass, ProcessSnapshot};
pub use optimize::{optimize_system, OptimizeReport, OptimizeRequest};

#[cfg(test)]
mod release_packaging {
    #[test]
    fn packaging_files_embed_crate_version() {
        let version = env!("CARGO_PKG_VERSION");
        let versioned_files = [
            ("changelog.md", include_str!("../changelog.md")),
            ("README.md", include_str!("../README.md")),
        ];
        for (name, body) in versioned_files {
            assert!(
                body.contains(version),
                "{name} does not mention crate version {version}"
            );
        }
        assert!(include_str!("../assets/app.manifest").contains(&format!("{version}.0")));
        assert!(include_str!("../assets/app.manifest").contains("level=\"requireAdministrator\""));
        assert!(include_str!("../installer/package.ps1").contains("Cargo.toml"));
        assert!(include_str!("../installer/GameOptimizer.iss").contains("GetFileVersion"));
        assert!(include_str!("../installer/GameOptimizer.iss")
            .contains("SetupVersion = '{#AppVersion}'"));
        assert!(include_str!("../installer/GameOptimizer.iss").contains("Len(AppVersionFull) - 2"));
    }

    #[test]
    fn end_user_installer_does_not_require_cargo_by_default() {
        let install = include_str!("../installer/install.ps1");
        assert!(install.contains("[switch]$Build"));
        assert!(
            install.contains("Does NOT run cargo unless -Build")
                || install.contains("does NOT run cargo unless -Build")
        );
        assert!(install.contains("Uninstall\\GameOptimizer"));
        assert!(install.contains("HKCU:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"));
        assert!(install.contains(".autostart-initialized"));
        assert!(install.contains("Required wintun.dll"));
        assert!(install.contains("wintun.dll"));
    }

    #[test]
    fn packaged_payload_contains_required_wintun() {
        let package = include_str!("../installer/package.ps1");
        let iss = include_str!("../installer/GameOptimizer.iss");
        let install = include_str!("../installer/install.ps1");
        assert!(package.contains("wintun.dll"));
        assert!(package.contains("refusing to create an incomplete package"));
        assert!(iss.contains("Source: \"{#PackageDir}\\wintun.dll\"; DestDir: \"{app}\""));
        assert!(iss.contains("Package payload missing required wintun.dll"));
        assert!(package.contains("--locked --release --target-dir $BuildDir"));
        assert!(package.contains("ZipFileExtensions]::CreateEntryFromFile"));
        assert!(install.contains("Destination (Join-Path $InstallDir \"wintun.dll\")"));
        let uninstall = include_str!("../installer/uninstall.ps1");
        assert!(uninstall.contains("config.toml"));
        assert!(uninstall.contains("wintun.dll"));
        assert!(uninstall.contains("[System.IO.Directory]::Delete($InstallDir, $false)"));
        assert!(!uninstall.contains("Remove-ItemIfExists -Path $InstallDir -Recurse"));
    }

    #[test]
    fn inno_script_is_a_full_setup_wizard() {
        let iss = include_str!("../installer/GameOptimizer.iss");
        assert!(iss.contains("DisableWelcomePage=no"));
        assert!(iss.contains("InfoBeforeFile"));
        assert!(iss.contains("LicenseFile"));
        assert!(iss.contains("CreateOutputMsgMemoPage"));
        assert!(iss.contains("WizardStyle=modern"));
        assert!(iss.contains("ShowLanguageDialog=yes"));
        assert!(iss.contains(".setup-wizard-complete"));
        assert!(iss.contains("wizard-sidebar.bmp"));
    }

    #[test]
    fn installer_requires_admin_but_runs_gui_as_current_user() {
        let iss = include_str!("../installer/GameOptimizer.iss");
        assert!(iss.contains("PrivilegesRequired=admin"));
        assert!(!iss.contains("PrivilegesRequired=lowest"));
        assert!(iss.contains("postinstall skipifsilent runascurrentuser"));
        assert!(!iss.contains("asInvoker"));
        assert!(iss.contains("autostart ' + AutoStartArg"));
        assert!(iss.contains("WizardIsTaskSelected('autostart')"));
        let install = include_str!("../installer/install.ps1");
        let uninstall = include_str!("../installer/uninstall.ps1");
        assert!(install.contains("#Requires -RunAsAdministrator"));
        assert!(uninstall.contains("#Requires -RunAsAdministrator"));
        assert!(install.contains("@(\"autostart\", $mode)"));
        assert!(install.contains("$process.ExitCode -ne 0"));
        let main = include_str!("main.rs");
        assert!(main.contains("Commands::Autostart { on, off }"));
        assert!(main.contains("autostart::set_enabled(on)"));
        assert!(iss.contains("if ResultCode <> 0"));
        assert!(iss.contains("Flags: deletevalue uninsdeletevalue"));
        assert!(!iss.contains("ValueData: \"\"\"{app}\\{#AppExeName}\"\" --gui\""));
        assert!(!iss.contains("Root: HKLM;"));
        assert!(!uninstall.contains("HKLM:\\Software\\Microsoft\\Windows\\CurrentVersion\\Run"));
    }

    #[test]
    fn admin_installer_docs_explain_per_user_profile() {
        let iss = include_str!("../installer/GameOptimizer.iss");
        for key in ["WelcomeLabel2", "FeaturesBody", "UpgradeWelcomeBody"] {
            assert!(iss.contains(key));
        }
        assert!(
            include_str!("../installer/welcome-pt.txt").contains("autorização de administrador")
        );
        assert!(include_str!("../installer/welcome-en.txt").contains("Administrator authorization"));
        assert!(include_str!("../.github/workflows/release.yml")
            .contains("autorização de administrador"));
    }

    #[test]
    fn inno_strips_duplicate_autostart_on_upgrade() {
        let iss = include_str!("../installer/GameOptimizer.iss");
        assert!(iss.contains("RemoveDuplicateAutostart"));
        assert!(iss.contains("ssPostInstall"));
        assert!(iss.contains("Game Optimizer.lnk"));
        assert!(iss.contains("WizardSelectTasks('!autostart')"));
    }

    #[test]
    fn inno_detects_previous_install_and_asks_before_close() {
        let iss = include_str!("../installer/GameOptimizer.iss");
        assert!(iss.contains("function IsUpgrade"));
        assert!(iss.contains("ConfirmCloseIfRunning"));
        assert!(iss.contains("AppRunningPrompt"));
        assert!(iss.contains("AlreadyNewer"));
        assert!(iss.contains("FindWindowByWindowName"));
        assert!(iss.contains("DisableDirPage=auto"));
        assert!(iss.contains("UpgradeWelcomeBody"));
        let install = include_str!("../installer/install.ps1");
        assert!(install.contains("Request-CloseGameOptimizer"));
        assert!(install.contains("Instalação anterior encontrada"));
    }

    #[test]
    fn single_instance_mutex_is_named() {
        let win = include_str!("win_window.rs");
        assert!(win.contains("GameOptimizerSingleInstance"));
        assert!(win.contains("try_become_single_instance"));
    }

    #[test]
    fn update_module_targets_github_releases() {
        let update = include_str!("update.rs");
        assert!(update.contains("releases/latest"));
        assert!(update.contains("-setup.exe"));
        assert!(update.contains("curl"));
    }
}
