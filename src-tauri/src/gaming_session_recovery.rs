//! Conservative startup inspection of the single durable gaming-session journal.

use crate::gaming_session_persistence::{
    self as persistence, AggregateTargets, DurationKind, Finalization, GamingSessionJournal, HistoryRow,
    RecoveryAction, RecoveryObservation,
};
use std::path::Path;

/// Returns the journal and a retained handle only when the pinned identity is still the
/// same live process. Everything else is left to [`reconcile`].
pub fn take_running_session(
    directory: &Path,
) -> Result<Option<(GamingSessionJournal, crate::process_monitor::TrackedProcess)>, String> {
    let Some(journal) = persistence::load_journal(directory)? else {
        return Ok(None);
    };
    if journal.finalization.is_some() {
        return Ok(None);
    }
    let (Some(pid), Some(created)) = (journal.game_pid, journal.process_created_at.as_deref()) else {
        return Ok(None);
    };
    let Ok(created) = created.parse::<u64>() else {
        return Ok(None);
    };
    let (status, tracked) = crate::process_monitor::reopen_persisted(pid, created);
    match (status, tracked) {
        (crate::process_monitor::ProcessStatus::Running, Some(tracked)) => Ok(Some((journal, tracked))),
        _ => Ok(None),
    }
}

pub fn reconcile(directory: &Path) -> Result<RecoveryAction, String> {
    let Some(mut journal) = persistence::load_journal(directory)? else {
        return Ok(RecoveryAction::NoJournal);
    };

    let observation = match (journal.game_pid, journal.process_created_at.as_deref()) {
        (Some(pid), Some(created)) => match created.parse::<u64>() {
            Ok(created) => {
                let (status, tracked) = crate::process_monitor::reopen_persisted(pid, created);
                drop(tracked);
                match status {
                    crate::process_monitor::ProcessStatus::Running => RecoveryObservation::Running,
                    crate::process_monitor::ProcessStatus::Exited => RecoveryObservation::Exited,
                    crate::process_monitor::ProcessStatus::Unknown(_) => RecoveryObservation::Unknown,
                }
            }
            Err(_) => RecoveryObservation::Unknown,
        },
        _ => RecoveryObservation::Unknown,
    };
    let action = persistence::recovery_action(&journal, observation);
    match action {
        RecoveryAction::NoJournal => Ok(action),
        RecoveryAction::KeepRunning => Ok(action),
        RecoveryAction::PreserveExitedUnresolved => {
            journal.status = "interrupted_exit_unresolved".into();
            persistence::write_journal(directory, &journal)?;
            Ok(action)
        }
        RecoveryAction::PreserveUncertain => {
            journal.retain_uncertain(chrono::Local::now().to_rfc3339());
            persistence::write_journal(directory, &journal)?;
            Ok(action)
        }
        RecoveryAction::FinalizeInterrupted => {
            // Definite exit plus a durable elapsed checkpoint: commit once, marked
            // uncertain, and never double-count (the commit is session-ID idempotent).
            // The checkpoint is a lower bound — the app may have been down for an
            // unknown part of the session — so these minutes go into the uncertain
            // bucket and never into the exact playtime total. No exit timestamp is
            // invented: the last durable checkpoint time is used instead.
            let elapsed = journal
                .elapsed_seconds
                .ok_or("missing elapsed checkpoint for interrupted session")?;
            let minutes = elapsed.div_ceil(60) as u32;
            let games: Vec<crate::commands::Game> = match std::fs::read(directory.join("games.json")) {
                Ok(bytes) => serde_json::from_slice(&bytes).map_err(|error| error.to_string())?,
                Err(error) => return Err(error.to_string()),
            };
            let game = games
                .iter()
                .find(|game| game.id == journal.game_id)
                .ok_or("game not found in games.json")?;
            journal.finalization = Some(Finalization {
                duration_minutes: minutes,
                uncertain: true,
                status: "recovered_uncertain".into(),
                targets: AggregateTargets {
                    playtime_minutes: game.playtime_minutes,
                    uncertain_playtime_minutes: game.uncertain_playtime_minutes.saturating_add(minutes),
                    session_count: game.session_count.saturating_add(1),
                    last_played: Some(journal.checkpoint_at_utc.clone()),
                },
            });
            let row = HistoryRow {
                session_id: Some(journal.session_id.clone()),
                game_id: journal.game_id.clone(),
                started_at: journal.started_at_utc.clone(),
                minutes,
                duration_kind: DurationKind::Uncertain,
            };
            persistence::finalize(directory, &mut journal, row)?;
            Ok(action)
        }
        RecoveryAction::RetryFinalization => {
            let finalization = journal.finalization.as_ref().ok_or("missing finalization payload")?;
            let row = HistoryRow {
                session_id: Some(journal.session_id.clone()),
                game_id: journal.game_id.clone(),
                started_at: journal.started_at_utc.clone(),
                minutes: finalization.duration_minutes,
                duration_kind: if finalization.uncertain {
                    DurationKind::Uncertain
                } else {
                    DurationKind::Exact
                },
            };
            persistence::finalize(directory, &mut journal, row)?;
            Ok(action)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::commands::Game;
    use crate::gaming_session_persistence::{AggregateTargets, Finalization, GamingSessionJournal, HistoryRow};
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    #[test]
    fn repeated_startup_reconciliation_replays_finalization_once() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("silo-recovery-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        let mut journal = GamingSessionJournal::new("g1".into(), 0, "started".into());
        journal.finalization = Some(Finalization {
            duration_minutes: 3,
            uncertain: false,
            status: "completed".into(),
            targets: AggregateTargets {
                playtime_minutes: 13,
                uncertain_playtime_minutes: 0,
                session_count: 4,
                last_played: Some("finished".into()),
            },
        });
        persistence::write_journal(&directory, &journal).unwrap();
        let game = Game {
            id: "g1".into(),
            name: "Game".into(),
            exe_path: Some("game.exe".into()),
            wallpaper: None,
            logo_path: None,
            font_family: None,
            font_color: None,
            playtime_minutes: 10,
            uncertain_playtime_minutes: 0,
            session_count: 3,
            last_played: None,
            status: None,
            added_at: "added".into(),
            save_path: None,
            save_path_source: None,
            backup_count: None,
            is_installed: None,
            favorite: false,
            tags: vec![],
            launch_target: None,
            launch_child_path: None,
            exe_dir: None,
        };
        fs::write(directory.join("games.json"), serde_json::to_vec(&[game]).unwrap()).unwrap();

        assert_eq!(reconcile(&directory).unwrap(), RecoveryAction::RetryFinalization);
        assert_eq!(reconcile(&directory).unwrap(), RecoveryAction::NoJournal);
        let games: Vec<Game> = serde_json::from_slice(&fs::read(directory.join("games.json")).unwrap()).unwrap();
        let history: Vec<HistoryRow> =
            serde_json::from_slice(&fs::read(directory.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(games[0].playtime_minutes, 13);
        assert_eq!(games[0].session_count, 4);
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].duration_kind, DurationKind::Exact);
        fs::remove_dir_all(directory).unwrap();
    }

    // The reopen step proves identity through Windows process tracking; on
    // other platforms reopen_persisted deliberately reports Unsupported, so
    // this Windows-only finalize path has no equivalent to assert.
    #[cfg(target_os = "windows")]
    #[test]
    fn an_interrupted_session_commits_only_uncertain_minutes_and_never_twice() {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let directory = std::env::temp_dir().join(format!("silo-recovery-uncertain-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&directory).unwrap();
        // A pinned journal whose target is gone, with a durable checkpoint.
        // The PID is deliberately outside the range Windows can hand out, so
        // reopening it deterministically reports "exited".
        let mut journal = GamingSessionJournal::new("g1".into(), 11, "started".into());
        journal.game_pid = Some(u32::MAX - 1);
        journal.process_created_at = Some("1".into());
        journal.checkpoint(90, "2026-11-04T10:01:30Z".into());
        persistence::write_journal(&directory, &journal).unwrap();
        let game = Game {
            id: "g1".into(),
            name: "Game".into(),
            exe_path: Some("game.exe".into()),
            wallpaper: None,
            logo_path: None,
            font_family: None,
            font_color: None,
            playtime_minutes: 10,
            uncertain_playtime_minutes: 0,
            session_count: 3,
            last_played: None,
            status: None,
            added_at: "added".into(),
            save_path: None,
            save_path_source: None,
            backup_count: None,
            is_installed: None,
            favorite: false,
            tags: vec![],
            launch_target: None,
            launch_child_path: None,
            exe_dir: None,
        };
        fs::write(directory.join("games.json"), serde_json::to_vec(&[game]).unwrap()).unwrap();

        // PID 4242 does not exist, so the observation is a definite exit.
        assert_eq!(reconcile(&directory).unwrap(), RecoveryAction::FinalizeInterrupted);
        assert_eq!(reconcile(&directory).unwrap(), RecoveryAction::NoJournal);
        let games: Vec<Game> = serde_json::from_slice(&fs::read(directory.join("games.json")).unwrap()).unwrap();
        let history: Vec<HistoryRow> =
            serde_json::from_slice(&fs::read(directory.join("sessions.json")).unwrap()).unwrap();
        assert_eq!(
            games[0].playtime_minutes, 10,
            "an uncertain session must not inflate the exact playtime total"
        );
        assert_eq!(games[0].uncertain_playtime_minutes, 2, "90 s rounds up to 2 minutes");
        assert_eq!(games[0].session_count, 4);
        assert_eq!(history.len(), 1, "a retry must not duplicate the history row");
        assert_eq!(history[0].duration_kind, DurationKind::Uncertain);
        fs::remove_dir_all(directory).unwrap();
    }
}
