//! Native Windows window visibility for the Gaming Mode launcher window.
//!
//! # Why this module exists
//!
//! `WebviewWindow::hide()` is a *request*, not a fact:
//!
//! * `tauri-runtime-wry` turns it into `Message::Window(id, WindowMessage::Hide)`
//!   and hands it to the event loop; the call returns `Ok(())` as soon as the
//!   message is queued (`tauri-runtime-wry-2.11.2/src/lib.rs:2263`).
//! * The queued message is handled by tao's `WindowState::apply_diff`, which
//!   **returns early when the requested flag set equals the stored one**
//!   (`tao-0.35.3/src/platform_impl/windows/window_state.rs:319-323`). In that
//!   case no `ShowWindow` call is made at all: the OS window keeps whatever
//!   visibility it had, while the Tauri call still reports success.
//!
//! So a successful `hide()` proves only that a message was queued. SILO
//! therefore never knew whether the launcher was actually hidden — the previous
//! implementation logged "hid the launcher window" while the window stayed on
//! screen.
//!
//! This module resolves the real top-level HWND of the `main` window, drives it
//! with Win32 `ShowWindow`, and verifies the result with `IsWindowVisible`, so
//! "hidden" is an observed state and never an assumption.
//!
//! The Tauri API is still called, but only to keep tao's internal flag
//! bookkeeping in sync with the OS. The authoritative change is the Win32 one.

use std::time::Duration;

#[cfg(target_os = "windows")]
use tauri::Manager;

/// How long a caller waits for the UI thread to run a visibility change.
const UI_HANDOFF_TIMEOUT: Duration = Duration::from_secs(5);

/// What the OS reports about a window's visibility.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisibilityState {
    Visible,
    Hidden,
}

/// A snapshot of one window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WindowProbe {
    /// The native handle, as an integer so it survives serialization.
    pub hwnd: isize,
    /// True when the handle is a top-level window rather than a child.
    pub is_root: bool,
    pub title: String,
    pub visible: bool,
}

/// The result of a hide/restore request. Every success variant is backed by an
/// `IsWindowVisible` observation; there is no "probably worked" variant.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VisibilityOutcome {
    /// The window was verified hidden at the OS level.
    Hidden { hwnd: isize },
    /// The window was verified visible at the OS level.
    Restored { hwnd: isize },
    /// The request failed, or the OS state did not match the request.
    Failed { hwnd: Option<isize>, reason: String },
}

impl VisibilityOutcome {
    pub fn is_hidden(&self) -> bool {
        matches!(self, VisibilityOutcome::Hidden { .. })
    }

    pub fn is_restored(&self) -> bool {
        matches!(self, VisibilityOutcome::Restored { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            VisibilityOutcome::Hidden { hwnd } => format!("hidden (hwnd {hwnd})"),
            VisibilityOutcome::Restored { hwnd } => format!("visible (hwnd {hwnd})"),
            VisibilityOutcome::Failed { hwnd, reason } => match hwnd {
                Some(hwnd) => format!("failed (hwnd {hwnd}): {reason}"),
                None => format!("failed: {reason}"),
            },
        }
    }
}

#[cfg(windows)]
mod platform {
    use super::{VisibilityState, WindowProbe};
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetAncestor, GetWindowTextW, IsIconic, IsWindow, IsWindowVisible, SetForegroundWindow, ShowWindow,
        GA_ROOT, SW_HIDE, SW_RESTORE, SW_SHOW,
    };

    /// True while the OS still holds this handle as a live window.
    ///
    /// This is the distinction between a hidden and a destroyed window: hiding
    /// leaves the handle valid, destroying it does not. It is how SILO proves a
    /// Stage 6 release really happened instead of assuming a queued request.
    pub fn exists(hwnd: HWND) -> bool {
        // SAFETY: `IsWindow` only reads the handle table; an invalid handle
        // returns FALSE instead of faulting.
        unsafe { IsWindow(hwnd) }.as_bool()
    }

    pub fn state(hwnd: HWND) -> VisibilityState {
        // SAFETY: `IsWindowVisible` only reads the window's visibility state.
        if unsafe { IsWindowVisible(hwnd) }.as_bool() {
            VisibilityState::Visible
        } else {
            VisibilityState::Hidden
        }
    }

