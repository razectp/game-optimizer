//! Windows / cross-platform auto-start helpers.

use std::env;
use std::fs;
use std::path::PathBuf;

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

const APP_NAME: &str = "GameOptimizer";
const MARKER_NAME: &str = ".autostart-initialized";

/// Build an auto-launch handle for the current executable.
pub fn builder_for_current_exe() -> anyhow::Result<AutoLaunch> {
    let exe = env::current_exe()?;
    let path = normalize_exe_path(exe);
    AutoLaunchBuilder::new()
        .set_app_name(APP_NAME)
        .set_app_path(&path)
        .set_args(&["--gui"])
        .build()
        .map_err(|err| anyhow::anyhow!("auto-launch setup failed: {err}"))
}

fn normalize_exe_path(exe: PathBuf) -> String {
    exe.to_string_lossy().replace('/', "\\")
}

/// Path of the first-run marker (install dir or `%LOCALAPPDATA%\GameOptimizer`).
pub fn initialized_marker_path() -> PathBuf {
    if let Ok(local) = env::var("LOCALAPPDATA") {
        return PathBuf::from(local).join("GameOptimizer").join(MARKER_NAME);
    }
    env::current_exe()
        .ok()
        .and_then(|exe| exe.parent().map(|parent| parent.join(MARKER_NAME)))
        .unwrap_or_else(|| PathBuf::from(MARKER_NAME))
}

fn write_initialized_marker() -> anyhow::Result<()> {
    let path = initialized_marker_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&path, b"1")?;
    Ok(())
}

fn initialized_marker_exists() -> bool {
    initialized_marker_path().is_file()
}

/// Whether the first-run default should turn auto-start on.
///
/// Once a marker exists (installer or a previous GUI decision), a missing Run
/// key means the user opted out — do not enable again.
pub fn should_enable_first_run_default(currently_enabled: bool, initialized: bool) -> bool {
    !currently_enabled && !initialized
}

/// Enable or disable start-with-Windows.
pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
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
}
