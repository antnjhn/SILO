#![cfg(all(windows, feature = "process-test-fixture"))]

//! Coverage for the Gaming Mode command tail: launcher restore ordering, SaveGuard
//! completion delegation, aggregate finalization, history persistence, the
//! playtime-updated notification and refusal to re-finalize a cleared session.
//!
//! The host here is a recording test double. These tests prove the tail's ordering and
//! real file effects; they do NOT prove that the Tauri window or event system works
//! (that is exercised only by the live application run).

use app_lib::gaming_session_monitor::MonitoredSession;
use app_lib::gaming_session_persistence::{self as persistence, DurationKind, GamingSessionJournal, RecoveryAction};
use app_lib::gaming_session_recovery::reconcile;
use app_lib::gaming_session_tail::{
    run_session_tail, GameTotals, SaveCompletion, SessionTail, SessionTailHost, TailOutcome,
};
use serde_json::Value;
use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

struct TestDir(Option<PathBuf>);

impl TestDir {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("silo-tail-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(Some(path))
    }

    fn path(&self) -> &std::path::Path {
        self.0.as_deref().unwrap()
    }
}

impl Drop for TestDir {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[derive(Default)]
struct RecordingHost {
    calls: Mutex<Vec<String>>,
    completions: Mutex<Vec<(bool, bool, Option<PathBuf>)>>,
    app_data: PathBuf,
}

impl SessionTailHost for RecordingHost {
    fn hide_launcher(&self) {}

    fn restore_launcher(&self) {
        self.calls.lock().unwrap().push("restore".into());
    }

    fn complete_save_guard(&self, request: SaveCompletion) {
        self.calls.lock().unwrap().push("save".into());
        self.completions
            .lock()
            .unwrap()
            .push((request.watch_for_saves, request.run_detection, request.detected_path));
    }

    fn game_totals(&self, game_id: &str) -> Option<GameTotals> {
        let games = read_games(&self.app_data);
        games.iter().find(|game| game["id"] == game_id).map(|game| GameTotals {
            playtime_minutes: game["playtimeMinutes"].as_u64().unwrap_or(0) as u32,
            uncertain_playtime_minutes: game["uncertainPlaytimeMinutes"].as_u64().unwrap_or(0) as u32,
            session_count: game["sessionCount"].as_u64().unwrap_or(0) as u32,
        })
    }

