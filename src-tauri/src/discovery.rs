/* ── discovery.rs — user-registered folders scanned for new game installs ────
The user registers one or more folders once; on every startup SILO scans
only those folders, compares what it finds with the library and with the
decisions it has already recorded, and offers only genuinely new games.

Everything here lives in its own file next to games.json — never inside it —
so discovery history can be reset without touching the library:

    discovery.json
    {
      "version": 1,
      "enabled": true,
      "folders": ["C:\\Games"],
      "decisions": { "<key>": { "state": "pending", "name": "…", … } }
    }

A candidate is identified by its install folder name plus its executable
name, normalised for case and separators. That is what keeps two copies of
the same game in two registered folders from producing two prompts, and what
keeps a game already in the library from being offered again.
───────────────────────────────────────────────────────────────────────── */

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashMap};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::Mutex;
use tauri::{AppHandle, Manager};

/// Discovered, and waiting for an answer. Kept across restarts: "Not now" must
/// not count as a rejection, so the candidate stays eligible next launch.
pub const STATE_PENDING: &str = "pending";
/// The user added it and the record was saved.
pub const STATE_ADDED: &str = "added";
/// The user said no. Never offered again until they reset the decision.
pub const STATE_IGNORED: &str = "ignored";

/// One remembered decision. `name` and `exe_path` are kept for the settings
/// list and for the log; identity is the map key, not these fields.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryDecision {
    pub state: String,
    #[serde(default)]
    pub name: String,
    #[serde(default, rename = "exePath")]
    pub exe_path: String,
    #[serde(default, rename = "installDir")]
    pub install_dir: String,
    /// RFC 3339 timestamp of the last time this candidate was seen.
    #[serde(default, rename = "seenAt")]
    pub seen_at: String,
}

/// The whole on-disk record. `#[serde(default)]` on the container means an
/// older or hand-edited file keeps working: missing fields come from
/// `Default`, so scanning defaults to enabled and unknown keys are ignored.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct DiscoveryStore {
    pub version: u32,
    pub enabled: bool,
    pub folders: Vec<String>,
    pub decisions: BTreeMap<String, DiscoveryDecision>,
}

impl Default for DiscoveryStore {
    fn default() -> Self {
        DiscoveryStore {
            version: 1,
            // Scanning is on by default, but nothing is scanned until the user
            // registers a folder: this flag never implies scanning a drive.
            enabled: true,
            folders: Vec::new(),
            decisions: BTreeMap::new(),
        }
    }
}

/// A game the scan believes is new. `key` is the remembered identity.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryCandidate {
    pub key: String,
    pub name: String,
    #[serde(rename = "exePath")]
    pub exe_path: String,
    #[serde(rename = "installDir")]
    pub install_dir: String,
    /// The registered folder this candidate was found under.
    pub folder: String,
}

/// What Settings renders: the configuration plus the decisions it can reset.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryStateView {
    pub enabled: bool,
    pub folders: Vec<String>,
    pub ignored: Vec<DiscoveryDecisionView>,
    pub pending: usize,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiscoveryDecisionView {
    pub key: String,
    pub name: String,
    #[serde(rename = "exePath")]
    pub exe_path: String,
    pub state: String,
}

fn get_store_path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("discovery.json")
}

fn read_store_at(path: &Path) -> DiscoveryStore {
    match fs::read_to_string(path) {
        Ok(data) => serde_json::from_str::<DiscoveryStore>(&data).unwrap_or_else(|error| {
            log::warn!("discovery.json could not be parsed ({error}); starting from defaults");
            DiscoveryStore::default()
        }),
        Err(_) => DiscoveryStore::default(),
    }
}

fn write_store_at(path: &Path, store: &DiscoveryStore) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(store).map_err(|e| e.to_string())?;
    let tmp = path.with_extension("json.tmp");
    {
        use std::io::Write;
        let mut file = fs::File::create(&tmp).map_err(|e| e.to_string())?;
        file.write_all(json.as_bytes()).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp, path).map_err(|e| e.to_string())
}

pub fn load_store(app: &AppHandle) -> DiscoveryStore {
    read_store_at(&get_store_path(app))
}

fn save_store(app: &AppHandle, store: &DiscoveryStore) -> Result<(), String> {
    write_store_at(&get_store_path(app), store)
}

// ── Identity ────────────────────────────────────────────────────────────────

