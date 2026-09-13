//! Native window restore after the GUI is hidden to the tray.
//!
//! Windows HWND calls live here; all `unsafe` is confined to this module.

#![allow(unsafe_code)]

/// Title used by both eframe and `FindWindowW` — must stay in sync.
pub const WINDOW_TITLE: &str = "Game Optimizer";

/// Win32 show command to apply when bringing the window back from the tray.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestoreShowCommand {
    /// Window was maximized before hide.
    ShowMaximized,
    /// Window is iconic / minimized; restore to last normal size.
    Restore,
    /// Window was a normal (not maximized) visible frame.
    ShowNormal,
}

/// Choose the restore show command from remembered maximize + iconic state.
///
/// `SW_SHOW` is intentionally not used: after `Visible(false)` / `SW_HIDE`
/// it can ignore the saved maximized bit. Explicit maximized vs normal is
/// what makes “maximize → tray → restore” work.
pub fn restore_show_command(maximized: bool, iconic: bool) -> RestoreShowCommand {
    if maximized {
        RestoreShowCommand::ShowMaximized
    } else if iconic {
        RestoreShowCommand::Restore
    } else {
        RestoreShowCommand::ShowNormal
    }
}

/// Prefer the GUI viewport’s maximized flag, then a native `IsZoomed` probe.
pub fn remembered_maximized(viewport: Option<bool>, native: Option<bool>) -> bool {
    viewport.or(native).unwrap_or(false)
}

/// Restore the main window, optionally back to a maximized state.
///
/// Returns whether a native HWND was found and shown. On non-Windows hosts
/// this is always `false` (eframe viewport commands still run).
pub fn restore_main_window(maximized: bool) -> bool {
    imp::restore_main_window(maximized)
}

/// Hide the main window at the Win32 layer (companion to eframe `Visible(false)`).
pub fn hide_main_window() -> bool {
    imp::hide_main_window()
}

/// Whether the main window is currently zoomed (maximized).
pub fn is_main_window_maximized() -> Option<bool> {
    imp::is_main_window_maximized()
}

#[cfg(windows)]
mod imp {
    use super::{restore_show_command, RestoreShowCommand, WINDOW_TITLE};

    use windows::core::HSTRING;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, FindWindowW, GetForegroundWindow, GetWindowPlacement,
        GetWindowThreadProcessId, IsIconic, IsZoomed, SetForegroundWindow, SetWindowPlacement,
        ShowWindow, SW_HIDE, SW_RESTORE, SW_SHOWMAXIMIZED, SW_SHOWNORMAL, WINDOWPLACEMENT,
    };

    fn find_hwnd() -> Option<HWND> {
        let title = HSTRING::from(WINDOW_TITLE);
        // SAFETY: HSTRING stays alive for the call; null class matches any.
        // windows 0.62 maps an invalid HWND to Err.
        unsafe { FindWindowW(None, windows::core::PCWSTR(title.as_ptr())).ok() }
    }

    pub fn is_main_window_maximized() -> Option<bool> {
        let hwnd = find_hwnd()?;
        // SAFETY: hwnd returned by FindWindowW for our titled window.
        Some(unsafe { IsZoomed(hwnd) }.as_bool())
    }

    pub fn hide_main_window() -> bool {
        let Some(hwnd) = find_hwnd() else {
            return false;
        };
        // SAFETY: hwnd is our window; hiding is best-effort.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
        true
    }

    pub fn restore_main_window(maximized: bool) -> bool {
        let Some(hwnd) = find_hwnd() else {
            return false;
        };

        // SAFETY: hwnd is a live window; placement/show calls are best-effort.
        unsafe {
            let iconic = IsIconic(hwnd).as_bool();
            let command = restore_show_command(maximized, iconic);
            if !apply_placement(hwnd, command) {
                let cmd = match command {
                    RestoreShowCommand::ShowMaximized => SW_SHOWMAXIMIZED,
                    RestoreShowCommand::Restore => SW_RESTORE,
                    RestoreShowCommand::ShowNormal => SW_SHOWNORMAL,
                };
                let _ = ShowWindow(hwnd, cmd);
            }
            force_foreground(hwnd);
        }
        true
    }

    unsafe fn apply_placement(hwnd: HWND, command: RestoreShowCommand) -> bool {
        let mut placement = WINDOWPLACEMENT {
            length: std::mem::size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if unsafe { GetWindowPlacement(hwnd, &mut placement) }.is_err() {
            return false;
        }
        placement.showCmd = match command {
            RestoreShowCommand::ShowMaximized => SW_SHOWMAXIMIZED.0 as u32,
            RestoreShowCommand::Restore => SW_RESTORE.0 as u32,
            RestoreShowCommand::ShowNormal => SW_SHOWNORMAL.0 as u32,
        };
        unsafe { SetWindowPlacement(hwnd, &placement) }.is_ok()
    }

    unsafe fn force_foreground(hwnd: HWND) {
        let foreground = unsafe { GetForegroundWindow() };
        let foreground_tid = unsafe { GetWindowThreadProcessId(foreground, None) };
        let our_tid = unsafe { GetCurrentThreadId() };
        if foreground_tid != 0 && foreground_tid != our_tid {
            let _ = unsafe { AttachThreadInput(foreground_tid, our_tid, true) };
        }
        let _ = unsafe { BringWindowToTop(hwnd) };
        let _ = unsafe { SetForegroundWindow(hwnd) };
        if foreground_tid != 0 && foreground_tid != our_tid {
            let _ = unsafe { AttachThreadInput(foreground_tid, our_tid, false) };
        }
    }
}

#[cfg(not(windows))]
mod imp {
    pub fn restore_main_window(_maximized: bool) -> bool {
        false
    }

    pub fn hide_main_window() -> bool {
        false
    }

    pub fn is_main_window_maximized() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn window_title_matches_product() {
        assert_eq!(WINDOW_TITLE, "Game Optimizer");
    }

    #[test]
    fn maximized_always_requests_show_maximized() {
        assert_eq!(
            restore_show_command(true, true),
            RestoreShowCommand::ShowMaximized
        );
        assert_eq!(
            restore_show_command(true, false),
            RestoreShowCommand::ShowMaximized
        );
    }

    #[test]
    fn non_maximized_iconic_uses_restore() {
        assert_eq!(
            restore_show_command(false, true),
            RestoreShowCommand::Restore
        );
    }

    #[test]
    fn non_maximized_visible_uses_shownormal_not_show() {
        assert_eq!(
            restore_show_command(false, false),
            RestoreShowCommand::ShowNormal
        );
    }

    #[test]
    fn remembered_maximized_prefers_viewport_then_native() {
        assert!(remembered_maximized(Some(true), Some(false)));
        assert!(remembered_maximized(None, Some(true)));
        assert!(!remembered_maximized(Some(false), Some(true)));
        assert!(!remembered_maximized(None, None));
    }

    #[test]
    fn stub_or_native_restore_does_not_panic() {
        let _ = restore_main_window(false);
        let _ = restore_main_window(true);
        let _ = hide_main_window();
        let _ = is_main_window_maximized();
    }
}
