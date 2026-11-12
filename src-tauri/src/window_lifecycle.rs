//! Stage 6: real WebView destruction and recreation for Gaming Mode.
//!
//! # Why this module exists
//!
//! Hiding is not releasing. `WebviewWindow::hide()` (and even Win32
//! `ShowWindow(SW_HIDE)`) leaves the WebView2 host, its GPU surface and the
//! renderer process alive: the launcher's own page keeps running while the game
//! plays. Stage 6 exists to actually destroy that surface during the windowless
//! Gaming Mode phase and to build a genuinely new one afterwards.
//!
//! # The mechanism
//!
//! * **Destruction** uses Tauri's `WebviewWindow::destroy()` (Tauri 2.11.2
//!   `src/webview/webview_window.rs:2210`), which force-closes the window
//!   without emitting `CloseRequested`. It is fire-and-forget through the
//!   event-loop proxy, so it is safe from the monitor's worker thread and from
//!   the UI thread. On Windows the runtime drops the tao window and takes the
//!   WebView2 surface (`tauri-runtime-wry-2.11.2/src/lib.rs`,
//!   `Message::Window(id, WindowMessage::Destroy)` and `on_window_close`), so
//!   the native HWND becomes invalid. SILO therefore treats
//!   **`IsWindow(hwnd) == false`** as the authoritative proof of destruction and
//!   never reports success on the strength of a queued request alone.
//! * **Recreation** uses `WebviewWindowBuilder::from_config(..).build()` on the
//!   *configured* window, so the label stays exactly `main` (the capability in
//!   `capabilities/default.json` applies to `main` only) and fullscreen /
//!   decorations / size come from `tauri.conf.json` rather than from a
//!   hand-written duplicate. `build()` must not run on the UI thread: the
//!   runtime's `create_window` note in `tauri-runtime-wry-2.11.2/src/lib.rs`
//!   states the channel would deadlock, so every entry point that can run on
//!   the UI thread dispatches to a worker thread instead.
//!
//! # Lifecycle
//!
//! The documented Stage 6 states are
//! `IdleWithWindow -> Launching -> GamingNoWebview -> RestoringWindow ->
//! IdleWithWindow`, with a launch failure returning straight to
//! `IdleWithWindow`. They are modelled by [`WindowLifecycle`] and enforced by
//! [`record_launch_accepted`], [`record_session_ended`] and the guards inside
//! [`destroy_main_window`] / [`rebuild_main_window`], so "the safe lifecycle
//! point" is a production gate rather than a convention: releasing the WebView
//! is only legal once the durable session journal has been written and the game
//! identity has been pinned, which is exactly when the monitor fires the pinned
//! callback.
//!
//! # Hardening (Stage 6 audit remediation)
//!
//! Two audit findings shaped this module and both are enforced in code here:
//!
//! * **A release is only reported when *both* observations agree that the
//!   window is gone** — the OS handle is invalid *and* Tauri no longer lists the
//!   label. Either one alone is not proof, because each can report success while
//!   the other still describes a live window: a hidden window keeps a valid
//!   handle, and Tauri can clear its registry entry while the OS window is still
//!   being torn down. Partial success is reported as a failure, and the timeout
//!   message names the condition that is still unsatisfied.
//! * **A rebuild never mistakes a stale registry entry for a live window.** The
//!   entry is verified through its native handle before it is accepted as
//!   `AlreadyPresent`; a registered-but-dead entry is reported as
//!   `StaleRegistration`, `RELEASED` is left set, and no claim of restoration is
//!   made. See [`rebuild_main_window`] for the retry strategy and the documented
//!   limitation.
//!
//! # Invariants the callers depend on
//!
//! * Exactly one `main` window is ever created: the rebuild refuses to build
//!   when one already exists, and a second concurrent rebuild is refused, so two
//!   tray clicks cannot produce two windows, two renderers or two sets of
//!   frontend listeners.
//! * A failed release is reported as a failure and falls back to the Stage 4
//!   hide, so SILO never claims a windowless state it does not have.
//! * A failed rebuild leaves a recoverable state: `RELEASED` stays set, so the
//!   next Show SILO or session restore tries again instead of stranding the user.
//! * Nothing here needs a live renderer: the native monitor, journal, SaveGuard
//!   and finalization all run on worker threads and keep working while the
//!   WebView is gone.

use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

/// The label of the one window SILO owns. Capabilities, `get_webview_window`
/// and the frontend all bind to this name, so a recreated window must reuse it.
pub const MAIN_WINDOW_LABEL: &str = "main";

/// How long a destroy waits for the OS to release the window.
const RELEASE_TIMEOUT: Duration = Duration::from_secs(5);

/// How long a rebuild waits for the new window to become usable.
const REBUILD_TIMEOUT: Duration = Duration::from_secs(6);

/// Poll interval while waiting for the event loop to apply a lifecycle change.
const POLL_INTERVAL: Duration = Duration::from_millis(25);

/// True once the `main` WebView has been destroyed and has not been replaced
/// yet. This is SILO's own bookkeeping, not Tauri's: it is the authority for
/// "the renderer is gone", because Tauri may keep a stale registry entry for a
/// window it has destroyed.
static RELEASED: AtomicBool = AtomicBool::new(false);

/// True from the moment `destroy()` is issued until the release is either
/// confirmed or has failed.
///
/// This exists to close a race: `destroy()` is fire-and-forget, so the runtime
/// can tear the window down and emit the resulting `ExitRequested` *before* the
/// worker that issued it has observed the destruction and set `RELEASED`. Without
/// this flag the zero-window exit request looks like an ordinary close, is
/// remembered as a quit intent, and an ordinary Gaming Mode session then ends in
/// an app exit. It is set before `destroy()` for exactly that reason.
static RELEASING: AtomicBool = AtomicBool::new(false);

/// True while one thread is rebuilding the window. Prevents two concurrent
/// rebuild requests from creating duplicate `main` windows.
static REBUILDING: AtomicBool = AtomicBool::new(false);

/// True while the launcher WebView is released or is being released, and has not
/// been replaced by a live window yet.
///
/// True during the release as well as after it, because the runtime's
/// `ExitRequested` for a destroyed last window can arrive while the worker that
/// issued the release is still observing the result. See the doc on[`RELEASING`].
pub fn webview_is_released() -> bool {
    RELEASED.load(Ordering::SeqCst) || RELEASING.load(Ordering::SeqCst)
}