/// Path comparison form: lowercase, forward slashes, no trailing slash. Two
/// spellings of the same Windows path then compare equal.
pub fn normalize_path(path: &str) -> String {
    let mut normalised = path.trim().replace('\\', "/").to_ascii_lowercase();
    while normalised.ends_with('/') && normalised.len() > 1 {
        normalised.pop();
    }
    normalised
}

fn file_name_lower(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().to_ascii_lowercase())
        .unwrap_or_default()
}

/// The identity a decision is remembered under: the install folder's name and
/// the executable's name, both lowercased. Deliberately not the full path —
/// that is what lets `C:\Games\Hades` and `D:\Backup\Hades` be recognised as
/// the same installation, and what stops a moved folder from re-prompting.
pub fn candidate_key(install_dir: &Path, exe_path: &Path) -> String {
    let folder = file_name_lower(install_dir);
    let exe = file_name_lower(exe_path);
    format!("{}|{}", folder, exe)
}

/// The folder directly under the scan root that holds this executable. When
/// the exe sits directly in the root, its parent folder is the install folder.
pub fn install_dir_for(root: &Path, exe_path: &Path) -> PathBuf {
    let Ok(relative) = exe_path.strip_prefix(root) else {
        return exe_path.parent().map(Path::to_path_buf).unwrap_or_default();
    };
    let mut components = relative.components();
    match (components.next(), components.next()) {
        // <root>/<game>/… → <root>/<game>
        (Some(first), Some(_)) => root.join(first.as_os_str()),
        // <root>/game.exe → the root itself is the install folder
        _ => root.to_path_buf(),
    }
}

/// True for SILO's own binaries and for anything inside its data folders: the
/// launcher must never offer itself as a discovered game.
fn is_launcher_itself(exe_path: &Path, own_exe: Option<&Path>, own_dirs: &[PathBuf]) -> bool {
    if let Some(own) = own_exe {
        if file_name_lower(exe_path) == file_name_lower(own) {
            return true;
        }
    }
    if file_name_lower(exe_path) == "silo.exe" || file_name_lower(exe_path) == "silo-uninstall.exe" {
        return true;
    }
    let exe = normalize_path(&exe_path.to_string_lossy());
    own_dirs.iter().any(|dir| {
        let dir = normalize_path(&dir.to_string_lossy());
        !dir.is_empty() && (exe == dir || exe.starts_with(&format!("{dir}/")))
    })
}

/// Keeps the candidates that deserve a prompt, in scan order, one per identity.
///
/// Excluded: anything already in the library (by normalised exe path, or by a
/// remembered `added` decision), anything the user ignored, and a repeat of an
/// identity already queued in this same scan.
pub fn keep_new_candidates(
    candidates: Vec<DiscoveryCandidate>,
    library_paths: &[String],
    store: &DiscoveryStore,
) -> Vec<DiscoveryCandidate> {
    let library: Vec<String> = library_paths.iter().map(|p| normalize_path(p)).collect();
    let mut seen: HashMap<String, ()> = HashMap::new();
    let mut out = Vec::new();

    for candidate in candidates {
        let exe = normalize_path(&candidate.exe_path);
        if library.iter().any(|known| known == &exe) {
            continue;
        }
        match store.decisions.get(&candidate.key).map(|d| d.state.as_str()) {
            Some(STATE_IGNORED) | Some(STATE_ADDED) => continue,
            // `pending` is deliberately allowed: "Not now" must ask again next
            // startup, and an answer can be lost to a crash mid-prompt.
            _ => {}
        }
        if seen.insert(candidate.key.clone(), ()).is_some() {
            continue;
        }
        out.push(candidate);
    }
    out
}

fn state_view(store: &DiscoveryStore) -> DiscoveryStateView {
    let ignored = store
        .decisions
        .iter()
        .filter(|(_, decision)| decision.state == STATE_IGNORED)
        .map(|(key, decision)| DiscoveryDecisionView {
            key: key.clone(),
            name: decision.name.clone(),
            exe_path: decision.exe_path.clone(),
            state: decision.state.clone(),
        })
        .collect();
    let pending = store
        .decisions
        .values()
        .filter(|decision| decision.state == STATE_PENDING)
        .count();
    DiscoveryStateView {
        enabled: store.enabled,
        folders: store.folders.clone(),
        ignored,
        pending,
    }
}

fn now_rfc3339() -> String {
    chrono::Local::now().to_rfc3339()
}

