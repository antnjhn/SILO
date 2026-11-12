#![cfg(all(windows, feature = "process-test-fixture"))]

//! Stage 4 coverage for the production Gaming Mode launch → pin → monitor stage.
//!
//! These tests call `gaming_session_monitor::monitor_session`, the exact function the
//! opt-in `launch_game` worker runs, against real fixture processes copied per test.
//! Tauri-only steps (window hide/show, SaveGuard completion, aggregate finalization)
//! stay in `commands.rs`; they are NOT asserted here and are documented as remaining
//! Stage 4 gaps.

use app_lib::game_launch::{spawn_direct_game, TargetExpectation};
use app_lib::gaming_session_monitor::{
    monitor_session, reattach_session, LaunchMonitorConfig, MonitorOutcome, ReattachConfig,
};
use app_lib::gaming_session_persistence::{self as persistence, DurationKind, GamingSessionJournal, RecoveryAction};
use app_lib::gaming_session_recovery::{reconcile, take_running_session};
use app_lib::gaming_session_tail::{
    run_session_tail, GameTotals, SaveCompletion, SessionTail, SessionTailHost, TailOutcome,
};
use app_lib::process_monitor::TrackedProcess;
use serde::Deserialize;
use std::cell::Cell;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const FIXTURE_EXE: &str = env!("CARGO_BIN_EXE_process-test-fixture");

struct TestTemp(Option<PathBuf>);

impl TestTemp {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("silo-gaming-mode-{label}-{}-{nonce}", std::process::id()));
        fs::create_dir_all(&path).unwrap();
        Self(Some(path))
    }

    fn path(&self, name: &str) -> PathBuf {
        self.0.as_ref().unwrap().join(name)
    }

    fn root(&self) -> &Path {
        self.0.as_deref().unwrap()
    }

    fn cleanup(&mut self) {
        if let Some(path) = self.0.take() {
            // A fixture process that is still exiting can briefly hold its own
            // executable open, which makes `remove_dir_all` fail with access
            // denied. Retry for a bounded time instead of turning teardown into
            // a flaky product assertion.
            let mut last_error = None;
            for _ in 0..30 {
                match fs::remove_dir_all(&path) {
                    Ok(()) => return,
                    Err(_error) if !path.exists() => return,
                    Err(error) => {
                        last_error = Some(error);
                        thread::sleep(Duration::from_millis(100));
                    }
                }
            }
            // The product behaviour under test has already been asserted; leave
            // the directory for `Drop` (best effort) and report the leak.
            eprintln!(
                "warning: could not remove gaming mode test directory {}: {}",
                path.display(),
                last_error.map(|error| error.to_string()).unwrap_or_default()
            );
        }
    }
}

impl Drop for TestTemp {
    fn drop(&mut self) {
        if let Some(path) = self.0.take() {
            let _ = fs::remove_dir_all(path);
        }
    }
}

#[derive(Deserialize)]
struct ChildReport {
    pid: u32,
}

#[derive(Deserialize)]
struct LauncherReport {
    launcher_pid: u32,
    child_pid: u32,
}

/// Copies the fixture and writes its no-argument behaviour next to the copy.
fn fixture_at(temp: &TestTemp, config: serde_json::Value) -> PathBuf {
    let exe = temp.path("程序 files\\game monitor fixture.exe");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::copy(FIXTURE_EXE, &exe).unwrap();
    fs::write(exe.parent().unwrap().join("fixture.json"), config.to_string()).unwrap();
    exe
}

/// A save folder that already exists and names the game, so the monitor installs no
/// filesystem watcher over the machine's real save locations during tests.
fn settled_save_path(temp: &TestTemp) -> (Option<String>, Vec<String>) {
    let saves = temp.path("SiloMonitorGame");
    fs::create_dir_all(&saves).unwrap();
    (
        Some(saves.to_string_lossy().into_owned()),
        vec!["silomonitorgame".to_string()],
    )
}

fn base_config(temp: &TestTemp, executable: PathBuf, expectation: TargetExpectation) -> LaunchMonitorConfig {
    let (initial_save_path, name_keys) = settled_save_path(temp);
    LaunchMonitorConfig {
        app_data: temp.path("appdata"),
        game_id: "monitor-game".into(),
        executable,
        expectation,
        cwd: temp.root().to_path_buf(),
        initial_save_path,
        name_keys,
        discovery_timeout: Duration::from_secs(5),
        poll_interval: Duration::from_millis(60),
        checkpoint_interval: Duration::from_millis(400),
    }
}

