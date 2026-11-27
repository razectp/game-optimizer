//! Classify running processes as games or leave them untouched.

use std::path::Path;

use sysinfo::{Pid, Process, ProcessesToUpdate, System};
#[cfg(target_os = "linux")]
use sysinfo::{ProcessRefreshKind, UpdateKind};

use crate::config::{game_path_markers, normalize_process_name, AppConfig};

/// High-level role of a process for the optimizer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ProcessClass {
    /// Matched game — GPC optimize; never terminate.
    Game,
    /// OS / security critical — leave untouched.
    Protected,
    /// Large **browser** eligible for working-set reclaim (Chrome, Edge, Firefox).
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

/// Detailed information for interactive process picking and root detection in GUI.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RunningProcessInfo {
    /// OS process identifier.
    pub pid: u32,
    /// Parent process identifier if available.
    pub parent_pid: Option<u32>,
    /// Root process PID in this application tree.
    pub root_pid: u32,
    /// Root process executable name (without `.exe`, normalized).
    pub root_name: String,
    /// Process display name as reported by the OS.
    pub name: String,
    /// Normalized process name used for matching.
    pub normalized_name: String,
    /// Working set memory in bytes.
    pub memory_bytes: u64,
    /// Full path to the executable if accessible.
    pub exe_path: Option<String>,
    /// Whether this process is recognized as a game in current config.
    pub is_game: bool,
}

/// Enumerate all non-system running processes with root process tree detection.
pub fn list_running_processes(config: &AppConfig) -> Vec<RunningProcessInfo> {
    let system = scan_system();
    let self_pid = std::process::id();

    let mut result = Vec::new();

    for (pid, process) in system.processes() {
        let pid_u32 = pid_as_u32(*pid);
        if pid_u32 == 0 || pid_u32 == self_pid {
            continue;
        }

        let (name, exe_path) = process_identity(process);
        let normalized = normalize_process_name(&name);

        if is_protected_name(&normalized) {
            continue;
        }

        let memory_bytes = process.memory();
        let parent_pid = process.parent().map(pid_as_u32);

        // Walk up parent tree to find root process
        let mut root_pid = pid_u32;
        let mut root_name = normalized.clone();
        let mut curr_pid = *pid;

        while let Some(parent_p) = system.process(curr_pid).and_then(|p| p.parent()) {
            let parent_u32 = pid_as_u32(parent_p);
            if parent_u32 == 0 || parent_u32 == pid_u32 {
                break;
            }
            if let Some(parent_proc) = system.process(parent_p) {
                let p_norm = normalize_process_name(&process_identity(parent_proc).0);
                if is_protected_name(&p_norm) || is_launcher_name(&p_norm) {
                    root_pid = pid_as_u32(curr_pid);
                    if let Some(cp) = system.process(curr_pid) {
                        root_name = normalize_process_name(&process_identity(cp).0);
                    }
                    break;
                }
                root_pid = parent_u32;
                root_name = p_norm;
                curr_pid = parent_p;
            } else {
                break;
            }
        }

        let is_game = is_game(config, &normalized, exe_path.as_deref().map(Path::new))
            || (cfg!(windows) && config.game_names.contains(&root_name));

        result.push(RunningProcessInfo {
            pid: pid_u32,
            parent_pid,
            root_pid,
            root_name,
            name,
            normalized_name: normalized,
            memory_bytes,
            exe_path,
            is_game,
        });
    }

    result.sort_by_key(|p| std::cmp::Reverse(p.memory_bytes));
    result
}

/// Known game launcher process names (without `.exe`).
pub fn is_launcher_name(name: &str) -> bool {
    matches!(
        name,
        "steam"
            | "steam-runtime-launcher-service"
            | "proton"
            | "wineserver"
            | "wineserver64"
            | "wine"
            | "wine64"
            | "wine-preloader"
            | "wine64-preloader"
            | "pressure-vessel-wrap"
            | "steam-launch-wrapper"
            | "systemd"
            | "systemd-run"
            | "steamwebhelper"
            | "steamservice"
            | "epicgameslauncher"
            | "epicwebhelper"
            | "galaxyclient"
            | "origin"
            | "eadesktop"
            | "riotclientservices"
            | "wemade"
            | "wemadeclient"
            | "battlenet"
            | "upc"
    )
}

