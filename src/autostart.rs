//! Windows / cross-platform auto-start helpers.
//!
//! On Windows, startup remains per-user (HKCU Run) and launches the companion
//! GUI-subsystem `asInvoker` helper (`game_optimizer_launcher.exe`), which
//! requests the normal Windows `RunAs` consent prompt via `ShellExecuteW`
//! without ever spawning a console, `cmd.exe`, `powershell.exe`, or Windows
//! Terminal window. Declining UAC means the application does not start; this
//! does not create an elevated scheduled task, service, or bypass.

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

/// Build an auto-launch handle for the current user.
pub fn builder_for_current_exe() -> anyhow::Result<AutoLaunch> {
    let exe = env::current_exe()?;
    let mut builder = AutoLaunchBuilder::new();
    #[cfg(windows)]
    let path = platform::normalize_path(&platform::launcher_path(&exe));
    #[cfg(target_os = "linux")]
    let path = desktop_exec_path(&exe.to_string_lossy());
    #[cfg(not(any(windows, target_os = "linux")))]
    let path = exe.to_string_lossy();
    builder.set_app_name(APP_NAME).set_app_path(&path);
    #[cfg(target_os = "linux")]
    builder.set_args(&["--gui", "--start-in-tray"]);
    #[cfg(not(any(windows, target_os = "linux")))]
    builder.set_args(&["--gui"]);
    #[cfg(windows)]
    {
        builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    }
    builder
        .build()
        .map_err(|err| anyhow::anyhow!("auto-launch setup failed: {err}"))
}