fn wait_for_file(path: &Path, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if path.exists() {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    path.exists()
}

/// The single-active-session reservation is process-wide, so monitor tests in this
/// binary take turns instead of rejecting each other's launches.
static SESSION_LOCK: Mutex<()> = Mutex::new(());

fn serial() -> std::sync::MutexGuard<'static, ()> {
    SESSION_LOCK.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

#[test]
fn direct_launch_pins_identity_checkpoints_elapsed_and_reports_exit_code() {
    let _serial = serial();
    let mut temp = TestTemp::new("direct");
    let exe_dir = temp.path("程序 files");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "child report.json", "exit": 37, "delayMs": 2200 }),
    );

    let mut config = base_config(&temp, executable.clone(), TargetExpectation::SpawnedProcess);
    config.checkpoint_interval = Duration::from_millis(400);
    let journal_file = persistence::journal_path(&config.app_data);

    let stop = Arc::new(AtomicBool::new(false));
    let max_elapsed = Arc::new(AtomicU64::new(0));
    let watcher = {
        let journal_file = journal_file.clone();
        let stop = Arc::clone(&stop);
        let max_elapsed = Arc::clone(&max_elapsed);
        thread::spawn(move || {
            while !stop.load(Ordering::Relaxed) {
                if let Ok(bytes) = fs::read(&journal_file) {
                    if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        if let Some(seconds) = value.get("elapsedSeconds").and_then(|value| value.as_u64()) {
                            max_elapsed.fetch_max(seconds, Ordering::Relaxed);
                        }
                    }
                }
                thread::sleep(Duration::from_millis(50));
            }
        })
    };

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    stop.store(true, Ordering::Relaxed);
    watcher.join().unwrap();

    let MonitorOutcome::Ended(session) = outcome else {
        panic!("expected a definite end from a running fixture, got {outcome:?}");
    };
    assert_eq!(pinned_calls.get(), 1, "launcher effects run once, only after pinning");
    assert_eq!(
        session.exit_code,
        Some(37),
        "the production stage must observe a non-zero exit"
    );
    assert!(
        session.elapsed_secs >= 2,
        "session must measure the tracked target's lifetime"
    );
    assert!(
        max_elapsed.load(Ordering::Relaxed) >= 1,
        "durable checkpoints must be written while the target is still running"
    );

    let report_path = exe_dir.join("child report.json");
    assert!(wait_for_file(&report_path, Duration::from_secs(1)));
    let report: ChildReport = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();

    let journal = persistence::load_journal(&config.app_data)
        .unwrap()
        .expect("an ended session leaves its pinned journal for the command tail to finalize");
    assert_eq!(journal.status, "active");
    assert_eq!(journal.game_pid, Some(report.pid), "journal must pin the real target");
    assert!(
        journal.process_created_at.is_some(),
        "identity needs the creation FILETIME"
    );
    assert!(
        journal.elapsed_seconds.unwrap_or(0) >= 2,
        "final elapsed must be persisted before finalization"
    );
    drop(journal);
    temp.cleanup();
}

#[test]
fn launcher_handoff_pins_the_descendant_and_ignores_launcher_exit() {
    let _serial = serial();
    let mut temp = TestTemp::new("handoff");
    let exe_dir = temp.path("程序 files");
    let executable = fixture_at(
        &temp,
        serde_json::json!({
            "mode": "launcher",
            "parentReport": "launcher report.json",
            "childReport": "child report.json",
            "delayMs": 1800
        }),
    );
    let mut config = base_config(
        &temp,
        executable.clone(),
        TargetExpectation::ChildExecutable(executable.clone()),
    );
    config.poll_interval = Duration::from_millis(100);

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    let MonitorOutcome::Ended(session) = outcome else {
        panic!("expected the handoff target to be tracked to its own exit, got {outcome:?}");
    };
    assert_eq!(pinned_calls.get(), 1);
    assert!(
        session.elapsed_secs >= 1,
        "the child, not the launcher, defines the session"
    );

    let launcher: LauncherReport =
        serde_json::from_slice(&fs::read(exe_dir.join("launcher report.json")).unwrap()).unwrap();
    let journal = persistence::load_journal(&config.app_data)
        .unwrap()
        .expect("ended handoff session must leave its journal");
    assert_eq!(
        journal.game_pid,
        Some(launcher.child_pid),
        "the game, not the launcher, must be pinned"
    );
    assert_ne!(
        journal.game_pid,
        Some(launcher.launcher_pid),
        "the launcher PID must never be treated as the game"
    );
    drop(journal);
    temp.cleanup();
}

#[test]
fn missing_executable_fails_launch_without_leaving_a_journal() {
    let _serial = serial();
    let mut temp = TestTemp::new("launch-failure");
    let executable = temp.path("does not exist\\missing game.exe");
    let config = base_config(&temp, executable.clone(), TargetExpectation::SpawnedProcess);

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    assert!(
        matches!(outcome, MonitorOutcome::LaunchFailed),
        "spawn failure must be reported explicitly, got {outcome:?}"
    );
    assert_eq!(pinned_calls.get(), 0, "no launcher effects on a failed launch");
    assert!(
        !persistence::journal_path(&config.app_data).exists(),
        "a failed launch must not leave session evidence behind"
    );
    temp.cleanup();
}

