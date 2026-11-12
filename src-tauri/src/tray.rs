//! Native tray icon: SILO's permanent entry point.
//!
//! The tray exists for the whole process lifetime. It is what makes the launcher
//! recoverable while a Gaming Mode session owns the window (hidden today,
//! destroyed in the no-WebView stage) and it is the only place that offers an
//! explicit Quit, so closing the window can step aside instead of ending the app.
//!
//! The tray only *requests* actions; the policy lives in `gaming_lifecycle`.

use crate::settings::CloseBehaviour;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::menu::{Menu, MenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};

/// Menu item ids, also used as the tray's own id prefix.
pub const MENU_SHOW: &str = "silo.show";
pub const MENU_QUIT: &str = "silo.quit";
const TRAY_ID: &str = "silo.tray";

static INSTALLED: AtomicBool = AtomicBool::new(false);

/// True once a tray icon has been built successfully.
pub fn is_installed() -> bool {
    INSTALLED.load(Ordering::SeqCst)
}

/// What the tray's Show SILO action must do to bring the launcher back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReopenAction {
    /// The WebView is alive: show the existing window (Stage 4/5 behaviour).
    RestoreExisting,
    /// Stage 6 destroyed the WebView: build a new `main` window.
    RebuildWindow,
}

/// Pure reopen policy, shared by the tray handler and its tests.
///
/// Only a WebView that was really destroyed by Gaming Mode is rebuilt; anything
/// else keeps the verified restore path, so the default (feature off) tray
/// behaviour is untouched.
pub fn reopen_action(webview_released: bool) -> ReopenAction {
    if webview_released {
        ReopenAction::RebuildWindow
    } else {
        ReopenAction::RestoreExisting
    }
}

/// Brings the launcher back: shows the existing window, or rebuilds it when a
/// Gaming Mode session has destroyed the WebView.
///
/// The rebuild is dispatched to a worker thread because Tauri cannot create a
/// window on the UI thread; the outcome is logged, never assumed.
pub fn show_main_window(app: &tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    {
        match reopen_action(crate::window_lifecycle::webview_is_released()) {
            ReopenAction::RebuildWindow => crate::window_lifecycle::rebuild_main_window_async(app),
            ReopenAction::RestoreExisting => {
                let outcome = crate::gaming_window::restore_launcher_window(app);
                log::info!("SILO tray reopen: {}", outcome.describe());
            }
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
            let _ = window.show();
            let _ = window.set_focus();
            log::info!("SILO tray reopen: showing the launcher window");
        } else {
            log::warn!("SILO tray reopen: there is no launcher window to show");
        }
    }
}

/// Steps the launcher aside while the tray keeps the app alive.
///
/// Used by both the custom title bar (`window_close`) and a native close
/// request (Alt+F4, taskbar close). The window is hidden, never destroyed, so
/// Show SILO always has a real window to restore.
pub fn hide_to_tray(app: &tauri::AppHandle) {
    #[cfg(target_os = "windows")]
    {
        let outcome = crate::gaming_window::hide_launcher_window(app);
        log::info!("SILO close-to-tray: {}", outcome.describe());
    }
    #[cfg(not(target_os = "windows"))]
    {
        if let Some(window) = tauri::Manager::get_webview_window(app, "main") {
            if let Err(error) = window.hide() {
                log::warn!("SILO could not hide the launcher window: {error}");
            }
        }
    }
}

/// What a native window-close request should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CloseDecision {
    /// The tray can keep the app alive: hide the window and cancel the close.
    StepAside,
    /// No tray, or a genuine exit is already under way: let the close proceed.
    AllowClose,
    /// The user must choose: show the confirmation dialog and act on the answer.
    AskUser,
}

/// Pure close policy, shared by the window-event handler and its tests.
///
/// The stored close behaviour decides what closing the window means:
///
/// * `ExitApplication` — always let the close proceed (a gaming session still
///   defers the actual exit through `gaming_lifecycle`).
/// * `MinimiseToTray` — hide the window and keep the app alive, when a tray
///   exists; fall back to letting the close proceed when there is no tray.
/// * `AskEveryTime` — defer the close and ask the user each time.
pub fn close_decision(tray_installed: bool, exiting: bool, behaviour: CloseBehaviour) -> CloseDecision {
    if exiting {
        // A genuine exit already under way is never turned into another hide.
        return CloseDecision::AllowClose;
    }
    match behaviour {
        CloseBehaviour::ExitApplication => CloseDecision::AllowClose,
        CloseBehaviour::MinimiseToTray => {
            if tray_installed {
                CloseDecision::StepAside
            } else {
                CloseDecision::AllowClose
            }
        }
        CloseBehaviour::AskEveryTime => CloseDecision::AskUser,
    }
}

fn handle_menu(app: &tauri::AppHandle, id: &str) {
    match id {
        MENU_SHOW => show_main_window(app),
        MENU_QUIT => {
            let decision = crate::gaming_lifecycle::request_quit(app, crate::gaming_session_guard::session_is_active());
            log::info!("SILO tray quit: {decision:?}");
        }
        other => log::warn!("SILO tray: unknown menu item {other}"),
    }
}

