//! App-lifetime policy for the native Gaming Mode session.
//!
//! A gaming session outlives the launcher window: it is owned by a native
//! monitor thread, not by the renderer. Two consequences are handled here:
//!
//! * A quit request (window close, tray Quit, app exit request) while a session
//!   is active is **deferred** until the session has been accounted for, so a
//!   half-written session can never be abandoned by pressing X.
//! * Once the monitor ends and the session reservation is released, a deferred
//!   quit is honoured exactly once.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;

/// Set just before a programmatic `app.exit`, so the close-to-tray handler does
/// not mistake the teardown of the main window for a user close request.
static EXITING: AtomicBool = AtomicBool::new(false);

/// True once SILO has committed to a genuine exit.
pub fn is_exiting() -> bool {
    EXITING.load(Ordering::SeqCst)
}

fn exit_now(app: &tauri::AppHandle) {
    EXITING.store(true, Ordering::SeqCst);
    app.exit(0);
}

/// What a quit request becomes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QuitDecision {
    /// No session owns the app: exit now.
    ExitNow,
    /// A session owns the app: remember the request and exit when it ends.
    Deferred,
}

/// Pure policy: only an explicitly active session defers a quit.
pub fn decide_quit(session_active: bool) -> QuitDecision {
    if session_active {
        QuitDecision::Deferred
    } else {
        QuitDecision::ExitNow
    }
}

/// The pending quit request. One instance is used process-wide; tests use their
/// own so they never touch the global.
#[derive(Debug, Default)]
pub struct QuitIntent {
    deferred: bool,
}

impl QuitIntent {
    /// Records a quit request according to the session policy.
    pub fn request(&mut self, session_active: bool) -> QuitDecision {
        match decide_quit(session_active) {
            QuitDecision::ExitNow => QuitDecision::ExitNow,
            QuitDecision::Deferred => {
                self.deferred = true;
                QuitDecision::Deferred
            }
        }
    }

    /// Takes the pending request, leaving none.
    pub fn take(&mut self) -> bool {
        std::mem::take(&mut self.deferred)
    }

    pub fn is_deferred(&self) -> bool {
        self.deferred
    }
}

static PENDING: Mutex<Option<QuitIntent>> = Mutex::new(None);

fn with_pending<T>(action: impl FnOnce(&mut QuitIntent) -> T) -> T {
    let mut slot = PENDING.lock().unwrap_or_else(|poison| poison.into_inner());
    action(slot.get_or_insert_with(QuitIntent::default))
}

/// True while a quit is waiting for the session to finish. Test seam.
pub fn quit_is_deferred() -> bool {
    with_pending(|pending| pending.is_deferred())
}

/// Clears a deferred quit and reports whether there was one. Test seam.
pub fn take_deferred_quit() -> bool {
    with_pending(|pending| pending.take())
}

/// Handles an explicit quit request from the tray or the app.
///
/// While a session is active the request is only recorded; the caller must not
/// exit. The session tail honours it after the commit.
pub fn request_quit(app: &tauri::AppHandle, session_active: bool) -> QuitDecision {
    match with_pending(|pending| pending.request(session_active)) {
        QuitDecision::ExitNow => {
            exit_now(app);
            QuitDecision::ExitNow
        }
        QuitDecision::Deferred => {
            log::warn!("SILO quit deferred: a Gaming Mode session is still active");
            QuitDecision::Deferred
        }
    }
}

/// Handles `RunEvent::ExitRequested`.
///
/// Returns the decision so the caller can log it; when it is `Deferred` the exit
/// request has been prevented.
pub fn handle_exit_request(api: &tauri::ExitRequestApi, session_active: bool) -> QuitDecision {
    match with_pending(|pending| pending.request(session_active)) {
        QuitDecision::ExitNow => QuitDecision::ExitNow,
        QuitDecision::Deferred => {
            api.prevent_exit();
            log::warn!("SILO exit request prevented: a Gaming Mode session is still active");
            QuitDecision::Deferred
        }
    }
}

/// Called after a monitor ends. Exits only when no session owns the app and a
/// quit was deferred; returns true when the app is exiting.
pub fn finish_session(app: &tauri::AppHandle) -> bool {
    if crate::gaming_session_guard::session_is_active() {
        // Another session (or this monitor's own tail) still owns the app.
        return false;
    }
    if take_deferred_quit() {
        log::info!("SILO honouring the deferred quit now that the session has finished");
        exit_now(app);
        return true;
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_quit_is_only_deferred_while_a_session_is_active() {
        assert_eq!(decide_quit(false), QuitDecision::ExitNow);
        assert_eq!(decide_quit(true), QuitDecision::Deferred);
    }

    #[test]
    fn a_deferred_quit_is_held_until_the_session_ends_and_taken_once() {
        let mut pending = QuitIntent::default();
        assert_eq!(pending.request(true), QuitDecision::Deferred);
        assert!(pending.is_deferred(), "the request must be remembered");
        assert_eq!(
            pending.request(true),
            QuitDecision::Deferred,
            "a second request changes nothing"
        );
        assert!(pending.take(), "the first finish honours the quit");
        assert!(!pending.take(), "the quit is honoured exactly once");
        assert!(!pending.is_deferred());
    }

    #[test]
    fn an_idle_quit_is_not_deferred_and_leaves_no_pending_intent() {
        let mut pending = QuitIntent::default();
        assert_eq!(pending.request(false), QuitDecision::ExitNow);
        assert!(!pending.is_deferred(), "nothing is queued when no session runs");
        assert!(!pending.take());
    }
}