    pub fn is_root(hwnd: HWND) -> bool {
        // SAFETY: `GetAncestor` walks the parent chain of a valid window handle.
        unsafe { GetAncestor(hwnd, GA_ROOT) == hwnd }
    }

    pub fn title(hwnd: HWND) -> String {
        let mut buffer = vec![0u16; 512];
        // SAFETY: the buffer is writable and long enough for a window title.
        let length = unsafe { GetWindowTextW(hwnd, &mut buffer) };
        if length <= 0 {
            return String::new();
        }
        String::from_utf16_lossy(&buffer[..length as usize])
    }

    pub fn hide(hwnd: HWND) {
        // SAFETY: the handle is valid; `ShowWindow` only changes visibility.
        unsafe {
            let _ = ShowWindow(hwnd, SW_HIDE);
        }
    }

    pub fn restore(hwnd: HWND) {
        // SAFETY: the handle is valid; `ShowWindow` only changes visibility.
        unsafe {
            let _ = if IsIconic(hwnd).as_bool() {
                ShowWindow(hwnd, SW_RESTORE)
            } else {
                ShowWindow(hwnd, SW_SHOW)
            };
        }
    }

    pub fn focus(hwnd: HWND) {
        // SAFETY: the handle is valid; focus changes are best effort.
        unsafe {
            let _ = SetForegroundWindow(hwnd);
        }
    }

    pub fn top_level_windows() -> Vec<WindowProbe> {
        unsafe extern "system" fn callback(hwnd: HWND, lparam: LPARAM) -> BOOL {
            let probes = &mut *(lparam.0 as *mut Vec<WindowProbe>);
            probes.push(WindowProbe {
                hwnd: hwnd.0 as isize,
                is_root: GetAncestor(hwnd, GA_ROOT) == hwnd,
                title: title(hwnd),
                visible: IsWindowVisible(hwnd).as_bool(),
            });
            BOOL(1)
        }
        let mut found: Vec<WindowProbe> = Vec::new();
        // SAFETY: the callback only appends to the vector behind `lparam`, which
        // outlives the synchronous enumeration.
        unsafe {
            let _ = EnumWindows(Some(callback), LPARAM(&mut found as *mut Vec<WindowProbe> as isize));
        }
        found
    }
}

/// The native handle of a Windows window.
#[cfg(target_os = "windows")]
pub type NativeHandle = windows::Win32::Foundation::HWND;

/// Tauri and SILO depend on different `windows` crate releases, so their `HWND`
/// types are distinct even though both wrap the same raw pointer. This is the
/// explicit conversion between them.
#[cfg(target_os = "windows")]
fn to_local_handle(raw: *mut core::ffi::c_void) -> NativeHandle {
    windows::Win32::Foundation::HWND(raw)
}

/// Rebuilds a handle from its integer value.
///
/// A raw pointer is not `Send`, so the UI-thread closures carry the handle as an
/// integer and rebuild it on the other side.
#[cfg(target_os = "windows")]
fn from_raw_value(value: isize) -> NativeHandle {
    windows::Win32::Foundation::HWND(value as *mut core::ffi::c_void)
}

/// The public form of `from_raw_value`, for callers outside this module.
#[cfg(target_os = "windows")]
pub fn from_handle_value(value: isize) -> NativeHandle {
    from_raw_value(value)
}

/// True while the OS still holds this window handle as a live window.
///
/// A hidden window still exists; a destroyed one does not. This is the
/// observation that separates a Stage 6 release from a hide.
#[cfg(target_os = "windows")]
pub fn window_exists(hwnd: NativeHandle) -> bool {
    platform::exists(hwnd)
}

#[cfg(not(target_os = "windows"))]
pub fn window_exists(_hwnd: ()) -> bool {
    false
}

/// Reads the OS visibility of a window.
#[cfg(target_os = "windows")]
pub fn native_state(hwnd: NativeHandle) -> VisibilityState {
    platform::state(hwnd)
}

/// True when the handle is a top-level window rather than a child window.
#[cfg(target_os = "windows")]
pub fn is_root_window(hwnd: NativeHandle) -> bool {
    platform::is_root(hwnd)
}

/// The window title of a handle.
#[cfg(target_os = "windows")]
pub fn native_title(hwnd: NativeHandle) -> String {
    platform::title(hwnd)
}

