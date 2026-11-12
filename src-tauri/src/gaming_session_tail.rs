//! Command tail of the opt-in Gaming Mode worker.
//!
//! The launch monitor is deliberately Tauri-free; this tail is the small, injectable
//! boundary that owns the host effects and the durable commit: launcher restore,
//! SaveGuard completion, idempotent aggregate finalization, history upsert and the
//! `playtime-updated` notification.
//!
//! Tests implement [`SessionTailHost`] with a recording host, so they prove the tail's
//! ordering and file effects — not that the real Tauri window or event system works.

use crate::gaming_session_monitor::MonitoredSession;
use crate::gaming_session_persistence::{
    self as persistence, AggregateTargets, DurationKind, Finalization, HistoryRow,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::SystemTime;

/// Everything `finish_save_watch` needs, minus the `AppHandle`.
pub struct SaveCompletion {
    pub watch_for_saves: bool,
    pub run_detection: bool,
    pub detected_path: Option<PathBuf>,
    pub initial_save_path: Option<String>,
    pub name_keys: Vec<String>,
    pub started_sys: SystemTime,
    pub executable: String,
    pub game_id: String,
    pub backup_count: u32,
    pub detected_roots: HashSet<PathBuf>,
}

/// Minimal aggregate snapshot the tail needs; keeps `Game` private to `commands`.
pub struct GameTotals {
    /// Exact playtime total. Uncertain minutes are never part of this.
    pub playtime_minutes: u32,
    /// Uncertain (lower-bound) playtime total, kept in its own bucket.
    pub uncertain_playtime_minutes: u32,
    pub session_count: u32,
}

pub trait SessionTailHost {
    fn hide_launcher(&self);
    fn restore_launcher(&self);
    fn complete_save_guard(&self, request: SaveCompletion);
    fn game_totals(&self, game_id: &str) -> Option<GameTotals>;
    fn emit_playtime_updated(&self, game_id: &str);
}

pub struct SessionTail {
    pub app_data: PathBuf,
    pub game_id: String,
    pub initial_save_path: Option<String>,
    pub name_keys: Vec<String>,
    pub executable: String,
    pub backup_count: u32,
    /// Computed by the caller with the existing `save_watch_required` decision.
    pub watch_for_saves: bool,
    pub run_detection: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub enum TailOutcome {
    Finalized {
        minutes: u32,
    },
    /// The journal is gone or belongs to another session: nothing is committed.
    AlreadyFinalized,
    /// `games.json` no longer contains the game; evidence is retained.
    GameMissing,
    FinalizationFailed(String),
}

/// Runs the tail for a session whose target definitely exited.
///
/// The session carries the single-active-session reservation, which stays held until
/// the commit is finished so no concurrent launch can replace the journal mid-way.
pub fn run_session_tail(tail: SessionTail, session: MonitoredSession, host: &dyn SessionTailHost) -> TailOutcome {
    let MonitoredSession {
        journal,
        target_started_at,
        started_sys,
        elapsed_secs,
        exit_code,
        detected_path,
        detected_roots,
        duration_kind,
        guard,
        ..
    } = session;

    // Never commit over a journal that is absent or no longer this session's.
    let current = match persistence::load_journal(&tail.app_data) {
        Ok(Some(current)) if current.session_id == journal.session_id => current,
        Ok(_) | Err(_) => {
            drop(guard);
            return TailOutcome::AlreadyFinalized;
        }
    };

    host.restore_launcher();

    // A journal that still carries a finalization payload means an earlier commit
    // reached the journal but not the removal — a crash between the two writes.
    // Replaying it means re-running the *persisted* snapshot through the
    // session-idempotent finalizer. Recomputing the totals from the current game
    // record here would add this session's minutes a second time.
    if let Some(persisted) = current.finalization.clone() {
        let row = HistoryRow {
            session_id: Some(current.session_id.clone()),
            game_id: current.game_id.clone(),
            started_at: current.started_at_utc.clone(),
            minutes: persisted.duration_minutes,
            duration_kind: if persisted.uncertain {
                DurationKind::Uncertain
            } else {
                DurationKind::Exact
            },
        };
        let mut replay = current;
        let outcome = match persistence::finalize(&tail.app_data, &mut replay, row) {
            Ok(()) => {
                host.emit_playtime_updated(&tail.game_id);
                log::info!(
                    "Gaming Mode replayed the committed totals for {} (session already finalized)",
                    tail.game_id
                );
                TailOutcome::AlreadyFinalized
            }
            Err(error) => {
                log::error!("Gaming Mode finalization replay failed; journal retained: {error}");
                TailOutcome::FinalizationFailed(error)
            }
        };
        drop(guard);
        return outcome;
    }

    host.complete_save_guard(SaveCompletion {
        watch_for_saves: tail.watch_for_saves,
        run_detection: tail.run_detection,
        detected_path,
        initial_save_path: tail.initial_save_path.clone(),
        name_keys: tail.name_keys.clone(),
        started_sys,
        executable: tail.executable.clone(),
        game_id: tail.game_id.clone(),
        backup_count: tail.backup_count,
        detected_roots,
    });

    let minutes = if elapsed_secs > 0 {
        elapsed_secs.div_ceil(60) as u32
    } else {
        0
    };
    let Some(totals) = host.game_totals(&tail.game_id) else {
        drop(guard);
        return TailOutcome::GameMissing;
    };

    let mut journal = journal;
    // The uncertain bucket is deliberately separate: a lower-bound duration must
    // never be added to the exact playtime total.
    let uncertain = !duration_kind.is_exact();
    journal.finalization = Some(Finalization {
        duration_minutes: minutes,
        uncertain,
        status: if uncertain {
            "recovered_uncertain".into()
        } else {
            "completed".into()
        },
        targets: AggregateTargets {
            playtime_minutes: if uncertain {
                totals.playtime_minutes
            } else {
                totals.playtime_minutes.saturating_add(minutes)
            },
            uncertain_playtime_minutes: if uncertain {
                totals.uncertain_playtime_minutes.saturating_add(minutes)
            } else {
                totals.uncertain_playtime_minutes
            },
            session_count: totals.session_count.saturating_add(1),
            last_played: Some(chrono::Local::now().to_rfc3339()),
        },
    });
    let row = HistoryRow {
        session_id: Some(journal.session_id.clone()),
        game_id: tail.game_id.clone(),
        started_at: target_started_at,
        minutes,
        duration_kind,
    };
    if let Err(error) = persistence::finalize(&tail.app_data, &mut journal, row) {
        log::error!("Gaming Mode finalization failed; journal retained: {error}");
        drop(guard);
        return TailOutcome::FinalizationFailed(error);
    }
    host.emit_playtime_updated(&tail.game_id);
    if let Some(code) = exit_code {
        log::info!("Gaming Mode session for {} ended with exit code {code}", tail.game_id);
    }
    drop(guard);
    TailOutcome::Finalized { minutes }
}
