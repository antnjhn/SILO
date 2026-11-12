//! App-independent launch → pin → monitor stage of the opt-in Gaming Mode worker.
//!
//! Everything that must be proven with controlled processes lives here: provisional
//! journal persistence, direct spawning, exact target pinning (direct, or launcher
//! descendant handoff), SaveGuard write attribution while the target runs, durable
//! elapsed-time checkpoints, and definite-exit versus uncertain-identity reporting.
//!
//! It deliberately has no Tauri dependency. Window effects, SaveGuard completion and
//! aggregate finalization stay in `commands.rs`, which runs this stage first; that is
//! why those steps are exercised by the command path rather than by these tests.

use crate::game_launch::TargetExpectation;
use crate::gaming_session_guard::{try_acquire_session, SessionGuard};
use crate::gaming_session_persistence::{self as persistence, DurationKind, GamingSessionJournal};
use crate::process_monitor::{ProcessStatus, TrackedProcess};
use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::thread;
use std::time::{Duration, Instant, SystemTime};

/// Production cadences. Tests override them to stay bounded.
pub const PRODUCTION_POLL_INTERVAL: Duration = Duration::from_millis(2500);
pub const PRODUCTION_CHECKPOINT_INTERVAL: Duration = Duration::from_secs(15);
pub const PRODUCTION_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(20);

#[derive(Clone, Debug)]
pub struct LaunchMonitorConfig {
    pub app_data: PathBuf,
    pub game_id: String,
    /// The executable SILO starts.
    pub executable: PathBuf,
    /// What has to be pinned as "the game" once the launch happened.
    pub expectation: TargetExpectation,
    pub cwd: PathBuf,
    pub initial_save_path: Option<String>,
    pub name_keys: Vec<String>,
    pub discovery_timeout: Duration,
    pub poll_interval: Duration,
    pub checkpoint_interval: Duration,
}

#[derive(Debug)]
pub struct MonitoredSession {
    pub journal: GamingSessionJournal,
    pub target_started_at: String,
    pub started_sys: SystemTime,
    pub elapsed_secs: u64,
    pub exit_code: Option<u32>,
    pub detected_path: Option<PathBuf>,
    pub detected_roots: HashSet<PathBuf>,
    /// Whether the duration is exact or a checkpoint-derived lower bound. An
    /// uncertain duration is kept out of the exact playtime aggregate.
    pub duration_kind: DurationKind,
    /// Holds the single-active-session reservation until the tail finishes.
    pub guard: Option<SessionGuard>,
}

#[derive(Debug)]
pub enum MonitorOutcome {
    /// Confirmed target exit. The journal is pinned and left on disk; the caller
    /// completes SaveGuard, finalizes aggregates and clears it.
    Ended(Box<MonitoredSession>),
    /// Spawn failed; no journal is left behind.
    LaunchFailed,
    /// Launch happened (or may have happened) but target identity was never
    /// established, or became unknown. Journal is retained for recovery.
    TrackingUncertain,
    /// The durable journal could not be written; no session was started.
    JournalUnavailable,
    /// Another Gaming Mode session already owns the journal; nothing was touched.
    SessionAlreadyActive,
}