#[test]
fn unpinnable_target_is_uncertain_and_retains_the_journal() {
    let _serial = serial();
    let mut temp = TestTemp::new("uncertain");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 900 }),
    );
    // Launcher mode with a target path that never appears: the root is spawnable but no
    // descendant identity can be established.
    let mut config = base_config(
        &temp,
        executable.clone(),
        TargetExpectation::ChildExecutable(temp.path("never-appears.exe")),
    );
    config.discovery_timeout = Duration::from_millis(500);

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    assert!(
        matches!(outcome, MonitorOutcome::TrackingUncertain),
        "an unpinned target must never be reported as a clean session end, got {outcome:?}"
    );
    assert_eq!(
        pinned_calls.get(),
        0,
        "the launcher must not be hidden without a pinned game"
    );

    let journal = persistence::load_journal(&config.app_data)
        .unwrap()
        .expect("uncertain tracking must retain journal evidence");
    assert_eq!(journal.status, "tracking_uncertain");
    assert!(journal.game_pid.is_none(), "no identity may be invented");
    drop(journal);

    // Let the fixture finish before removing its directory.
    thread::sleep(Duration::from_millis(1200));
    temp.cleanup();
}

#[test]
fn interrupted_session_with_checkpoint_finalizes_once_after_definite_exit() {
    let _serial = serial();
    let mut temp = TestTemp::new("recovery-finalize");
    let exe_dir = temp.path("程序 files");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 1500 }),
    );
    let report_path = exe_dir.join("report.json");

    let args = vec![
        OsString::from("direct"),
        report_path.as_os_str().to_os_string(),
        OsString::from("0"),
        OsString::from("1500"),
    ];
    let app_lib::game_launch::DirectSpawnOutcome::Tracked(process) =
        spawn_direct_game(&executable, temp.root(), &args, true).unwrap()
    else {
        panic!("fixture must be trackable");
    };

    let app_data = temp.path("appdata");
    fs::create_dir_all(&app_data).unwrap();
    fs::write(
        app_data.join("games.json"),
        serde_json::json!([{
            "id": "monitor-game",
            "name": "Monitor Game",
            "exePath": null,
            "playtimeMinutes": 10,
            "sessionCount": 3,
            "addedAt": "added-at",
            "favorite": true,
            "tags": ["rpg", "owned"],
            "fontColor": "#ff8800",
            "savePath": "C:\\saves\\Monitor Game",
            "backupCount": 7,
            "launchTarget": "game"
        }])
        .to_string(),
    )
    .unwrap();

    let mut journal = GamingSessionJournal::new("monitor-game".into(), process.identity.pid, "started".into());
    journal.pin_target(&process.identity);
    journal.checkpoint(90, "checkpoint".into());
    persistence::write_journal(&app_data, &journal).unwrap();

    let deadline = Instant::now() + Duration::from_secs(10);
    while process.exit_code().unwrap().is_none() {
        assert!(Instant::now() < deadline, "fixture did not exit");
        thread::sleep(Duration::from_millis(20));
    }

    assert_eq!(
        reconcile(&app_data).unwrap(),
        RecoveryAction::FinalizeInterrupted,
        "a definite exit with a durable checkpoint must be finalized, not parked forever"
    );
    assert_eq!(
        reconcile(&app_data).unwrap(),
        RecoveryAction::NoJournal,
        "recovery must clear the journal and never replay the commit"
    );

    let games: serde_json::Value = serde_json::from_slice(&fs::read(app_data.join("games.json")).unwrap()).unwrap();
    let game = &games[0];
    assert_eq!(
        game["playtimeMinutes"], 10,
        "an interrupted session's checkpoint is a lower bound, so it must not inflate the exact total"
    );
    assert_eq!(
        game["uncertainPlaytimeMinutes"], 2,
        "90 checkpointed seconds round up to 2 uncertain minutes"
    );
    assert_eq!(game["sessionCount"], 4);
    assert_eq!(game["favorite"], true, "finalization must preserve metadata");
    assert_eq!(game["tags"][1], "owned");
    assert_eq!(game["fontColor"], "#ff8800");
    assert_eq!(game["backupCount"], 7);
    assert_eq!(game["launchTarget"], "game");

    let history: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 1, "exactly one history row");
    assert_eq!(history[0]["minutes"], 2);
    assert_eq!(history[0]["gameId"], "monitor-game");
    assert_eq!(
        history[0]["durationKind"], "uncertain",
        "the history row keeps the uncertainty marker"
    );
    drop(process);
    temp.cleanup();
}
/// Records the commit-time host effects without Tauri, so these tests prove the
/// finalization file effects rather than the real window or event system.
struct FinalizeHost {
    app_data: PathBuf,
    emitted: Mutex<Vec<String>>,
}

impl FinalizeHost {
    fn new(app_data: PathBuf) -> Self {
        Self {
            app_data,
            emitted: Mutex::new(Vec::new()),
        }
    }
}

