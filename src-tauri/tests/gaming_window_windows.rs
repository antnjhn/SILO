//! OS-level validation of the window primitive used by Gaming Mode and the tray.
//!
//! These tests create a real Win32 top-level window and drive the same
//! `ShowWindow` primitives that `gaming_window` uses in the application. They are
//! deliberately independent of Tauri so the hide/restore effect can be checked
//! without a live WebView.

#![cfg(windows)]

use app_lib::gaming_window::{hide_native, native_state, restore_native, window_exists, VisibilityState};
use app_lib::window_lifecycle::classify_registration;
use windows::Win32::Foundation::{HINSTANCE, HWND};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DestroyWindow, IsWindowVisible, IsZoomed, ShowWindow, SW_MAXIMIZE, SW_SHOW, WINDOW_EX_STYLE,
    WS_OVERLAPPEDWINDOW,
};

struct TestWindow(HWND);

impl TestWindow {
    fn create() -> Self {
        // SAFETY: creating a window with the built-in STATIC class is valid on
        // any thread; no message-only parent is needed for visibility checks.
        unsafe {
            let module = GetModuleHandleW(None).expect("the test module handle");
            let hwnd = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                windows::core::w!("STATIC"),
                windows::core::w!("SILO gaming window test"),
                WS_OVERLAPPEDWINDOW,
                0,
                0,
                320,
                240,
                None,
                None,
                HINSTANCE(module.0),
                None,
            )
            .expect("a real top-level test window");
            let _ = ShowWindow(hwnd, SW_SHOW);
            TestWindow(hwnd)
        }
    }

    fn hwnd(&self) -> HWND {
        self.0
    }

    fn is_visible(&self) -> bool {
        // SAFETY: the handle belongs to this test window.
        unsafe { IsWindowVisible(self.0).as_bool() }
    }

    fn is_maximized(&self) -> bool {
        // SAFETY: the handle belongs to this test window.
        unsafe { IsZoomed(self.0).as_bool() }
    }
}

impl Drop for TestWindow {
    fn drop(&mut self) {
        // SAFETY: the window was created by this test and is not used again.
        unsafe {
            let _ = DestroyWindow(self.0);
        }
    }
}

/// The observation that separates a Stage 6 release from a Stage 4/5 hide.
///
/// A hidden window is still a window: its handle stays valid, `IsWindowVisible`
/// says false and the WebView keeps running. A destroyed window does not exist
/// any more. This is the check `window_lifecycle::destroy_main_window` relies on
/// before it reports a release as real.
#[test]
fn a_hidden_window_still_exists_but_a_destroyed_one_does_not() {
    let window = TestWindow::create();
    let handle = window.hwnd();
    assert!(window_exists(handle), "a fresh window must exist");

    for cycle in 1..=3 {
        let after_hide = hide_native(handle);
        assert_eq!(after_hide, VisibilityState::Hidden, "hide cycle {cycle}");
        assert!(
            window_exists(handle),
            "hide cycle {cycle}: a hidden window is NOT a destroyed window"
        );
    }

    // SAFETY: the handle belongs to this test window.
    unsafe {
        let _ = DestroyWindow(handle);
    }
    assert!(
        !window_exists(handle),
        "only a real destruction makes the handle invalid"
    );
    assert_eq!(
        native_state(handle),
        VisibilityState::Hidden,
        "an invalid handle also reads as not visible, which is why existence is its own check"
    );
}

/// The production liveness rule, driven with real OS handles.
///
/// A registered-but-destroyed window must be classified as **stale**, because
/// that is what makes the rebuild path refuse to believe the launcher is back.
#[test]
fn a_real_live_window_is_live_and_a_destroyed_one_is_stale() {
    let window = TestWindow::create();
    let live = window.hwnd();
    assert!(window_exists(live), "a fresh test window must exist");

    // Live handle → Live, so an existing launcher is used rather than replaced.
    assert_eq!(
        classify_registration(Some(live.0 as isize), Some(true)),
        app_lib::window_lifecycle::RegistryLiveness::Live {
            hwnd: Some(live.0 as isize)
        }
    );

    // SAFETY: the handle belongs to this test window.
    unsafe {
        let _ = DestroyWindow(live);
    }
    let destroyed = live.0 as isize;
    assert!(!window_exists(live), "the destroyed handle must be gone");

    // Destroyed handle → Stale. This is Finding 2's regression case: accepting
    // it as live would strand the app with no usable window.
    assert_eq!(
        classify_registration(Some(destroyed), Some(false)),
        app_lib::window_lifecycle::RegistryLiveness::Stale { hwnd: Some(destroyed) }
    );
}

#[test]
fn repeated_hide_and_restore_reports_the_real_os_state_every_cycle() {
    let window = TestWindow::create();
    assert!(window.is_visible(), "the fresh test window must start visible");
    for cycle in 1..=3 {
        let after_hide = hide_native(window.hwnd());
        assert_eq!(after_hide, VisibilityState::Hidden, "hide cycle {cycle}");
        assert!(
            !window.is_visible(),
            "IsWindowVisible must be false after hide cycle {cycle}"
        );

        let after_restore = restore_native(window.hwnd());
        assert_eq!(after_restore, VisibilityState::Visible, "restore cycle {cycle}");
        assert!(
            window.is_visible(),
            "IsWindowVisible must be true after restore cycle {cycle}"
        );

        assert_eq!(native_state(window.hwnd()), VisibilityState::Visible);
    }
}

#[test]
fn a_maximized_window_is_hidden_and_restored_without_losing_maximization() {
    let window = TestWindow::create();

    // SAFETY: the handle belongs to this test window.
    unsafe {
        let _ = ShowWindow(window.hwnd(), SW_MAXIMIZE);
    }
    assert!(window.is_maximized(), "the window must be maximized before the cycle");

    assert_eq!(hide_native(window.hwnd()), VisibilityState::Hidden);
    assert!(!window.is_visible());

    assert_eq!(restore_native(window.hwnd()), VisibilityState::Visible);
    assert!(window.is_visible());
    assert!(
        window.is_maximized(),
        "restoring a hidden maximized window must not drop it back to normal size"
    );

    // A second cycle proves the state survives repeated hide/restore.
    assert_eq!(hide_native(window.hwnd()), VisibilityState::Hidden);
    assert_eq!(restore_native(window.hwnd()), VisibilityState::Visible);
    assert!(window.is_maximized());
}