/// Hides a window through Win32 and reports the verified state.
#[cfg(target_os = "windows")]
pub fn hide_native(hwnd: NativeHandle) -> VisibilityState {
    platform::hide(hwnd);
    platform::state(hwnd)
}

/// Restores a window through Win32 and reports the verified state.
///
/// A minimized window is restored to its previous size; every other window is
/// shown in the state it already had, so a maximized or fullscreen window keeps
/// that state instead of being silently un-maximized.
#[cfg(target_os = "windows")]
pub fn restore_native(hwnd: NativeHandle) -> VisibilityState {
    platform::restore(hwnd);
    platform::state(hwnd)
}

/// Enumerates the top-level windows of the current desktop.
#[cfg(target_os = "windows")]
pub fn top_level_windows() -> Vec<WindowProbe> {
    platform::top_level_windows()
}

/// Non-Windows builds have no HWND; every operation reports a failure.
#[cfg(not(target_os = "windows"))]
pub fn native_state(_hwnd: ()) -> VisibilityState {
    VisibilityState::Hidden
}

#[cfg(not(target_os = "windows"))]
pub fn hide_launcher_window(_app: &tauri::AppHandle) -> VisibilityOutcome {
    VisibilityOutcome::Failed {
        hwnd: None,
        reason: "native window visibility is only implemented on Windows".into(),
    }
}

#[cfg(not(target_os = "windows"))]
pub fn restore_launcher_window(_app: &tauri::AppHandle) -> VisibilityOutcome {
    VisibilityOutcome::Failed {
        hwnd: None,
        reason: "native window visibility is only implemented on Windows".into(),
    }
}

/// Runs `action` on the Tauri UI thread and waits for it to finish.
///
/// The Tauri event queue is FIFO, so a barrier issued after `window.hide()`
/// guarantees that the queued hide has already been processed when the barrier
/// returns. The wait is bounded so a stuck UI thread cannot hang a worker.
#[cfg(target_os = "windows")]
fn run_on_ui_thread(app: &tauri::AppHandle, action: impl FnOnce() + Send + 'static) -> Result<(), String> {
    let (tx, rx) = std::sync::mpsc::channel();
    app.run_on_main_thread(move || {
        action();
        let _ = tx.send(());
    })
    .map_err(|error| format!("could not reach the UI thread: {error}"))?;
    rx.recv_timeout(UI_HANDOFF_TIMEOUT)
        .map_err(|_| "timed out waiting for the UI thread".to_string())
}

/// Flushes the event queue and reads the window's OS visibility.
#[cfg(target_os = "windows")]
fn observe_after_flush(app: &tauri::AppHandle, hwnd: NativeHandle) -> VisibilityState {
    // A no-op task on the UI thread is a barrier: everything queued before it
    // has been handled by the time it returns.
    let _ = run_on_ui_thread(app, || {});
    platform::state(hwnd)
}

/// The native handle of the `main` window, resolved from the Tauri registry.
#[cfg(target_os = "windows")]
pub fn main_window_handle(app: &tauri::AppHandle) -> Result<NativeHandle, String> {
    let window = app
        .get_webview_window("main")
        .ok_or_else(|| "no main window".to_string())?;
    window
        .hwnd()
        .map(|hwnd| to_local_handle(hwnd.0))
        .map_err(|error| format!("the main window has no native handle: {error}"))
}

/// Describes the `main` window: its handle, whether it is a top-level window,
/// its title and its current OS visibility.
#[cfg(target_os = "windows")]
pub fn probe_main_window(app: &tauri::AppHandle) -> Result<WindowProbe, String> {
    let hwnd = main_window_handle(app)?;
    Ok(WindowProbe {
        hwnd: hwnd.0 as isize,
        is_root: platform::is_root(hwnd),
        title: platform::title(hwnd),
        visible: platform::state(hwnd) == VisibilityState::Visible,
    })
}

