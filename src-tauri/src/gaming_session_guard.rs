//! Process-wide single-active-session guard for opt-in Gaming Mode.
//!
//! The durable journal is a single file, so a second simultaneous launch would
//! overwrite the first session's evidence. The guard is taken before the provisional
//! journal is written and released only after the command tail finishes; a failed or
//! uncertain launch releases it as well.

use std::sync::atomic::{AtomicBool, Ordering};

static ACTIVE_SESSION: AtomicBool = AtomicBool::new(false);

/// Holds the single-active-session reservation for as long as it lives.
#[derive(Debug)]
pub struct SessionGuard;

impl Drop for SessionGuard {
    fn drop(&mut self) {
        ACTIVE_SESSION.store(false, Ordering::SeqCst);
    }
}

/// Returns `None` when another Gaming Mode session already owns the journal.
pub fn try_acquire_session() -> Option<SessionGuard> {
    if ACTIVE_SESSION
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        Some(SessionGuard)
    } else {
        None
    }
}

/// Test-only observation of the reservation state.
#[doc(hidden)]
pub fn session_is_active() -> bool {
    ACTIVE_SESSION.load(Ordering::SeqCst)
}