pub fn monitor_session(
    cfg: LaunchMonitorConfig,
    on_target_pinned: &dyn Fn(),
    on_spawn_confirmed: &dyn Fn(u32),
) -> MonitorOutcome {
    // The journal is a single file: reserve before writing anything.
    let Some(guard) = try_acquire_session() else {
        log::warn!(
            "Gaming Mode launch rejected: another session already owns the journal ({})",
            cfg.game_id
        );
        return MonitorOutcome::SessionAlreadyActive;
    };
    let started_at = chrono::Local::now().to_rfc3339();
    let mut journal = GamingSessionJournal::new(cfg.game_id.clone(), 0, started_at);
    if let Err(error) = persistence::write_journal(&cfg.app_data, &journal) {
        log::error!("Gaming Mode could not persist provisional journal: {error}");
        return MonitorOutcome::JournalUnavailable;
    }

    let spawn_args = Vec::<OsString>::new();
    let root = match crate::game_launch::spawn_direct_game(&cfg.executable, &cfg.cwd, &spawn_args, true) {
        Ok(crate::game_launch::DirectSpawnOutcome::Tracked(process)) => {
            on_spawn_confirmed(process.identity.pid);
            process
        }
        Ok(crate::game_launch::DirectSpawnOutcome::LaunchedButTrackingUncertain { pid, .. }) => {
            if let Some(pid) = pid {
                on_spawn_confirmed(pid);
            }
            journal.launch_root_pid = pid.unwrap_or(0);
            journal.retain_uncertain(chrono::Local::now().to_rfc3339());
            let _ = persistence::write_journal(&cfg.app_data, &journal);
            log::warn!(
                "Gaming Mode launch identity uncertain; retained journal for {}",
                cfg.game_id
            );
            return MonitorOutcome::TrackingUncertain;
        }
        Err(error) => {
            let _ = std::fs::remove_file(persistence::journal_path(&cfg.app_data));
            log::error!("Gaming Mode direct spawn failed: {error:?}");
            return MonitorOutcome::LaunchFailed;
        }
    };
    journal.launch_root_pid = root.identity.pid;

    // What may be pinned is decided before anything is observed:
    //
    // * `SpawnedProcess` — the process SILO spawned is the game. The retained
    //   handle from the spawn is used, so no PID round trip is needed.
    // * `ChildExecutable(path)` — only a strict descendant running that exact
    //   canonical executable path may be pinned. A helper, updater, anti-cheat
    //   or crash-reporter process has a different path and is never pinned.
    // * `UnidentifiableChild` — nothing may be pinned at all, so the session
    //   stays uncertain and the journal is retained.
    let mut target: Option<TrackedProcess> = None;
    let mut launcher: Option<TrackedProcess> = None;
    let mut descendant_path: Option<PathBuf> = None;
    match &cfg.expectation {
        TargetExpectation::SpawnedProcess => target = Some(root),
        TargetExpectation::ChildExecutable(path) => {
            launcher = Some(root);
            descendant_path = Some(path.clone());
        }
        TargetExpectation::UnidentifiableChild => {}
    }

    if descendant_path.is_none() && target.is_none() {
        // The game process cannot be identified from the available evidence.
        // The launch still happened, so the evidence is retained rather than
        // discarded, and nothing is pinned.
        journal.retain_uncertain(chrono::Local::now().to_rfc3339());
        let _ = persistence::write_journal(&cfg.app_data, &journal);
        log::warn!(
            "Gaming Mode could not identify the game process; journal retained for {}",
            cfg.game_id
        );
        return MonitorOutcome::TrackingUncertain;
    }

    let mut system = sysinfo::System::new();
    let discovery_started = Instant::now();
    while target.is_none() && discovery_started.elapsed() < cfg.discovery_timeout {
        let Some(launcher) = launcher.as_ref() else { break };
        let Some(descendant_path) = descendant_path.as_ref() else {
            break;
        };
        if launcher.poll() == ProcessStatus::Unknown(crate::process_monitor::UnknownReason::Unsupported) {
            break;
        }
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        let candidates: Vec<_> = system
            .processes()
            .iter()
            .map(|(pid, process)| crate::game_launch::ProcessCandidate {
                pid: pid.as_u32(),
                parent_pid: process.parent().map(sysinfo::Pid::as_u32),
                executable: process.exe().map(Path::to_path_buf),
            })
            .collect();
        if let Some(candidate) =
            crate::game_launch::find_target_descendant(launcher.identity.pid, descendant_path, &candidates)
        {
            if let Ok(pinned) = TrackedProcess::open(candidate.pid) {
                if pinned
                    .identity
                    .executable
                    .as_deref()
                    .is_some_and(|path| Path::new(path).canonicalize().ok() == descendant_path.canonicalize().ok())
                {
                    target = Some(pinned);
                }
            }
        }
        if launcher.poll() == ProcessStatus::Exited && target.is_none() {
            thread::sleep(Duration::from_millis(100));
        } else if target.is_none() {
            thread::sleep(Duration::from_millis(200));
        }
    }
    let Some(target) = target else {
        journal.retain_uncertain(chrono::Local::now().to_rfc3339());
        let _ = persistence::write_journal(&cfg.app_data, &journal);
        log::warn!(
            "Gaming Mode could not pin exact game identity; journal retained for {}",
            cfg.game_id
        );
        return MonitorOutcome::TrackingUncertain;
    };
    let target_started_at = chrono::Local::now().to_rfc3339();
    journal.started_at_utc = target_started_at.clone();
    journal.checkpoint_at_utc = target_started_at.clone();
    journal.pin_target(&target.identity);
    // Pin-time checkpoint: a crash inside the first checkpoint window still has a
    // durable elapsed baseline to reconcile from.
    journal.checkpoint(0, target_started_at.clone());
    if let Err(error) = persistence::write_journal(&cfg.app_data, &journal) {
        log::error!("Gaming Mode could not persist pinned game identity: {error}");
        return MonitorOutcome::JournalUnavailable;
    }
    // Launcher effects (hide window, background priority) belong to the caller and
    // only run once the exact target identity is pinned.
    on_target_pinned();

    let start = Instant::now();
    let started_sys = SystemTime::now();
    let (_, stored_is_suspect, watch_for_saves) =
        crate::commands::save_watch_required(cfg.initial_save_path.as_deref(), &cfg.name_keys);
    let mut detected_path = None;
    let mut save_watch = crate::commands::SaveWatchEvents::new(watch_for_saves);
    let watch_dirs = crate::commands::get_watch_directories();
    let mut roots = HashSet::new();
    let mut known_pids = HashSet::from([target.identity.pid, journal.launch_root_pid]);
    let mut last_checkpoint = Instant::now();
    loop {
        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
        known_pids.extend(crate::commands::collect_subtree_pids(&system, journal.launch_root_pid));
        if detected_path.is_some() {
            save_watch.stop();
        }
        let due = save_watch.take_due_paths(detected_path.is_some(), &roots);
        crate::commands::attribute_save_paths(
            due,
            &mut detected_path,
            &mut roots,
            stored_is_suspect,
            &mut system,
            &known_pids,
            Some(target.identity.pid),
            &watch_dirs,
            &cfg.name_keys,
        );
        // Durable checkpoint: bounds what a crash can lose and gives startup
        // reconciliation an elapsed time to work with.
        if last_checkpoint.elapsed() >= cfg.checkpoint_interval {
            journal.checkpoint(start.elapsed().as_secs(), chrono::Local::now().to_rfc3339());
            if let Err(error) = persistence::write_journal(&cfg.app_data, &journal) {
                log::warn!("Gaming Mode checkpoint failed; continuing session: {error}");
            }
            last_checkpoint = Instant::now();
        }
        match target.poll() {
            ProcessStatus::Running => thread::sleep(cfg.poll_interval),
            ProcessStatus::Exited => break,
            ProcessStatus::Unknown(_) => {
                journal.retain_uncertain(chrono::Local::now().to_rfc3339());
                let _ = persistence::write_journal(&cfg.app_data, &journal);
                log::warn!(
                    "Gaming Mode target monitoring uncertain; journal retained for {}",
                    cfg.game_id
                );
                return MonitorOutcome::TrackingUncertain;
            }
        }
    }
    save_watch.stop();
    let elapsed_secs = start.elapsed().as_secs();
    // Final elapsed is persisted before the caller finalizes, so the journal on disk
    // always describes the session that is about to be committed.
    journal.checkpoint(elapsed_secs, chrono::Local::now().to_rfc3339());
    // This process observed the target from pin to exit without interruption, so
    // the duration is an exact measurement. It is persisted separately from the
    // checkpoint-derived lower bound.
    journal.measure(elapsed_secs);
    let _ = persistence::write_journal(&cfg.app_data, &journal);
    MonitorOutcome::Ended(Box::new(MonitoredSession {
        journal,
        target_started_at,
        started_sys,
        elapsed_secs,
        exit_code: target.exit_code().ok().flatten(),
        detected_path,
        detected_roots: roots,
        duration_kind: DurationKind::Exact,
        guard: Some(guard),
    }))
}