impl SessionTailHost for FinalizeHost {
    fn hide_launcher(&self) {}

    fn restore_launcher(&self) {}

    fn complete_save_guard(&self, _request: SaveCompletion) {}

    fn game_totals(&self, game_id: &str) -> Option<GameTotals> {
        let value: serde_json::Value =
            serde_json::from_slice(&fs::read(self.app_data.join("games.json")).unwrap()).unwrap();
        value
            .as_array()?
            .iter()
            .find(|game| game["id"] == game_id)
            .map(|game| GameTotals {
                playtime_minutes: game["playtimeMinutes"].as_u64().unwrap_or(0) as u32,
                uncertain_playtime_minutes: game["uncertainPlaytimeMinutes"].as_u64().unwrap_or(0) as u32,
                session_count: game["sessionCount"].as_u64().unwrap_or(0) as u32,
            })
    }

    fn emit_playtime_updated(&self, game_id: &str) {
        self.emitted.lock().unwrap().push(game_id.to_string());
    }
}

fn write_monitor_game(app_data: &Path) {
    fs::create_dir_all(app_data).unwrap();
    fs::write(
        app_data.join("games.json"),
        serde_json::json!([{
            "id": "monitor-game",
            "name": "Monitor Game",
            "exePath": null,
            "playtimeMinutes": 10,
            "sessionCount": 3,
            "addedAt": "added-at",
            "favorite": true,
            "tags": ["rpg", "owned"],
            "fontColor": "#ff8800",
            "savePath": "C:\\saves\\Monitor Game",
            "backupCount": 7,
            "launchTarget": "game"
        }])
        .to_string(),
    )
    .unwrap();
}

fn wait_for_pinned_journal(app_data: &Path, timeout: Duration) -> GamingSessionJournal {
    let deadline = Instant::now() + timeout;
    loop {
        if let Ok(Some(journal)) = persistence::load_journal(app_data) {
            if journal.game_pid.is_some() {
                return journal;
            }
        }
        assert!(Instant::now() < deadline, "journal was never pinned");
        thread::sleep(Duration::from_millis(20));
    }
}

fn tail_for(app_data: &Path, executable: &Path) -> SessionTail {
    SessionTail {
        app_data: app_data.to_path_buf(),
        game_id: "monitor-game".into(),
        initial_save_path: None,
        name_keys: Vec::new(),
        executable: executable.to_string_lossy().into_owned(),
        backup_count: 7,
        watch_for_saves: false,
        run_detection: false,
    }
}

fn reattach_config(app_data: &Path) -> ReattachConfig {
    ReattachConfig {
        app_data: app_data.to_path_buf(),
        poll_interval: Duration::from_millis(60),
        checkpoint_interval: Duration::from_millis(300),
    }
}

/// Objective 2: an app restart reattaches to the still-running game, keeps
/// checkpointing, then commits once through the existing tail.
#[test]
fn restart_reattaches_to_the_still_running_game_and_finalizes_once() {
    let _serial = serial();
    let temp = TestTemp::new("reattach");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 2200 }),
    );
    let app_data = temp.path("appdata");
    write_monitor_game(&app_data);

    let app_lib::game_launch::DirectSpawnOutcome::Tracked(process) =
        spawn_direct_game(&executable, temp.root(), &[], true).unwrap()
    else {
        panic!("fixture must be trackable");
    };
    let game_pid = process.identity.pid;
    let mut journal = GamingSessionJournal::new("monitor-game".into(), game_pid, "started".into());
    journal.pin_target(&process.identity);
    persistence::write_journal(&app_data, &journal).unwrap();
    // The app exits: its retained handle is gone and only the journal survives.
    drop(process);

    let Some((journal, tracked)) = take_running_session(&app_data).unwrap() else {
        panic!("an identity-validated live target must be reattached");
    };
    assert_eq!(journal.game_pid, Some(game_pid));
    assert_eq!(journal.elapsed_seconds, None, "no elapsed time is invented on restart");

    let pinned = Cell::new(0);
    let outcome = reattach_session(reattach_config(&app_data), journal, tracked, &|| {
        pinned.set(pinned.get() + 1)
    });
    let MonitorOutcome::Ended(session) = outcome else {
        panic!("the reattached target must be tracked to its own exit, got {outcome:?}");
    };
    assert_eq!(pinned.get(), 1);
    assert_eq!(
        session.duration_kind,
        DurationKind::Uncertain,
        "downtime while the app was closed is unknowable, so the duration is only a lower bound"
    );
    assert!(
        session.elapsed_secs >= 2,
        "the reattached session keeps measuring elapsed time"
    );
    assert_eq!(
        session.journal.measured_seconds, None,
        "a reattached session never records an exact measurement"
    );
    let session_id = session.journal.session_id.clone();

    let host = FinalizeHost::new(app_data.clone());
    let TailOutcome::Finalized { minutes } = run_session_tail(tail_for(&app_data, &executable), *session, &host) else {
        panic!("a definite reattached exit must finalize");
    };
    assert!(minutes >= 1, "reattached elapsed time is still counted");
    assert_eq!(host.emitted.lock().unwrap().len(), 1);

    let games: serde_json::Value = serde_json::from_slice(&fs::read(app_data.join("games.json")).unwrap()).unwrap();
    assert_eq!(
        games[0]["playtimeMinutes"], 10,
        "an uncertain duration must never inflate the exact playtime total"
    );
    assert_eq!(
        games[0]["uncertainPlaytimeMinutes"], minutes,
        "the uncertain minutes are kept in their own bucket"
    );
    assert_eq!(games[0]["sessionCount"], 4);
    assert_eq!(games[0]["favorite"], true, "existing metadata is preserved");
    assert_eq!(games[0]["tags"][1], "owned");
    assert_eq!(games[0]["backupCount"], 7);

    let history: Vec<serde_json::Value> =
        serde_json::from_slice(&fs::read(app_data.join("sessions.json")).unwrap()).unwrap();
    assert_eq!(history.len(), 1, "exactly one history row");
    assert_eq!(history[0]["sessionId"], session_id.as_str());
    assert_eq!(history[0]["minutes"], minutes);
    assert_eq!(
        history[0]["durationKind"], "uncertain",
        "the session history keeps the uncertainty marker"
    );

    assert!(!persistence::journal_path(&app_data).exists());
    assert_eq!(reconcile(&app_data).unwrap(), RecoveryAction::NoJournal);
    assert!(
        take_running_session(&app_data).unwrap().is_none(),
        "a committed session must never be reattached again"
    );
}