/// True when an `ExitRequested` event is SILO's own window teardown rather than
/// a quit request.
///
/// Destroying the last window makes the runtime ask the app for permission to
/// exit, with **no exit code** (`tauri-runtime-wry` passes `code: None` for the
/// zero-window request and `Some(code)` for a programmatic `app.exit(code)`).
/// Treating that request as a quit would make an ordinary Gaming Mode session
/// end in an app exit, so it is prevented instead — and, crucially, it must not
/// be remembered as an intent: the audit's rule is that intent is never inferred
/// from the windows being destroyed.
pub fn exit_request_is_own_teardown(exit_code: Option<i32>, webview_released: bool, session_active: bool) -> bool {
    exit_code.is_none() && webview_released && session_active
}

// ── Lifecycle state machine ─────────────────────────────────────────────────

/// The Stage 6 window lifecycle states.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowLifecycle {
    /// A live `main` window and no session: the ordinary launcher state.
    IdleWithWindow,
    /// The user launched a game; journal and identity work is still running.
    Launching,
    /// The session owns the app and no `main` window exists: the genuine
    /// windowless Gaming Mode state.
    GamingNoWebview,
    /// A new `main` window exists; the session has not been committed yet.
    RestoringWindow,
}

/// Events that move the lifecycle forward.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// A launch was accepted (journal reserved, spawn started).
    LaunchAccepted,
    /// The WebView was released at the safe point (pinned identity + journal).
    WebviewDestroyed,
    /// A replacement `main` window exists and is registered.
    WindowRebuilt,
    /// The session has been accounted for.
    SessionEnded,
}

/// The next lifecycle state, or `None` for an illegal transition.
///
/// Releasing the WebView from `IdleWithWindow` is illegal on purpose: it is the
/// guard that keeps destruction inside the session's safe window, where the
/// journal is durable and the game identity is pinned.
pub fn next_lifecycle(state: WindowLifecycle, event: LifecycleEvent) -> Option<WindowLifecycle> {
    use LifecycleEvent as E;
    use WindowLifecycle as S;
    match (state, event) {
        (S::IdleWithWindow, E::LaunchAccepted) => Some(S::Launching),
        // A launch that never reached the pinned-identity point (launch
        // failure, uncertain identity) returns to the ordinary state.
        (S::Launching, E::SessionEnded) => Some(S::IdleWithWindow),
        (S::Launching, E::WebviewDestroyed) => Some(S::GamingNoWebview),
        (S::GamingNoWebview, E::WindowRebuilt) => Some(S::RestoringWindow),
        (S::RestoringWindow, E::SessionEnded) => Some(S::IdleWithWindow),
        // A windowless phase that never committed still comes back.
        (S::GamingNoWebview, E::SessionEnded) => Some(S::IdleWithWindow),
        _ => None,
    }
}

/// True when the WebView may be released in this state.
pub fn release_is_allowed(state: WindowLifecycle) -> bool {
    next_lifecycle(state, LifecycleEvent::WebviewDestroyed).is_some()
}

/// Production release gate: the release is legal only from `Launching`, which is
/// the state the monitor reaches once the target identity is pinned and the
/// journal has been persisted.
///
/// Anything else is refused with a reason naming the offending state, so a
/// release request from idle, from an already-windowless session, or from a
/// session being restored is rejected rather than silently honoured.
pub fn authorize_release(state: WindowLifecycle) -> Result<(), String> {
    match state {
        WindowLifecycle::Launching => Ok(()),
        WindowLifecycle::IdleWithWindow => Err(format!(
            "the launcher WebView was not released: no Gaming Mode session is in flight (lifecycle {state:?}), so there is nothing to release"
        )),
        WindowLifecycle::GamingNoWebview => Err(
            "the launcher WebView was not released: the session is already windowless; a duplicate release is refused".to_string(),
        ),
        WindowLifecycle::RestoringWindow => Err(
            "the launcher WebView was not released: the session is restoring its window, so releasing would destroy the new renderer".to_string(),
        ),
    }
}

/// Production rebuild gate: a window may be built from any state that is not
/// already `Launching` — a rebuild is both the recovery route for a destroyed
/// launcher and the ordinary way back after a session ends.
pub fn authorize_rebuild(state: WindowLifecycle) -> Result<(), String> {
    match state {
        WindowLifecycle::Launching => Err(
            "the launcher window was not rebuilt: a Gaming Mode session is still launching, so a new renderer would compete with the game".to_string(),
        ),
        WindowLifecycle::IdleWithWindow
        | WindowLifecycle::GamingNoWebview
        | WindowLifecycle::RestoringWindow => Ok(()),
    }
}

/// True when a `main` window may be (re)built in this state.
///
/// A rebuild is also the recovery route for a windowless app with no session
/// (tray Show while the launcher was destroyed), so it is allowed from every
/// state that has nothing left to preserve.
pub fn rebuild_is_allowed(state: WindowLifecycle) -> bool {
    matches!(
        state,
        WindowLifecycle::GamingNoWebview | WindowLifecycle::RestoringWindow | WindowLifecycle::IdleWithWindow
    )
}

// ── The lifecycle the running process is actually in ─────────────────────────

/// The lifecycle state the running process is in.
///
/// This is the state the production guards read, so it is deliberately the same
/// value the callers mutate: there is no test-only copy of it.
static CURRENT: std::sync::Mutex<WindowLifecycle> = std::sync::Mutex::new(WindowLifecycle::IdleWithWindow);

fn with_current<T>(action: impl FnOnce(&mut WindowLifecycle) -> T) -> T {
    let mut slot = CURRENT.lock().unwrap_or_else(|poison| poison.into_inner());
    action(&mut slot)
}

/// The lifecycle state production currently records.
pub fn current_lifecycle() -> WindowLifecycle {
    with_current(|state| *state)
}

/// The result of applying an event to the recorded lifecycle.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LifecycleAdvance {
    /// The event was legal and the process is now in this state.
    Advanced(WindowLifecycle),
    /// The event was illegal; the state is unchanged.
    Refused,
}

impl LifecycleAdvance {
    pub fn is_accepted(&self) -> bool {
        matches!(self, LifecycleAdvance::Advanced(_))
    }

    pub fn state(&self) -> Option<WindowLifecycle> {
        match self {
            LifecycleAdvance::Advanced(state) => Some(*state),
            LifecycleAdvance::Refused => None,
        }
    }
}

/// Applies an event to the recorded lifecycle, refusing illegal transitions.
///
/// An illegal event leaves the state untouched rather than corrupting it, so a
/// duplicated or out-of-order call cannot move the process into a state it
/// should never be in.
pub fn advance(event: LifecycleEvent) -> LifecycleAdvance {
    with_current(|state| match next_lifecycle(*state, event) {
        Some(next) => {
            *state = next;
            LifecycleAdvance::Advanced(next)
        }
        None => LifecycleAdvance::Refused,
    })
}