// Scans run inside one process at most once every few seconds: the renderer
// asks once at startup, and a second request (window focus, a settings refresh)
// must not walk the user's folders again.
static LAST_SCAN: Mutex<Option<std::time::Instant>> = Mutex::new(None);
const RESCAN_COOLDOWN: std::time::Duration = std::time::Duration::from_secs(30);

fn scan_allowed_now() -> bool {
    let mut last = match LAST_SCAN.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    };
    match *last {
        Some(at) if at.elapsed() < RESCAN_COOLDOWN => false,
        _ => {
            *last = Some(std::time::Instant::now());
            true
        }
    }
}

// ── Commands ────────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_discovery_state(app: AppHandle) -> DiscoveryStateView {
    state_view(&load_store(&app))
}

#[tauri::command]
pub fn discovery_add_folder(app: AppHandle, path: String) -> Result<DiscoveryStateView, String> {
    let folder = Path::new(path.trim());
    if !folder.is_dir() {
        return Err("That folder does not exist".to_string());
    }
    let mut store = load_store(&app);
    let normalised = normalize_path(&path);
    if store.folders.iter().any(|known| normalize_path(known) == normalised) {
        return Ok(state_view(&store));
    }
    store.folders.push(folder.to_string_lossy().into_owned());
    save_store(&app, &store)?;
    Ok(state_view(&store))
}

#[tauri::command]
pub fn discovery_remove_folder(app: AppHandle, path: String) -> Result<DiscoveryStateView, String> {
    let mut store = load_store(&app);
    let normalised = normalize_path(&path);
    store.folders.retain(|known| normalize_path(known) != normalised);
    save_store(&app, &store)?;
    Ok(state_view(&store))
}

#[tauri::command]
pub fn discovery_set_enabled(app: AppHandle, enabled: bool) -> Result<DiscoveryStateView, String> {
    let mut store = load_store(&app);
    store.enabled = enabled;
    save_store(&app, &store)?;
    Ok(state_view(&store))
}

/// Records what the user decided about one candidate. Closing the prompt does
/// not call this at all, which is what keeps a dismissed candidate pending.
#[tauri::command]
pub fn record_discovery_decision(
    app: AppHandle,
    candidate: DiscoveryCandidate,
    state: String,
) -> Result<DiscoveryStateView, String> {
    if !matches!(state.as_str(), STATE_ADDED | STATE_IGNORED | STATE_PENDING) {
        return Err("Unknown discovery decision".to_string());
    }
    let mut store = load_store(&app);
    store.decisions.insert(
        candidate.key.clone(),
        DiscoveryDecision {
            state,
            name: candidate.name,
            exe_path: candidate.exe_path,
            install_dir: candidate.install_dir,
            seen_at: now_rfc3339(),
        },
    );
    save_store(&app, &store)?;
    Ok(state_view(&store))
}

#[tauri::command]
pub fn clear_discovery_decision(app: AppHandle, key: String) -> Result<DiscoveryStateView, String> {
    let mut store = load_store(&app);
    store.decisions.remove(&key);
    save_store(&app, &store)?;
    Ok(state_view(&store))
}

/// "Reconsider ignored games": forget every rejection, leave `added` alone
/// (those are in the library already and are filtered by path anyway).
#[tauri::command]
pub fn reset_ignored_discoveries(app: AppHandle) -> Result<DiscoveryStateView, String> {
    let mut store = load_store(&app);
    store.decisions.retain(|_, decision| decision.state != STATE_IGNORED);
    save_store(&app, &store)?;
    Ok(state_view(&store))
}

