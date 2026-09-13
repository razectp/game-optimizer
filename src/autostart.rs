//! Windows / cross-platform auto-start helpers.

use std::env;
use std::path::PathBuf;

use auto_launch::{AutoLaunch, AutoLaunchBuilder};

const APP_NAME: &str = "GameOptimizer";

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
    if launch.is_enabled().unwrap_or(false) {
        return Ok(true);
    }
    // First-run product default: start with Windows.
    launch
        .enable()
        .map_err(|err| anyhow::anyhow!("failed to enable default auto-start: {err}"))?;
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
}