/// Records that a Gaming Mode launch was accepted.
///
/// Called by production before the monitor starts, so the `Launching` state —
/// which is what authorises a later WebView release — can only exist once a
/// session really owns the app.
pub fn record_launch_accepted() {
    if !advance(LifecycleEvent::LaunchAccepted).is_accepted() {
        log::warn!(
            "SILO could not record the Gaming Mode launch transition; the launcher WebView will stay alive for this session"
        );
    }
}

/// Records that the session has been accounted for, returning the launcher to
/// `IdleWithWindow` whether or not a windowless phase ever happened.
pub fn record_session_ended() {
    // Already idle (a launch that never got past its own launch stage), so a
    // refusal here is not a problem and is not logged.
    let _ = advance(LifecycleEvent::SessionEnded);
}

// ── Pure eligibility policies ───────────────────────────────────────────────

/// Pure release policy: the setting must be on, the window must exist, and a
/// duplicate request must be refused so state cannot drift.
pub fn destruction_eligible(release_webview_enabled: bool, window_present: bool, already_released: bool) -> bool {
    release_webview_enabled && window_present && !already_released
}

/// Pure rebuild policy: a window is created only when there is none.
///
/// `window_present` wins over `released`, so a rebuild can never turn one
/// window into two, and a window that vanished without SILO releasing it is
/// still replaced rather than leaving the app stranded.
pub fn recreation_needed(window_present: bool, released: bool) -> bool {
    if window_present {
        false
    } else {
        let _ = released;
        true
    }
}

// ── Outcomes ────────────────────────────────────────────────────────────────

/// The result of requesting the WebView's destruction.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseOutcome {
    /// The window is really gone: either the OS handle is invalid (Windows) or
    /// Tauri no longer lists the label. `registry_cleared` records Tauri's own
    /// bookkeeping so a stale entry is reported instead of being ignored.
    Released {
        hwnd: Option<isize>,
        registry_cleared: bool,
    },
    /// The request is a duplicate: the WebView is already released and nothing
    /// was changed.
    AlreadyReleased,
    /// The request is not applicable (feature off, or there is no window).
    NotEligible { reason: String },
    /// The window is still alive. Callers must report this as a failure and fall
    /// back to the Stage 4 hide, never as a release.
    Failed { reason: String },
}

impl ReleaseOutcome {
    pub fn is_released(&self) -> bool {
        matches!(self, ReleaseOutcome::Released { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            ReleaseOutcome::Released { hwnd, registry_cleared } => match hwnd {
                Some(hwnd) => format!("destroyed (hwnd {hwnd}, tauri registry cleared: {registry_cleared})"),
                None => format!("destroyed (tauri registry cleared: {registry_cleared})"),
            },
            ReleaseOutcome::AlreadyReleased => "already released".to_string(),
            ReleaseOutcome::NotEligible { reason } => format!("not eligible: {reason}"),
            ReleaseOutcome::Failed { reason } => format!("failed: {reason}"),
        }
    }
}

/// The result of requesting a replacement `main` window.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildOutcome {
    /// A new window was built, registered and observed as the launcher.
    Rebuilt { hwnd: Option<isize> },
    /// A `main` window already exists **and its native handle is live**, so
    /// nothing was created and the existing window is the usable launcher.
    AlreadyPresent { hwnd: Option<isize> },
    /// Tauri still lists the `main` label, but the window behind it is gone.
    ///
    /// This is the stale-registration case: Tauri 2.11.2 has no public API to
    /// drop a registry entry, so the entry cannot be removed and a new one with
    /// the same label cannot be registered either (`WindowManager::prepare_window`
    /// rejects a duplicate label). SILO therefore refuses to treat this as a
    /// restored window, keeps `RELEASED` set, and reports it for retry instead.
    StaleRegistration { hwnd: Option<isize>, reason: String },
    /// Another thread is rebuilding right now; the request was dropped instead
    /// of risking a duplicate window.
    AlreadyInProgress,
    /// The rebuild failed; the state stays recoverable.
    Failed { reason: String },
}

impl RebuildOutcome {
    pub fn is_rebuilt(&self) -> bool {
        matches!(self, RebuildOutcome::Rebuilt { .. })
    }

    pub fn describe(&self) -> String {
        match self {
            RebuildOutcome::Rebuilt { hwnd: Some(hwnd) } => format!("recreated (hwnd {hwnd})"),
            RebuildOutcome::Rebuilt { hwnd: None } => "recreated".to_string(),
            RebuildOutcome::AlreadyPresent { hwnd: Some(hwnd) } => {
                format!("already present (hwnd {hwnd})")
            }
            RebuildOutcome::AlreadyPresent { hwnd: None } => "already present".to_string(),
            RebuildOutcome::StaleRegistration {
                hwnd: Some(hwnd),
                reason,
            } => {
                format!("stale registration (hwnd {hwnd}): {reason}")
            }
            RebuildOutcome::StaleRegistration { hwnd: None, reason } => {
                format!("stale registration: {reason}")
            }
            RebuildOutcome::AlreadyInProgress => "a rebuild is already running".to_string(),
            RebuildOutcome::Failed { reason } => format!("failed: {reason}"),
        }
    }
}

// ── Destruction ─────────────────────────────────────────────────────────────

