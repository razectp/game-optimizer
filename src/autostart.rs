//! Windows / cross-platform auto-start helpers.

use std::env;
use std::fs;
use std::path::PathBuf;

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

const APP_NAME: &str = "GameOptimizer";
const MARKER_NAME: &str = ".autostart-initialized";

/// Extra Run-key names left by older builds or auto-launch Dynamic mode.
#[cfg_attr(not(windows), allow(dead_code))]
const LEFTOVER_RUN_NAMES: &[&str] = &[
    "Game Optimizer",
    "game_optimizer",
    "game_optimizer.exe",
    "GameOptimizer.exe",
];

/// Build an auto-launch handle for the current executable (HKCU only).
pub fn builder_for_current_exe() -> anyhow::Result<AutoLaunch> {
    let exe = env::current_exe()?;
    let path = normalize_exe_path(exe);
    let mut builder = AutoLaunchBuilder::new();
    builder
        .set_app_name(APP_NAME)
        .set_app_path(&path)
        .set_args(&["--gui"]);
    #[cfg(windows)]
    {
        builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    }
    builder
        .build()
        .map_err(|err| anyhow::anyhow!("auto-launch setup failed: {err}"))
}

fn normalize_exe_path(exe: PathBuf) -> String {
    exe.to_string_lossy().replace('/', "\\")
}

/// Per-user data directory (`%LOCALAPPDATA%\GameOptimizer`, or the exe folder).
pub fn data_dir() -> PathBuf {
    if let Ok(local) = env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("GameOptimizer");
    }
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."))
}

/// Path of the first-run marker (install dir or `%LOCALAPPDATA%\GameOptimizer`).
pub fn initialized_marker_path() -> PathBuf {
    data_dir().join(MARKER_NAME)
}

fn write_initialized_marker() -> anyhow::Result<()> {
    let path = initialized_marker_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, b"1")?;
    Ok(())
}

/// Whether the first-run autostart marker already exists.
pub fn initialized_marker_exists() -> bool {
    initialized_marker_path().is_file()
}

/// Whether the first-run default should turn auto-start on.
///
/// Once a marker exists (installer or a previous GUI decision), a missing Run
/// key means the user opted out — do not enable again.
pub fn should_enable_first_run_default(currently_enabled: bool, initialized: bool) -> bool {
    !currently_enabled && !initialized
}

/// Drop leftover Run keys / Startup shortcuts so Windows starts the app once.
pub fn remove_duplicate_entries() {
    platform::remove_duplicate_entries();
}

/// Enable or disable start-with-Windows.
pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
    remove_duplicate_entries();
    let launch = builder_for_current_exe()?;
    if enabled {
        launch
            .enable()
            .map_err(|err| anyhow::anyhow!("failed to enable auto-start: {err}"))?;
    } else if launch.is_enabled().unwrap_or(false) {
        launch
            .disable()
            .map_err(|err| anyhow::anyhow!("failed to disable auto-start: {err}"))?;
    }
    let _ = write_initialized_marker();
    Ok(())
}

/// Whether auto-start is currently enabled.
pub fn is_enabled() -> bool {
    builder_for_current_exe()
        .ok()
        .and_then(|launch| launch.is_enabled().ok())
        .unwrap_or(false)
}

/// Enable auto-start on first GUI launch when not configured yet.
pub fn ensure_default_enabled() -> anyhow::Result<bool> {
    remove_duplicate_entries();
    let launch = builder_for_current_exe()?;
    let enabled = launch.is_enabled().unwrap_or(false);
    if enabled {
        let _ = write_initialized_marker();
        return Ok(true);
    }
    if !should_enable_first_run_default(false, initialized_marker_exists()) {
        return Ok(false);
    }
    launch
        .enable()
        .map_err(|err| anyhow::anyhow!("failed to enable default auto-start: {err}"))?;
    let _ = write_initialized_marker();
    Ok(true)
}

#[cfg(windows)]
mod platform {
    use super::{APP_NAME, LEFTOVER_RUN_NAMES};
    use std::env;
    use std::fs;
    use std::path::PathBuf;
    use std::process::{Command, Stdio};

    pub fn remove_duplicate_entries() {
        remove_startup_shortcuts();
        for name in LEFTOVER_RUN_NAMES {
            delete_run_value("HKCU", name);
            delete_run_value("HKLM", name);
        }
        // Never keep a machine-wide copy: Inno + GUI both belong in HKCU.
        delete_run_value("HKLM", APP_NAME);
    }

    fn delete_run_value(hive: &str, name: &str) {
        let key = format!(r"{hive}\Software\Microsoft\Windows\CurrentVersion\Run");
        let _ = Command::new("reg.exe")
            .args(["delete", &key, "/v", name, "/f"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }

    fn remove_startup_shortcuts() {
        let Some(dir) = startup_dir() else {
            return;
        };
        for name in [
            "Game Optimizer.lnk",
            "GameOptimizer.lnk",
            "game_optimizer.lnk",
        ] {
            let path = dir.join(name);
            let _ = fs::remove_file(path);
        }
    }

    fn startup_dir() -> Option<PathBuf> {
        env::var_os("APPDATA").map(|appdata| {
            PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs\Startup")
        })
    }
}

#[cfg(not(windows))]
mod platform {
    pub fn remove_duplicate_entries() {}
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalize_keeps_windows_separators() {
        let path = PathBuf::from(r"C:\Apps\game_optimizer.exe");
        assert!(normalize_exe_path(path).contains('\\'));
    }

    #[test]
    fn first_run_enables_when_never_configured() {
        assert!(should_enable_first_run_default(false, false));
    }

    #[test]
    fn opt_out_is_not_overwritten() {
        assert!(!should_enable_first_run_default(false, true));
    }

    #[test]
    fn already_enabled_does_not_need_first_run_enable() {
        assert!(!should_enable_first_run_default(true, false));
        assert!(!should_enable_first_run_default(true, true));
    }

    #[test]
    fn leftover_run_names_cover_old_aliases() {
        assert!(LEFTOVER_RUN_NAMES.contains(&"Game Optimizer"));
        assert!(LEFTOVER_RUN_NAMES.contains(&"game_optimizer"));
        assert!(!LEFTOVER_RUN_NAMES.contains(&"GameOptimizer"));
    }

    #[test]
    fn data_dir_is_named_game_optimizer_when_localappdata_is_set() {
        if env::var_os("LOCALAPPDATA").is_some() {
            assert_eq!(
                data_dir().file_name().and_then(|n| n.to_str()),
                Some("GameOptimizer")
            );
        }
    }
}
