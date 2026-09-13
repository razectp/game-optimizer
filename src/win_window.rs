//! Native window restore after the GUI is hidden to the tray.
//!
//! Windows HWND calls live here; all `unsafe` is confined to this module.

#![allow(unsafe_code)]

/// Restore the main window, optionally back to a maximized state.
///
/// Returns whether a native HWND was found and shown. On non-Windows hosts
/// this is always `false` (eframe viewport commands still run).
pub fn restore_main_window(maximized: bool) -> bool {
    imp::restore_main_window(maximized)
}

/// Whether the main window is currently zoomed (maximized).
pub fn is_main_window_maximized() -> Option<bool> {
    imp::is_main_window_maximized()
}

#[cfg(windows)]
mod imp {
    use windows::core::w;
    use windows::Win32::Foundation::HWND;
    use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
    use windows::Win32::UI::WindowsAndMessaging::{
        BringWindowToTop, FindWindowW, GetForegroundWindow, GetWindowThreadProcessId, IsIconic,
        IsZoomed, SetForegroundWindow, ShowWindow, SW_RESTORE, SW_SHOW, SW_SHOWMAXIMIZED,
    };

    const TITLE: windows::core::PCWSTR = w!("Game Optimizer");

    fn find_hwnd() -> Option<HWND> {
        // SAFETY: title is a static UTF-16 string; null class matches any.
        // windows 0.62 maps an invalid HWND to Err.
        unsafe { FindWindowW(None, TITLE).ok() }
    }

    pub fn is_main_window_maximized() -> Option<bool> {
        let hwnd = find_hwnd()?;
        // SAFETY: hwnd returned by FindWindowW for our titled window.
        Some(unsafe { IsZoomed(hwnd) }.as_bool())
    }

    pub fn restore_main_window(maximized: bool) -> bool {
        let Some(hwnd) = find_hwnd() else {
            return false;
        };

        // SAFETY: hwnd is a live window; ShowWindow/foreground calls are best-effort.
        unsafe {
            let iconic = IsIconic(hwnd).as_bool();
            let cmd = if maximized {
                SW_SHOWMAXIMIZED
            } else if iconic {
                SW_RESTORE
            } else {
                SW_SHOW
            };
            let _ = ShowWindow(hwnd, cmd);
            force_foreground(hwnd);
        }
        true
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

    pub fn is_main_window_maximized() -> Option<bool> {
        None
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn stub_or_native_restore_does_not_panic() {
        let _ = super::restore_main_window(false);
        let _ = super::is_main_window_maximized();
    }
}