/// Configuration for reattaching to a session that survived an app restart.
pub struct ReattachConfig {
    pub app_data: PathBuf,
    pub poll_interval: Duration,
    pub checkpoint_interval: Duration,
}

/// Continues monitoring a still-running target using its retained identity handle.
///
/// The duration is a lower bound: the app may have been down for an unknown period
/// while the game ran, so the commit is marked uncertain instead of claiming an exact
/// playtime.
pub fn reattach_session(
    cfg: ReattachConfig,
    mut journal: GamingSessionJournal,
    target: TrackedProcess,
    on_target_pinned: &dyn Fn(),
) -> MonitorOutcome {
    let Some(guard) = try_acquire_session() else {
        log::warn!(
            "Gaming Mode reattachment skipped: another session already owns the journal ({})",
            journal.game_id
        );
        return MonitorOutcome::SessionAlreadyActive;
    };
    let app_data = cfg.app_data.clone();
    journal.retain_uncertain(chrono::Local::now().to_rfc3339());
    if let Err(error) = persistence::write_journal(&app_data, &journal) {
        log::error!("Gaming Mode could not persist reattached journal: {error}");
        return MonitorOutcome::JournalUnavailable;
    }
    on_target_pinned();

    let prior_elapsed = journal.elapsed_seconds.unwrap_or(0);
    let start = Instant::now();
    let started_sys = SystemTime::now();
    let mut last_checkpoint = Instant::now();
    loop {
        if last_checkpoint.elapsed() >= cfg.checkpoint_interval {
            journal.checkpoint(
                prior_elapsed + start.elapsed().as_secs(),
                chrono::Local::now().to_rfc3339(),
            );
            if let Err(error) = persistence::write_journal(&app_data, &journal) {
                log::warn!("Gaming Mode reattach checkpoint failed: {error}");
            }
            last_checkpoint = Instant::now();
        }
        match target.poll() {
            ProcessStatus::Running => thread::sleep(cfg.poll_interval),
            ProcessStatus::Exited => break,
            ProcessStatus::Unknown(_) => {
                journal.retain_uncertain(chrono::Local::now().to_rfc3339());
                let _ = persistence::write_journal(&app_data, &journal);
                log::warn!(
                    "Gaming Mode reattached monitoring uncertain; journal retained for {}",
                    journal.game_id
                );
                return MonitorOutcome::TrackingUncertain;
            }
        }
    }
    let elapsed_secs = prior_elapsed + start.elapsed().as_secs();
    journal.checkpoint(elapsed_secs, chrono::Local::now().to_rfc3339());
    // No exact measurement: the app was down for an unknown part of the session,
    // so the duration stays a lower bound and is never recorded as exact.
    let _ = persistence::write_journal(&app_data, &journal);
    let target_started_at = journal.started_at_utc.clone();
    MonitorOutcome::Ended(Box::new(MonitoredSession {
        journal,
        target_started_at,
        started_sys,
        elapsed_secs,
        exit_code: target.exit_code().ok().flatten(),
        detected_path: None,
        detected_roots: HashSet::new(),
        duration_kind: DurationKind::Uncertain,
        guard: Some(guard),
    }))
}