    fn emit_playtime_updated(&self, game_id: &str) {
        self.calls.lock().unwrap().push(format!("emit:{game_id}"));
    }
}

fn read_games(app_data: &std::path::Path) -> Vec<Value> {
    serde_json::from_slice(&fs::read(app_data.join("games.json")).unwrap()).unwrap()
}

fn write_games(app_data: &std::path::Path, games: Vec<Value>) {
    fs::write(app_data.join("games.json"), serde_json::to_vec_pretty(&games).unwrap()).unwrap();
}

fn game(id: &str) -> Value {
    serde_json::json!({
        "id": id,
        "name": "Tail Fixture Game",
        "exePath": null,
        "playtimeMinutes": 5,
        "sessionCount": 2,
        "addedAt": "added-at",
        "favorite": true,
        "tags": ["rpg", "owned"],
        "fontColor": "#ff8800",
        "savePath": "C:\\saves\\Tail Fixture Game",
        "backupCount": 7,
        "launchTarget": "game"
    })
}

fn tail(app_data: &std::path::Path, game_id: &str) -> SessionTail {
    SessionTail {
        app_data: app_data.to_path_buf(),
        game_id: game_id.to_string(),
        initial_save_path: None,
        name_keys: vec!["tail fixture game".into()],
        executable: "C:\\fixture\\game.exe".into(),
        backup_count: 7,
        watch_for_saves: true,
        run_detection: true,
    }
}

fn session(app_data: &std::path::Path, game_id: &str, elapsed_secs: u64, uncertain: bool) -> MonitoredSession {
    session_with_id(app_data, game_id, elapsed_secs, uncertain, None)
}

fn session_with_id(
    _app_data: &std::path::Path,
    game_id: &str,
    elapsed_secs: u64,
    uncertain: bool,
    session_id: Option<&str>,
) -> MonitoredSession {
    let mut journal = GamingSessionJournal::new(game_id.into(), 4242, "2026-11-04T10:00:00Z".into());
    if let Some(session_id) = session_id {
        journal.session_id = session_id.to_string();
    }
    journal.pin_target(&app_lib::process_monitor::ProcessIdentity {
        pid: 4242,
        creation_filetime: 7,
        executable: Some("C:\\fixture\\game.exe".into()),
    });
    let duration_kind = if uncertain {
        DurationKind::Uncertain
    } else {
        DurationKind::Exact
    };
    if duration_kind.is_exact() {
        journal.measure(elapsed_secs);
    }
    MonitoredSession {
        journal,
        target_started_at: "2026-11-04T10:00:00Z".into(),
        started_sys: SystemTime::now(),
        elapsed_secs,
        exit_code: Some(0),
        detected_path: None,
        detected_roots: HashSet::new(),
        duration_kind,
        guard: None,
    }
}

/// The tail refuses to commit unless the durable journal on disk is this session's,
/// so every "still running" fixture journal is written for real.
fn written(app_data: &std::path::Path, session: MonitoredSession) -> MonitoredSession {
    persistence::write_journal(app_data, &session.journal).unwrap();
    session
}

#[test]
fn tail_finalizes_playtime_history_and_preserves_game_metadata() {
    let dir = TestDir::new("finalize");
    let app_data = dir.path().join("appdata");
    fs::create_dir_all(&app_data).unwrap();
    write_games(&app_data, vec![game("tail-game")]);

    let host = RecordingHost {
        app_data: app_data.clone(),
        ..Default::default()
    };
    let session = written(&app_data, session(&app_data, "tail-game", 125, false));
    let session_id = session.journal.session_id.clone();

    let outcome = run_session_tail(tail(&app_data, "tail-game"), session, &host);
    assert_eq!(outcome, TailOutcome::Finalized { minutes: 3 });

    let games = read_games(&app_data);
    assert_eq!(games[0]["playtimeMinutes"], 8, "5 existing + ceil(125s/60)");
    assert_eq!(
        games[0]["uncertainPlaytimeMinutes"], 0,
        "an exact session leaves the uncertain bucket untouched"
    );
    assert_eq!(games[0]["sessionCount"], 3);
    assert_eq!(games[0]["favorite"], true);
    assert_eq!(games[0]["tags"][1], "owned");
    assert_eq!(games[0]["fontColor"], "#ff8800");
    assert_eq!(games[0]["savePath"], "C:\\saves\\Tail Fixture Game");
    assert_eq!(games[0]["backupCount"], 7);
    assert_eq!(games[0]["launchTarget"], "game");

    let history: Vec<Value> = serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(history[0]["sessionId"], session_id.as_str());
    assert_eq!(history[0]["minutes"], 3);
    assert!(
        !persistence::journal_path(&app_data).exists(),
        "a committed session must clear its journal"
    );

    let calls = host.calls.lock().unwrap().clone();
    assert_eq!(
        calls,
        vec!["restore".to_string(), "save".to_string(), "emit:tail-game".to_string()],
        "restore happens before SaveGuard completion, and the event after the commit"
    );
    let completions = host.completions.lock().unwrap().clone();
    assert_eq!(completions.len(), 1);
    assert_eq!(completions[0].0, true);
    assert_eq!(completions[0].1, true);

    assert_eq!(reconcile(&app_data).unwrap(), RecoveryAction::NoJournal);
}

#[test]
fn tail_refuses_to_finalize_a_session_whose_journal_is_already_gone() {
    let dir = TestDir::new("repeat");
    let app_data = dir.path().join("appdata");
    fs::create_dir_all(&app_data).unwrap();
    write_games(&app_data, vec![game("tail-game")]);

    let host = RecordingHost {
        app_data: app_data.clone(),
        ..Default::default()
    };
    let first_session = written(&app_data, session(&app_data, "tail-game", 60, false));
    let first_id = first_session.journal.session_id.clone();
    let first = run_session_tail(tail(&app_data, "tail-game"), first_session, &host);
    assert_eq!(first, TailOutcome::Finalized { minutes: 1 });

    // A duplicate monitor replaying the same session must not commit again.
    let duplicate = session_with_id(&app_data, "tail-game", 60, false, Some(&first_id));
    let second = run_session_tail(tail(&app_data, "tail-game"), duplicate, &host);
    assert_eq!(second, TailOutcome::AlreadyFinalized);

    let games = read_games(&app_data);
    assert_eq!(games[0]["playtimeMinutes"], 6, "no double counting");
    assert_eq!(games[0]["sessionCount"], 3);
    let history: Vec<Value> = serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 1);
    assert_eq!(host.completions.lock().unwrap().len(), 1, "SaveGuard ran once only");
}

#[test]
fn tail_retains_evidence_when_the_game_is_missing() {
    let dir = TestDir::new("missing-game");
    let app_data = dir.path().join("appdata");
    fs::create_dir_all(&app_data).unwrap();
    write_games(&app_data, vec![]);

    let host = RecordingHost {
        app_data: app_data.clone(),
        ..Default::default()
    };
    let outcome = run_session_tail(
        tail(&app_data, "tail-game"),
        written(&app_data, session(&app_data, "tail-game", 60, false)),
        &host,
    );
    assert_eq!(outcome, TailOutcome::GameMissing);
    assert!(
        persistence::journal_path(&app_data).exists(),
        "unresolved evidence must be retained"
    );
    assert!(host.calls.lock().unwrap().iter().all(|call| call != "emit:tail-game"));
}