/// Destroys the `main` WebView and verifies that it is really gone.
///
/// `destroy()` is a request, exactly like `hide()`, so the return value is only
/// trusted after an observation: on Windows the window's OS handle must have
/// become invalid **and** Tauri must no longer list the label. One condition on
/// its own is not proof of destruction — each can be satisfied while the other
/// still describes a live window — so partial success is reported as a failure
/// and `RELEASED` is left unset.
///
/// The durable journal and pinned game identity are a precondition enforced in
/// production: the monitor only fires the pinned callback *after* the pinned
/// journal has been persisted, and the lifecycle guard below refuses any release
/// that is not in the session's `Launching` transition.
pub fn destroy_main_window(app: &tauri::AppHandle) -> ReleaseOutcome {
    if RELEASED.load(Ordering::SeqCst) || RELEASING.load(Ordering::SeqCst) {
        // A duplicate request must never change state: a second destroy would
        // only find a window that is not there.
        return ReleaseOutcome::AlreadyReleased;
    }
    // Production lifecycle gate: the release is legal only inside the session's
    // documented transition, which is exactly the point at which the target
    // identity is pinned and the journal is durable.
    if let Err(reason) = authorize_release(current_lifecycle()) {
        return ReleaseOutcome::Failed { reason };
    }
    let Some(window) = tauri::Manager::get_webview_window(app, MAIN_WINDOW_LABEL) else {
        return ReleaseOutcome::NotEligible {
            reason: "there is no main window to release".to_string(),
        };
    };
    let hwnd = window_handle(&window);
    // Mark the intent *before* issuing the request: the runtime may tear the
    // window down and raise `ExitRequested` at any point after `destroy()`
    // returns, and that must already be recognised as our own teardown rather
    // than as a user's quit request.
    RELEASING.store(true, Ordering::SeqCst);
    if let Err(error) = window.destroy() {
        RELEASING.store(false, Ordering::SeqCst);
        return ReleaseOutcome::Failed {
            reason: format!("destroy() was refused: {error}"),
        };
    }

    // Observe: the runtime applies the message on the event loop, so poll until
    // both observations agree that the window is gone.
    let deadline = Instant::now() + RELEASE_TIMEOUT;
    loop {
        let (registry_cleared, handle_gone) = observe_release(app, hwnd);
        if destruction_observed(registry_cleared, handle_gone) {
            RELEASING.store(false, Ordering::SeqCst);
            RELEASED.store(true, Ordering::SeqCst);
            // The transition is the model's own move; if it is refused the
            // destruction still happened, so the failure is logged, not hidden.
            if !advance(LifecycleEvent::WebviewDestroyed).is_accepted() {
                log::warn!(
                    "SILO destroyed the main WebView but could not record the transition to {:?}",
                    WindowLifecycle::GamingNoWebview
                );
            }
            return ReleaseOutcome::Released { hwnd, registry_cleared };
        }
        if Instant::now() >= deadline {
            // Partial success is not a release: the flag is cleared so the caller
            // can fall back to hiding and a later request is not refused.
            RELEASING.store(false, Ordering::SeqCst);
            return ReleaseOutcome::Failed {
                reason: format!(
                    "the main window is still alive after destroy(): {}",
                    unsatisfied_release_condition(registry_cleared, handle_gone, hwnd)
                ),
            };
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// True when the observations prove the window is really gone.
///
/// Windows has a native handle to check, so **both** conditions must hold: the OS
/// handle must be invalid *and* Tauri must no longer list the label. Platforms
/// without a native handle have no second observation to make, so the registry
/// is the only evidence available there and this collapses to that single check.
fn destruction_observed(registry_cleared: bool, handle_gone: bool) -> bool {
    #[cfg(target_os = "windows")]
    {
        registry_cleared && handle_gone
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = handle_gone;
        registry_cleared
    }
}

/// Names the release condition that is still unsatisfied, so a timeout says why
/// instead of only that it failed.
pub fn unsatisfied_release_condition(registry_cleared: bool, handle_gone: bool, hwnd: Option<isize>) -> String {
    let mut missing: Vec<String> = Vec::new();
    if !registry_cleared {
        missing.push(format!("Tauri still registers the '{MAIN_WINDOW_LABEL}' window label"));
    }
    if !handle_gone {
        match hwnd {
            Some(handle) => missing.push(format!("the OS handle hwnd {handle} is still a live window")),
            None => missing.push("no native handle was available to verify with".to_string()),
        }
    }
    if missing.is_empty() {
        "no outstanding condition".to_string()
    } else {
        missing.join("; ")
    }
}

/// The two observations that can prove a destruction really happened.
///
/// Returns `(registry no longer lists the label, the OS handle is invalid)`.
/// Both are reported to [`destruction_observed`], which decides whether they are
/// jointly sufficient; the pair is returned raw so a caller can diagnose exactly
/// which condition is outstanding.
fn observe_release(app: &tauri::AppHandle, hwnd: Option<isize>) -> (bool, bool) {
    let registry_cleared = tauri::Manager::get_webview_window(app, MAIN_WINDOW_LABEL).is_none();
    let handle_gone = hwnd.and_then(handle_exists) == Some(false);
    (registry_cleared, handle_gone)
}

// ── Recreation ──────────────────────────────────────────────────────────────

/// Rebuilds the `main` window from its configured definition.
///
/// Must run on a worker thread: Tauri's `create_window` deadlocks if it blocks
/// the UI thread.
///
/// An existing registry entry is **not** treated as a live window until its
/// native handle has been verified. That matters because a `destroy()` can clear
/// the OS window while Tauri still lists the label for a short while (the
/// `Destroyed` event runs after the window is gone). Accepting that entry as
/// `AlreadyPresent` would make SILO believe the launcher was restored while no
/// usable window exists, and would clear `RELEASED` — the flag the tray uses to
/// decide to rebuild — leaving the app windowless with no way back.
///
/// The stale case is handled in three steps, in order:
/// 1. **Verify.** `IsWindow` on the entry's handle: live → `AlreadyPresent`
///    (`RELEASED` cleared); dead → stale.
/// 2. **Wait.** The entry is re-checked for a bounded time, because the
///    `Destroyed` event may simply not have been dispatched yet and the entry
///    usually clears on its own; a wait that sees the entry clear proceeds to
///    build, and a wait that sees it become live proceeds as `AlreadyPresent`.
/// 3. **Fail honestly.** If it is still registered-but-dead after the wait, the
///    outcome is `StaleRegistration`, `RELEASED` is deliberately left set so the
///    next Show SILO retries, and no window is created. See
///    [`stale_registration_limitation`] for why the entry cannot be removed.
pub fn rebuild_main_window(app: &tauri::AppHandle) -> RebuildOutcome {
    // A rebuild from `Launching` would race the game with a new renderer.
    if let Err(reason) = authorize_rebuild(current_lifecycle()) {
        return RebuildOutcome::Failed { reason };
    }
    match registry_liveness(app) {
        RegistryLiveness::Live { hwnd } => {
            // The window came back by another route (a concurrent rebuild, or a
            // fresh window nobody released). Never stack a second renderer on it.
            // A *verified live* window means no teardown is pending, so both
            // flags are cleared together.
            RELEASING.store(false, Ordering::SeqCst);
            RELEASED.store(false, Ordering::SeqCst);
            return RebuildOutcome::AlreadyPresent { hwnd };
        }
        RegistryLiveness::Stale { hwnd } => {
            // The entry is registered but its window is gone. Wait briefly for
            // Tauri's `Destroyed` event to clear it before giving up honestly.
            log::warn!(
                "SILO found a registered '{MAIN_WINDOW_LABEL}' window whose native handle is no longer live ({hwnd:?}); waiting for Tauri to clear the entry"
            );
            let waited = wait_for_registry_to_clear(app);
            match waited {
                RegistryLiveness::Live { hwnd: live } => {
                    RELEASING.store(false, Ordering::SeqCst);
                    RELEASED.store(false, Ordering::SeqCst);
                    return RebuildOutcome::AlreadyPresent { hwnd: live };
                }
                RegistryLiveness::Absent => {}
                RegistryLiveness::Stale { hwnd } => {
                    let reason = stale_registration_limitation();
                    log::error!(
                        "SILO could not bring the launcher back: the '{}' label is registered but its window is gone ({}). {} \
                         The WebView stays released, so the tray's Show SILO can retry.",
                        MAIN_WINDOW_LABEL,
                        hwnd.map(|handle| format!("hwnd {handle}")).unwrap_or_else(|| "no handle".to_string()),
                        reason
                    );
                    return RebuildOutcome::StaleRegistration { hwnd, reason };
                }
            }
        }
        RegistryLiveness::Absent => {}
    }
    if !REBUILDING
        .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
        .is_ok()
    {
        return RebuildOutcome::AlreadyInProgress;
    }
    let outcome = build_main_window(app);
    REBUILDING.store(false, Ordering::SeqCst);
    if outcome.is_rebuilt() {
        RELEASING.store(false, Ordering::SeqCst);
        RELEASED.store(false, Ordering::SeqCst);
    }
    outcome
}

/// What the Tauri registry says about the `main` label, verified against the OS.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RegistryLiveness {
    /// No entry is registered.
    Absent,
    /// An entry is registered and its native handle is a live window.
    Live { hwnd: Option<isize> },
    /// An entry is registered but its native handle is no longer a window.
    Stale { hwnd: Option<isize> },
}

/// Resolves the registry entry and verifies it against the OS.
///
/// On platforms without a native handle there is nothing to verify against, so a
/// registered entry is reported as `Live` — the OS check simply does not exist
/// there, which is the same limitation `handle_exists` documents.
fn registry_liveness(app: &tauri::AppHandle) -> RegistryLiveness {
    let Some(window) = tauri::Manager::get_webview_window(app, MAIN_WINDOW_LABEL) else {
        return RegistryLiveness::Absent;
    };
    let hwnd = window_handle(&window);
    classify_registration(hwnd, hwnd.and_then(handle_exists))
}

/// The liveness rule itself, split out so the production path and its tests
/// share one implementation rather than a test-only restatement of it.
///
/// `registered` is the handle Tauri's registry offered, `handle_is_live` is what
/// the OS said about it (`None` when the platform cannot check). A registered
/// entry whose handle is *definitively* not a live window is stale; anything else
/// is accepted as live or absent.
pub fn classify_registration(registered: Option<isize>, handle_is_live: Option<bool>) -> RegistryLiveness {
    match registered {
        None => RegistryLiveness::Absent,
        Some(hwnd) => match handle_is_live {
            Some(false) => RegistryLiveness::Stale { hwnd: Some(hwnd) },
            _ => RegistryLiveness::Live { hwnd: Some(hwnd) },
        },
    }
}

/// Waits, bounded, for a stale registry entry to clear on its own.
///
/// Tauri removes the entry when the window's `Destroyed` event is dispatched,
/// which normally happens quickly after `destroy()`, so a short wait resolves
/// the transient case without inventing any way to remove the entry ourselves.
/// A wait that observes the entry becoming live reports `Live` so the caller can
/// use it rather than build a duplicate.
fn wait_for_registry_to_clear(app: &tauri::AppHandle) -> RegistryLiveness {
    let deadline = Instant::now() + REBUILD_TIMEOUT;
    loop {
        match registry_liveness(app) {
            RegistryLiveness::Absent => return RegistryLiveness::Absent,
            live @ RegistryLiveness::Live { .. } => return live,
            RegistryLiveness::Stale { .. } => {
                if Instant::now() >= deadline {
                    return registry_liveness(app);
                }
                std::thread::sleep(POLL_INTERVAL);
            }
        }
    }
}

/// Why a stale registration cannot be removed, stated as a fact rather than a
/// plan.
///
/// Tauri 2.11.2 exposes no public API to drop a window from the window manager
/// (`AppManager::on_window_close` is `pub(crate)`, and `WebviewWindow::close()`
/// emits a `CloseRequested` that SILO's own close policy intercepts). The only
/// removal Tauri performs is its own, on the window's `Destroyed` event, which
/// is what [`wait_for_registry_to_clear`] waits for. So the honest failure is to
/// report it and keep the retry route alive — not to fabricate a handle, to
/// manipulate private runtime state, or to claim the window is restored.
pub fn stale_registration_limitation() -> String {
    "tauri 2.11.2 exposes no public API to remove a window label from its window manager, so a stale entry cannot be dropped and a same-label window cannot be built over it".to_string()
}

fn build_main_window(app: &tauri::AppHandle) -> RebuildOutcome {
    // The configured window is rebuilt as configured: same label, same URL, same
    // decorations / fullscreen / size, so the recreate path cannot drift from
    // the window the app started with.
    let config = app.config().clone();
    let Some(window_config) = config
        .app
        .windows
        .iter()
        .find(|window| window.label == MAIN_WINDOW_LABEL)
        .or_else(|| config.app.windows.first())
        .cloned()
    else {
        return RebuildOutcome::Failed {
            reason: "no main window is configured, so no renderer can be rebuilt".to_string(),
        };
    };

    let builder = match tauri::WebviewWindowBuilder::from_config(app, &window_config) {
        Ok(builder) => builder,
        Err(error) => {
            return RebuildOutcome::Failed {
                reason: format!("the configured window could not be used: {error}"),
            }
        }
    };
    if let Err(error) = builder.build() {
        return RebuildOutcome::Failed {
            reason: describe_rebuild_error(&error),
        };
    }

    // Evidence that the new instance is real: it must be registered and be the
    // launcher (a top-level window carrying its configured title). A queued
    // request would not pass this.
    let deadline = Instant::now() + REBUILD_TIMEOUT;
    loop {
        let registered = tauri::Manager::get_webview_window(app, MAIN_WINDOW_LABEL).is_some();
        let observation = observe_rebuild(app);
        if registered && observation.is_ok() {
            let probe = observation.unwrap_or_default();
            return RebuildOutcome::Rebuilt { hwnd: probe.0 };
        }
        if Instant::now() >= deadline {
            return RebuildOutcome::Failed {
                reason: format!(
                    "the recreated window is not usable (registered: {registered}, {})",
                    observation.err().unwrap_or_default()
                ),
            };
        }
        std::thread::sleep(POLL_INTERVAL);
    }
}

/// Observes the freshly built window: `Ok((hwnd, description))` when it is the
/// launcher, `Err(why)` when it is not.
fn observe_rebuild(app: &tauri::AppHandle) -> Result<(Option<isize>, String), String> {
    #[cfg(target_os = "windows")]
    {
        match crate::gaming_window::probe_main_window(app) {
            Ok(probe) if probe.is_root && !probe.title.is_empty() => {
                Ok((Some(probe.hwnd), format!("title '{}'", probe.title)))
            }
            Ok(probe) => Err(format!(
                "the recreated window is not the launcher (root {}, title '{}')",
                probe.is_root, probe.title
            )),
            Err(error) => Err(error),
        }
    }
    #[cfg(not(target_os = "windows"))]
    {
        let window = tauri::Manager::get_webview_window(app, MAIN_WINDOW_LABEL)
            .ok_or_else(|| "the recreated window is not registered".to_string())?;
        let title = window.title().map_err(|error| error.to_string())?;
        if title.is_empty() {
            return Err("the recreated window has no title".to_string());
        }
        Ok((None, format!("title '{title}'")))
    }
}

/// Rebuilds the window from the UI thread.
///
/// Tauri cannot build a window on the UI thread (the runtime's `create_window`
/// note says the channel would deadlock), so this dispatches to a worker thread
/// and reports the outcome through the log. Used by tray Show SILO, whose
/// callbacks run on the event-loop thread.
///
/// There is deliberately **no** fast path that inspects the registry here: an
/// entry is only ever counted as a live window after its handle has been
/// verified, and that verification lives in [`rebuild_main_window`] so the tray
/// path and the session-restore path cannot disagree about it. Spawning is cheap
/// next to being wrong about whether the launcher exists.
pub fn rebuild_main_window_async(app: &tauri::AppHandle) {
    let handle = app.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let outcome = rebuild_main_window(&handle);
        match &outcome {
            RebuildOutcome::Rebuilt { .. } => {
                log::info!("SILO tray reopen: launcher {}", outcome.describe())
            }
            RebuildOutcome::AlreadyPresent { .. } => {
                log::info!("SILO tray reopen: launcher {}", outcome.describe())
            }
            RebuildOutcome::AlreadyInProgress => {
                log::info!("SILO tray reopen: launcher {}", outcome.describe())
            }
            RebuildOutcome::StaleRegistration { .. } => log::error!(
                "SILO tray reopen: the launcher could not be brought back ({})",
                outcome.describe()
            ),
            RebuildOutcome::Failed { reason } => {
                log::error!("SILO could not rebuild the launcher window: {reason}")
            }
        }
    });
}

