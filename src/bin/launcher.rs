//! Zero-console Windows auto-start launcher for Game Optimizer.
//!
//! Invoked from `HKCU\Software\Microsoft\Windows\CurrentVersion\Run` as a GUI
//! (`windows_subsystem = "windows"`) `asInvoker` executable so Windows never
//! opens a console / PowerShell / Windows Terminal window at sign-in, while
//! still requesting normal UAC elevation (`runas`) for `game_optimizer.exe`.

#![cfg_attr(windows, windows_subsystem = "windows")]

#[cfg(windows)]
fn main() {
    use std::path::PathBuf;
    use windows::core::{w, PCWSTR};
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

    let Ok(current_exe) = std::env::current_exe() else {
        return;
    };
    let Some(dir) = current_exe.parent() else {
        return;
    };
    let target: PathBuf = dir.join("game_optimizer.exe");
    if !target.is_file() {
        return;
    }

    let target_wide: Vec<u16> = target
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    let dir_wide: Vec<u16> = dir
        .as_os_str()
        .to_string_lossy()
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();

    // SAFETY: All pointers reference live NUL-terminated UTF-16 buffers or static
    // wide string literals for the duration of the synchronous ShellExecuteW call.
    unsafe {
        let _ = ShellExecuteW(
            None,
            w!("runas"),
            PCWSTR(target_wide.as_ptr()),
            w!("--gui"),
            PCWSTR(dir_wide.as_ptr()),
            SW_SHOWNORMAL,
        );
    }
}

#[cfg(not(windows))]
fn main() {}