/// Collect and classify all processes using `config`.
pub fn snapshot_processes(config: &AppConfig) -> Vec<ProcessSnapshot> {
    let system = scan_system();

    let self_pid = std::process::id();
    let mut out = Vec::new();

    for (pid, process) in system.processes() {
        let pid_u32 = pid_as_u32(*pid);
        let (name, exe_path) = process_identity(process);
        let normalized = normalize_process_name(&name);
        let memory_bytes = process.memory();

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

fn scan_system() -> System {
    let mut system = System::new();
    #[cfg(target_os = "linux")]
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing()
            .without_tasks()
            .with_memory()
            .with_exe(UpdateKind::OnlyIfNotSet)
            .with_cmd(UpdateKind::OnlyIfNotSet),
    );
    #[cfg(not(target_os = "linux"))]
    system.refresh_processes(ProcessesToUpdate::All, true);
    system
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
    if is_helper_tool(normalized_name) {
        return ProcessClass::Ignored;
    }
    if is_game(config, normalized_name, exe_path) {
        return ProcessClass::Game;
    }
    if config.never_trim_names.contains(normalized_name) {
        return ProcessClass::Ignored;
    }
    if config.reclaim_names.contains(normalized_name)
        && memory_bytes >= config.memory_trim_threshold_bytes
    {
        return ProcessClass::ReclaimCandidate;
    }
    ProcessClass::Ignored
}

fn is_game(config: &AppConfig, normalized_name: &str, exe_path: Option<&Path>) -> bool {
    if is_helper_tool(normalized_name) {
        return false;
    }
    #[cfg(target_os = "linux")]
    if is_launcher_name(normalized_name)
        || matches!(
            normalized_name,
            "start" | "wineboot" | "winedevice" | "explorer" | "mir4launcher"
        )
    {
        return false;
    }
    if config.game_names.contains(normalized_name) {
        return true;
    }
    if let Some(path) = exe_path {
        let lower = path.to_string_lossy().to_lowercase().replace('/', "\\");
        #[cfg(target_os = "linux")]
        if let Some((_, library_path)) = lower.split_once(r"\steamapps\common\") {
            let directory = library_path.split('\\').next().unwrap_or_default();
            return !(directory.starts_with("proton ")
                || directory.starts_with("steamlinuxruntime")
                || directory == "steamworks shared");
        }
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

fn process_identity(process: &Process) -> (String, Option<String>) {
    let name = process.name().to_string_lossy().into_owned();
    #[cfg(target_os = "linux")]
    if let Some(argv0) = process.cmd().first() {
        if let Some(executable) = proton_executable(&argv0.to_string_lossy()) {
            return executable;
        }
    }
    let path = process.exe().map(|p| p.to_string_lossy().into_owned());
    (name, path)
}

#[cfg(target_os = "linux")]
fn proton_executable(argv0: &str) -> Option<(String, Option<String>)> {
    let name = argv0.rsplit(['/', '\\']).next()?;
    if !name.to_ascii_lowercase().ends_with(".exe") {
        return None;
    }
    Some((name.to_owned(), Some(argv0.to_owned())))
}

/// Encoders and crash helpers must never be treated as games or trim targets.
///
/// `EmptyWorkingSet` pages RAM to disk; doing that to ffmpeg (or a similar
/// encoder) while it is assembling video causes stalls and dropped frames.
fn is_helper_tool(normalized: &str) -> bool {
    matches!(
        normalized,
        "ffmpeg"
            | "ffplay"
            | "ffprobe"
            | "handbrake"
            | "handbrakecli"
            | "obs64"
            | "obs32"
            | "obs"
            | "afterfx"
            | "blender"
            | "capcut"
            | "shotcut"
            | "kdenlive"
            | "resolve"
            | "davinci resolve"
            | "adobe premiere pro"
            | "premiere pro"
            | "adobe media encoder"
            | "nvencc"
            | "qsvencc"
            | "vceencc"
            | "crashpad_handler"
            | "unitycrashhandler64"
            | "unitycrashhandler32"
    )
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
    fn firefox_and_edge_are_reclaim_candidates() {
        let cfg = cfg();
        assert_eq!(
            classify_process(&cfg, 1, 2, "firefox", 400 * 1024 * 1024, None),
            ProcessClass::ReclaimCandidate
        );
        assert_eq!(
            classify_process(&cfg, 1, 3, "msedge", 400 * 1024 * 1024, None),
            ProcessClass::ReclaimCandidate
        );
    }

    #[test]
    fn large_non_browser_is_not_trimmed() {
        let class = classify_process(&cfg(), 1, 44, "notepad++", 800 * 1024 * 1024, None);
        assert_eq!(class, ProcessClass::Ignored);
    }

    #[test]
    fn ffmpeg_is_never_reclaim_or_game() {
        let cfg = cfg();
        let class = classify_process(&cfg, 1, 70, "ffmpeg", 2_000_000_000, None);
        assert_eq!(class, ProcessClass::Ignored);
        let steam_ffmpeg =
            Path::new(r"C:\Program Files (x86)\Steam\steamapps\common\Tool\ffmpeg.exe");
        let class = classify_process(&cfg, 1, 71, "ffmpeg", 2_000_000_000, Some(steam_ffmpeg));
        assert_eq!(class, ProcessClass::Ignored);
    }

    #[test]
    fn chrome_below_threshold_is_ignored() {
        let class = classify_process(&cfg(), 1, 99, "chrome", 50 * 1024 * 1024, None);
        assert_eq!(class, ProcessClass::Ignored);
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

    #[cfg(target_os = "linux")]
    #[test]
    fn proton_uses_the_windows_image_name_from_argv0() {
        for executable in [
            r"Z:\home\player\Steam Library\steamapps\common\MIR4\Mir4G.EXE",
            "/home/player/.steam/steam/steamapps/common/MIR4/Mir4G.exe",
            "Mir4G.exe",
        ] {
            let (name, path) = proton_executable(executable).unwrap();
            assert!(is_game(
                &cfg(),
                &normalize_process_name(&name),
                path.as_deref().map(Path::new),
            ));
        }
        assert!(proton_executable("/opt/Proton/proton").is_none());
        assert!(proton_executable("/opt/Proton/files/bin/wine64-preloader").is_none());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn alternate_steam_libraries_are_detected_without_counting_proton() {
        let library = Path::new("/games/SteamLibrary/steamapps/common");
        assert!(is_game(
            &cfg(),
            "client-win64-shipping",
            Some(&library.join("Example/Client-Win64-Shipping.exe")),
        ));
        assert!(!is_game(
            &cfg(),
            "wine64",
            Some(&library.join("Proton/files/bin/wine64")),
        ));
        assert!(!is_game(
            &cfg(),
            "pv-bwrap",
            Some(&library.join("SteamLinuxRuntime_sniper/pressure-vessel/bin/pv-bwrap")),
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scanning_linux_omits_worker_threads() {
        std::thread::spawn(|| {
            let thread_path = std::fs::read_link("/proc/thread-self").unwrap();
            let tid: u32 = thread_path
                .file_name()
                .unwrap()
                .to_str()
                .unwrap()
                .parse()
                .unwrap();
            let pid = std::process::id();

            let system = scan_system();
            assert!(system.process(Pid::from_u32(pid)).is_some());
            assert!(system.process(Pid::from_u32(tid)).is_none());
        })
        .join()
        .unwrap();
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn scanning_a_live_process_uses_its_windows_command_name() {
        use std::os::unix::process::CommandExt;
        let mut process = std::process::Command::new("/bin/sleep")
            .arg0(r"Z:\Steam Library\steamapps\common\MIR4\Mir4G.exe")
            .arg("10")
            .spawn()
            .unwrap();
        let processes = snapshot_processes(&cfg());
        let _ = process.kill();
        process.wait().unwrap();
        assert!(processes.iter().any(|entry| {
            entry.pid == process.id()
                && entry.normalized_name == "mir4g"
                && entry.class == ProcessClass::Game
        }));
    }
}