/// Objective 2: the launcher process must not be mistaken for the game.
#[test]
fn reattachment_targets_the_pinned_game_and_ignores_the_launcher() {
    let _serial = serial();
    let temp = TestTemp::new("reattach-launcher");
    let exe_dir = temp.path("程序 files");
    let executable = fixture_at(
        &temp,
        serde_json::json!({
            "mode": "launcher",
            "parentReport": "launcher report.json",
            "childReport": "child report.json",
            "delayMs": 1800
        }),
    );
    let mut launcher = Command::new(&executable).current_dir(temp.root()).spawn().unwrap();
    let report_path = exe_dir.join("launcher report.json");
    assert!(wait_for_file(&report_path, Duration::from_secs(5)));
    let report: LauncherReport = loop {
        if let Ok(report) = serde_json::from_slice::<LauncherReport>(&fs::read(&report_path).unwrap()) {
            break report;
        }
        thread::sleep(Duration::from_millis(20));
    };
    assert_ne!(report.child_pid, report.launcher_pid);

    let identity = match TrackedProcess::open(report.child_pid) {
        Ok(tracked) => tracked.identity,
        Err(status) => panic!("the child must be openable while it runs, got {status:?}"),
    };
    let app_data = temp.path("appdata");
    write_monitor_game(&app_data);
    // The journal pins the game; the launch root is recorded separately.
    let mut journal = GamingSessionJournal::new("monitor-game".into(), report.launcher_pid, "started".into());
    journal.pin_target(&identity);
    persistence::write_journal(&app_data, &journal).unwrap();

    let Some((journal, tracked)) = take_running_session(&app_data).unwrap() else {
        panic!("the pinned child must be reattached");
    };
    assert_eq!(journal.game_pid, Some(report.child_pid));
    let outcome = reattach_session(reattach_config(&app_data), journal, tracked, &|| {});
    let MonitorOutcome::Ended(session) = outcome else {
        panic!("the child's exit must define the session end, got {outcome:?}");
    };
    assert_eq!(
        session.journal.game_pid,
        Some(report.child_pid),
        "the launcher pid must never own the session"
    );
    assert!(session.exit_code.is_some());
    let _ = launcher.wait();
}

/// Objective 2: an identity mismatch preserves the journal and commits nothing.
#[test]
fn identity_mismatch_preserves_the_journal_and_does_not_finalize() {
    let _serial = serial();
    let temp = TestTemp::new("reattach-mismatch");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 2500 }),
    );
    let app_data = temp.path("appdata");
    write_monitor_game(&app_data);

    let app_lib::game_launch::DirectSpawnOutcome::Tracked(process) =
        spawn_direct_game(&executable, temp.root(), &[], true).unwrap()
    else {
        panic!("fixture must be trackable");
    };
    let mut journal = GamingSessionJournal::new("monitor-game".into(), process.identity.pid, "started".into());
    journal.pin_target(&process.identity);
    // Same pid, different creation time: this is exactly the identity test, not an
    // induced OS pid reuse.
    let forged_creation = process.identity.creation_filetime.saturating_add(1).to_string();
    journal.process_created_at = Some(forged_creation.clone());
    persistence::write_journal(&app_data, &journal).unwrap();

    assert!(
        take_running_session(&app_data).unwrap().is_none(),
        "a pid with a different creation time is not the game"
    );
    assert_eq!(
        reconcile(&app_data).unwrap(),
        RecoveryAction::PreserveExitedUnresolved,
        "unproven identity must retain evidence"
    );
    let retained = persistence::load_journal(&app_data).unwrap().unwrap();
    assert!(retained.finalization.is_none());
    assert_eq!(retained.process_created_at, Some(forged_creation));
    assert!(!app_data.join("sessions.json").exists());
    drop(process);
}

