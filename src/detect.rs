//! Classify running processes as games or leave them untouched.

use std::path::Path;

use sysinfo::{Pid, ProcessesToUpdate, System};

use crate::config::{game_path_markers, normalize_process_name, AppConfig};

/// High-level role of a process for the optimizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessClass {
    /// Matched game — GPC optimize; never terminate.
    Game,
    /// OS / security critical — leave untouched.
    Protected,
    /// Large non-game eligible for working-set reclaim (e.g. Chrome).
    ReclaimCandidate,
    /// Everything else — left alone.
    Ignored,
}

/// Snapshot of one process relevant to optimization decisions.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProcessSnapshot {
    /// OS process id.
    pub pid: u32,
    /// Display name (as reported by the OS).
    pub name: String,
    /// Normalized name used for matching.
    pub normalized_name: String,
    /// Working set / memory footprint in bytes.
    pub memory_bytes: u64,
    /// Executable path when available.
    pub exe_path: Option<String>,
    /// Classification result.
    pub class: ProcessClass,
}

/// Collect and classify all processes using `config`.
pub fn snapshot_processes(config: &AppConfig) -> Vec<ProcessSnapshot> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);

    let self_pid = std::process::id();
    let mut out = Vec::new();

    for (pid, process) in system.processes() {
        let pid_u32 = pid_as_u32(*pid);
        let name = process.name().to_string_lossy().into_owned();
        let normalized = normalize_process_name(&name);
        let memory_bytes = process.memory();
        let exe_path = process.exe().map(|p| p.to_string_lossy().into_owned());

        let class = classify_process(
            config,
            self_pid,
            pid_u32,
            &normalized,
            memory_bytes,
            exe_path.as_deref().map(Path::new),
        );

        out.push(ProcessSnapshot {
            pid: pid_u32,
            name,
            normalized_name: normalized,
            memory_bytes,
            exe_path,
            class,
        });
    }

    out.sort_by_key(|a| std::cmp::Reverse(a.memory_bytes));
    out
}

/// Classify a single process from discrete fields (unit-test friendly).
pub fn classify_process(
    config: &AppConfig,
    self_pid: u32,
    pid: u32,
    normalized_name: &str,
    memory_bytes: u64,
    exe_path: Option<&Path>,
) -> ProcessClass {
    if pid == 0 || pid == self_pid {
        return ProcessClass::Protected;
    }
    if is_protected_name(normalized_name) {
        return ProcessClass::Protected;
    }
    if is_game(config, normalized_name, exe_path) {
        return ProcessClass::Game;
    }
    if config.never_trim_names.contains(normalized_name) {
        return ProcessClass::Ignored;
    }
    if memory_bytes >= config.memory_trim_threshold_bytes {
        return ProcessClass::ReclaimCandidate;
    }
    ProcessClass::Ignored
}

fn is_game(config: &AppConfig, normalized_name: &str, exe_path: Option<&Path>) -> bool {
    if config.game_names.contains(normalized_name) {
        return true;
    }
    if let Some(path) = exe_path {
        let lower = path.to_string_lossy().to_lowercase();
        if game_path_markers()
            .iter()
            .any(|marker| lower.contains(marker))
        {
            if matches!(
                normalized_name,
                "steam"
                    | "steamwebhelper"
                    | "steamservice"
                    | "epicgameslauncher"
                    | "epicwebhelper"
                    | "galaxyclient"
                    | "origin"
                    | "eadesktop"
            ) {
                return false;
            }
            return true;
        }
    }
    false
}

fn is_protected_name(normalized: &str) -> bool {
    matches!(
        normalized,
        "system"
            | "registry"
            | "smss"
            | "csrss"
            | "wininit"
            | "services"
            | "lsass"
            | "winlogon"
            | "svchost"
            | "dwm"
            | "fontdrvhost"
            | "lsaiso"
            | "memory compression"
            | "secure system"
            | "idle"
            | "system interrupts"
            | "conhost"
            | "sihost"
            | "taskhostw"
            | "runtimebroker"
            | "searchhost"
            | "startmenuexperiencehost"
            | "shellexperiencehost"
            | "explorer"
            | "securityhealthservice"
            | "securityhealthsystray"
            | "msmpeng"
            | "nissrv"
            | "mpdefendercoreservice"
            | "wdagutilityaccount"
            | "game_optimizer"
    )
}

fn pid_as_u32(pid: Pid) -> u32 {
    pid.as_u32()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    fn cfg() -> AppConfig {
        AppConfig::default()
    }

    #[test]
    fn known_title_is_game() {
        let class = classify_process(&cfg(), 1, 42, "mir4g", 2_000_000_000, None);
        assert_eq!(class, ProcessClass::Game);
    }

    #[test]
    fn chrome_large_is_reclaim_candidate() {
        let class = classify_process(&cfg(), 1, 99, "chrome", 500 * 1024 * 1024, None);
        assert_eq!(class, ProcessClass::ReclaimCandidate);
    }

    #[test]
    fn cursor_is_never_trim_even_if_large() {
        let class = classify_process(&cfg(), 1, 88, "cursor", 800 * 1024 * 1024, None);
        assert_eq!(class, ProcessClass::Ignored);
    }

    #[test]
    fn lsass_is_protected() {
        let class = classify_process(&cfg(), 1, 77, "lsass", 900 * 1024 * 1024, None);
        assert_eq!(class, ProcessClass::Protected);
    }

    #[test]
    fn self_process_is_protected() {
        let class = classify_process(&cfg(), 55, 55, "game_optimizer", 50 * 1024 * 1024, None);
        assert_eq!(class, ProcessClass::Protected);
    }

    #[test]
    fn steam_common_path_marks_game_exe() {
        let path = Path::new(r"C:\Program Files (x86)\Steam\steamapps\common\Foo\Foo.exe");
        let class = classify_process(&cfg(), 1, 10, "foo", 100 * 1024 * 1024, Some(path));
        assert_eq!(class, ProcessClass::Game);
    }

    #[test]
    fn steam_client_in_steam_path_is_not_game() {
        let path = Path::new(r"C:\Program Files (x86)\Steam\steam.exe");
        let class = classify_process(&cfg(), 1, 10, "steam", 300 * 1024 * 1024, Some(path));
        assert_ne!(class, ProcessClass::Game);
    }
}
