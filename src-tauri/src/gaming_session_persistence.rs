//! Durable, replay-safe persistence primitives for gaming sessions.
//!
//! This module intentionally has no Tauri or process-monitor dependency. Callers
//! provide the app-data directory and a snapshot of the game's current totals.
use crate::commands::Game;
use serde::{Deserialize, Serialize};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

const VERSION: u32 = 1;
const JOURNAL_FILE: &str = "gaming-session.json";
const HISTORY_FILE: &str = "sessions.json";
const GAMES_FILE: &str = "games.json";

/// Whether a committed duration is an exact measurement or a lower bound.
///
/// `Exact` means this process measured the target's lifetime continuously.
/// `Uncertain` means the duration was reconstructed from durable checkpoints:
/// the app was not running for an unknown part of the session, so the value is
/// a lower bound and must never be presented as exact playtime.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(rename_all = "camelCase")]
pub enum DurationKind {
    /// The duration was measured continuously and is exact.
    #[default]
    Exact,
    /// The duration is a checkpoint-derived lower bound, not an exact measurement.
    Uncertain,
}

impl DurationKind {
    pub fn is_exact(&self) -> bool {
        matches!(self, DurationKind::Exact)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct AggregateTargets {
    /// Exact playtime total. Uncertain minutes are never added to this.
    pub playtime_minutes: u32,
    /// Uncertain (lower-bound) minutes, kept apart from the exact total.
    #[serde(default)]
    pub uncertain_playtime_minutes: u32,
    pub session_count: u32,
    pub last_played: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct Finalization {
    pub duration_minutes: u32,
    /// True when the committed duration is a lower bound rather than an exact
    /// measurement. Uncertain minutes are kept out of the exact playtime total.
    pub uncertain: bool,
    pub status: String,
    pub targets: AggregateTargets,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct GamingSessionJournal {
    pub version: u32,
    pub session_id: String,
    pub game_id: String,
    pub launch_root_pid: u32,
    #[serde(default)]
    pub game_pid: Option<u32>,
    #[serde(default)]
    pub process_created_at: Option<String>,
    #[serde(default)]
    pub executable: Option<String>,
    pub started_at_utc: String,
    pub checkpoint_at_utc: String,
    /// Durable elapsed time at the last checkpoint. This is always a **lower
    /// bound** on the session duration: the app may have been down for an
    /// unknown period between checkpoints. Older journals omit it, which keeps
    /// their recovery behaviour unchanged (evidence is preserved, not finalized).
    #[serde(default)]
    pub elapsed_seconds: Option<u64>,
    /// Elapsed time measured continuously by this process. `None` means the
    /// duration is only known as a lower bound, so it is not exact playtime.
    /// This is persisted separately from `elapsed_seconds` so the two are never
    /// confused with one another.
    #[serde(default)]
    pub measured_seconds: Option<u64>,
    pub status: String,
    #[serde(default)]
    pub finalization: Option<Finalization>,
}

impl GamingSessionJournal {
    pub fn new(game_id: String, launch_root_pid: u32, started_at_utc: String) -> Self {
        Self {
            version: VERSION,
            session_id: Uuid::new_v4().to_string(),
            game_id,
            launch_root_pid,
            game_pid: None,
            process_created_at: None,
            executable: None,
            checkpoint_at_utc: started_at_utc.clone(),
            elapsed_seconds: None,
            measured_seconds: None,
            started_at_utc,
            status: "active".into(),
            finalization: None,
        }
    }

    pub fn pin_target(&mut self, identity: &crate::process_monitor::ProcessIdentity) {
        self.game_pid = Some(identity.pid);
        self.process_created_at = Some(identity.creation_filetime.to_string());
        self.executable.clone_from(&identity.executable);
        self.status = "active".into();
    }

    pub fn retain_uncertain(&mut self, checkpoint_at_utc: String) {
        self.checkpoint_at_utc = checkpoint_at_utc;
        self.status = "tracking_uncertain".into();
    }

    /// Records durable elapsed time for crash recovery. Does not change the status.
    ///
    /// The value is a lower bound: it is the elapsed time at the last durable
    /// checkpoint, not a claim about the session's total length.
    pub fn checkpoint(&mut self, elapsed_seconds: u64, checkpoint_at_utc: String) {
        self.elapsed_seconds = Some(elapsed_seconds);
        self.checkpoint_at_utc = checkpoint_at_utc;
    }

    /// Records an exact, continuously measured duration.
    ///
    /// Only a process that observed the target from pin to exit may call this.
    /// A reattached session never does, because the app was down for an unknown
    /// part of it.
    pub fn measure(&mut self, measured_seconds: u64) {
        self.measured_seconds = Some(measured_seconds);
    }

    /// The duration kind this journal can support.
    ///
    /// A journal with an exact measurement commits exact playtime; a journal
    /// without one commits a lower bound.
    pub fn duration_kind(&self) -> DurationKind {
        if self.measured_seconds.is_some() {
            DurationKind::Exact
        } else {
            DurationKind::Uncertain
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HistoryRow {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub session_id: Option<String>,
    pub game_id: String,
    pub started_at: String,
    pub minutes: u32,
    /// Whether `minutes` is an exact measurement or a lower bound. Legacy rows
    /// omit it and are therefore exact.
    #[serde(default, skip_serializing_if = "DurationKind::is_exact")]
    pub duration_kind: DurationKind,
}

pub fn journal_path(directory: &Path) -> PathBuf {
    directory.join(JOURNAL_FILE)
}

pub fn write_journal(directory: &Path, journal: &GamingSessionJournal) -> Result<(), String> {
    if journal.version != VERSION {
        return Err("unsupported journal version".into());
    }
    write_json_atomic(&journal_path(directory), journal)
}

/// Parse valid supported journals; corrupt/unsupported bytes are retained under
/// a unique `.evidence` sibling rather than silently discarded.
pub fn load_journal(directory: &Path) -> Result<Option<GamingSessionJournal>, String> {
    let path = journal_path(directory);
    let mut bytes = Vec::new();
    match File::open(&path) {
        Ok(mut file) => file.read_to_end(&mut bytes).map_err(|e| e.to_string())?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
    };
    let parsed = serde_json::from_slice::<GamingSessionJournal>(&bytes);
    match parsed {
        Ok(journal) if journal.version == VERSION => Ok(Some(journal)),
        _ => {
            let evidence = path.with_file_name(format!("gaming-session.{}.evidence", Uuid::new_v4()));
            fs::write(evidence, bytes).map_err(|e| format!("preserve invalid journal: {e}"))?;
            Ok(None)
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryObservation {
    Running,
    Exited,
    Unknown,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum RecoveryAction {
    NoJournal,
    KeepRunning,
    PreserveExitedUnresolved,
    PreserveUncertain,
    RetryFinalization,
    /// Definite exit with a durable elapsed checkpoint: finalize conservatively
    /// (rounded up, marked uncertain) instead of preserving dead evidence forever.
    FinalizeInterrupted,
}

/// Decides recovery only from an identity-validated observation. An exited
/// process without a committed finalization has no reliable end-time checkpoint.
pub fn recovery_action(journal: &GamingSessionJournal, observation: RecoveryObservation) -> RecoveryAction {
    if journal.finalization.is_some() {
        return RecoveryAction::RetryFinalization;
    }
    if journal.game_pid.is_none() || journal.process_created_at.is_none() {
        return RecoveryAction::PreserveUncertain;
    }
    match observation {
        RecoveryObservation::Running => RecoveryAction::KeepRunning,
        RecoveryObservation::Exited if journal.elapsed_seconds.is_some() => RecoveryAction::FinalizeInterrupted,
        RecoveryObservation::Exited => RecoveryAction::PreserveExitedUnresolved,
        RecoveryObservation::Unknown => RecoveryAction::PreserveUncertain,
    }
}

/// Finalization is retryable: history is upserted by UUID and game totals are
/// assigned from the journal's immutable snapshot, never incremented on replay.
pub fn finalize(directory: &Path, journal: &mut GamingSessionJournal, row: HistoryRow) -> Result<(), String> {
    finalize_with_boundary(directory, journal, row, |_| Ok(()))
}

fn finalize_with_boundary(
    directory: &Path,
    journal: &mut GamingSessionJournal,
    row: HistoryRow,
    mut boundary: impl FnMut(CommitBoundary) -> Result<(), String>,
) -> Result<(), String> {
    if row.session_id.as_deref() != Some(journal.session_id.as_str()) {
        return Err("history row session ID does not match journal".into());
    }
    if journal.finalization.is_none() {
        // Caller constructs targets from a single snapshot and embeds them before
        // calling this helper; the row's minutes/status remain exact on replay.
        return Err("finalization payload must be persisted before commit".into());
    }
    let finalization = journal.finalization.as_ref().ok_or("missing finalization")?;
    journal.status = finalization.status.clone();
    write_journal(directory, journal)?;
    boundary(CommitBoundary::Journal)?;

    let history_path = directory.join(HISTORY_FILE);
    let mut history = read_json_or_default::<Vec<HistoryRow>>(&history_path)?;
    if let Some(existing) = history
        .iter_mut()
        .find(|entry| entry.session_id.as_deref() == Some(&journal.session_id))
    {
        *existing = row;
    } else {
        history.push(row);
    }
    write_json_atomic(&history_path, &history)?;
    boundary(CommitBoundary::History)?;

    let games_path = directory.join(GAMES_FILE);
    let mut games = read_json_or_default::<Vec<Game>>(&games_path)?;
    let game = games
        .iter_mut()
        .find(|game| game.id == journal.game_id)
        .ok_or("game not found in games.json")?;
    // The exact total and the uncertain (lower-bound) total are separate
    // buckets: an uncertain minute never inflates the exact playtime.
    game.playtime_minutes = finalization.targets.playtime_minutes;
    game.uncertain_playtime_minutes = finalization.targets.uncertain_playtime_minutes;
    game.session_count = finalization.targets.session_count;
    game.last_played.clone_from(&finalization.targets.last_played);
    write_json_atomic(&games_path, &games)?;
    boundary(CommitBoundary::Games)?;

    // Clearing only follows both durable writes. Failure leaves replay evidence.
    fs::remove_file(journal_path(directory)).map_err(|e| e.to_string())?;
    sync_directory(directory)?;
    boundary(CommitBoundary::Cleared)?;
    Ok(())
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CommitBoundary {
    Journal,
    History,
    Games,
    Cleared,
}

fn read_json_or_default<T: for<'de> Deserialize<'de>>(path: &Path) -> Result<T, String> {
    match fs::read(path) {
        Ok(bytes) => serde_json::from_slice(&bytes).map_err(|e| format!("{}: {e}", path.display())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            serde_json::from_str("[]").map_err(|e| e.to_string())
        }
        Err(error) => Err(error.to_string()),
    }
}

fn write_json_atomic(path: &Path, value: &impl Serialize) -> Result<(), String> {
    let parent = path.parent().ok_or("path has no parent")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    let bytes = serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?;
    let mut file = File::create(&tmp).map_err(|e| e.to_string())?;
    file.write_all(&bytes).map_err(|e| e.to_string())?;
    file.flush().map_err(|e| e.to_string())?;
    file.sync_all().map_err(|e| e.to_string())?;
    drop(file);
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        use windows::core::PCWSTR;
        use windows::Win32::Storage::FileSystem::{ReplaceFileW, REPLACE_FILE_FLAGS};
        let source: Vec<u16> = tmp.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
        // ReplaceFileW atomically replaces an existing file while preserving a backup.
        if path.exists() {
            let backup = path.with_extension("json.bak");
            let backup_wide: Vec<u16> = backup.as_os_str().encode_wide().chain(Some(0)).collect();
            // SAFETY: all pointers reference NUL-terminated UTF-16 paths valid for this call.
            unsafe {
                ReplaceFileW(
                    PCWSTR(destination.as_ptr()),
                    PCWSTR(source.as_ptr()),
                    PCWSTR(backup_wide.as_ptr()),
                    REPLACE_FILE_FLAGS(0),
                    None,
                    None,
                )
            }
            .map_err(|e| e.to_string())?;
        } else {
            // First creation is a same-volume rename; no destination exists to replace.
            fs::rename(&tmp, path).map_err(|e| e.to_string())?;
        }
    }
    #[cfg(not(windows))]
    fs::rename(&tmp, path).map_err(|e| e.to_string())?;
    sync_directory(parent)
}

fn sync_directory(path: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        File::open(path)
            .and_then(|file| file.sync_all())
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(unix))]
    let _ = path;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};

    struct TestDir(PathBuf);
    impl TestDir {
        fn new() -> Self {
            let nonce = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos());
            let path = std::env::temp_dir().join(format!("silo-session-{}-{nonce}", std::process::id()));
            fs::create_dir_all(&path).expect("create isolated fixture");
            Self(path)
        }
        fn path(&self) -> &Path {
            &self.0
        }
    }
    impl Drop for TestDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn fixture(dir: &Path) -> (GamingSessionJournal, HistoryRow) {
        let mut journal = GamingSessionJournal::new("g1".into(), 42, "2026-11-04T10:00:00Z".into());
        journal.finalization = Some(Finalization {
            duration_minutes: 4,
            uncertain: true,
            status: "recovered_uncertain".into(),
            targets: AggregateTargets {
                playtime_minutes: 19,
                uncertain_playtime_minutes: 4,
                session_count: 3,
                last_played: Some("2026-11-04T10:04:00Z".into()),
            },
        });
        let row = HistoryRow {
            session_id: Some(journal.session_id.clone()),
            game_id: "g1".into(),
            started_at: journal.started_at_utc.clone(),
            minutes: 4,
            duration_kind: DurationKind::Uncertain,
        };
        let games = vec![Game {
            id: "g1".into(),
            name: "Example".into(),
            exe_path: Some("example.exe".into()),
            wallpaper: Some("wallpaper.jpg".into()),
            logo_path: Some("logo.png".into()),
            font_family: Some("serif".into()),
            font_color: Some("#123456".into()),
            playtime_minutes: 15,
            uncertain_playtime_minutes: 0,
            session_count: 2,
            last_played: None,
            status: Some("installed".into()),
            added_at: "added".into(),
            save_path: Some("saves".into()),
            save_path_source: Some("manual".into()),
            backup_count: Some(7),
            is_installed: Some(true),
            favorite: true,
            tags: vec!["tag".into()],
            launch_target: Some("game".into()),
            launch_child_path: None,
            exe_dir: Some("ignored".into()),
        }];
        write_json_atomic(&dir.join(GAMES_FILE), &games).expect("fixture game data");
        (journal, row)
    }

    #[test]
    fn replay_after_each_commit_stage_deduplicates_and_assigns_targets() {
        for fail_at in [CommitBoundary::Journal, CommitBoundary::History, CommitBoundary::Games] {
            let dir = TestDir::new();
            let (mut journal, row) = fixture(dir.path());
            write_journal(dir.path(), &journal).expect("journal");
            assert!(
                finalize_with_boundary(dir.path(), &mut journal, row.clone(), |boundary| {
                    if boundary == fail_at {
                        Err("simulated crash".into())
                    } else {
                        Ok(())
                    }
                })
                .is_err()
            );
            let mut recovered = load_journal(dir.path())
                .expect("reload journal")
                .expect("journal retained");
            finalize(dir.path(), &mut recovered, row.clone()).expect("retry finalize");
            let history: Vec<HistoryRow> = read_json_or_default(&dir.path().join(HISTORY_FILE)).expect("history read");
            let games: Vec<Game> = read_json_or_default(&dir.path().join(GAMES_FILE)).expect("game read");
            assert_eq!(history, vec![row]);
            assert_eq!(games[0].playtime_minutes, 19);
            assert_eq!(games[0].uncertain_playtime_minutes, 4);
            assert_eq!(games[0].session_count, 3);
            assert_eq!(games[0].last_played.as_deref(), Some("2026-11-04T10:04:00Z"));
            assert_eq!(games[0].name, "Example");
            assert_eq!(games[0].exe_path.as_deref(), Some("example.exe"));
            assert_eq!(games[0].wallpaper.as_deref(), Some("wallpaper.jpg"));
            assert_eq!(games[0].logo_path.as_deref(), Some("logo.png"));
            assert!(games[0].favorite);
            assert_eq!(games[0].tags, vec!["tag"]);
            assert!(!journal_path(dir.path()).exists());
        }
    }

    #[test]
    fn crash_after_journal_clear_keeps_committed_files_consistent() {
        let dir = TestDir::new();
        let (mut journal, row) = fixture(dir.path());
        write_journal(dir.path(), &journal).expect("journal");
        assert!(
            finalize_with_boundary(dir.path(), &mut journal, row.clone(), |boundary| {
                if boundary == CommitBoundary::Cleared {
                    Err("simulated crash after clear".into())
                } else {
                    Ok(())
                }
            })
            .is_err()
        );
        assert!(load_journal(dir.path()).expect("journal load").is_none());
        let history: Vec<HistoryRow> = read_json_or_default(&dir.path().join(HISTORY_FILE)).expect("history");
        let games: Vec<Game> = read_json_or_default(&dir.path().join(GAMES_FILE)).expect("games");
        assert_eq!(history, vec![row]);
        assert_eq!(games[0].playtime_minutes, 19);
        assert_eq!(games[0].session_count, 3);
    }

    #[test]
    fn legacy_history_rows_load_without_session_id() {
        let rows: Vec<HistoryRow> =
            serde_json::from_str(r#"[{"gameId":"g","startedAt":"old","minutes":7}]"#).expect("legacy rows");
        assert_eq!(rows[0].session_id, None);
        assert_eq!(rows[0].minutes, 7);
        assert_eq!(
            rows[0].duration_kind,
            DurationKind::Exact,
            "a legacy row without a marker is an exact session"
        );
    }

    #[test]
    fn legacy_aggregate_targets_load_without_the_uncertain_bucket() {
        let targets: AggregateTargets =
            serde_json::from_str(r#"{"playtimeMinutes":10,"sessionCount":2,"lastPlayed":null}"#)
                .expect("legacy targets");
        assert_eq!(targets.playtime_minutes, 10);
        assert_eq!(targets.uncertain_playtime_minutes, 0);
        assert_eq!(targets.session_count, 2);
    }

    #[test]
    fn an_uncertain_session_never_inflates_the_exact_playtime_total() {
        let dir = TestDir::new();
        let (mut journal, row) = fixture(dir.path());
        write_journal(dir.path(), &journal).expect("journal");
        finalize(dir.path(), &mut journal, row.clone()).expect("finalize");
        let games: Vec<Game> = read_json_or_default(&dir.path().join(GAMES_FILE)).expect("games");
        // The fixture commits 19 exact minutes and 4 uncertain minutes.
        assert_eq!(games[0].playtime_minutes, 19);
        assert_eq!(games[0].uncertain_playtime_minutes, 4);
        let history: Vec<HistoryRow> = read_json_or_default(&dir.path().join(HISTORY_FILE)).expect("history");
        assert_eq!(history[0].duration_kind, DurationKind::Uncertain);
        // The exact and uncertain buckets are stored separately on disk.
        let raw = fs::read_to_string(dir.path().join(GAMES_FILE)).expect("raw games");
        assert!(raw.contains("\"playtimeMinutes\": 19"), "{raw}");
        assert!(raw.contains("\"uncertainPlaytimeMinutes\": 4"), "{raw}");
    }

    #[test]
    fn the_journal_keeps_the_exact_measurement_apart_from_the_checkpoint_lower_bound() {
        let dir = TestDir::new();
        let mut journal = GamingSessionJournal::new("g1".into(), 42, "start".into());
        journal.checkpoint(30, "2026-11-04T10:00:30Z".into());
        write_journal(dir.path(), &journal).expect("journal");
        let loaded = load_journal(dir.path()).expect("load").expect("journal");
        assert_eq!(loaded.elapsed_seconds, Some(30));
        assert_eq!(
            loaded.measured_seconds, None,
            "a checkpoint alone is not an exact measurement"
        );
        assert_eq!(loaded.duration_kind(), DurationKind::Uncertain);

        journal.measure(42);
        write_journal(dir.path(), &journal).expect("journal");
        let loaded = load_journal(dir.path()).expect("load").expect("journal");
        assert_eq!(loaded.elapsed_seconds, Some(30));
        assert_eq!(loaded.measured_seconds, Some(42));
        assert_eq!(loaded.duration_kind(), DurationKind::Exact);
    }

    #[test]
    fn invalid_journal_is_preserved_as_evidence() {
        let dir = TestDir::new();
        fs::write(journal_path(dir.path()), b"not json").expect("write corrupt");
        assert!(load_journal(dir.path()).expect("load").is_none());
        let entries: Vec<_> = fs::read_dir(dir.path()).expect("list").collect();
        assert_eq!(entries.len(), 2, "corrupt journal and preserved evidence remain");
    }

    #[test]
    fn provisional_journal_survives_uncertainty_and_pinned_identity_is_exact() {
        let dir = TestDir::new();
        let mut journal = GamingSessionJournal::new("g1".into(), 42, "2026-11-04T10:00:00Z".into());
        write_journal(dir.path(), &journal).unwrap();
        journal.retain_uncertain("2026-11-04T10:00:03Z".into());
        write_journal(dir.path(), &journal).unwrap();
        let unresolved = load_journal(dir.path()).unwrap().unwrap();
        assert_eq!(unresolved.status, "tracking_uncertain");
        assert_eq!(unresolved.game_pid, None);
        assert!(journal_path(dir.path()).exists());

        let identity = crate::process_monitor::ProcessIdentity {
            pid: 77,
            creation_filetime: 123456,
            executable: Some("C:\\\\Games\\\\game.exe".into()),
        };
        journal.pin_target(&identity);
        write_journal(dir.path(), &journal).unwrap();
        let pinned = load_journal(dir.path()).unwrap().unwrap();
        assert_eq!(pinned.game_pid, Some(77));
        assert_eq!(pinned.process_created_at.as_deref(), Some("123456"));
        assert_eq!(pinned.executable, identity.executable);
    }

    #[test]
    fn startup_recovery_preserves_running_exited_and_uncertain_journals() {
        let mut journal = GamingSessionJournal::new("g1".into(), 42, "start".into());
        assert_eq!(
            recovery_action(&journal, RecoveryObservation::Exited),
            RecoveryAction::PreserveUncertain,
            "provisional journal has no process identity"
        );
        journal.game_pid = Some(70);
        journal.process_created_at = Some("700".into());
        assert_eq!(
            recovery_action(&journal, RecoveryObservation::Running),
            RecoveryAction::KeepRunning
        );
        assert_eq!(
            recovery_action(&journal, RecoveryObservation::Exited),
            RecoveryAction::PreserveExitedUnresolved
        );
        assert_eq!(
            recovery_action(&journal, RecoveryObservation::Unknown),
            RecoveryAction::PreserveUncertain
        );
        journal.finalization = Some(Finalization {
            duration_minutes: 2,
            uncertain: false,
            status: "completed".into(),
            targets: AggregateTargets {
                playtime_minutes: 12,
                uncertain_playtime_minutes: 0,
                session_count: 2,
                last_played: Some("end".into()),
            },
        });
        assert_eq!(
            recovery_action(&journal, RecoveryObservation::Unknown),
            RecoveryAction::RetryFinalization,
            "already committed target payload is replayed idempotently regardless of PID state"
        );
    }
}