/// Scans the registered folders for games that are not in the library yet.
/// Runs off the UI thread and never fails the caller: an unreadable folder is
/// logged and skipped, so a disconnected drive cannot break startup.
#[tauri::command]
pub async fn discover_games(app: AppHandle) -> Vec<DiscoveryCandidate> {
    let store = load_store(&app);
    if !store.enabled || store.folders.is_empty() {
        return Vec::new();
    }
    if !scan_allowed_now() {
        log::info!("discovery scan skipped: a scan already ran moments ago");
        return Vec::new();
    }

    let games = crate::commands::get_games(app.clone());
    let library_paths: Vec<String> = games.iter().filter_map(|game| game.exe_path.clone()).collect();

    let own_exe = std::env::current_exe().ok();
    let own_dirs: Vec<PathBuf> = {
        let mut dirs = Vec::new();
        if let Some(exe) = own_exe.as_deref() {
            if let Some(parent) = exe.parent() {
                dirs.push(parent.to_path_buf());
            }
        }
        if let Ok(data) = app.path().app_data_dir() {
            dirs.push(data);
        }
        if let Ok(local) = app.path().app_local_data_dir() {
            dirs.push(local);
        }
        dirs
    };

    let folders = store.folders.clone();
    let scanned = tauri::async_runtime::spawn_blocking(move || {
        let mut found = Vec::new();
        for folder in folders {
            match crate::commands::scan_folder_core(&folder) {
                Ok(games) => {
                    log::info!("discovery: {} candidate(s) under {}", games.len(), folder);
                    found.push((folder, games));
                }
                Err(error) => log::warn!("discovery: cannot scan {}: {}", folder, error),
            }
        }
        found
    })
    .await
    .unwrap_or_default();

    let mut candidates: Vec<DiscoveryCandidate> = Vec::new();
    for (folder, scanned_games) in scanned {
        let root = PathBuf::from(&folder);
        for game in scanned_games {
            let exe_path = PathBuf::from(&game.exe_path);
            if is_launcher_itself(&exe_path, own_exe.as_deref(), &own_dirs) {
                continue;
            }
            let install_dir = install_dir_for(&root, &exe_path);
            candidates.push(DiscoveryCandidate {
                key: candidate_key(&install_dir, &exe_path),
                name: game.name,
                exe_path: game.exe_path,
                install_dir: install_dir.to_string_lossy().into_owned(),
                folder: folder.clone(),
            });
        }
    }

    let mut store = load_store(&app);
    let fresh = keep_new_candidates(candidates, &library_paths, &store);
    let seen_at = now_rfc3339();
    for candidate in &fresh {
        store.decisions.insert(
            candidate.key.clone(),
            DiscoveryDecision {
                state: STATE_PENDING.to_string(),
                name: candidate.name.clone(),
                exe_path: candidate.exe_path.clone(),
                install_dir: candidate.install_dir.clone(),
                seen_at: seen_at.clone(),
            },
        );
    }
    if !fresh.is_empty() {
        if let Err(error) = save_store(&app, &store) {
            log::warn!("discovery: could not save pending candidates: {}", error);
        }
    }
    log::info!("discovery: {} new candidate(s) to offer", fresh.len());
    fresh
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decision(state: &str) -> DiscoveryDecision {
        DiscoveryDecision {
            state: state.to_string(),
            name: String::new(),
            exe_path: String::new(),
            install_dir: String::new(),
            seen_at: String::new(),
        }
    }

    fn candidate(key: &str, exe: &str) -> DiscoveryCandidate {
        DiscoveryCandidate {
            key: key.to_string(),
            name: key.to_string(),
            exe_path: exe.to_string(),
            install_dir: String::new(),
            folder: String::new(),
        }
    }

    #[test]
    fn normalize_path_ignores_case_and_separators() {
        assert_eq!(
            normalize_path("C:\\Games\\Hades\\Hades.exe"),
            normalize_path("c:/games/hades/hades.exe")
        );
        assert_eq!(normalize_path("C:\\Games\\"), "c:/games");
    }

    #[test]
    fn candidate_key_groups_the_same_game_in_two_roots() {
        let a = candidate_key(Path::new("C:\\Games\\Hades"), Path::new("C:\\Games\\Hades\\Hades.exe"));
        let b = candidate_key(
            Path::new("D:\\Backup\\Hades"),
            Path::new("D:\\Backup\\Hades\\Hades.exe"),
        );
        assert_eq!(a, b, "the same folder and exe names must share one identity");
    }

    #[test]
    fn candidate_key_separates_games_in_the_same_base_folder_name() {
        let a = candidate_key(Path::new("C:\\Games\\Hades"), Path::new("C:\\Games\\Hades\\Hades.exe"));
        let b = candidate_key(Path::new("C:\\Games\\Hades"), Path::new("C:\\Games\\Hades\\Hades2.exe"));
        assert_ne!(a, b);
    }

    #[test]
    fn install_dir_is_the_folder_under_the_scan_root() {
        let dir = install_dir_for(Path::new("C:\\Games"), Path::new("C:\\Games\\Hades\\Win64\\Hades.exe"));
        assert_eq!(normalize_path(&dir.to_string_lossy()), "c:/games/hades");
    }

    #[test]
    fn install_dir_falls_back_to_the_root_for_a_loose_exe() {
        let dir = install_dir_for(Path::new("C:\\Games"), Path::new("C:\\Games\\Hades.exe"));
        assert_eq!(normalize_path(&dir.to_string_lossy()), "c:/games");
    }

    #[test]
    fn launcher_is_never_a_candidate() {
        let own = PathBuf::from("C:\\Users\\me\\AppData\\Local\\SILO\\silo.exe");
        let dirs = vec![PathBuf::from("C:\\Users\\me\\AppData\\Local\\SILO")];
        assert!(is_launcher_itself(&own, Some(&own), &dirs));
        assert!(is_launcher_itself(
            Path::new("C:\\Users\\me\\AppData\\Local\\SILO\\silo.exe"),
            None,
            &dirs
        ));
        assert!(!is_launcher_itself(
            Path::new("C:\\Games\\Hades\\Hades.exe"),
            Some(&own),
            &dirs
        ));
    }

    #[test]
    fn library_games_are_not_offered_again() {
        let store = DiscoveryStore::default();
        let library = vec!["C:\\Games\\Hades\\Hades.exe".to_string()];
        let fresh = keep_new_candidates(
            vec![
                candidate("hades|hades.exe", "c:/games/hades/hades.exe"),
                candidate("celeste|celeste.exe", "C:\\Games\\Celeste\\Celeste.exe"),
            ],
            &library,
            &store,
        );
        assert_eq!(fresh.len(), 1);
        assert_eq!(fresh[0].key, "celeste|celeste.exe");
    }

    #[test]
    fn ignored_and_added_candidates_are_skipped_but_pending_is_kept() {
        let mut store = DiscoveryStore::default();
        store.decisions.insert("a|a.exe".into(), decision(STATE_IGNORED));
        store.decisions.insert("b|b.exe".into(), decision(STATE_ADDED));
        store.decisions.insert("c|c.exe".into(), decision(STATE_PENDING));

        let fresh = keep_new_candidates(
            vec![
                candidate("a|a.exe", "C:/G/a/a.exe"),
                candidate("b|b.exe", "C:/G/b/b.exe"),
                candidate("c|c.exe", "C:/G/c/c.exe"),
                candidate("d|d.exe", "C:/G/d/d.exe"),
            ],
            &[],
            &store,
        );
        let keys: Vec<&str> = fresh.iter().map(|c| c.key.as_str()).collect();
        assert_eq!(keys, vec!["c|c.exe", "d|d.exe"]);
    }

    #[test]
    fn one_scan_never_queues_the_same_identity_twice() {
        let store = DiscoveryStore::default();
        let fresh = keep_new_candidates(
            vec![
                candidate("hades|hades.exe", "C:/Games/Hades/Hades.exe"),
                candidate("hades|hades.exe", "D:/Backup/Hades/Hades.exe"),
            ],
            &[],
            &store,
        );
        assert_eq!(fresh.len(), 1);
    }

    #[test]
    fn store_round_trips_and_tolerates_a_missing_file() {
        let dir = std::env::temp_dir().join(format!(
            "silo-discovery-test-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("discovery.json");

        assert_eq!(read_store_at(&path).folders.len(), 0);
        assert!(read_store_at(&path).enabled, "scanning defaults to enabled");

        let mut store = read_store_at(&path);
        store.folders.push("C:\\Games".into());
        store
            .decisions
            .insert("hades|hades.exe".into(), decision(STATE_IGNORED));
        write_store_at(&path, &store).unwrap();

        let loaded = read_store_at(&path);
        assert_eq!(loaded.folders, vec!["C:\\Games".to_string()]);
        assert_eq!(loaded.decisions.len(), 1);

        fs::remove_dir_all(&dir).ok();
    }

    #[test]
    fn a_missing_folder_is_reported_instead_of_panicking() {
        // A registered folder on a drive that is gone: the scan must answer,
        // never crash startup.
        match crate::commands::scan_folder_core("C:\\__silo_no_such_folder__") {
            Ok(games) => assert!(games.is_empty(), "nothing can be inside a missing folder"),
            Err(message) => assert!(!message.trim().is_empty(), "the reason must be readable"),
        }
    }

    #[test]
    fn ignored_view_lists_only_rejections() {
        let mut store = DiscoveryStore::default();
        store.decisions.insert("a|a.exe".into(), decision(STATE_IGNORED));
        store.decisions.insert("b|b.exe".into(), decision(STATE_PENDING));
        let view = state_view(&store);
        assert_eq!(view.ignored.len(), 1);
        assert_eq!(view.pending, 1);
    }
}
