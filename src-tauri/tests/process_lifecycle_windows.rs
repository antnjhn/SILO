#![cfg(windows)]

use app_lib::game_launch::{spawn_direct_game, DirectSpawnOutcome, ProcessCandidate};
use app_lib::process_monitor::{reopen_persisted, ProcessStatus, TrackedProcess};
use serde::Deserialize;
use std::ffi::OsString;
use std::fs;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use sysinfo::{Pid, System};

fn child_exe() -> PathBuf {
    // Build the test fixture binary if it doesn't exist yet.
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let exe_path = manifest_dir
        .join("target")
        .join("debug")
        .join("process_test_fixture.exe");
    if !exe_path.exists() {
        let _ = std::process::Command::new("cargo")
            .arg("build")
            .arg("--features")
            .arg("process-test-fixture")
            .arg("--bin")
            .arg("process_test_fixture")
            .current_dir(&manifest_dir)
            .status();
    }
    exe_path
}

struct TestTemp(Option<PathBuf>);

impl TestTemp {
    fn new(label: &str) -> Self {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let path = std::env::temp_dir().join(format!("silo-process-test-{label}-{}-{nonce}", std::process::id()));
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
            fs::remove_dir_all(path).expect("remove controlled process test directory");
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
    cwd: PathBuf,
    args: Vec<String>,
}

#[derive(Deserialize)]
struct LauncherReport {
    launcher_pid: u32,
    child_pid: u32,
}

fn fixture_at(temp: &TestTemp) -> PathBuf {
    let exe = temp.path("程序 files\\controlled child.exe");
    fs::create_dir_all(exe.parent().unwrap()).unwrap();
    fs::copy(child_exe(), &exe).unwrap();
    exe
}

fn tracked(result: Result<DirectSpawnOutcome, app_lib::game_launch::DirectSpawnError>) -> TrackedProcess {
    match result.unwrap() {
        DirectSpawnOutcome::Tracked(process) => process,
        DirectSpawnOutcome::LaunchedButTrackingUncertain { pid, reason } => {
            panic!("controlled child PID {pid:?} was not trackable: {reason:?}")
        }
    }
}

fn assert_same_windows_path(actual: &str, expected: &Path) {
    fn strip_extended_prefix(path: &str) -> &str {
        path.strip_prefix("\\\\?\\").unwrap_or(path)
    }
    assert!(
        strip_extended_prefix(actual).eq_ignore_ascii_case(strip_extended_prefix(expected.to_str().unwrap())),
        "path mismatch: {actual:?} != {expected:?}"
    );
}

fn wait_for_exit(process: &TrackedProcess) -> u32 {
    let deadline = Instant::now() + Duration::from_secs(10);
    loop {
        match process.exit_code() {
            Ok(Some(code)) => return code,
            Ok(None) => assert!(Instant::now() < deadline, "controlled process did not exit"),
            Err(status) => panic!("process status became uncertain: {status:?}"),
        }
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn direct_spawn_round_trips_unicode_arguments_cwd_identity_priority_and_exit_code() {
    let mut temp = TestTemp::new("direct");
    let executable = fixture_at(&temp);
    let cwd = temp.path("working dir ü");
    fs::create_dir_all(&cwd).unwrap();
    let report_path = temp.path("child report.json");
    let expected_args = ["two words", "embedded\"quote", "雪だるま", "", "trailing\\"];
    let mut args = vec![
        OsString::from("direct"),
        report_path.as_os_str().to_os_string(),
        "37".into(),
        "1200".into(),
    ];
    args.extend(expected_args.iter().map(OsString::from));

    let process = tracked(spawn_direct_game(&executable, &cwd, &args, true));
    assert!(process.identity.pid != 0);
    assert_same_windows_path(
        process.identity.executable.as_deref().unwrap(),
        &executable.canonicalize().unwrap(),
    );
    assert_ne!(process.identity.creation_filetime, 0);
    assert_eq!(process.poll(), ProcessStatus::Running);
    assert_eq!(
        process.priority_class().unwrap(),
        0x80,
        "CreateProcessW must preserve HIGH priority"
    );

    let report_deadline = Instant::now() + Duration::from_secs(5);
    while !report_path.exists() && Instant::now() < report_deadline {
        thread::sleep(Duration::from_millis(10));
    }
    if !report_path.exists() {
        panic!("fixture exited without report, code={:?}", process.exit_code());
    }
    let report: ChildReport = serde_json::from_slice(&fs::read(&report_path).unwrap()).unwrap();
    assert_eq!(report.pid, process.identity.pid);
    assert_same_windows_path(report.cwd.to_str().unwrap(), &cwd.canonicalize().unwrap());
    assert_eq!(report.args, expected_args);
    assert_eq!(wait_for_exit(&process), 37);
    assert_eq!(process.poll(), ProcessStatus::Exited);
    drop(process);
    assert!(fs::remove_file(&report_path).is_ok());
    temp.cleanup();
}

#[test]
fn temp_files_are_cleaned_during_test_unwind() {
    let temp = TestTemp::new("unwind");
    let temp_root = temp.root().to_path_buf();
    let executable = fixture_at(&temp);
    let report = temp.path("will-be-removed.json");
    let args = vec![
        OsString::from("direct"),
        report.as_os_str().to_os_string(),
        OsString::from("0"),
        OsString::from("1"),
    ];
    let child_cwd = temp_root.clone();
    let panic_result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || {
        let _temp_owner = temp;
        let process = tracked(spawn_direct_game(&executable, &child_cwd, &args, true));
        assert_eq!(wait_for_exit(&process), 0);
        panic!("intentional unwind to verify temporary-directory cleanup");
    }));
    assert!(panic_result.is_err());
    assert!(!temp_root.exists(), "test temp directory must be removed during unwind");
    thread::sleep(Duration::from_millis(400));
}

#[test]
fn launcher_handoff_tracks_actual_descendant_after_launcher_exit() {
    let mut temp = TestTemp::new("handoff");
    let executable = fixture_at(&temp);
    let parent_report_path = temp.path("launcher report.json");
    let child_report_path = temp.path("target report.json");
    let args = vec![
        OsString::from("launcher"),
        parent_report_path.as_os_str().to_os_string(),
        child_report_path.as_os_str().to_os_string(),
        OsString::from("2500"),
    ];
    let launcher = tracked(spawn_direct_game(&executable, temp.root(), &args, true));
    let launcher_pid = launcher.identity.pid;
    assert_eq!(wait_for_exit(&launcher), 0);
    let report: LauncherReport = serde_json::from_slice(&fs::read(&parent_report_path).unwrap()).unwrap();
    assert_eq!(report.launcher_pid, launcher_pid);

    let child = TrackedProcess::open(report.child_pid).expect("open handoff target by reported PID");
    assert_eq!(child.poll(), ProcessStatus::Running);
    assert_same_windows_path(
        child.identity.executable.as_deref().unwrap(),
        &executable.canonicalize().unwrap(),
    );

    let mut system = System::new_all();
    system.refresh_all();
    let child_parent = system.process(Pid::from_u32(report.child_pid)).and_then(|p| p.parent());
    assert_eq!(child_parent.map(Pid::as_u32), Some(launcher_pid));
    let candidates: Vec<_> = system
        .processes()
        .iter()
        .map(|(pid, process)| ProcessCandidate {
            pid: pid.as_u32(),
            parent_pid: process.parent().map(Pid::as_u32),
            executable: Some(process.exe().unwrap_or(Path::new("")).to_path_buf()),
        })
        .collect();
    assert_eq!(
        app_lib::game_launch::find_target_descendant(launcher_pid, &executable, &candidates).map(|p| p.pid),
        Some(report.child_pid)
    );

    let (mismatch_status, mismatch_handle) =
        reopen_persisted(child.identity.pid, child.identity.creation_filetime.saturating_add(1));
    assert_eq!(
        mismatch_status,
        ProcessStatus::Exited,
        "different creation time must reject same PID"
    );
    assert!(mismatch_handle.is_none());
    assert_eq!(child.poll(), ProcessStatus::Running, "launcher exit is not target exit");
    assert_eq!(wait_for_exit(&child), 0);
    drop(child);
    drop(launcher);
    temp.cleanup();
}

#[test]
fn startup_reconciliation_uses_full_identity_and_preserves_unfinalized_exit() {
    use app_lib::gaming_session_persistence::{write_journal, GamingSessionJournal, RecoveryAction};
    use app_lib::gaming_session_recovery::reconcile;

    let mut temp = TestTemp::new("recovery");
    let executable = fixture_at(&temp);
    let report = temp.path("recovery-report.json");
    let args = vec![
        OsString::from("direct"),
        report.as_os_str().to_os_string(),
        OsString::from("37"),
        OsString::from("1200"),
    ];
    let process = tracked(spawn_direct_game(&executable, temp.root(), &args, true));
    let mut journal = GamingSessionJournal::new("fixture-game".into(), process.identity.pid, "started".into());
    journal.pin_target(&process.identity);
    write_journal(temp.root(), &journal).unwrap();

    assert_eq!(reconcile(temp.root()).unwrap(), RecoveryAction::KeepRunning);
    assert!(app_lib::gaming_session_persistence::journal_path(temp.root()).exists());
    assert_eq!(wait_for_exit(&process), 37);
    assert_eq!(
        reconcile(temp.root()).unwrap(),
        RecoveryAction::PreserveExitedUnresolved
    );
    let recovered = app_lib::gaming_session_persistence::load_journal(temp.root())
        .unwrap()
        .unwrap();
    assert_eq!(recovered.status, "interrupted_exit_unresolved");
    assert_eq!(
        reconcile(temp.root()).unwrap(),
        RecoveryAction::PreserveExitedUnresolved
    );
    assert!(app_lib::gaming_session_persistence::journal_path(temp.root()).exists());
    drop(process);
    temp.cleanup();
}

#[test]
fn startup_reconciliation_preserves_interrupted_provisional_journal() {
    use app_lib::gaming_session_persistence::{write_journal, GamingSessionJournal, RecoveryAction};
    use app_lib::gaming_session_recovery::reconcile;

    let mut temp = TestTemp::new("provisional-recovery");
    let journal = GamingSessionJournal::new("fixture-game".into(), 0, "started".into());
    write_journal(temp.root(), &journal).unwrap();
    assert_eq!(reconcile(temp.root()).unwrap(), RecoveryAction::PreserveUncertain);
    assert_eq!(reconcile(temp.root()).unwrap(), RecoveryAction::PreserveUncertain);
    assert!(app_lib::gaming_session_persistence::journal_path(temp.root()).exists());
    temp.cleanup();
}