#[cfg(target_os = "linux")]
fn desktop_exec_path(path: &str) -> String {
    let escaped = path
        .replace('\\', r"\\\\")
        .replace('"', r#"\\""#)
        .replace('$', r"\\$")
        .replace('\x60', "\\\\\x60")
        .replace('%', "%%")
        .replace('\n', r"\n")
        .replace('\r', r"\r");
    format!("\"{escaped}\"")
}

/// Per-user configuration directory.
pub fn data_dir() -> PathBuf {
    #[cfg(target_os = "linux")]
    {
        if let Some(config) = env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            return config.join("game-optimizer");
        }
        if let Some(home) = env::var_os("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
        {
            return home.join(".config/game-optimizer");
        }
    }
    #[cfg(windows)]
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

/// Drop matching legacy per-user Run values without removing the main entry.
pub fn remove_duplicate_entries() {
    platform::remove_duplicate_entries();
}

/// Enable or disable launch at sign-in.
pub fn set_enabled(enabled: bool) -> anyhow::Result<()> {
    #[cfg(windows)]
    {
        let exe = env::current_exe()?;
        platform::set_enabled(&exe, enabled)?;
        let _ = write_initialized_marker();
        Ok(())
    }
    #[cfg(not(windows))]
    {
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
}

/// Whether auto-start is currently enabled.
pub fn is_enabled() -> bool {
    #[cfg(windows)]
    {
        match env::current_exe() {
            Ok(exe) => platform::is_enabled(&exe).unwrap_or(false),
            Err(_) => false,
        }
    }
    #[cfg(not(windows))]
    {
        builder_for_current_exe()
            .ok()
            .and_then(|launch| launch.is_enabled().ok())
            .unwrap_or(false)
    }
}

/// Enable auto-start on first GUI launch when not configured yet.
pub fn ensure_default_enabled() -> anyhow::Result<bool> {
    #[cfg(windows)]
    {
        let exe = env::current_exe()?;
        let enabled = platform::is_enabled(&exe)?;
        if enabled {
            // Rewrite recognized legacy raw-executable entries to the UAC
            // launcher without changing the user's enabled preference.
            platform::set_enabled(&exe, true)?;
            let _ = write_initialized_marker();
            return Ok(true);
        }
        if !should_enable_first_run_default(false, initialized_marker_exists()) {
            return Ok(false);
        }
        platform::set_enabled(&exe, true)?;
        let _ = write_initialized_marker();
        Ok(true)
    }
    #[cfg(not(windows))]
    {
        let launch = builder_for_current_exe()?;
        let enabled = launch.is_enabled().unwrap_or(false);
        if enabled {
            #[cfg(target_os = "linux")]
            if installed_autostart_needs_update() {
                launch
                    .enable()
                    .map_err(|err| anyhow::anyhow!("failed to update auto-start: {err}"))?;
            }
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
}

#[cfg(target_os = "linux")]
fn installed_autostart_needs_update() -> bool {
    use crate::linux_install::APPLICATION;
    if !env::current_exe().is_ok_and(|exe| exe == std::path::Path::new(APPLICATION)) {
        return false;
    }
    let profile = data_dir();
    let Some(config_home) = profile.parent() else {
        return false;
    };
    let entry = config_home.join("autostart/GameOptimizer.desktop");
    let Ok(contents) = fs::read_to_string(entry) else {
        return false;
    };
    let desired = format!(
        "Exec={} --gui --start-in-tray",
        desktop_exec_path(APPLICATION)
    );
    contents
        .lines()
        .any(|line| line == "Comment=GameOptimizer startup script")
        && !contents
            .lines()
            .any(|line| line == "Hidden=true" || line == desired)
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod platform {
    use super::{APP_NAME, LEFTOVER_RUN_NAMES};
    use std::env;
    use std::path::PathBuf;
    use windows::Win32::System::Registry::{
        RegDeleteKeyValueW, RegGetValueW, RegSetKeyValueW, HKEY_CURRENT_USER, REG_SZ, RRF_RT_REG_SZ,
    };

    const RUN_SUBKEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
    const LAUNCHER_EXE_NAME: &str = "game_optimizer_launcher.exe";

    pub fn launcher_path(exe: &std::path::Path) -> PathBuf {
        exe.with_file_name(LAUNCHER_EXE_NAME)
    }

    pub fn startup_command(exe: &std::path::Path) -> String {
        format!("\"{}\"", normalize_path(&launcher_path(exe)))
    }

    fn validate_startup_command(command: &str) -> anyhow::Result<()> {
        // Windows Run entries are limited to 260 UTF-16 code units.
        if command.encode_utf16().count() > 260 {
            anyhow::bail!("The installation path is too long for Windows auto-start; choose a shorter installation folder.");
        }
        Ok(())
    }

    pub fn is_enabled(exe: &std::path::Path) -> anyhow::Result<bool> {
        for name in std::iter::once(APP_NAME).chain(LEFTOVER_RUN_NAMES.iter().copied()) {
            if let Some(data) = get_value(name)? {
                if owns_value(name, &data, exe) {
                    return Ok(true);
                }
            }
        }
        Ok(false)
    }

    pub fn set_enabled(exe: &std::path::Path, enabled: bool) -> anyhow::Result<()> {
        if enabled {
            let launcher = launcher_path(exe);
            if !launcher.is_file() {
                anyhow::bail!(
                    "O iniciador sem console ({}) não foi encontrado ao lado de {}. Reinstale o pacote completo ou compile todos os binários.",
                    launcher.display(),
                    exe.display()
                );
            }
            let command = startup_command(exe);
            validate_startup_command(&command)?;
            let name = wide(APP_NAME);
            let subkey = wide(RUN_SUBKEY);
            let data = wide(&command);
            // SAFETY: predefined HKCU handle; all strings are NUL-terminated
            // UTF-16 buffers kept alive through this synchronous call.
            let result = unsafe {
                RegSetKeyValueW(
                    HKEY_CURRENT_USER,
                    windows::core::PCWSTR(subkey.as_ptr()),
                    windows::core::PCWSTR(name.as_ptr()),
                    REG_SZ.0,
                    Some(data.as_ptr().cast()),
                    (data.len() * 2) as u32,
                )
            };
            check_result(result)?;
            // Preserve the working principal if writing it failed; only clean aliases afterward.
            remove_owned_aliases(exe)?;
        } else {
            for name in std::iter::once(APP_NAME).chain(LEFTOVER_RUN_NAMES.iter().copied()) {
                if get_value(name)?
                    .as_deref()
                    .is_some_and(|data| owns_value(name, data, exe))
                {
                    delete_value(name)?;
                }
            }
        }
        Ok(())
    }

    fn remove_owned_aliases(exe: &std::path::Path) -> anyhow::Result<()> {
        for alias in LEFTOVER_RUN_NAMES {
            if get_value(alias)?
                .as_deref()
                .is_some_and(|data| owns_value(alias, data, exe))
            {
                delete_value(alias)?;
            }
        }
        Ok(())
    }

    fn get_value(name: &str) -> anyhow::Result<Option<String>> {
        let subkey = wide(RUN_SUBKEY);
        let name_wide = wide(name);
        let mut size = 0u32;
        // SAFETY: this size probe has no output buffer; both key strings live
        // through the synchronous call.
        let first = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(subkey.as_ptr()),
                windows::core::PCWSTR(name_wide.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                None,
                Some(&mut size),
            )
        };
        if matches!(first.0, 2 | 3) {
            return Ok(None);
        }
        if first.0 != 0 && first.0 != 234 {
            return Err(registry_error(first.0));
        }
        if size > 128 * 1024 {
            anyhow::bail!("auto-start registry value exceeds size limit");
        }
        let mut data = vec![0u16; (size as usize).div_ceil(2)];
        // SAFETY: the output buffer has at least `size` writable bytes. A
        // concurrent value change returns ERROR_MORE_DATA instead of overrunning it.
        let result = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(subkey.as_ptr()),
                windows::core::PCWSTR(name_wide.as_ptr()),
                RRF_RT_REG_SZ,
                None,
                Some(data.as_mut_ptr().cast()),
                Some(&mut size),
            )
        };
        if result.0 != 0 {
            return Err(registry_error(result.0));
        }
        data.truncate(size as usize / 2);
        if data.last() == Some(&0) {
            data.pop();
        }
        String::from_utf16(&data)
            .map(Some)
            .map_err(|e| anyhow::anyhow!("invalid UTF-16 auto-start registry value: {e}"))
    }

    fn delete_value(name: &str) -> anyhow::Result<()> {
        let subkey = wide(RUN_SUBKEY);
        let name = wide(name);
        // SAFETY: predefined HKCU handle and live NUL-terminated UTF-16 strings.
        let result = unsafe {
            RegDeleteKeyValueW(
                HKEY_CURRENT_USER,
                windows::core::PCWSTR(subkey.as_ptr()),
                windows::core::PCWSTR(name.as_ptr()),
            )
        };
        if matches!(result.0, 2 | 3) {
            Ok(())
        } else {
            check_result(result)
        }
    }

    fn check_result(result: windows::Win32::Foundation::WIN32_ERROR) -> anyhow::Result<()> {
        if result.0 == 0 {
            Ok(())
        } else {
            Err(registry_error(result.0))
        }
    }

    fn registry_error(code: u32) -> anyhow::Error {
        anyhow::anyhow!(
            "Windows registry operation failed: {}",
            std::io::Error::from_raw_os_error(code as i32)
        )
    }
    fn wide(s: &str) -> Vec<u16> {
        s.encode_utf16().chain(std::iter::once(0)).collect()
    }
    pub fn normalize_path(path: &std::path::Path) -> String {
        let mut value = path.to_string_lossy().replace('/', "\\");
        if let Some(rest) = value.strip_prefix(r"\??\") {
            value = rest.to_owned();
        }
        if let Some(rest) = value.strip_prefix(r"\\?\UNC\") {
            value = format!(r"\\{rest}");
        } else if let Some(rest) = value.strip_prefix(r"\\?\") {
            value = rest.to_owned();
        }
        value
    }
    pub(super) fn owns_value(name: &str, data: &str, exe: &std::path::Path) -> bool {
        let principal = name.eq_ignore_ascii_case(APP_NAME);
        if !principal
            && !LEFTOVER_RUN_NAMES
                .iter()
                .any(|a| name.eq_ignore_ascii_case(a))
        {
            return false;
        }
        is_launcher_command(data, exe)
            || is_legacy_powershell_command(data, exe)
            || is_legacy_exe(data, exe)
    }
    fn split_command_and_args(data: &str) -> (&str, &str) {
        let data = data.trim();
        if let Some((p, rest)) = data.strip_prefix('"').and_then(|s| s.split_once('"')) {
            (p, rest.trim())
        } else {
            data.split_once(char::is_whitespace)
                .map_or((data, ""), |(p, a)| (p, a.trim()))
        }
    }
    fn is_launcher_command(data: &str, exe: &std::path::Path) -> bool {
        let (path, args) = split_command_and_args(data);
        normalize_path(std::path::Path::new(path))
            .eq_ignore_ascii_case(&normalize_path(&launcher_path(exe)))
            && args.is_empty()
    }
    pub(super) fn is_legacy_powershell_command(data: &str, exe: &std::path::Path) -> bool {
        let lower = data.trim().to_ascii_lowercase();
        let needle = format!(
            "start-process -filepath '{}' -argumentlist '--gui' -verb runas",
            normalize_path(exe).replace('\'', "''").to_ascii_lowercase()
        );
        lower.contains("powershell.exe") && lower.contains(&needle)
    }
    pub(super) fn is_legacy_exe(data: &str, exe: &std::path::Path) -> bool {
        let (path, args) = split_command_and_args(data);
        normalize_path(std::path::Path::new(path)).eq_ignore_ascii_case(&normalize_path(exe))
            && (args.is_empty() || args == "--gui")
    }
    pub fn remove_duplicate_entries() {
        if let Ok(exe) = env::current_exe() {
            let _ = remove_owned_aliases(&exe);
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn default_installation_fits_windows_run_limit() {
            let exe = std::path::Path::new(
                r"C:\Users\Player\AppData\Local\GameOptimizer\game_optimizer.exe",
            );
            assert!(validate_startup_command(&startup_command(exe)).is_ok());
            assert!(validate_startup_command(&"x".repeat(261)).is_err());
        }
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
    #[cfg(windows)]
    fn startup_command_uses_gui_launcher_without_powershell_or_cmd() {
        let exe = PathBuf::from("C:\\Apps\\O'Brien 游戏\\game_optimizer.exe");
        let cmd = platform::startup_command(&exe);
        assert_eq!(
            cmd,
            "\"C:\\Apps\\O'Brien 游戏\\game_optimizer_launcher.exe\""
        );
        assert!(!cmd.to_ascii_lowercase().contains("powershell"));
        assert!(!cmd.to_ascii_lowercase().contains("cmd.exe"));
        assert!(platform::owns_value(APP_NAME, &cmd, &exe));
    }

    #[test]
    #[cfg(windows)]
    fn legacy_powershell_run_entry_is_owned_for_automatic_upgrade_or_removal() {
        let exe = PathBuf::from(r"C:\Users\cezar\AppData\Local\GameOptimizer\game_optimizer.exe");
        let legacy = r#""C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -WindowStyle Hidden -Command "Start-Process -FilePath 'C:\Users\cezar\AppData\Local\GameOptimizer\game_optimizer.exe' -ArgumentList '--gui' -Verb RunAs""#;
        assert!(platform::owns_value(APP_NAME, legacy, &exe));

        let other_exe = r#""C:\WINDOWS\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -NonInteractive -WindowStyle Hidden -Command "Start-Process -FilePath 'C:\Other\game_optimizer.exe' -ArgumentList '--gui' -Verb RunAs""#;
        assert!(!platform::owns_value(APP_NAME, other_exe, &exe));
    }

    #[test]
    #[cfg(windows)]
    fn legacy_value_ownership_requires_matching_executable_and_known_arguments() {
        let exe = PathBuf::from(r"C:\Apps\game_optimizer.exe");
        assert!(platform::is_legacy_exe(
            r#""C:\Apps\game_optimizer.exe" --gui"#,
            &exe
        ));
        assert!(platform::is_legacy_exe(
            r#""\\?\C:\Apps\game_optimizer.exe" --gui"#,
            &exe
        ));
        assert!(platform::is_legacy_exe(
            r#""C:\Apps\O'Brien 游戏\game_optimizer.exe""#,
            &PathBuf::from("C:\\Apps\\O'Brien 游戏\\game_optimizer.exe")
        ));
        assert!(!platform::is_legacy_exe(
            r#""C:\Other\game_optimizer.exe" --gui"#,
            &exe
        ));
        assert!(!platform::is_legacy_exe(
            r#""C:\Apps\game_optimizer.exe" --evil"#,
            &exe
        ));
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