/// Objective 2: repeated startup cannot create a second monitor for one journal.
#[test]
fn duplicate_reattachment_is_rejected_and_leaves_the_journal_untouched() {
    let _serial = serial();
    let temp = TestTemp::new("reattach-duplicate");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 2200 }),
    );
    let app_data = temp.path("appdata");
    write_monitor_game(&app_data);

    let app_lib::game_launch::DirectSpawnOutcome::Tracked(process) =
        spawn_direct_game(&executable, temp.root(), &[], true).unwrap()
    else {
        panic!("fixture must be trackable");
    };
    let mut journal = GamingSessionJournal::new("monitor-game".into(), process.identity.pid, "started".into());
    journal.pin_target(&process.identity);
    persistence::write_journal(&app_data, &journal).unwrap();
    let session_id = journal.session_id.clone();
    let game_pid = process.identity.pid;
    drop(process);

    let worker = {
        let app_data = app_data.clone();
        thread::spawn(move || {
            let Some((journal, tracked)) = take_running_session(&app_data).unwrap() else {
                panic!("the first startup must reattach");
            };
            reattach_session(reattach_config(&app_data), journal, tracked, &|| {})
        })
    };

    // Wait until the first monitor owns the reservation and re-marked the journal.
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        if let Ok(Some(current)) = persistence::load_journal(&app_data) {
            if current.status == "tracking_uncertain" {
                break;
            }
        }
        assert!(Instant::now() < deadline, "the first monitor never reattached");
        thread::sleep(Duration::from_millis(20));
    }

    let Some((second_journal, second_tracked)) = take_running_session(&app_data).unwrap() else {
        panic!("the game is still running, so a second startup still finds it");
    };
    let second = reattach_session(reattach_config(&app_data), second_journal, second_tracked, &|| {});
    assert!(
        matches!(second, MonitorOutcome::SessionAlreadyActive),
        "a second monitor for the same journal must be refused"
    );

    let current = persistence::load_journal(&app_data).unwrap().unwrap();
    assert_eq!(current.session_id, session_id, "the active journal is not replaced");
    assert_eq!(current.game_pid, Some(game_pid));
    assert_eq!(current.game_pid, Some(current_pid(&current)));
    assert_eq!(current.status, "tracking_uncertain");

    let first = worker.join().unwrap();
    assert!(matches!(first, MonitorOutcome::Ended(_)));
}

fn current_pid(journal: &GamingSessionJournal) -> u32 {
    journal.game_pid.expect("journal keeps its pinned pid")
}

/// Objective 2: the reservation outlives monitoring and is released only once the
/// session has been committed.
#[test]
fn reattachment_reservation_is_released_only_after_finalization() {
    let _serial = serial();
    let temp = TestTemp::new("reattach-reservation");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 1500 }),
    );
    let app_data = temp.path("appdata");
    write_monitor_game(&app_data);

    let app_lib::game_launch::DirectSpawnOutcome::Tracked(process) =
        spawn_direct_game(&executable, temp.root(), &[], true).unwrap()
    else {
        panic!("fixture must be trackable");
    };
    let mut journal = GamingSessionJournal::new("monitor-game".into(), process.identity.pid, "started".into());
    journal.pin_target(&process.identity);
    persistence::write_journal(&app_data, &journal).unwrap();
    drop(process);

    let (journal, tracked) = take_running_session(&app_data).unwrap().unwrap();
    let outcome = reattach_session(reattach_config(&app_data), journal, tracked, &|| {});
    let MonitorOutcome::Ended(session) = outcome else {
        panic!("expected a definite exit");
    };
    assert!(
        persistence::journal_path(&app_data).exists(),
        "the ended session has not been committed yet, so its journal must remain"
    );
    assert!(matches!(
        run_session_tail(
            tail_for(&app_data, &executable),
            *session,
            &FinalizeHost::new(app_data.clone())
        ),
        TailOutcome::Finalized { .. }
    ));
    assert_eq!(reconcile(&app_data).unwrap(), RecoveryAction::NoJournal);
}

