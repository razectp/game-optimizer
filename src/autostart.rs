//! Windows / cross-platform auto-start helpers.
//!
//! On Windows, startup remains per-user (HKCU Run) and invokes PowerShell only
//! to request the normal Windows `RunAs` consent prompt for the GUI. Declining
//! UAC means the application does not start; this does not create an elevated
//! scheduled task, service, or bypass.

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
    let mut builder = AutoLaunchBuilder::new();
    #[cfg(windows)]
    let path = platform::powershell_path()?;
    #[cfg(not(windows))]
    let path = normalize_exe_path(exe);
    builder.set_app_name(APP_NAME).set_app_path(&path);
    #[cfg(windows)]
    builder.set_args(&platform::builder_args(&exe));
    #[cfg(not(windows))]
    builder.set_args(&["--gui"]);
    #[cfg(windows)]
    {
        builder.set_windows_enable_mode(auto_launch::WindowsEnableMode::CurrentUser);
    }
    builder
        .build()
        .map_err(|err| anyhow::anyhow!("auto-launch setup failed: {err}"))
}

#[cfg(any(not(windows), test))]
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

/// Drop matching legacy per-user Run values without removing the main entry.
pub fn remove_duplicate_entries() {
    platform::remove_duplicate_entries();
}

/// Enable or disable start-with-Windows.
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

    pub fn powershell_path() -> anyhow::Result<String> {
        let root = env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
        Ok(PathBuf::from(root)
            .join(r"System32\WindowsPowerShell\v1.0\powershell.exe")
            .to_string_lossy()
            .into_owned())
    }

    pub fn builder_args(exe: &std::path::Path) -> Vec<String> {
        vec![
            "-NoProfile".into(),
            "-NonInteractive".into(),
            "-WindowStyle".into(),
            "Hidden".into(),
            "-Command".into(),
            format!("\"{}\"", powershell_script(exe)),
        ]
    }

    pub fn startup_command(exe: &std::path::Path) -> String {
        let args = builder_args(exe);
        format!(
            "\"{}\" {}",
            powershell_path().unwrap_or_else(|_| {
                r"C:\Windows\System32\WindowsPowerShell\v1.0\powershell.exe".into()
            }),
            args.join(" ")
        )
    }

    fn validate_startup_command(command: &str) -> anyhow::Result<()> {
        // Windows Run entries are limited to 260 UTF-16 code units. Inlining
        // the quoted script avoids the large UTF-16/Base64 expansion.
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
    fn powershell_script(exe: &std::path::Path) -> String {
        format!(
            "Start-Process -FilePath '{}' -ArgumentList '--gui' -Verb RunAs",
            normalize_path(exe).replace('\'', "''")
        )
    }
    fn normalize_path(path: &std::path::Path) -> String {
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
    fn owns_value(name: &str, data: &str, exe: &std::path::Path) -> bool {
        let principal = name.eq_ignore_ascii_case(APP_NAME);
        if !principal
            && !LEFTOVER_RUN_NAMES
                .iter()
                .any(|a| name.eq_ignore_ascii_case(a))
        {
            return false;
        }
        data == startup_command(exe) || is_legacy_exe(data, exe)
    }
    pub(super) fn is_legacy_exe(data: &str, exe: &std::path::Path) -> bool {
        let data = data.trim();
        let (path, args) =
            if let Some((p, rest)) = data.strip_prefix('"').and_then(|s| s.split_once('"')) {
                (p, rest.trim())
            } else {
                data.split_once(char::is_whitespace)
                    .map_or((data, ""), |(p, a)| (p, a.trim()))
            };
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
    #[cfg(windows)]
    fn powershell_script_quotes_apostrophes_and_preserves_unicode_path() {
        let exe = PathBuf::from("C:\\Apps\\O'Brien 游戏\\game_optimizer.exe");
        let arguments = platform::builder_args(&exe);
        let script = arguments.last().unwrap();
        assert!(script.starts_with('"') && script.ends_with('"'));
        assert!(arguments.contains(&"-Command".to_string()));
        assert!(script.contains("O''Brien 游戏"));
        assert!(script.contains("-Verb RunAs"));
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
