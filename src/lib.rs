//! Game-only Windows performance optimizer (GPC).
//!
//! Applies a Game Performance Contract to detected games — never terminates
//! them. May reclaim working sets of large non-game apps when configured.

#![deny(unsafe_code)]
#![deny(missing_docs)]

pub mod autostart;
pub mod config;
pub mod contract;
pub mod detect;
pub mod gui;
pub mod optimize;
pub mod report;

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