/// Objective 3: a second simultaneous opt-in launch is refused and leaves the first
/// session's journal intact.
#[test]
fn concurrent_launch_is_rejected_without_overwriting_the_active_journal() {
    let _serial = serial();
    let temp = TestTemp::new("concurrent");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 2000 }),
    );
    let first_config = base_config(&temp, executable.clone(), TargetExpectation::SpawnedProcess);
    let app_data = first_config.app_data.clone();

    let worker = {
        let config = first_config.clone();
        thread::spawn(move || monitor_session(config, &|| {}))
    };
    let first_journal = wait_for_pinned_journal(&app_data, Duration::from_secs(10));
    let first_session_id = first_journal.session_id.clone();

    let mut second_config = base_config(&temp, executable.clone(), TargetExpectation::SpawnedProcess);
    second_config.game_id = "second-game".into();
    assert!(
        matches!(
            monitor_session(second_config, &|| {}),
            MonitorOutcome::SessionAlreadyActive
        ),
        "a simultaneous opt-in launch must be refused"
    );

    let current = persistence::load_journal(&app_data).unwrap().unwrap();
    assert_eq!(
        current.session_id, first_session_id,
        "the first session's evidence is untouched"
    );
    assert_eq!(current.game_id, "monitor-game");
    assert_eq!(current.game_pid, first_journal.game_pid);

    let MonitorOutcome::Ended(session) = worker.join().unwrap() else {
        panic!("the first launch must still complete normally");
    };
    assert_eq!(session.journal.session_id, first_session_id);

    // While the ended session still holds the reservation nothing may replace it.
    let mut third_config = base_config(&temp, executable.clone(), TargetExpectation::SpawnedProcess);
    third_config.game_id = "third-game".into();
    assert!(matches!(
        monitor_session(third_config, &|| {}),
        MonitorOutcome::SessionAlreadyActive
    ));
    drop(session);

    let mut fourth_config = base_config(&temp, executable, TargetExpectation::SpawnedProcess);
    fourth_config.game_id = "fourth-game".into();
    assert!(
        matches!(monitor_session(fourth_config, &|| {}), MonitorOutcome::Ended(_)),
        "the reservation is released once the session is finished"
    );
}

/// Copies the fixture as a launcher into its own directory and writes the
/// launcher behaviour next to it.
fn launcher_at(temp: &TestTemp, dir_name: &str, config: serde_json::Value) -> (PathBuf, PathBuf) {
    let dir = temp.path(dir_name);
    fs::create_dir_all(&dir).unwrap();
    let launcher = dir.join("launcher.exe");
    fs::copy(FIXTURE_EXE, &launcher).unwrap();
    fs::write(dir.join("fixture.json"), config.to_string()).unwrap();
    (launcher, dir)
}

/// Copies the fixture under a different name into the launcher's directory, so the
/// game child it starts has its own canonical executable path.
fn game_copy_in(dir: &Path, name: &str) -> PathBuf {
    let exe = dir.join(name);
    fs::copy(FIXTURE_EXE, &exe).unwrap();
    exe
}

/// Objective 3: `Game` means the configured executable *is* the game, so the
/// process SILO spawned is the tracked process.
#[test]
fn a_game_target_pins_the_process_silo_spawned() {
    let _serial = serial();
    let mut temp = TestTemp::new("objective3-game");
    let exe_dir = temp.path("程序 files");
    let executable = fixture_at(
        &temp,
        serde_json::json!({ "mode": "direct", "report": "report.json", "exit": 0, "delayMs": 1200 }),
    );
    let mut config = base_config(&temp, executable.clone(), TargetExpectation::SpawnedProcess);
    config.poll_interval = Duration::from_millis(60);

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    let MonitorOutcome::Ended(session) = outcome else {
        panic!("a direct game must be tracked to its own exit, got {outcome:?}");
    };
    assert_eq!(pinned_calls.get(), 1);

    let report: ChildReport = serde_json::from_slice(&fs::read(exe_dir.join("report.json")).unwrap()).unwrap();
    let journal = persistence::load_journal(&config.app_data).unwrap().unwrap();
    assert_eq!(
        journal.game_pid,
        Some(report.pid),
        "the spawned process is the tracked game"
    );
    assert_eq!(session.duration_kind, DurationKind::Exact);
    assert!(
        session.journal.measured_seconds.is_some(),
        "a continuously observed session records an exact measurement"
    );
    drop(journal);
    temp.cleanup();
}