#[test]
fn reattached_durations_are_committed_as_uncertain() {
    let dir = TestDir::new("reattached");
    let app_data = dir.path().join("appdata");
    fs::create_dir_all(&app_data).unwrap();
    write_games(&app_data, vec![game("tail-game")]);

    let host = RecordingHost {
        app_data: app_data.clone(),
        ..Default::default()
    };
    let outcome = run_session_tail(
        tail(&app_data, "tail-game"),
        written(&app_data, session(&app_data, "tail-game", 90, true)),
        &host,
    );
    assert_eq!(outcome, TailOutcome::Finalized { minutes: 2 });

    let games = read_games(&app_data);
    assert_eq!(
        games[0]["playtimeMinutes"], 5,
        "an uncertain duration must never inflate the exact playtime total"
    );
    assert_eq!(
        games[0]["uncertainPlaytimeMinutes"], 2,
        "the uncertain minutes are kept in their own bucket"
    );
    assert_eq!(games[0]["sessionCount"], 3, "the session still counts as a session");
    let history: Vec<Value> = serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 1, "one row, marked by the commit, never duplicated");
    assert_eq!(history[0]["minutes"], 2);
    assert_eq!(
        history[0]["durationKind"], "uncertain",
        "the session history keeps the uncertainty marker"
    );
}

/// Objective 2: an uncertain session and an exact session are accounted separately,
/// and a retry never double-counts either one.
#[test]
fn exact_and_uncertain_sessions_are_accounted_in_separate_buckets() {
    let dir = TestDir::new("buckets");
    let app_data = dir.path().join("appdata");
    fs::create_dir_all(&app_data).unwrap();
    write_games(&app_data, vec![game("tail-game")]);

    let host = RecordingHost {
        app_data: app_data.clone(),
        ..Default::default()
    };
    let exact = written(&app_data, session(&app_data, "tail-game", 125, false));
    let exact_id = exact.journal.session_id.clone();
    assert_eq!(
        run_session_tail(tail(&app_data, "tail-game"), exact, &host),
        TailOutcome::Finalized { minutes: 3 }
    );

    let uncertain = written(&app_data, session(&app_data, "tail-game", 90, true));
    let uncertain_id = uncertain.journal.session_id.clone();
    assert_eq!(
        run_session_tail(tail(&app_data, "tail-game"), uncertain, &host),
        TailOutcome::Finalized { minutes: 2 }
    );

    let games = read_games(&app_data);
    assert_eq!(games[0]["playtimeMinutes"], 8, "5 existing + 3 exact minutes");
    assert_eq!(
        games[0]["uncertainPlaytimeMinutes"], 2,
        "only the lower bound is uncertain"
    );
    assert_eq!(games[0]["sessionCount"], 4);

    let history: Vec<Value> = serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 2);
    assert_eq!(history[0]["sessionId"], exact_id.as_str());
    assert!(history[0].get("durationKind").is_none(), "an exact row needs no marker");
    assert_eq!(history[1]["sessionId"], uncertain_id.as_str());
    assert_eq!(history[1]["durationKind"], "uncertain");

    // The real replay is the crash boundary: the earlier commit wrote the journal
    // with its finalization payload but was interrupted before removing it. The tail
    // must replay that persisted snapshot — recomputing totals would count the
    // session a second time — and finish the interrupted commit.
    let mut replay = session_with_id(&app_data, "tail-game", 90, true, Some(&uncertain_id));
    replay.journal.finalization = Some(persistence::Finalization {
        duration_minutes: 2,
        uncertain: true,
        status: "recovered_uncertain".into(),
        targets: persistence::AggregateTargets {
            playtime_minutes: 8,
            uncertain_playtime_minutes: 2,
            session_count: 4,
            last_played: Some("2026-11-04T10:05:00Z".into()),
        },
    });
    persistence::write_journal(&app_data, &replay.journal).unwrap();
    assert_eq!(
        run_session_tail(tail(&app_data, "tail-game"), replay, &host),
        TailOutcome::AlreadyFinalized,
        "a committed snapshot is replayed, never recommitted"
    );
    assert!(
        !persistence::journal_path(&app_data).exists(),
        "the replay finishes the commit the crash interrupted"
    );
    let games = read_games(&app_data);
    assert_eq!(games[0]["playtimeMinutes"], 8, "no double counting");
    assert_eq!(games[0]["uncertainPlaytimeMinutes"], 2, "no double counting");
    assert_eq!(games[0]["sessionCount"], 4);
    let history: Vec<Value> = serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 2, "the history row is never duplicated");
}