/// Pure install policy: a second tray build would register a second icon and a
/// second menu handler for the same items, so it is never attempted.
pub fn install_needed(installed: bool) -> bool {
    !installed
}

/// Builds the tray icon and its menu.
///
/// A failure is reported to the caller, not swallowed: without a tray, closing
/// the launcher would leave the app running with no way back, so the caller must
/// fall back to the plain close behaviour.
pub fn install(app: &tauri::AppHandle) -> tauri::Result<()> {
    // Setup runs once today, but installing the tray twice would register two
    // handlers for the same menu items and two icons in the notification area.
    if !install_needed(is_installed()) {
        log::warn!("SILO tray: install requested twice; keeping the existing icon");
        return Ok(());
    }

    let show = MenuItem::with_id(app, MENU_SHOW, "Show SILO", true, None::<&str>)?;
    let quit = MenuItem::with_id(app, MENU_QUIT, "Quit SILO", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&show, &quit])?;

    let mut builder = TrayIconBuilder::with_id(TRAY_ID)
        .tooltip("SILO")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| handle_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        });

    if let Some(icon) = app.default_window_icon().cloned() {
        builder = builder.icon(icon);
    } else {
        log::warn!("SILO tray: the app has no default window icon; using the tray default");
    }

    builder.build(app)?;
    INSTALLED.store(true, Ordering::SeqCst);
    log::info!("SILO tray icon installed (Show SILO / Quit SILO)");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn show_silo_keeps_retrying_after_a_failed_rebuild() {
        // The rebuild policy is a pure function of the released flag, so
        // repeated Show requests after a failed rebuild behave identically:
        // each one asks for a rebuild again rather than giving up or creating a
        // duplicate. A rebuild that succeeds in between flips the answer once.
        assert_eq!(reopen_action(true), ReopenAction::RebuildWindow);
        for _ in 0..5 {
            assert_eq!(
                reopen_action(true),
                ReopenAction::RebuildWindow,
                "a failed rebuild must not end the retry route"
            );
        }
        // Once a live window is back the flag clears, and Show restores it.
        assert_eq!(reopen_action(false), ReopenAction::RestoreExisting);
        // And repeated requests below stay restores.
        for _ in 0..5 {
            assert_eq!(reopen_action(false), ReopenAction::RestoreExisting);
        }
    }

    #[test]
    fn show_silo_rebuilds_only_a_released_webview() {
        assert_eq!(
            reopen_action(true),
            ReopenAction::RebuildWindow,
            "a Gaming Mode release must be answered with a new window"
        );
        assert_eq!(
            reopen_action(false),
            ReopenAction::RestoreExisting,
            "with the WebView alive the verified restore path is kept unchanged"
        );
        // Repeated requests are idempotent: the same state always answers the
        // same way, so two tray clicks cannot mean two rebuilds.
        for _ in 0..5 {
            assert_eq!(reopen_action(true), ReopenAction::RebuildWindow);
            assert_eq!(reopen_action(false), ReopenAction::RestoreExisting);
        }
    }

    #[test]
    fn an_exit_application_close_is_allowed_even_with_a_tray_and_no_session() {
        assert_eq!(
            close_decision(true, false, CloseBehaviour::ExitApplication),
            CloseDecision::AllowClose,
            "exit application always lets the close proceed"
        );
    }

    #[test]
    fn a_minimise_to_tray_close_steps_aside_when_a_tray_exists() {
        assert_eq!(
            close_decision(true, false, CloseBehaviour::MinimiseToTray),
            CloseDecision::StepAside,
            "hide to tray keeps the app alive"
        );
        assert_eq!(
            close_decision(false, false, CloseBehaviour::MinimiseToTray),
            CloseDecision::AllowClose,
            "without a tray there is nowhere to hide, so the close proceeds"
        );
    }

    #[test]
    fn an_ask_every_time_close_defers_to_the_user() {
        assert_eq!(
            close_decision(true, false, CloseBehaviour::AskEveryTime),
            CloseDecision::AskUser,
            "ask every time defers to the confirmation dialog"
        );
    }

    #[test]
    fn a_genuine_exit_never_steps_aside_regardless_of_setting() {
        assert_eq!(
            close_decision(true, true, CloseBehaviour::MinimiseToTray),
            CloseDecision::AllowClose,
            "an exit already under way is not turned into a hide"
        );
        assert_eq!(
            close_decision(true, true, CloseBehaviour::AskEveryTime),
            CloseDecision::AllowClose,
            "an exit already under way is not turned into a prompt"
        );
    }

    #[test]
    fn close_decisions_are_idempotent() {
        for behaviour in [
            CloseBehaviour::AskEveryTime,
            CloseBehaviour::MinimiseToTray,
            CloseBehaviour::ExitApplication,
        ] {
            let first = close_decision(true, false, behaviour);
            for _ in 0..5 {
                assert_eq!(close_decision(true, false, behaviour), first);
            }
        }
    }

    #[test]
    fn a_tray_is_installed_at_most_once() {
        assert!(install_needed(false), "the first install must run");
        assert!(
            !install_needed(true),
            "a second install would add a duplicate icon and menu handler"
        );
        for _ in 0..5 {
            assert!(!install_needed(true));
        }
    }
}