/// Objective 3: `Launcher` with an explicitly configured child executable. The
/// launcher and the game child have different executable paths, and only the
/// configured child may be pinned.
#[test]
fn a_launcher_with_a_configured_child_pins_only_that_exact_child_executable() {
    let _serial = serial();
    let mut temp = TestTemp::new("objective3-launcher-child");
    let (launcher, dir) = launcher_at(
        &temp,
        "launcher dir",
        serde_json::json!({
            "mode": "launcher",
            "parentReport": "launcher report.json",
            "childReport": "child report.json",
            "delayMs": 1800,
            "childExe": "game child.exe"
        }),
    );
    let child = game_copy_in(&dir, "game child.exe");
    assert_ne!(
        child.canonicalize().unwrap(),
        launcher.canonicalize().unwrap(),
        "the game child must have a different executable path than the launcher"
    );

    let mut config = base_config(
        &temp,
        launcher.clone(),
        TargetExpectation::ChildExecutable(child.clone()),
    );
    config.poll_interval = Duration::from_millis(80);

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    let MonitorOutcome::Ended(session) = outcome else {
        panic!("the configured child must be tracked to its own exit, got {outcome:?}");
    };
    assert_eq!(pinned_calls.get(), 1);

    let report: LauncherReport = serde_json::from_slice(&fs::read(dir.join("launcher report.json")).unwrap()).unwrap();
    let journal = persistence::load_journal(&config.app_data).unwrap().unwrap();
    assert_eq!(
        journal.game_pid,
        Some(report.child_pid),
        "the configured child executable is the game"
    );
    assert_ne!(
        journal.game_pid,
        Some(report.launcher_pid),
        "the launcher itself is never the game"
    );
    assert!(
        session.elapsed_secs >= 1,
        "the child, not the launcher, defines the session"
    );
    assert_eq!(session.duration_kind, DurationKind::Exact);
    drop(journal);
    temp.cleanup();
}

/// Objective 3: an ambiguous launcher — no configured child and no confident
/// identification. Nothing may be pinned, the session stays uncertain, and the
/// evidence is retained instead of being finalized.
#[test]
fn an_ambiguous_launcher_pins_nothing_and_keeps_the_evidence() {
    let _serial = serial();
    let mut temp = TestTemp::new("objective3-ambiguous");
    let (launcher, dir) = launcher_at(
        &temp,
        "launcher dir",
        serde_json::json!({
            "mode": "launcher",
            "parentReport": "launcher report.json",
            "childReport": "child report.json",
            "delayMs": 1200,
            "childExe": "game child.exe"
        }),
    );
    let _child = game_copy_in(&dir, "game child.exe");

    let config = base_config(&temp, launcher, TargetExpectation::UnidentifiableChild);
    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    assert!(
        matches!(outcome, MonitorOutcome::TrackingUncertain),
        "an ambiguous launcher must never be reported as a tracked session, got {outcome:?}"
    );
    assert_eq!(
        pinned_calls.get(),
        0,
        "no launcher effects (and no hiding) without a pinned game"
    );

    let journal = persistence::load_journal(&config.app_data)
        .unwrap()
        .expect("uncertain tracking must retain journal evidence");
    assert_eq!(journal.status, "tracking_uncertain");
    assert!(journal.game_pid.is_none(), "no identity may be invented");
    assert!(journal.finalization.is_none(), "nothing may be finalized");
    assert!(
        !config.app_data.join("sessions.json").exists(),
        "an uncertain session commits no history"
    );
    drop(journal);

    thread::sleep(Duration::from_millis(1400));
    temp.cleanup();
}

/// Objective 3: a descendant that runs a *different* executable is never pinned as
/// the game. The launcher starts a helper; the configured child path points at a
/// binary that is not started at all.
#[test]
fn a_descendant_with_a_different_executable_path_is_never_pinned_as_the_game() {
    let _serial = serial();
    let mut temp = TestTemp::new("objective3-helper");
    let (launcher, dir) = launcher_at(
        &temp,
        "launcher dir",
        serde_json::json!({
            "mode": "launcher",
            "parentReport": "launcher report.json",
            "childReport": "child report.json",
            "delayMs": 1200,
            "childExe": "updater helper.exe"
        }),
    );
    let helper = game_copy_in(&dir, "updater helper.exe");
    // The configured child is a third binary that the launcher never starts.
    let configured_child = game_copy_in(&dir, "configured game.exe");

    let mut config = base_config(&temp, launcher, TargetExpectation::ChildExecutable(configured_child));
    config.discovery_timeout = Duration::from_millis(900);
    config.poll_interval = Duration::from_millis(80);

    let pinned_calls = Cell::new(0);
    let outcome = monitor_session(config.clone(), &|| pinned_calls.set(pinned_calls.get() + 1));
    assert!(
        matches!(outcome, MonitorOutcome::TrackingUncertain),
        "a helper, updater or crash reporter must never be mistaken for the game, got {outcome:?}"
    );
    assert_eq!(pinned_calls.get(), 0);
    let journal = persistence::load_journal(&config.app_data).unwrap().unwrap();
    assert!(journal.game_pid.is_none());
    assert_ne!(
        journal.game_pid,
        Some(
            fs::read_to_string(dir.join("launcher report.json"))
                .ok()
                .and_then(|raw| serde_json::from_str::<LauncherReport>(&raw).ok())
                .map(|report| report.child_pid)
                .unwrap_or(0)
        ),
        "the helper process must never be pinned"
    );
    let _ = helper;
    drop(journal);
    thread::sleep(Duration::from_millis(1400));
    temp.cleanup();
}