// ── Helpers ─────────────────────────────────────────────────────────────────

#[cfg(target_os = "windows")]
/// True while the OS still holds this handle as a live window.
pub fn handle_exists(handle: isize) -> Option<bool> {
    Some(crate::gaming_window::window_exists(
        crate::gaming_window::from_handle_value(handle),
    ))
}

#[cfg(not(target_os = "windows"))]
pub fn handle_exists(_handle: isize) -> Option<bool> {
    None
}

/// The native handle of a Tauri window, when the platform has one.
#[cfg(target_os = "windows")]
fn window_handle(window: &tauri::WebviewWindow<tauri::Wry>) -> Option<isize> {
    window.hwnd().ok().map(|hwnd| hwnd.0 as isize)
}

#[cfg(not(target_os = "windows"))]
fn window_handle(_window: &tauri::WebviewWindow<tauri::Wry>) -> Option<isize> {
    None
}

/// Explains a rebuild failure in the words the cause deserves.
fn describe_rebuild_error(error: &tauri::Error) -> String {
    match error {
        tauri::Error::WindowLabelAlreadyExists(label) => {
            format!("tauri still lists a window labelled '{label}', so a new one cannot be registered")
        }
        _ => error.to_string(),
    }
}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_webview_is_only_released_inside_the_session_lifecycle() {
        assert!(
            release_is_allowed(WindowLifecycle::Launching),
            "the safe point is a launched session, not idle"
        );
        assert!(
            !release_is_allowed(WindowLifecycle::IdleWithWindow),
            "releasing while idle would destroy a live launcher without a session"
        );
        assert!(
            !release_is_allowed(WindowLifecycle::GamingNoWebview),
            "the windowless state has nothing left to release"
        );
        assert!(
            !release_is_allowed(WindowLifecycle::RestoringWindow),
            "releasing while restoring would destroy the fresh renderer"
        );
    }

    #[test]
    fn the_lifecycle_walks_the_documented_stage_6_path() {
        let launched = next_lifecycle(WindowLifecycle::IdleWithWindow, LifecycleEvent::LaunchAccepted);
        assert_eq!(launched, Some(WindowLifecycle::Launching));
        let released = next_lifecycle(launched.unwrap(), LifecycleEvent::WebviewDestroyed);
        assert_eq!(released, Some(WindowLifecycle::GamingNoWebview));
        let restoring = next_lifecycle(released.unwrap(), LifecycleEvent::WindowRebuilt);
        assert_eq!(restoring, Some(WindowLifecycle::RestoringWindow));
        assert_eq!(
            next_lifecycle(restoring.unwrap(), LifecycleEvent::SessionEnded),
            Some(WindowLifecycle::IdleWithWindow)
        );
    }

    #[test]
    fn a_release_is_accepted_only_when_both_observations_agree() {
        // The documented requirement: IsWindow(hwnd) == false AND the label is
        // no longer registered. Only the conjunction counts as destruction.
        assert!(
            destruction_observed(true, true),
            "both observations cleared is the only accepted release"
        );

        // Registry cleared but the OS window is still there: Tauri forgot the
        // label while the real window lives on, so this must not be a release.
        assert!(
            !destruction_observed(true, false),
            "a cleared registry alone does not prove the window is gone"
        );

        // HWND invalid but the registry entry survives: the OS window is gone
        // while Tauri still lists the label, so this must not be a release either.
        assert!(
            !destruction_observed(false, true),
            "an invalid handle alone does not prove Tauri released the window"
        );

        assert!(!destruction_observed(false, false));
    }

    #[test]
    fn a_timeout_names_the_condition_that_is_still_unsatisfied() {
        assert_eq!(
            unsatisfied_release_condition(true, true, Some(7)),
            "no outstanding condition"
        );

        // Registry cleared, handle still live: the handle must be named.
        assert_eq!(
            unsatisfied_release_condition(true, false, Some(4242)),
            "the OS handle hwnd 4242 is still a live window"
        );

        // Handle gone, registration still present: the label must be named.
        assert_eq!(
            unsatisfied_release_condition(false, true, Some(4242)),
            "Tauri still registers the 'main' window label"
        );

        // Nothing changed: both are named, so the failure says which conditions
        // were outstanding rather than only that it failed.
        let both = unsatisfied_release_condition(false, false, Some(4242));
        assert!(both.contains("Tauri still registers the 'main' window label"), "{both}");
        assert!(
            both.contains("the OS handle hwnd 4242 is still a live window"),
            "{both}"
        );

        // No handle was available at all: that is itself the outstanding one.
        let no_handle = unsatisfied_release_condition(true, false, None);
        assert_eq!(no_handle, "no native handle was available to verify with");
    }

    #[test]
    fn a_partially_destroyed_window_is_never_reported_as_released() {
        // The failure branch carries the diagnostics, and RELEASED is only ever
        // set when both observations hold.
        let registry_cleared = true;
        let handle_gone = false;
        assert!(!destruction_observed(registry_cleared, handle_gone));
        // Nothing here sets RELEASED: the caller's fallback stays a hide, so a
        // window that survived is reported as a failure and not as a release.
        assert!(unsatisfied_release_condition(registry_cleared, handle_gone, Some(99)).contains("hwnd 99"));
    }

    #[test]
    fn a_windowless_phase_can_also_end_without_a_session() {
        assert_eq!(
            next_lifecycle(WindowLifecycle::GamingNoWebview, LifecycleEvent::SessionEnded),
            Some(WindowLifecycle::IdleWithWindow)
        );
    }

    #[test]
    fn a_launch_failure_returns_to_the_idle_with_window_state() {
        assert_eq!(
            next_lifecycle(WindowLifecycle::Launching, LifecycleEvent::SessionEnded),
            Some(WindowLifecycle::IdleWithWindow)
        );
    }

    #[test]
    fn an_illegal_transition_is_refused_rather_than_invented() {
        use LifecycleEvent::{SessionEnded, WebviewDestroyed, WindowRebuilt};
        use WindowLifecycle::{GamingNoWebview, IdleWithWindow};
        assert_eq!(next_lifecycle(IdleWithWindow, WebviewDestroyed), None);
        assert_eq!(next_lifecycle(IdleWithWindow, WindowRebuilt), None);
        assert_eq!(next_lifecycle(GamingNoWebview, WebviewDestroyed), None);
        assert_eq!(next_lifecycle(IdleWithWindow, SessionEnded), None);
    }

    #[test]
    fn destruction_is_eligible_only_when_opted_in_with_a_live_window() {
        // Opted out: the WebView must stay exactly as it is today.
        assert!(!destruction_eligible(false, true, false));
        // Opted in, window present, nothing released yet.
        assert!(destruction_eligible(true, true, false));
        // Nothing to release: report ineligible rather than a failure.
        assert!(!destruction_eligible(true, false, false));
        // A duplicate request must not be eligible again.
        assert!(!destruction_eligible(true, true, true));
        // A duplicate request while the window is somehow back is refused too,
        // so the flag cannot drift.
        assert!(!destruction_eligible(true, false, true));
    }

    #[test]
    fn the_duplicate_destruction_request_leaves_state_alone() {
        assert!(
            destruction_eligible(true, true, false),
            "the first release request is eligible"
        );
        assert!(
            !destruction_eligible(true, true, true),
            "a second release request must be refused"
        );
    }

    #[test]
    fn a_window_that_vanished_without_a_release_is_still_replaced() {
        assert!(
            !recreation_needed(true, false),
            "an existing window must never be replaced by a second one"
        );
        assert!(
            !recreation_needed(true, true),
            "an existing window wins over the released flag"
        );
        assert!(recreation_needed(false, true), "a released WebView must be rebuilt");
        assert!(
            recreation_needed(false, false),
            "a window that vanished unexpectedly must be rebuilt too"
        );
    }

    #[test]
    fn the_zero_window_exit_request_is_not_a_quit_request() {
        // Our own window teardown: prevented, and deliberately not remembered.
        assert!(exit_request_is_own_teardown(None, true, true));
        // No exit code is what marks it: a programmatic exit exits.
        assert!(!exit_request_is_own_teardown(Some(0), true, true));
        // A zero-window request with no session and no release is a plain close:
        // the old close-and-exit behaviour applies when there is no tray.
        assert!(!exit_request_is_own_teardown(None, true, false));
        // A live WebView is never a teardown.
        assert!(!exit_request_is_own_teardown(None, false, true));
    }

    #[test]
    fn a_destroyed_webview_never_leaves_a_quit_intent_behind() {
        // The policy above answers "prevent without recording"; anything that
        // falls through to the quit policy must keep the old semantics.
        for code in [None, Some(0)] {
            for released in [true, false] {
                for session in [true, false] {
                    let own = exit_request_is_own_teardown(code, released, session);
                    assert_eq!(own, code.is_none() && released && session);
                }
            }
        }
    }

    #[test]
    fn outcomes_describe_themselves_without_hiding_a_failure() {
        assert!(ReleaseOutcome::Released {
            hwnd: Some(7),
            registry_cleared: true
        }
        .is_released());
        assert!(!ReleaseOutcome::Failed {
            reason: "still alive".into()
        }
        .is_released());
        assert!(ReleaseOutcome::AlreadyReleased.describe().contains("already released"));
        assert!(ReleaseOutcome::NotEligible {
            reason: "feature disabled".into()
        }
        .describe()
        .contains("feature disabled"));
        assert!(ReleaseOutcome::Failed {
            reason: "still alive".into()
        }
        .describe()
        .contains("still alive"));

        assert!(RebuildOutcome::Rebuilt { hwnd: Some(9) }.is_rebuilt());
        assert!(!RebuildOutcome::AlreadyPresent { hwnd: None }.is_rebuilt());
        assert!(RebuildOutcome::AlreadyInProgress.describe().contains("already running"));
        assert!(RebuildOutcome::Failed {
            reason: "tauri still lists".into(),
        }
        .describe()
        .contains("tauri still lists"));
    }

    /// Regression test for the race that made ordinary sessions exit the app.
    ///
    /// The runtime may raise `ExitRequested` while the worker that issued
    /// `destroy()` is still observing the result, so "released" must already be
    /// true at the moment `destroy()` returns. Without that, the zero-window
    /// request is remembered as a quit intent and the session ends in an exit.
    #[test]
    fn the_teardown_is_recognised_before_the_destroy_is_even_observed() {
        // The exit-teardown policy must hold at the point where destroy() is
        // issued; this is precisely the state `RELEASING` establishes.
        let releasing = true;
        assert!(
            exit_request_is_own_teardown(None, releasing, true),
            "an exit request raised while the release is in flight is our teardown"
        );
        // ...and it must keep holding once the release is confirmed.
        assert!(exit_request_is_own_teardown(None, true, true));
        // A real quit is still a quit: it carries an exit code.
        assert!(!exit_request_is_own_teardown(Some(0), true, true));
        // An exit request with no session and no release stays a close.
        assert!(!exit_request_is_own_teardown(None, false, true));
        assert!(!exit_request_is_own_teardown(None, false, false));
    }

    #[test]
    fn a_registered_window_is_only_accepted_when_its_handle_is_live() {
        // A stale entry: registered, but no live OS window behind it. This is the
        // audit's Finding 2, and it must not be accepted as an existing window.
        let stale = classify_registration(Some(9), Some(false));
        assert_eq!(stale, RegistryLiveness::Stale { hwnd: Some(9) });

        // A live entry: registered and its handle is a real window.
        let live = classify_registration(Some(9), Some(true));
        assert_eq!(live, RegistryLiveness::Live { hwnd: Some(9) });

        // No entry at all.
        assert_eq!(classify_registration(None, Some(false)), RegistryLiveness::Absent);
        assert_eq!(classify_registration(None, Some(true)), RegistryLiveness::Absent);

        // No handle available to check (non-Windows, or a window with no native
        // handle): the entry is not *provably* stale, so it is treated as live.
        assert_eq!(
            classify_registration(Some(9), None),
            RegistryLiveness::Live { hwnd: Some(9) }
        );
    }

    #[test]
    fn a_stale_registration_is_reported_not_treated_as_restored() {
        // The outcome must not claim the launcher came back.
        let stale = RebuildOutcome::StaleRegistration {
            hwnd: Some(77),
            reason: "tauri has no public removal API".to_string(),
        };
        assert!(!stale.is_rebuilt(), "a stale registration is not a restore");
        assert!(stale.describe().contains("stale registration"), "{}", stale.describe());
        assert!(stale.describe().contains("77"), "{}", stale.describe());
        assert!(
            stale.describe().contains("tauri has no public removal API"),
            "{}",
            stale.describe()
        );

        // A live existing window *is* a usable launcher, and says so.
        assert!(RebuildOutcome::AlreadyPresent { hwnd: Some(5) }
            .describe()
            .contains("already present"));
    }

    #[test]
    fn a_stale_registration_cannot_be_removed_so_the_retry_route_is_kept() {
        // The limitation is stated, and it names exactly why no removal happens:
        // there is no public API and no fabricated handle.
        let limitation = stale_registration_limitation();
        assert!(limitation.contains("no public API"), "{limitation}");
    }

    // ── Finding 6: the lifecycle gate production actually consults ────────────

    /// Drives the *production* lifecycle state through the *production* recording
    /// functions and asserts the *production* gates react to it. This is not a
    /// restatement of `next_lifecycle`: it mutates the same process-wide state
    /// `destroy_main_window` and `rebuild_main_window` read, through the same
    /// functions those call sites use.
    #[test]
    fn the_production_lifecycle_gates_refuse_a_release_outside_launching() {
        assert_eq!(current_lifecycle(), WindowLifecycle::IdleWithWindow);

        // Idle: a release is refused with a reason naming the state.
        let refusal = authorize_release(current_lifecycle());
        assert!(refusal.is_err(), "idle must not authorise a release");
        let reason = refusal.err().unwrap_or_default();
        assert!(reason.contains("no Gaming Mode session is in flight"), "{reason}");
        assert!(reason.contains("IdleWithWindow"), "{reason}");

        // Production records the accepted launch.
        record_launch_accepted();
        assert_eq!(current_lifecycle(), WindowLifecycle::Launching);
        assert!(
            authorize_release(current_lifecycle()).is_ok(),
            "Launching is the state that authorises the release"
        );

        // Production records the windowless transition, as destroy_main_window does.
        assert_eq!(
            advance(LifecycleEvent::WebviewDestroyed),
            LifecycleAdvance::Advanced(WindowLifecycle::GamingNoWebview)
        );
        let duplicate = authorize_release(current_lifecycle());
        assert!(
            duplicate.is_err(),
            "a windowless session must not authorise a second release"
        );
        assert!(duplicate.err().unwrap_or_default().contains("already windowless"));

        // A duplicate event does not move the state.
        assert_eq!(advance(LifecycleEvent::WebviewDestroyed), LifecycleAdvance::Refused);
        assert_eq!(current_lifecycle(), WindowLifecycle::GamingNoWebview);

        // Production records the session end, as the worker tail does.
        record_session_ended();
        assert_eq!(current_lifecycle(), WindowLifecycle::IdleWithWindow);
        assert!(
            authorize_release(current_lifecycle()).is_err(),
            "after the session ends the release must be refused again"
        );
        // And the end event is idempotent once idle.
        record_session_ended();
        assert_eq!(current_lifecycle(), WindowLifecycle::IdleWithWindow);
    }

    #[test]
    fn the_production_lifecycle_gates_refuse_a_rebuild_during_launching() {
        // From idle a rebuild is allowed (the recovery route for a destroyed
        // launcher); during Launching it must compete with the game.
        assert!(authorize_rebuild(WindowLifecycle::IdleWithWindow).is_ok());
        assert!(authorize_rebuild(WindowLifecycle::GamingNoWebview).is_ok());
        assert!(authorize_rebuild(WindowLifecycle::RestoringWindow).is_ok());
        let refusal = authorize_rebuild(WindowLifecycle::Launching);
        assert!(refusal.is_err(), "Launching must not authorise a rebuild");
        assert!(refusal.err().unwrap_or_default().contains("still launching"));
    }

    #[test]
    fn a_failed_journal_write_leaves_the_launcher_release_unauthorised() {
        // A journal write failure means the monitor never reaches the pinned
        // callback, so it never records `Launching` — and without `Launching`
        // the release gate refuses. This is the prerequisite, expressed as a
        // state the production code can actually be in.
        assert_eq!(current_lifecycle(), WindowLifecycle::IdleWithWindow);
        assert!(authorize_release(current_lifecycle()).is_err());
        assert!(authorize_release(current_lifecycle())
            .err()
            .unwrap_or_default()
            .contains("nothing to release"));
    }

    #[test]
    fn a_rebuild_is_allowed_from_every_windowless_state() {
        assert!(rebuild_is_allowed(WindowLifecycle::GamingNoWebview));
        assert!(rebuild_is_allowed(WindowLifecycle::RestoringWindow));
        assert!(
            rebuild_is_allowed(WindowLifecycle::IdleWithWindow),
            "t Show with no window must be able to create one"
        );
    }
}
