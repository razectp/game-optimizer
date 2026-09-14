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
pub mod optimize;
pub mod report;
pub mod setup_wizard;
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
        let files = [
            (
                "installer/install.ps1",
                include_str!("../installer/install.ps1"),
            ),
            (
                "installer/GameOptimizer.iss",
                include_str!("../installer/GameOptimizer.iss"),
            ),
            (
                "installer/package.ps1",
                include_str!("../installer/package.ps1"),
            ),
            ("changelog.md", include_str!("../changelog.md")),
            ("README.md", include_str!("../README.md")),
            (
                "assets/app.manifest",
                include_str!("../assets/app.manifest"),
            ),
        ];
        for (name, body) in files {
            assert!(
                body.contains(version),
                "{name} does not mention crate version {version}"
            );
        }
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
    }

    #[test]
    fn gui_has_no_activity_log_surface() {
        let gui = include_str!("gui.rs");
        assert!(
            !gui.contains("Atividade"),
            "activity log section should stay removed"
        );
        assert!(!gui.contains("push_log"));
        assert!(!gui.contains("logs: Vec"));
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
    fn gui_embeds_first_run_setup_wizard() {
        let gui = include_str!("gui.rs");
        assert!(gui.contains("draw_setup_wizard"));
        assert!(gui.contains("Assistente de configuração"));
        let wizard = include_str!("setup_wizard.rs");
        assert!(wizard.contains("Bem-vindo"));
        assert!(wizard.contains("PAGE_COUNT: usize = 4"));
    }

    #[test]
    fn gui_scrolls_instead_of_forcing_window_height() {
        let gui = include_str!("gui.rs");
        assert!(gui.contains("id_salt(\"main_scroll\")"));
        assert!(!gui.contains("ui.set_min_height(available)"));
        assert!(gui.contains("Verificar atualização"));
    }

    #[test]
    fn autostart_stays_on_current_user_run_key() {
        let autostart = include_str!("autostart.rs");
        assert!(autostart.contains("WindowsEnableMode::CurrentUser"));
        assert!(autostart.contains("remove_duplicate_entries"));
        assert!(autostart.contains("HKLM"));
    }

    #[test]
    fn inno_strips_duplicate_autostart_on_upgrade() {
        let iss = include_str!("../installer/GameOptimizer.iss");
        assert!(iss.contains("RemoveDuplicateAutostart"));
        assert!(iss.contains("ssPostInstall"));
        assert!(iss.contains("Game Optimizer.lnk"));
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