/// Hides the launcher window and verifies that it is really hidden.
///
/// The order matters: the Win32 `ShowWindow` is the authoritative change, the
/// Tauri call afterwards only re-synchronizes tao's flags. The final state is
/// always read back with `IsWindowVisible`; a window that is still visible is
/// reported as a failure rather than as a success.
#[cfg(target_os = "windows")]
pub fn hide_launcher_window(app: &tauri::AppHandle) -> VisibilityOutcome {
    let Some(window) = app.get_webview_window("main") else {
        return VisibilityOutcome::Failed {
            hwnd: None,
            reason: "no main window".into(),
        };
    };
    let hwnd = match window.hwnd() {
        Ok(handle) => to_local_handle(handle.0),
        Err(error) => {
            return VisibilityOutcome::Failed {
                hwnd: None,
                reason: format!("the main window has no native handle: {error}"),
            }
        }
    };
    let handle = hwnd.0 as isize;

    // 1. The authoritative OS-level change.
    if let Err(error) = run_on_ui_thread(app, move || platform::hide(from_raw_value(handle))) {
        return VisibilityOutcome::Failed {
            hwnd: Some(handle),
            reason: error,
        };
    }

    // 2. Re-synchronize tao's flags with the OS state.
    let tao_result = window.hide();

    // 3. Observe the result after the queued message has been handled.
    match observe_after_flush(app, hwnd) {
        VisibilityState::Hidden => VisibilityOutcome::Hidden { hwnd: handle },
        VisibilityState::Visible => {
            // tao's request did not take effect. Force the Win32 change once
            // more and check again, so the outcome is still an observation.
            let _ = run_on_ui_thread(app, move || platform::hide(from_raw_value(handle)));
            match observe_after_flush(app, hwnd) {
                VisibilityState::Hidden => VisibilityOutcome::Hidden { hwnd: handle },
                VisibilityState::Visible => VisibilityOutcome::Failed {
                    hwnd: Some(handle),
                    reason: format!(
                        "the window is still visible after ShowWindow(SW_HIDE) (tao hide returned {tao_result:?})"
                    ),
                },
            }
        }
    }
}

/// Restores the launcher window and verifies that it is really visible.
///
/// A minimized window is restored to its previous size; every other window is
/// shown in the state it already had, so a maximized or fullscreen window keeps
/// that state.
#[cfg(target_os = "windows")]
pub fn restore_launcher_window(app: &tauri::AppHandle) -> VisibilityOutcome {
    let Some(window) = app.get_webview_window("main") else {
        return VisibilityOutcome::Failed {
            hwnd: None,
            reason: "no main window".into(),
        };
    };
    let hwnd = match window.hwnd() {
        Ok(handle) => to_local_handle(handle.0),
        Err(error) => {
            return VisibilityOutcome::Failed {
                hwnd: None,
                reason: format!("the main window has no native handle: {error}"),
            }
        }
    };
    let handle = hwnd.0 as isize;

    if let Err(error) = run_on_ui_thread(app, move || {
        let hwnd = from_raw_value(handle);
        platform::restore(hwnd);
        platform::focus(hwnd);
    }) {
        return VisibilityOutcome::Failed {
            hwnd: Some(handle),
            reason: error,
        };
    }

    // Re-synchronize tao's flags with the OS state.
    let tao_result = window.show();

    match observe_after_flush(app, hwnd) {
        VisibilityState::Visible => VisibilityOutcome::Restored { hwnd: handle },
        VisibilityState::Hidden => {
            let _ = run_on_ui_thread(app, move || {
                let hwnd = from_raw_value(handle);
                platform::restore(hwnd);
                platform::focus(hwnd);
            });
            match observe_after_flush(app, hwnd) {
                VisibilityState::Visible => VisibilityOutcome::Restored { hwnd: handle },
                VisibilityState::Hidden => VisibilityOutcome::Failed {
                    hwnd: Some(handle),
                    reason: format!(
                        "the window is still hidden after ShowWindow(SW_SHOW) (tao show returned {tao_result:?})"
                    ),
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn outcomes_describe_themselves() {
        assert!(VisibilityOutcome::Hidden { hwnd: 1 }.is_hidden());
        assert!(VisibilityOutcome::Restored { hwnd: 1 }.is_restored());
        assert!(!VisibilityOutcome::Failed {
            hwnd: None,
            reason: "x".into()
        }
        .is_hidden());
        assert!(VisibilityOutcome::Failed {
            hwnd: Some(7),
            reason: "x".into()
        }
        .describe()
        .contains('7'));
    }
}
