use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, Emitter};
use std::process::Command;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::sync::mpsc::channel;
use std::collections::{HashMap, HashSet};
use notify::{Watcher, RecursiveMode, EventKind};
use walkdir::WalkDir;
use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;
#[cfg(not(target_os = "windows"))]
use std::os::unix::fs::PermissionsExt;


#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Game {
    pub id: String,
    pub name: String,
    #[serde(rename = "exePath", default)]
    pub exe_path: Option<String>,
    #[serde(default)]
    pub wallpaper: Option<String>,
    #[serde(rename = "logoPath", default)]
    pub logo_path: Option<String>,
    #[serde(rename = "fontFamily", default)]
    pub font_family: Option<String>,
    #[serde(rename = "fontColor", default)]
    pub font_color: Option<String>,
    #[serde(rename = "playtimeMinutes", default)]
    pub playtime_minutes: u32,
    #[serde(rename = "sessionCount", default)]
    pub session_count: u32,
    #[serde(rename = "lastPlayed", default)]
    pub last_played: Option<String>,
    #[serde(default)]
    pub status: Option<String>,
    #[serde(rename = "addedAt", default)]
    pub added_at: String,
    #[serde(rename = "savePath", default)]
    pub save_path: Option<String>,
    #[serde(rename = "savePathSource", default)]
    pub save_path_source: Option<String>,
    #[serde(rename = "backupCount", default)]
    pub backup_count: Option<u32>,
    #[serde(rename = "isInstalled", default)]
    pub is_installed: Option<bool>,
    #[serde(default)]
    pub favorite: bool,
    #[serde(default)]
    pub tags: Vec<String>,
    /// What the exePath points at, as chosen in the Edit modal: "auto" (SILO's
    /// guess, resolved per-launch), "launcher", or "game". Serialization uses
    /// camelCase like every other field.
    #[serde(rename = "launchTarget", default)]
    pub launch_target: Option<String>,
    /// Directory containing the executable. Reserved for future per-game exe
    /// caching; not currently read.
    #[serde(default, skip_serializing)]
    #[allow(dead_code)]
    pub exe_dir: Option<String>,
}

fn get_data_path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("games.json")
}

fn get_wallpapers_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("wallpapers")
}

fn normalize_save_path(path: Option<&str>) -> Result<Option<String>, String> {
    let Some(path) = path.map(str::trim).filter(|path| !path.is_empty()) else {
        return Ok(None);
    };

    let path = Path::new(path);
    if !path.exists() {
        return Err("Save folder does not exist".to_string());
    }
    if !path.is_dir() {
        return Err("Save path must point to a folder".to_string());
    }

    let canonical = path
        .canonicalize()
        .map_err(|e| format!("Could not resolve save folder: {}", e))?;
    if canonical.file_name().is_none() {
        return Err("The root of a drive cannot be used as a save folder".to_string());
    }

    Ok(Some(canonical.to_string_lossy().into_owned()))
}

fn save_games_atomic(app: &AppHandle, games: &[Game]) -> Result<(), String> {
    let data_path = get_data_path(app);
    if let Some(parent) = data_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }

    let json_str = serde_json::to_string_pretty(games).map_err(|e| e.to_string())?;
    let tmp_path = data_path.with_extension("json.tmp");
    let bak_path = data_path.with_extension("json.bak");

    {
        let mut file = File::create(&tmp_path).map_err(|e| e.to_string())?;
        file.write_all(json_str.as_bytes()).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        file.sync_all().map_err(|e| e.to_string())?;
    }

    if data_path.exists() {
        let _ = fs::copy(&data_path, &bak_path);
    }

    fs::rename(&tmp_path, &data_path).map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn get_games(app: AppHandle) -> Vec<Game> {
    let data_path = get_data_path(&app);
    let bak_path = data_path.with_extension("json.bak");

    if !data_path.exists() {
        if bak_path.exists() {
            if let Ok(data) = fs::read_to_string(&bak_path) {
                if let Ok(mut games) = serde_json::from_str::<Vec<Game>>(&data) {
                    let _ = app.emit("library-corrupt-recovered", "Primary library missing. Recovered from backup.");
                    for game in &mut games {
                        game.is_installed = Some(game.exe_path.as_ref().is_some_and(|p| Path::new(p).exists()));
                    }
                    return games;
                }
            }
        }
        return vec![];
    }

    let data = fs::read_to_string(&data_path).unwrap_or_default();
    match serde_json::from_str::<Vec<Game>>(&data) {
        Ok(mut games) => {
            for game in &mut games {
                game.is_installed = Some(game.exe_path.as_ref().is_some_and(|p| Path::new(p).exists()));
            }
            games
        },
        Err(e) => {
            log::error!("Failed to parse games.json: {}", e);
            if bak_path.exists() {
                if let Ok(bak_data) = fs::read_to_string(&bak_path) {
                    if let Ok(mut games) = serde_json::from_str::<Vec<Game>>(&bak_data) {
                        let _ = app.emit("library-corrupt-recovered", format!("Primary library corrupt ({}). Recovered from backup.", e));
                        for game in &mut games {
                            game.is_installed = Some(game.exe_path.as_ref().is_some_and(|p| Path::new(p).exists()));
                        }
                        return games;
                    }
                }
            }
            let _ = app.emit("library-corrupt-failed", format!("Primary library corrupt and backup recovery failed: {}", e));
            vec![]
        }
    }
}

/* ═══════════════════════════════════════════════════════════════════════════
   STARTUP INSTALL CHECK

   Verifies that every game in the library is still installed. Runs fully off
   the UI thread, a few paths at a time, and each probe is bounded by
   INSTALL_TIMEOUT so a disconnected network share or an unplugged external
   drive can never hang startup. Three states per game:

     installed   — the executable (or, for launcher entries, the resolved game
                   binary) is present
     missing     — not launchable, but its drive/root is reachable, so the game
                   was most likely uninstalled or moved
     unavailable — the drive/root itself is gone (external drive unplugged)

   The result is kept in app state with a timestamp. It is never persisted, and
   no game, save, backup or statistic is ever modified because of a check.
   ═══════════════════════════════════════════════════════════════════════════ */

pub const INSTALL_INSTALLED: &str = "installed";
pub const INSTALL_MISSING: &str = "missing";
pub const INSTALL_UNAVAILABLE: &str = "unavailable";

/// Per-path budget. A path that does not answer within this window is reported
/// as `unavailable`: we genuinely cannot tell, and we never guess "missing".
const INSTALL_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);
/// How many paths are probed at the same time.
const INSTALL_WORKERS: usize = 4;

#[derive(Debug, Clone, Serialize)]
pub struct InstallStatus {
    #[serde(rename = "game_id")]
    pub game_id: String,
    pub status: String,
}

/// In-memory only: the last check and when it ran. Never written to disk.
#[derive(Default)]
struct InstallSnapshot {
    checked_at: Option<Instant>,
    entries: Vec<InstallStatus>,
}

static INSTALL_SNAPSHOT: std::sync::OnceLock<std::sync::Mutex<InstallSnapshot>> =
    std::sync::OnceLock::new();

fn install_snapshot() -> &'static std::sync::Mutex<InstallSnapshot> {
    INSTALL_SNAPSHOT.get_or_init(|| std::sync::Mutex::new(InstallSnapshot::default()))
}

/// Windows: the drive is reachable when the volume root itself (`C:\`) exists.
#[cfg(target_os = "windows")]
fn drive_available(path: &Path) -> bool {
    match path.components().next() {
        Some(Component::Prefix(prefix)) => {
            let root = format!("{}\\", prefix.as_os_str().to_string_lossy());
            Path::new(&root).exists()
        }
        // Relative or UNC path: nothing better to probe than the path itself.
        _ => path.exists(),
    }
}

/// Unix: `/` always exists, so the mount point is what carries the meaning —
/// removable media vanishes with the device under one of the automount roots.
#[cfg(not(target_os = "windows"))]
fn drive_available(path: &Path) -> bool {
    const AUTOMOUNT_ROOTS: [&str; 4] = ["/media", "/mnt", "/run/media", "/Volumes"];
    for root in AUTOMOUNT_ROOTS {
        let Ok(rest) = path.strip_prefix(root) else {
            continue;
        };
        // Probe only the volume level: `/media/<user>/<volume>` on most distros.
        // A deleted game folder deeper down must not read as an unplugged disk.
        let mut probe = PathBuf::from(root);
        for part in rest.components().take(2) {
            probe.push(part);
            if !probe.exists() {
                return false;
            }
        }
        return true;
    }
    true
}

/// One game's probe, resolved inside the worker thread (finding the real game
/// exe for a launcher entry walks the install tree).
struct InstallJob {
    game_id: String,
    exe: Option<PathBuf>,
    launch_target: String,
}

fn install_job(game: &Game) -> InstallJob {
    InstallJob {
        game_id: game.id.clone(),
        exe: game
            .exe_path
            .as_deref()
            .map(str::trim)
            .filter(|p| !p.is_empty())
            .map(PathBuf::from),
        launch_target: game
            .launch_target
            .clone()
            .unwrap_or_else(|| "auto".to_string()),
    }
}

fn install_status_for(job: &InstallJob) -> &'static str {
    let Some(exe) = job.exe.as_deref() else {
        // No stored path: nothing to verify, so never nag about it.
        return INSTALL_INSTALLED;
    };

    if exe.is_file() {
        return INSTALL_INSTALLED;
    }

    // The stored exe is gone. When it was a launcher, SILO would actually run
    // the resolved game binary, so an install that still has *that* is fine.
    if job.launch_target != "game" {
        if let Some(real) = find_real_game_exe(exe) {
            if real.is_file() {
                return INSTALL_INSTALLED;
            }
        }
    }

    if drive_available(exe) {
        INSTALL_MISSING
    } else {
        INSTALL_UNAVAILABLE
    }
}

/// Probe a batch of games: INSTALL_WORKERS at a time, each probe timed out
/// individually. A probe that overruns leaves its thread behind rather than
/// stalling the batch — the app must never wait on a dead disk.
fn verify_installs(games: &[Game]) -> Vec<InstallStatus> {
    let jobs: Vec<InstallJob> = games.iter().map(install_job).collect();
    let mut out = Vec::with_capacity(jobs.len());

    for chunk in jobs.chunks(INSTALL_WORKERS) {
        let mut pending = Vec::with_capacity(chunk.len());
        for job in chunk {
            let (tx, rx) = channel();
            let probe = InstallJob {
                game_id: job.game_id.clone(),
                exe: job.exe.clone(),
                launch_target: job.launch_target.clone(),
            };
            std::thread::spawn(move || {
                let _ = tx.send(install_status_for(&probe));
            });
            pending.push((job.game_id.clone(), rx));
        }
        for (game_id, rx) in pending {
            let status = rx.recv_timeout(INSTALL_TIMEOUT).unwrap_or(INSTALL_UNAVAILABLE);
            out.push(InstallStatus {
                game_id,
                status: status.to_string(),
            });
        }
    }

    out
}

#[tauri::command]
pub async fn verify_library_installs(app: AppHandle) -> Vec<InstallStatus> {
    let games = get_games(app);
    let results = match tauri::async_runtime::spawn_blocking(move || verify_installs(&games)).await {
        Ok(results) => results,
        Err(e) => {
            log::error!("install check failed to run: {}", e);
            return Vec::new();
        }
    };

    let broken = results
        .iter()
        .filter(|r| r.status != INSTALL_INSTALLED)
        .count();

    if let Ok(mut snap) = install_snapshot().lock() {
        // Report what changed since the previous check, then store this one.
        let newly_broken = results
            .iter()
            .filter(|r| {
                r.status != INSTALL_INSTALLED
                    && !snap
                        .entries
                        .iter()
                        .any(|old| old.game_id == r.game_id && old.status != INSTALL_INSTALLED)
            })
            .count();
        snap.checked_at = Some(Instant::now());
        snap.entries = results.clone();
        log::info!(
            "install check: {} verified, {} need attention ({} newly)",
            results.len(),
            broken,
            newly_broken
        );
    }

    results
}

/// Re-verify a single game right before launching it. Updates the cached entry.
#[tauri::command]
pub async fn verify_game_install(app: AppHandle, game_id: String) -> Option<InstallStatus> {
    let games = get_games(app);
    let wanted = game_id.clone();
    let status = tauri::async_runtime::spawn_blocking(move || {
        games
            .iter()
            .find(|g| g.id == wanted)
            .map(|g| install_status_for(&install_job(g)).to_string())
    })
    .await
    .ok()
    .flatten()?;

    let entry = InstallStatus { game_id, status };
    if let Ok(mut snap) = install_snapshot().lock() {
        snap.entries.retain(|e| e.game_id != entry.game_id);
        snap.entries.push(entry.clone());
        snap.checked_at = Some(Instant::now());
    }
    Some(entry)
}

#[tauri::command]
// The IPC contract sends one argument per field, so the count is set by the
// command surface rather than by a design smell.
#[allow(clippy::too_many_arguments)]
pub fn add_game(app: AppHandle, name: String, exe_path: Option<String>, wallpaper: Option<String>, logo_path: Option<String>, font_family: Option<String>, font_color: Option<String>, save_path: Option<String>, launch_target: Option<String>) -> Result<Game, String> {
    let mut games = get_games(app.clone());
    let save_path = normalize_save_path(save_path.as_deref())?;
    let new_game = Game {
        id: std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis().to_string(),
        name,
        exe_path,
        wallpaper,
        logo_path,
        font_family,
        font_color,
        playtime_minutes: 0,
        session_count: 0,
        last_played: None,
        is_installed: Some(true),
        status: None,
        added_at: format!("{:?}", std::time::SystemTime::now()),
        save_path_source: save_path.as_ref().map(|_| "manual".to_string()),
        save_path,
        backup_count: Some(5),
        favorite: false,
        tags: vec![],
        launch_target: launch_target.filter(|t| matches!(t.as_str(), "auto" | "launcher" | "game")),
        exe_dir: None,
    };
    games.push(new_game.clone());
    save_games_atomic(&app, &games)?;
    Ok(new_game)
}

// Optional string fields on a game can be cleared by sending an explicit JSON null.
fn apply_optional(value: Option<&serde_json::Value>, target: &mut Option<String>) {
    match value {
        Some(value) if value.is_null() => *target = None,
        Some(value) => {
            if let Some(s) = value.as_str() {
                *target = Some(s.to_string());
            }
        }
        None => {}
    }
}

#[tauri::command]
pub fn update_game(app: AppHandle, id: String, updates: serde_json::Value) -> Result<Option<Game>, String> {
    let mut games = get_games(app.clone());
    if let Some(game) = games.iter_mut().find(|g| g.id == id) {
        if let Some(name) = updates.get("name").and_then(|v| v.as_str()) { game.name = name.to_string(); }
        apply_optional(updates.get("exePath"), &mut game.exe_path);
        apply_optional(updates.get("wallpaper"), &mut game.wallpaper);
        apply_optional(updates.get("logoPath"), &mut game.logo_path);
        apply_optional(updates.get("fontFamily"), &mut game.font_family);
        apply_optional(updates.get("fontColor"), &mut game.font_color);
        apply_optional(updates.get("status"), &mut game.status);
        apply_optional(updates.get("launchTarget"), &mut game.launch_target);

        if let Some(save_path) = updates.get("savePath") {
            if save_path.is_null() {
                game.save_path = None;
                game.save_path_source = None;
            } else if let Some(s) = save_path.as_str() {
                game.save_path = normalize_save_path(Some(s))?;
                game.save_path_source = game.save_path.as_ref().map(|_| "manual".to_string());
            }
        }
        if let Some(backup_count) = updates.get("backupCount").and_then(|v| v.as_u64()) {
            game.backup_count = Some(backup_count as u32);
        }
        if let Some(favorite) = updates.get("favorite").and_then(|v| v.as_bool()) {
            game.favorite = favorite;
        }
        match updates.get("tags") {
            Some(v) if v.is_null() => game.tags = vec![],
            Some(v) if v.is_array() => {
                game.tags = v
                    .as_array()
                    .unwrap()
                    .iter()
                    .filter_map(|t| t.as_str().map(|s| s.to_string()))
                    .collect();
            }
            _ => {}
        }

        let updated = game.clone();
        save_games_atomic(&app, &games)?;
        Ok(Some(updated))
    } else {
        Ok(None)
    }
}

/// Inspect the configured exe and report whether it looks like a launcher and
/// which sibling exe SILO would pick as the real game. The Edit modal uses this
/// to offer a direct "go to the game" launch option. `real_game_path` is null
/// when the configured exe already looks like the game itself.
#[derive(serde::Serialize)]
#[serde(rename_all = "camelCase")]
pub struct LaunchTargetInfo {
    pub launcher_like: bool,
    pub real_game_path: Option<String>,
}

#[tauri::command]
pub fn detect_launch_target(exe_path: String) -> LaunchTargetInfo {
    let path = Path::new(&exe_path);
    let launcher_like = path
        .file_name()
        .map(|n| is_launcher_like(&n.to_string_lossy().to_lowercase()))
        .unwrap_or(false);
    let real_game_path = if launcher_like {
        find_real_game_exe(path).map(|p| p.to_string_lossy().into_owned())
    } else {
        None
    };
    LaunchTargetInfo { launcher_like, real_game_path }
}

#[tauri::command]
pub fn delete_game(app: AppHandle, id: String) -> Result<bool, String> {
    let mut games = get_games(app.clone());
    let initial_len = games.len();
    games.retain(|g| g.id != id);
    if games.len() != initial_len {
        save_games_atomic(&app, &games)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

static SYSTEM_FONTS_CACHE: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();

#[tauri::command]
pub fn get_system_fonts() -> Vec<String> {
    SYSTEM_FONTS_CACHE.get_or_init(|| {
        #[cfg(target_os = "windows")]
        {
            const CREATE_NO_WINDOW: u32 = 0x08000000;
            let output = Command::new("powershell")
                .args(["-NoProfile", "-Command", "Add-Type -AssemblyName PresentationCore; [System.Windows.Media.Fonts]::SystemFontFamilies | Select-Object -ExpandProperty Source"])
                .creation_flags(CREATE_NO_WINDOW)
                .output();

            if let Ok(output) = output {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut fonts: Vec<String> = stdout.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
                fonts.sort();
                if !fonts.is_empty() {
                    return fonts;
                }
            }
        }
        #[cfg(not(target_os = "windows"))]
        {
            // Use fontconfig's fc-list which is available on all major Linux distros
            if let Ok(output) = Command::new("fc-list").args([":", "family"]).output() {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let mut fonts: std::collections::HashSet<String> = stdout
                    .lines()
                    .flat_map(|line| line.split(','))
                    .map(|s| s.trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                // Always include common Linux fonts as fallback
                for f in ["Ubuntu", "DejaVu Sans", "Liberation Sans", "Noto Sans", "Roboto"] {
                    fonts.insert(f.to_string());
                }
                let mut sorted: Vec<String> = fonts.into_iter().collect();
                sorted.sort();
                if !sorted.is_empty() {
                    return sorted;
                }
            }
        }
        vec!["Arial".to_string()]
    }).clone()
}

use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn pick_exe(app: AppHandle) -> Option<String> {
    #[cfg(target_os = "windows")]
    let dialog = app.dialog().file().add_filter("Executables", &["exe"]);
    #[cfg(not(target_os = "windows"))]
    let dialog = app.dialog().file()
        .add_filter("Executables", &["sh", "x86_64", "bin", "AppImage", "appimage"])
        .add_filter("Wine/Proton", &["exe"])
        .add_filter("All Files", &["*"]);
    dialog.blocking_pick_file().map(|p| p.to_string())
}

#[tauri::command]
pub async fn pick_save_folder(app: AppHandle) -> Option<String> {
    let folder = app.dialog().file().set_title("Select Save Folder").blocking_pick_folder();
    folder.map(|path| path.to_string())
}

#[tauri::command]
pub async fn pick_wallpaper(app: AppHandle, game_id: String) -> Result<Option<String>, String> {
    let file_path = app.dialog().file().add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"]).blocking_pick_file();
    if let Some(src) = file_path {
        let src_str = src.to_string();
        let ext = std::path::Path::new(&src_str).extension().and_then(|s| s.to_str()).unwrap_or("png");
        let wallpapers_dir = get_wallpapers_dir(&app);
        fs::create_dir_all(&wallpapers_dir).map_err(|e| e.to_string())?;
        let dest = wallpapers_dir.join(format!("{}.{}", game_id, ext));
        fs::copy(src.to_string(), &dest).map_err(|e| format!("Failed to copy wallpaper: {}", e))?;
        return Ok(Some(dest.to_string_lossy().to_string()));
    }
    Ok(None)
}

#[tauri::command]
pub async fn pick_logo(app: AppHandle, game_id: String) -> Result<Option<String>, String> {
    let file_path = app.dialog().file().add_filter("Images", &["jpg", "jpeg", "png", "webp"]).blocking_pick_file();
    if let Some(src) = file_path {
        let src_str = src.to_string();
        let ext = std::path::Path::new(&src_str).extension().and_then(|s| s.to_str()).unwrap_or("png");
        let wallpapers_dir = get_wallpapers_dir(&app);
        fs::create_dir_all(&wallpapers_dir).map_err(|e| e.to_string())?;
        let dest = wallpapers_dir.join(format!("logo_{}.{}", game_id, ext));
        fs::copy(src.to_string(), &dest).map_err(|e| format!("Failed to copy logo: {}", e))?;
        return Ok(Some(dest.to_string_lossy().to_string()));
    }
    Ok(None)
}

/// While a game runs, drop this launcher's own process to a background priority class
/// so Windows is far more likely to reclaim SILO's pages (WebView2, cached images)
/// under memory pressure instead of the game's. Restored the moment the game exits.
#[cfg(target_os = "windows")]
fn set_launcher_background(background: bool) {
    use windows::Win32::System::Threading::{
        GetCurrentProcess, SetPriorityClass, PROCESS_MODE_BACKGROUND_BEGIN, PROCESS_MODE_BACKGROUND_END,
    };
    unsafe {
        // Windows "process background mode" sets idle CPU scheduling AND a low memory
        // priority in one call — the deepest reclaim hint the exposed API offers. While a
        // game runs the launcher's pages (WebView2, images/cache) are the first the OS
        // trims; restored to foreground mode the instant the game ends.
        let _ = SetPriorityClass(
            GetCurrentProcess(),
            if background { PROCESS_MODE_BACKGROUND_BEGIN } else { PROCESS_MODE_BACKGROUND_END },
        );
    }
}
#[cfg(not(target_os = "windows"))]
fn set_launcher_background(_background: bool) {
    // Disabled for now - renice/ionice may cause issues on some systems
}

/// SaveGuard should watch for save writes while a game runs when there is no usable
/// folder yet — or when the stored folder no longer exists (game moved/reinstalled)
/// so a fresh location gets re-found instead of silently never backing up again.
fn save_detection_needed(save_path: Option<&str>) -> bool {
    match save_path {
        None => true,
        Some(path) => !Path::new(path).is_dir(),
    }
}

/// True when the stored save folder is the game's own — i.e. its last path
/// component names the game. Used to spot a folder that exists but was stored
/// too broadly by an older build (the publisher folder
/// `Saved Games\CD Projekt Red`, a cache folder), which would otherwise
/// suppress detection for that game forever.
fn save_path_names_game(save_path: &str, name_keys: &[String]) -> bool {
    let Some(name) = Path::new(save_path)
        .file_name()
        .map(|n| n.to_string_lossy().to_lowercase())
    else {
        return false;
    };
    if name.is_empty() || crate::saveguard::is_save_root_excluded(&name) {
        return false;
    }
    name_keys.iter().any(|key| crate::saveguard::name_matches(&name, key))
}

// ── Restoring onto this machine ─────────────────────────────────────────────
// A library backup carries absolute save paths, so importing one on a different
// PC brings paths that describe the *old* machine: `C:\Users\Old\Documents\My
// Games\X` usually does not exist here. `restore_save_transactionally` opens by
// creating the destination's parent, so a restore would happily resurrect the
// old account's folder tree and unpack the saves somewhere the game never
// reads — while reporting success. Resolve a destination this machine can
// justify before touching the disk.

/// How a stored save path relates to the machine running right now.
#[derive(Debug, PartialEq, Eq)]
enum StoredSavePath {
    /// Absolute, on a volume this machine has, and under this account's profile
    /// (or a shared one, such as `Public`).
    Usable,
    /// Inside a *different* account's profile directory that does not exist
    /// here — the signature of a backup taken on another machine.
    ForeignProfile(String),
    /// Recorded by the *other* operating system, whose paths cannot exist here:
    /// `C:\...` is not absolute on Unix, `/home/...` is not absolute on Windows,
    /// and underneath the shape the folders only ever exist on their own side.
    ForeignOs(OsFamily),
    /// The drive or volume the path lives on is not present.
    RootUnavailable,
    /// Not an absolute path, so there is nothing to anchor it to.
    NotAbsolute,
}

/// The two families of path this application has to reason about. Used both for
/// the machine running now and for the one a stored path was written on, so a
/// PC move in either direction can be reasoned about — and tested — from either
/// side.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OsFamily {
    Windows,
    Unix,
}

impl OsFamily {
    fn current() -> Self {
        if cfg!(windows) {
            OsFamily::Windows
        } else {
            OsFamily::Unix
        }
    }
}

impl std::fmt::Display for OsFamily {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OsFamily::Windows => write!(f, "Windows"),
            OsFamily::Unix => write!(f, "Linux or macOS"),
        }
    }
}

/// A Windows path, however its separators are written and whichever host is
/// reading it: a drive letter, a UNC share, or a tree written with backslashes.
fn is_windows_shaped(recorded: &str) -> bool {
    let bytes = recorded.as_bytes();
    let drive_letter = bytes.len() >= 3 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':';
    let unc = recorded.starts_with("\\\\");
    // Backslashes and no forward slashes: `Documents\My Games\X`.
    let backslash_tree = recorded.contains('\\') && !recorded.contains('/');
    drive_letter || unc || backslash_tree
}

/// Whether a recorded path was written by the *other* operating system. Worth
/// naming, because it changes what the user has to do: the same game keeps its
/// saves somewhere else on this machine, so the recorded path cannot simply be
/// repaired in place.
fn foreign_os_shape(recorded: &str, host: OsFamily) -> Option<OsFamily> {
    let recorded = recorded.trim();
    match host {
        // A drive letter or a UNC prefix on a Unix host can only have been
        // written by Windows.
        OsFamily::Unix if is_windows_shaped(recorded) => Some(OsFamily::Windows),
        // On Windows, only a Unix-absolute path is unambiguously foreign. (`//`
        // is left alone: POSIX gives it its own meaning.)
        OsFamily::Windows if recorded.starts_with('/') && !recorded.starts_with("//") => Some(OsFamily::Unix),
        _ => None,
    }
}

/// Directory names that hold user profiles.
const PROFILE_PARENTS: [&str; 3] = ["users", "home", "documents and settings"];

/// Split a path at its profile directory: the prefix up to and including it
/// (`C:\Users\Old`) and everything below it (`Documents\My Games\X`).
fn split_at_profile(path: &Path) -> Option<(PathBuf, PathBuf)> {
    let components: Vec<Component> = path.components().collect();
    let parent = components.iter().position(|component| match component {
        Component::Normal(name) => PROFILE_PARENTS
            .iter()
            .any(|candidate| name.to_string_lossy().eq_ignore_ascii_case(candidate)),
        _ => false,
    })?;
    // The profile name itself has to be there for the split to mean anything.
    if !matches!(components.get(parent + 1), Some(Component::Normal(_))) {
        return None;
    }

    let mut prefix = PathBuf::new();
    for component in &components[..=parent + 1] {
        prefix.push(component.as_os_str());
    }
    let mut tail = PathBuf::new();
    for component in &components[parent + 2..] {
        tail.push(component.as_os_str());
    }
    Some((prefix, tail))
}

/// Where the current account's profile lives (`%USERPROFILE%`, `$HOME`).
fn current_profile_dir() -> Option<PathBuf> {
    #[cfg(target_os = "windows")]
    let variable = "USERPROFILE";
    #[cfg(not(target_os = "windows"))]
    let variable = "HOME";

    std::env::var_os(variable).map(PathBuf::from).filter(|dir| dir.is_dir())
}

/// The current account's profile folder name, used to recognise a stored path
/// as someone else's. Empty when the profile cannot be determined, which leaves
/// only the existence check to go on.
fn current_profile_name() -> String {
    current_profile_dir()
        .and_then(|dir| dir.file_name().map(|name| name.to_string_lossy().into_owned()))
        .unwrap_or_default()
}

/// Classify a stored save path against this machine.
fn stored_path_verdict(path: &Path, current_user: &str) -> StoredSavePath {
    stored_path_verdict_of(path, current_user, OsFamily::current())
}

/// Classify a stored save path against a machine of a known kind.
fn stored_path_verdict_of(path: &Path, current_user: &str, host: OsFamily) -> StoredSavePath {
    // Ahead of the absoluteness check, because a path from the other OS is never
    // absolute here and "this was recorded on Windows" is the useful thing to
    // say about it.
    if let Some(os) = foreign_os_shape(&path.to_string_lossy(), host) {
        return StoredSavePath::ForeignOs(os);
    }
    if !path.is_absolute() {
        return StoredSavePath::NotAbsolute;
    }
    if !drive_available(path) {
        return StoredSavePath::RootUnavailable;
    }
    if let Some((profile, _)) = split_at_profile(path) {
        let name = profile
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_default();
        // Another account's folder only counts as foreign when it is genuinely
        // absent: a shared profile such as `Public`, or a second account that
        // really does exist on this machine, is a legitimate destination.
        if !name.is_empty() && !name.eq_ignore_ascii_case(current_user) && !profile.is_dir() {
            return StoredSavePath::ForeignProfile(name);
        }
    }
    StoredSavePath::Usable
}

/// Rewrite a path under another machine's profile so it points at this one,
/// keeping everything below the profile directory:
/// `C:\Users\Old\Documents\My Games\X` -> `C:\Users\New\Documents\My Games\X`.
fn remap_onto_profile(path: &Path, profile: &Path) -> Option<PathBuf> {
    let (_, tail) = split_at_profile(path)?;
    if tail.as_os_str().is_empty() {
        return None;
    }
    Some(profile.join(tail))
}

/// The part of a Windows path below the Windows profile, so it can be looked for
/// inside a Proton or Wine prefix on this machine:
/// `C:\Users\Old\Documents\My Games\X` -> `Documents\My Games\X`.
fn windows_path_tail(recorded: &str) -> Option<PathBuf> {
    let components: Vec<&str> = recorded
        .split(['\\', '/'])
        .filter(|part| !part.is_empty() && *part != ".")
        .collect();
    // A leading `C:` is the drive, not a folder.
    let start = match components.first() {
        Some(part) if part.ends_with(':') => 1,
        _ => 0,
    };
    let mut rest = components.get(start..)?;
    if let Some(parent) = rest
        .iter()
        .position(|part| PROFILE_PARENTS.iter().any(|name| part.eq_ignore_ascii_case(name)))
    {
        // Between the parent and the tail sits this path's own user directory,
        // which the prefix supplies its own version of.
        rest = rest.get(parent + 2..)?;
    }
    if rest.is_empty() {
        return None;
    }
    Some(rest.iter().collect())
}

/// One Proton or Wine prefix on this machine, and the Steam app id it belongs to
/// when the layout says which game it is for.
#[derive(Debug, Clone, PartialEq, Eq)]
struct WinePrefix {
    drive_c: PathBuf,
    /// Steam puts each game's prefix in `steamapps/compatdata/<appid>/`, so the
    /// directory name *is* the game's identity. A plain Wine prefix serves every
    /// game run through it and so names none.
    app_id: Option<String>,
}

/// The `steamapps` directory of every Steam install on this machine: the native
/// layout, an XDG data home, and Flatpak's own copy of Steam.
fn steam_steamapps_dirs_in(home: &Path, data: &Path) -> Vec<PathBuf> {
    let candidates = [
        home.join(".steam/steam/steamapps"),
        data.join("Steam/steamapps"),
        home.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps"),
    ];
    // Only the ones really there, and each one once: `data` defaults to
    // `home/.local/share`, where native Steam also lives.
    let mut dirs = Vec::new();
    for candidate in candidates {
        if candidate.is_dir() && !dirs.contains(&candidate) {
            dirs.push(candidate);
        }
    }
    dirs
}

/// Every Proton and Wine prefix on this machine. One implementation for every
/// host: Windows has no `$HOME` holding these, and prefixes that are not there
/// are skipped, so it comes back empty there.
fn wine_prefixes_in(home: &Path, data: &Path) -> Vec<WinePrefix> {
    // One prefix per game under compatdata; a plain Wine prefix is a single one.
    let mut prefixes = Vec::new();
    for steamapps in steam_steamapps_dirs_in(home, data) {
        let Ok(entries) = fs::read_dir(steamapps.join("compatdata")) else {
            continue;
        };
        for entry in entries.flatten() {
            let drive_c = entry.path().join("pfx").join("drive_c");
            if !drive_c.is_dir() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let app_id = (!name.is_empty() && name.chars().all(|c| c.is_ascii_digit())).then_some(name);
            prefixes.push(WinePrefix { drive_c, app_id });
        }
    }
    let wine = home.join(".wine/drive_c");
    if wine.is_dir() {
        prefixes.push(WinePrefix {
            drive_c: wine,
            app_id: None,
        });
    }
    prefixes
}

/// The app id a path names outright, when the game ran from its own prefix:
/// `…/compatdata/<appid>/…` -> `<appid>`.
fn steam_app_id_in(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['\\', '/']).filter(|part| !part.is_empty()).collect();
    let id = parts
        .iter()
        .position(|part| part.eq_ignore_ascii_case("compatdata"))
        .and_then(|at| parts.get(at + 1))?;
    (!id.is_empty() && id.chars().all(|c| c.is_ascii_digit())).then(|| (*id).to_string())
}

/// The install directory a path names, when those are a Steam game's own files:
/// `…/steamapps/common/<dir>/…` -> `<dir>`. Steam uses the same directory name on
/// Windows and on Linux, which is what makes it the bridge between the two.
fn steam_install_dir_in(path: &str) -> Option<String> {
    let parts: Vec<&str> = path.split(['\\', '/']).filter(|part| !part.is_empty()).collect();
    let at = parts.iter().position(|part| part.eq_ignore_ascii_case("steamapps"))?;
    match parts.get(at + 1) {
        Some(next) if next.eq_ignore_ascii_case("common") => parts.get(at + 2).map(|dir| dir.to_string()),
        _ => None,
    }
}

/// The app id and install directory an `appmanifest_<appid>.acf` names. The file
/// is Valve's key-value text, and both values sit on their own quoted lines.
fn read_app_manifest(path: &Path) -> Option<(String, String)> {
    let data = fs::read_to_string(path).ok()?;
    let mut app_id = None;
    let mut install_dir = None;
    for line in data.lines() {
        let fields: Vec<&str> = line
            .split('"')
            .map(str::trim)
            .filter(|field| !field.is_empty())
            .collect();
        if fields.len() != 2 {
            continue;
        }
        match fields[0] {
            "appid" => app_id = Some(fields[1].to_string()),
            "installdir" => install_dir = Some(fields[1].to_string()),
            _ => {}
        }
    }
    Some((app_id?, install_dir?))
}

/// The app id of the Steam game installed in `install_dir`, from the manifests in
/// this machine's Steam libraries. Steam keeps the install directory the same on
/// every platform, so this is what turns a path the old machine recorded into the
/// app id that owns a prefix here.
fn app_id_for_install_dir(steamapps_dirs: &[PathBuf], install_dir: &str) -> Option<String> {
    for steamapps in steamapps_dirs {
        let Ok(entries) = fs::read_dir(steamapps) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("appmanifest_") || !name.ends_with(".acf") {
                continue;
            }
            let Some((app_id, dir)) = read_app_manifest(&entry.path()) else {
                continue;
            };
            if dir.eq_ignore_ascii_case(install_dir) {
                return Some(app_id);
            }
        }
    }
    None
}

/// A game's Steam identity on this machine, read from the paths the library
/// records for it. Its own prefix names the app id outright; otherwise the
/// install directory in the recorded path is matched against this machine's
/// manifests, which is what carries a library recorded on Windows across to
/// Linux, where the same game's files sit in `steamapps/common/<installdir>`.
fn steam_app_id_of(game: &Game, recorded: &str, steamapps_dirs: &[PathBuf]) -> Option<String> {
    let paths = [Some(recorded), game.exe_path.as_deref()];
    if let Some(app_id) = paths.iter().flatten().find_map(|path| steam_app_id_in(path)) {
        return Some(app_id);
    }
    let install_dir = paths.iter().flatten().find_map(|path| steam_install_dir_in(path))?;
    app_id_for_install_dir(steamapps_dirs, &install_dir)
}

/// What the restore and the migration survey need to read about this machine: its
/// prefixes, the Steam libraries whose manifests identify them, and the
/// directories searched by folder name as a last resort.
struct MachinePaths {
    prefixes: Vec<WinePrefix>,
    steamapps: Vec<PathBuf>,
    search_dirs: Vec<PathBuf>,
}

/// This machine's own prefixes, Steam libraries and search directories.
fn machine_paths() -> MachinePaths {
    let Some(home) = std::env::var_os("HOME").map(PathBuf::from) else {
        // No home directory to hold prefixes; the search directories still work,
        // which is what a Windows host has.
        return MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: get_watch_directories(),
        };
    };
    let data = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| home.join(".local/share"));
    machine_paths_in(&home, &data, get_watch_directories())
}

/// The same, for a home directory, a data directory and a set of search
/// directories named explicitly.
fn machine_paths_in(home: &Path, data: &Path, search_dirs: Vec<PathBuf>) -> MachinePaths {
    MachinePaths {
        prefixes: wine_prefixes_in(home, data),
        steamapps: steam_steamapps_dirs_in(home, data),
        search_dirs,
    }
}

/// Where a Windows-recorded save folder lives inside this machine's Proton or
/// Wine prefixes, and how far that can be trusted. A prefix Steam named after the
/// game's own app id is the game's own prefix, so the folder being there is
/// evidence; anywhere else it is only an existence check over prefixes that may
/// well belong to other games.
fn find_in_wine_prefixes(
    prefixes: &[WinePrefix],
    recorded: &str,
    own_app_id: Option<&str>,
) -> Option<(PathBuf, MatchConfidence)> {
    let tail = windows_path_tail(recorded)?;
    if let Some(app_id) = own_app_id {
        let own = prefixes.iter().find(|prefix| prefix.app_id.as_deref() == Some(app_id));
        if let Some(found) = own.and_then(|prefix| folder_in_prefix(&prefix.drive_c, &tail)) {
            return Some((found, MatchConfidence::Confirmed));
        }
    }
    prefixes
        .iter()
        .find_map(|prefix| folder_in_prefix(&prefix.drive_c, &tail).map(|found| (found, MatchConfidence::Likely)))
}

/// The recorded path's tail inside one prefix, looked for under the prefix's own
/// user directories first — that is the layout the recording machine used.
fn folder_in_prefix(drive_c: &Path, tail: &Path) -> Option<PathBuf> {
    if let Ok(entries) = fs::read_dir(drive_c.join("users")) {
        for entry in entries.flatten() {
            let candidate = entry.path().join(tail);
            if candidate.is_dir() {
                return Some(candidate);
            }
        }
    }
    // Then the drive itself, for a path that was never under a profile.
    let candidate = drive_c.join(tail);
    candidate.is_dir().then_some(candidate)
}

/// Name keys a re-detection may match against this machine. When the recorded
/// install is not here it describes the *old* machine's layout, and its folder
/// name is often something generic — a game sitting in `…\Games\Cyberpunk2077.exe`
/// would key on "games", which matches unrelated folders on this machine such as
/// the launcher data directories under AppData. Then only the game's own name is
/// trusted, because that is what the user recognises and what its save folder is
/// named after.
fn redetect_name_keys(game: &Game) -> Vec<String> {
    if recorded_install_present(game) {
        restore_name_keys(game)
    } else {
        vec![game.name.clone()]
    }
}

/// Name keys a restore detects with — the same set SaveGuard uses, falling back
/// to the game's name when no executable is recorded.
fn restore_name_keys(game: &Game) -> Vec<String> {
    save_name_keys(&game.name, Path::new(game.exe_path.as_deref().unwrap_or("")))
}

/// How strongly a save folder on this machine was matched to a game, ordered
/// from the weakest evidence to the strongest. Matching is categorical rather
/// than a fuzzy percentage: the recorded path either translated onto something
/// real or it did not, and the genuinely unsure cases are distinct levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum MatchConfidence {
    /// Nothing but the folder's name ties it to this game. A restore may still
    /// use it, because the user named the game it is for and can see where the
    /// saves landed, but the migration assistant never rewrites a save path on
    /// this alone: the folder may well hold another game's saves.
    Guess,
    /// The folder is really inside a Proton or Wine prefix on this machine, but
    /// nothing ties that prefix to this game, so the match is an existence check
    /// over prefixes that may belong to other games. Reported, not applied.
    Likely,
    /// The recorded layout translated onto this account, or found inside the
    /// prefix that belongs to this game: Steam names a game's prefix after its
    /// app id, and the app id is read from this machine's own app manifests.
    Confirmed,
    /// The recorded path itself, which works here unchanged.
    Exact,
}

/// The least evidence that lets SILO rewrite a game's save path without being
/// asked. Below it sit the two matches that are only hints — a folder sharing the
/// game's name, and a folder in a prefix that is not this game's — and both are
/// reported for the user to confirm in Edit instead of being applied.
const MIN_RELINK_CONFIDENCE: MatchConfidence = MatchConfidence::Confirmed;

impl MatchConfidence {
    /// An ordinal evidence score for explaining a proposal to the user.
    /// Scores are intentionally not the safety policy; see is_enough_for_a_relink.
    fn score(self) -> u8 {
        match self {
            MatchConfidence::Guess => 25,
            MatchConfidence::Likely => 55,
            MatchConfidence::Confirmed => 90,
            MatchConfidence::Exact => 100,
        }
    }

    /// Whether this is enough to point a game at a folder on SILO's own say-so.
    fn is_enough_for_a_relink(self) -> bool {
        self >= MIN_RELINK_CONFIDENCE
    }
}

/// Where a restore should write, how that folder was matched, and whether the
/// match differs from what the library recorded.
#[derive(Debug)]
struct RestoreTarget {
    path: PathBuf,
    rediscovered: bool,
    confidence: MatchConfidence,
}

/// Decide where a restore should land on this machine, preferring evidence: the
/// recorded path when it is usable here, the same layout under this account's
/// profile when it already exists, the game's own Proton prefix when its Steam
/// identity can be established, and otherwise any folder on this machine that
/// names the game. Returns an actionable error rather than guessing, and never
/// creates anything.
fn resolve_restore_target(game: &Game, stored: &str) -> Result<RestoreTarget, String> {
    resolve_restore_target_with(game, stored, &machine_paths())
}

/// The same decision, with this machine's prefixes and Steam libraries named
/// explicitly, so every branch of it can be exercised on a host that has none of
/// its own.
fn resolve_restore_target_with(game: &Game, stored: &str, machine: &MachinePaths) -> Result<RestoreTarget, String> {
    let stored_path = Path::new(stored);
    let verdict = stored_path_verdict(stored_path, &current_profile_name());

    if verdict == StoredSavePath::Usable {
        return Ok(RestoreTarget {
            path: stored_path.to_path_buf(),
            rediscovered: false,
            confidence: MatchConfidence::Exact,
        });
    }

    // The same layout under this account, accepted only when the folder is
    // actually there — so the rewrite is evidence rather than a guess.
    if let Some(profile) = current_profile_dir() {
        if let Some(remapped) = remap_onto_profile(stored_path, &profile) {
            if remapped.is_dir() {
                return Ok(RestoreTarget {
                    path: remapped,
                    rediscovered: true,
                    confidence: MatchConfidence::Confirmed,
                });
            }
        }
    }

    // A path written by Windows reaches Linux and macOS through Proton or Wine,
    // which keeps the same tree inside the game's own prefix. Steam names that
    // prefix after the game's app id, so whenever this machine's own manifests
    // name the game, the prefix to look in is known before anything is searched
    // for — which is what makes this evidence rather than a hunt through whatever
    // prefix happens to hold a folder with the same name.
    let own_app_id = steam_app_id_of(game, stored, &machine.steamapps);
    if let Some((found, confidence)) = find_in_wine_prefixes(&machine.prefixes, stored, own_app_id.as_deref()) {
        return Ok(RestoreTarget {
            path: found,
            rediscovered: true,
            confidence,
        });
    }

    // Otherwise take the folder this machine already has for the game. Recency is
    // deliberately irrelevant here (`now`), so this is a pure name match over
    // folders that exist — and a name is not evidence, which is what the lowest
    // confidence level on the way out records.
    let install_dir = game.exe_path.as_deref().map(Path::new).and_then(Path::parent);
    if let Some(found) = crate::saveguard::find_save_root_by_scan(
        &machine.search_dirs,
        &redetect_name_keys(game),
        std::time::SystemTime::now(),
        install_dir,
    ) {
        return Ok(RestoreTarget {
            path: found,
            rediscovered: true,
            confidence: MatchConfidence::Guess,
        });
    }

    Err(format!(
        "Cannot restore yet: {}{}",
        unusable_save_path_reason(stored, &verdict),
        save_recovery_hint(&verdict, recorded_install_present(game))
    ))
}

/// What is wrong with a stored save folder this machine cannot use.
fn unusable_save_path_reason(stored: &str, verdict: &StoredSavePath) -> String {
    match verdict {
        StoredSavePath::ForeignProfile(user) => format!(
            "{stored} sits under {user}'s profile, which does not exist on this machine — it was recorded on a different PC."
        ),
        StoredSavePath::ForeignOs(os) => format!(
            "{stored} was recorded on a {os} machine, so that path cannot exist on this one."
        ),
        StoredSavePath::RootUnavailable => {
            format!("{stored} is on a drive that is not available on this machine.")
        }
        _ => format!("{stored} does not exist on this machine."),
    }
}

/// What to do about a save folder that comes back on its own once the game is
/// launched — the usual case, where only the path moved.
const SAVE_FOLDER_RECOVERY_HINT: &str =
    " Launch the game once so SaveGuard re-finds the folder, or set it in Edit, then try again.";

/// What to do about one whose game is not on this machine either. Launching it
/// cannot re-find anything, so the saves themselves have to come across.
const SAVE_FOLDER_FROM_OLD_PC_HINT: &str = " SILO cannot see this game's files on this machine \
     either, so its saves are still on the other PC: copy them across (or restore a SILO backup of \
     this game), then check again.";

/// What to do about one on a drive that is not mounted right now. The saves are
/// on that drive, not on another PC, so bringing them across is the wrong fix.
const SAVE_FOLDER_UNPLUGGED_HINT: &str =
    " Reconnect the drive that holds it, or set the folder in Edit, then try again.";

/// What to do about a Windows folder seen on Linux or macOS. Most Windows games
/// run through Proton or Wine there, which keeps the game's own Windows tree
/// inside its prefix — so a launch finds the saves without anything being copied.
const SAVE_FOLDER_PROTON_HINT: &str = " Install and launch the game here: through Proton or \
     Wine its saves live inside the game's own prefix and SaveGuard will find them. Otherwise copy \
     the saves across and set the folder in Edit.";

/// What to do about a Linux or macOS folder seen on Windows. There is no prefix
/// to look inside, so the folder has to be re-created here.
const SAVE_FOLDER_FROM_OTHER_OS_HINT: &str = " Install the game here and launch it once so \
     SaveGuard re-finds the folder, or copy the saves across and set the folder in Edit.";

/// Whether the recorded executable is actually on this machine. Tells "the game
/// is here and only its save folder moved" apart from "this game came from the
/// old PC too", which need different advice.
fn recorded_install_present(game: &Game) -> bool {
    game.exe_path
        .as_deref()
        .map(str::trim)
        .filter(|exe| !exe.is_empty())
        .is_some_and(|exe| Path::new(exe).is_file())
}

/// What the user can do about a save folder this machine cannot use. The advice
/// depends on why it does not work: a drive that went away, a game that came
/// from the old PC and is not here, or a folder the game itself can re-find.
fn save_recovery_hint(verdict: &StoredSavePath, install_present: bool) -> &'static str {
    match verdict {
        // The saves are on a drive that is not mounted right now.
        StoredSavePath::RootUnavailable => SAVE_FOLDER_UNPLUGGED_HINT,
        // Recorded by the other operating system, in the direction that has a
        // compatibility layer to look inside.
        StoredSavePath::ForeignOs(OsFamily::Windows) => SAVE_FOLDER_PROTON_HINT,
        StoredSavePath::ForeignOs(OsFamily::Unix) => SAVE_FOLDER_FROM_OTHER_OS_HINT,
        // Left behind by another account, and the game is not here either, so
        // the saved files themselves have to come across.
        StoredSavePath::ForeignProfile(_) if !install_present => SAVE_FOLDER_FROM_OLD_PC_HINT,
        _ => SAVE_FOLDER_RECOVERY_HINT,
    }
}

/// Message for a manual backup whose recorded save folder is not on this
/// machine — usually the same "the library came from another PC" situation.
fn save_folder_missing_message(install_present: bool, stored: &str) -> String {
    let verdict = stored_path_verdict(Path::new(stored), &current_profile_name());
    format!(
        "Cannot back up: {}{}",
        unusable_save_path_reason(stored, &verdict),
        save_recovery_hint(&verdict, install_present)
    )
}

/// What this machine can do about one game's recorded save folder.
#[derive(Debug, PartialEq, Eq)]
enum SaveFolderOutcome {
    /// The library records no save folder yet, so there is nothing to relink.
    NoneRecorded,
    /// The recorded folder is the right one on this machine already.
    Usable,
    /// A folder on this machine for a game whose recorded one does not work here,
    /// and how strongly it was matched.
    Relink {
        found: PathBuf,
        reason: String,
        confidence: MatchConfidence,
    },
    /// Nothing on this machine matches; the user has to point SILO at it.
    Unresolved { reason: String },
}

/// One game a migration survey wants to tell the user about.
#[derive(Debug, Clone, Serialize)]
pub struct SaveFolderIssue {
    #[serde(rename = "gameId")]
    pub game_id: String,
    #[serde(rename = "gameName")]
    pub game_name: String,
    /// The save folder the library records, exactly as recorded.
    #[serde(rename = "recordedPath")]
    pub recorded_path: String,
    /// Where the saves belong on this machine, when SILO found the folder.
    #[serde(rename = "foundPath", skip_serializing_if = "Option::is_none")]
    pub found_path: Option<String>,
    /// Categorical evidence used to enforce the safe automatic-relink threshold.
    /// Absent when nothing matched.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub confidence: Option<MatchConfidence>,
    /// Human-readable confidence score from 0 to 100. This is an explanation for
    /// the UI, not the relink policy: automatic relinking uses the categorical
    /// evidence threshold above, so a numeric score cannot make a weak match safe.
    #[serde(rename = "confidenceScore", skip_serializing_if = "Option::is_none")]
    pub confidence_score: Option<u8>,
    /// Whether the game's recorded executable is on this machine. When it is
    /// not, the game came from the old PC too and its saves can only arrive by
    /// being copied over or restored from a backup.
    #[serde(rename = "installPresent")]
    pub install_present: bool,
    /// Why the recorded folder does not work here, in plain language.
    pub reason: String,
}

/// What every game's save folder looks like on this machine. Produced without
/// changing anything, so it can be shown before the user commits to a relink.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SaveFolderSurvey {
    /// Games that have a save folder recorded at all.
    #[serde(rename = "gamesWithSaveFolder")]
    pub games_with_save_folder: usize,
    /// Recorded folders that already work here, so there is nothing to do.
    pub usable: usize,
    /// Games a relink can point at the right folder with no further input.
    pub relinkable: Vec<SaveFolderIssue>,
    /// Games nothing on this machine could be matched to.
    pub unresolved: Vec<SaveFolderIssue>,
}
/// Classify one game's recorded save folder against this machine. Reads the
/// filesystem but never writes to it, so it is safe to run over a whole library.
fn survey_save_folder_with(game: &Game, machine: &MachinePaths) -> SaveFolderOutcome {
    let Some(recorded) = game.save_path.as_deref().map(str::trim).filter(|path| !path.is_empty()) else {
        return SaveFolderOutcome::NoneRecorded;
    };
    let verdict = stored_path_verdict(Path::new(recorded), &current_profile_name());
    if verdict == StoredSavePath::Usable {
        return SaveFolderOutcome::Usable;
    }
    let reason = unusable_save_path_reason(recorded, &verdict);
    match resolve_restore_target_with(game, recorded, machine) {
        // Only a *different* folder that already exists counts as a found
        // replacement; re-detection never invents one.
        Ok(target) if target.path != Path::new(recorded) => SaveFolderOutcome::Relink {
            found: target.path,
            reason,
            confidence: target.confidence,
        },
        Ok(_) => SaveFolderOutcome::Unresolved { reason },
        Err(error) => SaveFolderOutcome::Unresolved { reason: error },
    }
}

/// Why a folder matched with less than the relink's evidence was reported
/// instead of applied. A relink rewrites save paths in bulk without asking, so
/// anything weaker than "this game's own folder" has to be the user's decision.
fn refused_match_reason(found: &Path, confidence: MatchConfidence, cause: &str) -> String {
    match confidence {
        MatchConfidence::Guess => format!(
            "{cause} SILO did find {}, but it matches the game's name alone and may hold another game's \
             saves, so this game was left for you to set in Edit.",
            found.display()
        ),
        // A prefix match, but not one Steam names after this game's app id: the
        // folder may be another game's, and the identity is what would have
        // settled it.
        _ => format!(
            "{cause} SILO did find {} inside a Proton or Wine prefix on this machine, but not one it can \
             tie to this game, so this game was left for you to set in Edit.",
            found.display()
        ),
    }
}

/// A save folder's identity for "does another game already save here?" — case
/// and separators folded, because Windows paths are case-insensitive and a
/// library may record either slash. Folding errs towards treating two folders as
/// one, which is the safe direction: the relink is then reported for the user
/// instead of applied.
fn folder_identity(path: &Path) -> String {
    path.to_string_lossy()
        .to_lowercase()
        .replace('\\', "/")
        .trim_end_matches('/')
        .to_string()
}

/// Survey a whole library against this machine. Pure with respect to the
/// library: it reports what a relink would do without touching a single game.
fn survey_save_folders_of(games: &[Game]) -> SaveFolderSurvey {
    survey_save_folders_with(games, &machine_paths())
}

/// The same survey, with this machine's prefixes, Steam libraries and search
/// directories named explicitly. Every game is classified against one set of
/// them, so the prefixes are discovered once rather than once per game.
fn survey_save_folders_with(games: &[Game], machine: &MachinePaths) -> SaveFolderSurvey {
    let classified: Vec<SaveFolderOutcome> = games
        .iter()
        .map(|game| survey_save_folder_with(game, machine))
        .collect();

    // The save folders this library already uses here. A game that is not
    // installed on this machine can only be matched by folder name, so this is
    // what keeps that name match from claiming another game's saves.
    let mut occupied: Vec<(String, String)> = games
        .iter()
        .zip(&classified)
        .filter(|(_, outcome)| **outcome == SaveFolderOutcome::Usable)
        .filter_map(|(game, _)| {
            let recorded = game.save_path.as_deref()?;
            Some((folder_identity(Path::new(recorded)), game.name.clone()))
        })
        .collect();

    let mut survey = SaveFolderSurvey::default();
    for (game, outcome) in games.iter().zip(&classified) {
        let (found_path, confidence, reason) = match outcome {
            SaveFolderOutcome::NoneRecorded => continue,
            SaveFolderOutcome::Usable => {
                survey.games_with_save_folder += 1;
                survey.usable += 1;
                continue;
            }
            SaveFolderOutcome::Relink {
                found,
                reason,
                confidence,
            } => {
                // A match with nothing but a name behind it is reported rather
                // than applied: the folder may well hold another game's saves,
                // and the user is the only one who can tell.
                if !confidence.is_enough_for_a_relink() {
                    (
                        None,
                        Some(*confidence),
                        refused_match_reason(found, *confidence, reason),
                    )
                } else {
                    let identity = folder_identity(found);
                    match occupied.iter().find(|(key, _)| *key == identity) {
                        // Two games never get pointed at one folder: whichever game
                        // already saves there keeps it, and this one is left alone.
                        Some((_, other)) => (
                            None,
                            Some(*confidence),
                            format!(
                                "{reason} {} is already {other}'s save folder here, so this game was left for you to set in Edit.",
                                found.display()
                            ),
                        ),
                        None => {
                            occupied.push((identity, game.name.clone()));
                            (Some(found.to_string_lossy().into_owned()), Some(*confidence), reason.clone())
                        }
                    }
                }
            }
            SaveFolderOutcome::Unresolved { reason } => (None, None, reason.clone()),
        };

        survey.games_with_save_folder += 1;
        let issue = SaveFolderIssue {
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            recorded_path: game.save_path.clone().unwrap_or_default(),
            found_path,
            confidence,
            confidence_score: confidence.map(MatchConfidence::score),
            install_present: recorded_install_present(game),
            reason,
        };
        if issue.found_path.is_some() {
            survey.relinkable.push(issue);
        } else {
            survey.unresolved.push(issue);
        }
    }
    survey
}

/// Check every game's save folder against this machine. Read-only: the library
/// is left exactly as it was found, so the result can be shown before anything
/// is written.
#[tauri::command]
pub fn survey_save_folders(app: AppHandle) -> SaveFolderSurvey {
    survey_save_folders_of(&get_games(app))
}

/// One game's recorded save folder, and the folder SILO found for it on this
/// machine — the per-game counterpart to the whole-library survey.
///
/// The bulk relink refuses a match with nothing but a name or a prefix behind
/// it, because it rewrites save paths without asking. Here the user is asking
/// about one game and will see the path before saving, so the candidate is
/// returned even then; `confidence` says how much evidence backs it. `None`
/// means there is nothing to propose: no save folder is recorded, or the one
/// recorded already works on this machine.
#[tauri::command]
pub fn propose_save_folder(app: AppHandle, game_id: String) -> Option<SaveFolderIssue> {
    let games = get_games(app);
    let game = games.iter().find(|game| game.id == game_id)?;
    save_folder_proposal(game, &machine_paths())
}

/// Resolve one game's save folder for the Edit dialog. Read-only, and never
/// creates anything — the caller decides whether to accept the candidate.
fn save_folder_proposal(game: &Game, machine: &MachinePaths) -> Option<SaveFolderIssue> {
    let recorded = game
        .save_path
        .as_deref()
        .map(str::trim)
        .filter(|path| !path.is_empty())?;
    // A folder that already works here is not something to change.
    if stored_path_verdict(Path::new(recorded), &current_profile_name()) == StoredSavePath::Usable {
        return None;
    }

    let (found_path, confidence, reason) = match survey_save_folder_with(game, machine) {
        SaveFolderOutcome::Relink {
            found,
            reason,
            confidence,
        } => (
            Some(found.to_string_lossy().into_owned()),
            Some(confidence),
            reason,
        ),
        SaveFolderOutcome::Unresolved { reason } => (None, None, reason),
        // Guarded by the check above, and a game with no recorded folder already
        // returned; nothing else can reach here.
        SaveFolderOutcome::Usable | SaveFolderOutcome::NoneRecorded => return None,
    };

    Some(SaveFolderIssue {
        game_id: game.id.clone(),
        game_name: game.name.clone(),
        recorded_path: recorded.to_string(),
        found_path,
        confidence,
        confidence_score: confidence.map(MatchConfidence::score),
        install_present: recorded_install_present(game),
        reason,
    })
}

/// Point every game whose save folder came from another machine at the matching
/// folder on this one. Games that already work here, games SILO cannot place at
/// all, and games it could only match by name are left alone.
#[tauri::command]
pub async fn relink_save_folders(app: AppHandle) -> Result<SaveFolderSurvey, String> {
    let mut games = get_games(app.clone());
    let survey = survey_save_folders_of(&games);
    if survey.relinkable.is_empty() {
        return Ok(survey);
    }

    let relinked = apply_relink_proposals(&mut games, &survey.relinkable);

    // One atomic write for the whole library: either every relink lands or none
    // does, so the survey that was shown and the library cannot disagree.
    save_games_atomic(&app, &games)?;
    log::info!("Relinked {} save folder(s) recorded on another machine", relinked.len());
    let _ = app.emit(
        "savefolders-relinked",
        serde_json::json!({ "count": relinked.len(), "games": relinked }),
    );
    Ok(survey)
}

/// Apply a survey's proposals to a library held in memory: point every game SILO
/// has evidence for at the folder on this machine, and hand back the payload the
/// UI needs. Kept out of the command so the rule that refuses a match with no
/// evidence behind it is enforced — and tested — exactly where a save path would
/// otherwise be rewritten.
fn apply_relink_proposals(games: &mut [Game], proposals: &[SaveFolderIssue]) -> Vec<serde_json::Value> {
    let mut relinked = Vec::new();
    for issue in proposals {
        let Some(found) = issue.found_path.as_deref() else {
            continue;
        };
        // The survey only ever proposes matches with evidence behind them; this is
        // the same bar applied where the library is actually written, so a
        // name-only guess can never reach a game's save path.
        if !issue.confidence.is_some_and(MatchConfidence::is_enough_for_a_relink) {
            continue;
        }
        if let Some(game) = games.iter_mut().find(|game| game.id == issue.game_id) {
            game.save_path = Some(found.to_string());
            game.save_path_source = Some("auto".to_string());
            relinked.push(serde_json::json!({ "gameId": issue.game_id, "savePath": found, "savePathSource": "auto" }));
        }
    }
    relinked
}

/// Where saves belong on this machine for a game whose recorded folder cannot be
/// used here. Only the recorded path translated onto this account is trusted:
/// the game wrote that layout itself, so the same tree under this account is
/// where it looks again once it is installed. A path that does not translate —
/// another operating system, or another drive with no profile in it — has no
/// destination, because guessing one would hand the game a folder it never
/// reads.
fn import_destination(recorded: &str, profile: Option<&Path>) -> Option<PathBuf> {
    // A path from the other operating system has no counterpart here, and this is
    // the one place the answer decides whether a folder gets *created*: on Linux a
    // Windows path such as `C:\Users\Old\Documents\X` parses as a plain relative
    // tree, so translating it would build a mirror of the other machine's home
    // layout inside this account that the game never reads.
    if foreign_os_shape(recorded, OsFamily::current()).is_some() {
        return None;
    }
    remap_onto_profile(Path::new(recorded), profile?)
}

/// Whether a folder already holds anything. One that does is left alone: the
/// saves in it can be newer than the ones being brought across.
fn folder_has_content(path: &Path) -> bool {
    fs::read_dir(path).is_ok_and(|mut entries| entries.next().is_some())
}

/// Saves to bring across for one game: where they come from, and where they go.
#[derive(Debug, Clone, Serialize, PartialEq, Eq)]
pub struct SaveImportItem {
    #[serde(rename = "gameId")]
    pub game_id: String,
    #[serde(rename = "gameName")]
    pub game_name: String,
    /// `folder` for the folder the user picked, `backup` for this game's own
    /// SILO backup.
    pub source: String,
    #[serde(rename = "sourcePath")]
    pub source_path: String,
    #[serde(rename = "targetPath")]
    pub target_path: String,
    /// The folder the library still records, so the report can show what was
    /// wrong with it.
    #[serde(rename = "recordedPath")]
    pub recorded_path: String,
}

/// What bringing the missing saves across would do — worked out without changing
/// anything, so it can be shown before the user commits.
#[derive(Debug, Clone, Serialize, Default)]
pub struct SaveImportPlan {
    /// Games whose saves can be brought across, with where from and where to.
    pub ready: Vec<SaveImportItem>,
    /// Games nothing could be brought for, and why.
    #[serde(rename = "stillMissing")]
    pub still_missing: Vec<SaveFolderIssue>,
}

/// Plan bringing a library's missing save folders across. `source_dir` is the
/// folder the user pointed at — the old PC's drive, or a copy of its saves — and
/// `newest_backup` names a game's newest SILO backup when it has one. Read-only:
/// nothing is copied and nothing is changed. Each game costs one walk of the
/// source folder, which is the same shape of work the relink survey does.
fn save_import_plan(
    games: &[Game],
    profile: Option<&Path>,
    source_dir: Option<&Path>,
    newest_backup: &HashMap<String, PathBuf>,
) -> SaveImportPlan {
    let mut plan = SaveImportPlan::default();
    // Classified against one machine for the whole plan, exactly as the relink
    // survey does, so the two agree game for game.
    let machine = machine_paths();
    for game in games {
        let Some(recorded) = game.save_path.as_deref().map(str::trim).filter(|path| !path.is_empty()) else {
            // Nothing recorded to work from; the relink check reports these too.
            continue;
        };
        // Only the games with nowhere at all to point. A folder that exists here
        // is the relink's business, and a usable one needs nothing.
        if !matches!(
            survey_save_folder_with(game, &machine),
            SaveFolderOutcome::Unresolved { .. }
        ) {
            continue;
        }

        let verdict = stored_path_verdict(Path::new(recorded), &current_profile_name());
        let cause = unusable_save_path_reason(recorded, &verdict);
        let issue = |reason: String| SaveFolderIssue {
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            recorded_path: recorded.to_string(),
            found_path: None,
            // Nothing on this machine was matched at all, which is why this plan
            // offers to make the target folder itself rather than relink to it.
            confidence: None,
            confidence_score: None,
            install_present: recorded_install_present(game),
            reason,
        };

        let Some(destination) = import_destination(recorded, profile) else {
            // The cause above already says what to do about the recorded folder;
            // this only has to explain why nothing could be brought across.
            plan.still_missing.push(issue(format!(
                "{cause} SILO has nowhere on this machine to put its saves, so there is nothing to copy \
                 them into."
            )));
            continue;
        };
        if folder_has_content(&destination) {
            plan.still_missing.push(issue(format!(
                "{} already holds saves, so it was left alone — restore this game's own backup to \
                 replace them.",
                destination.display()
            )));
            continue;
        }

        // The folder the user picked first: it is the live copy. Otherwise this
        // game's own SILO backup, which came across with the library.
        let from_folder = source_dir.and_then(|dir| {
            crate::saveguard::find_save_root_by_scan(
                std::slice::from_ref(&dir.to_path_buf()),
                &redetect_name_keys(game),
                std::time::SystemTime::now(),
                game.exe_path.as_deref().map(Path::new).and_then(Path::parent),
            )
        });
        // The destination itself is not a source: an empty folder there is not
        // saves, and naming the game after it does not make it one.
        let from_folder = from_folder.filter(|found| *found != destination);
        let (source, from) = match (from_folder, newest_backup.get(&game.id)) {
            (Some(found), _) => ("folder", found),
            (None, Some(archive)) => ("backup", archive.clone()),
            (None, None) => {
                plan.still_missing.push(issue(format!(
                    "{cause} No saves for it were found in the folder you picked, and it has no SILO \
                     backup."
                )));
                continue;
            }
        };

        plan.ready.push(SaveImportItem {
            game_id: game.id.clone(),
            game_name: game.name.clone(),
            source: source.to_string(),
            source_path: from.to_string_lossy().into_owned(),
            target_path: destination.to_string_lossy().into_owned(),
            recorded_path: recorded.to_string(),
        });
    }
    plan
}

/// The timestamp a backup's name carries when it has one; its file time is the
/// fallback. A name with no usable time never outranks a dated one — the Backup
/// Manager's plain string sort does exactly that, which is the opposite of what
/// "newest" means to the user here.
const BACKUP_TIMESTAMP_UNKNOWN: &str = "Unknown Time";

fn backup_recency_key(name: &str, modified: Option<std::time::SystemTime>) -> (u8, String) {
    let file_time = modified
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|age| format!("{:020}", age.as_millis()))
        .unwrap_or_default();
    match parse_backup_snapshot(name) {
        Some(snapshot) if snapshot.timestamp != BACKUP_TIMESTAMP_UNKNOWN => (1, snapshot.timestamp),
        _ => (0, file_time),
    }
}

/// This game's newest SILO backup, when it has one. Files the Backup Manager
/// would not list are not considered here either.
fn newest_backup_for(app: &AppHandle, game_id: &str) -> Option<PathBuf> {
    let dir = get_backups_dir(app, game_id);
    let mut archives: Vec<((u8, String), PathBuf)> = Vec::new();
    for entry in fs::read_dir(&dir).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() || path.extension().and_then(|e| e.to_str()) != Some("zip") {
            continue;
        }
        let name = entry.file_name().to_string_lossy().into_owned();
        if parse_backup_snapshot(&name).is_none() {
            continue;
        }
        let modified = entry.metadata().ok().and_then(|meta| meta.modified().ok());
        archives.push((backup_recency_key(&name, modified), path));
    }
    archives.sort_by(|a, b| b.0.cmp(&a.0));
    archives.into_iter().next().map(|(_, path)| path)
}

/// The newest SILO backup of every game that has one.
fn newest_backups(app: &AppHandle, games: &[Game]) -> HashMap<String, PathBuf> {
    games
        .iter()
        .filter_map(|game| newest_backup_for(app, &game.id).map(|path| (game.id.clone(), path)))
        .collect()
}

/// What bringing the missing saves across would do, before anything is touched.
#[tauri::command]
pub fn plan_save_import(app: AppHandle, source_dir: Option<String>) -> SaveImportPlan {
    let games = get_games(app.clone());
    let backups = newest_backups(&app, &games);
    save_import_plan(
        &games,
        current_profile_dir().as_deref(),
        source_dir.as_deref().map(Path::new),
        &backups,
    )
}

/// Bring the saves across for the games this machine has nowhere to point: from
/// the folder the user picked when a game has a folder there, and from the game's
/// own SILO backup otherwise. Nothing is deleted — a destination already holding
/// saves is left alone, and the previous contents of one that is replaced are
/// snapshotted into that game's backups first.
#[tauri::command]
pub async fn bring_saves_across(app: AppHandle, source_dir: Option<String>) -> Result<SaveImportPlan, String> {
    let mut games = get_games(app.clone());
    let plan = save_import_plan(
        &games,
        current_profile_dir().as_deref(),
        source_dir.as_deref().map(Path::new),
        &newest_backups(&app, &games),
    );

    let mut applied = SaveImportPlan::default();
    let mut changed = Vec::new();
    for item in plan.ready {
        let from = PathBuf::from(&item.source_path);
        let destination = PathBuf::from(&item.target_path);
        let backups_dir = get_backups_dir(&app, &item.game_id);
        let install_present = games
            .iter()
            .find(|game| game.id == item.game_id)
            .is_some_and(recorded_install_present);
        let missed = |reason: String| SaveFolderIssue {
            game_id: item.game_id.clone(),
            game_name: item.game_name.clone(),
            recorded_path: item.recorded_path.clone(),
            found_path: None,
            // Matched nothing on this machine when it was planned, and still does
            // not: the offer was to create the folder, not to relink to one.
            confidence: None,
            confidence_score: None,
            install_present,
            reason,
        };
        // The plan was shown to the user, so the disk is re-checked before every
        // write rather than trusted as it was.
        if folder_has_content(&destination) {
            applied.still_missing.push(missed(format!(
                "{} now holds saves, so it was left alone.",
                destination.display()
            )));
            continue;
        }

        let brought = if item.source == "folder" {
            stage_and_swap(&destination, &backups_dir, "import", |staging| {
                copy_tree(&from, staging)
            })
        } else {
            stage_and_swap(&destination, &backups_dir, "import", |staging| {
                unzip_file(&from, staging).map_err(|error| format!("Backup could not be extracted safely: {}", error))
            })
        };

        match brought {
            Ok(()) => {
                if let Some(game) = games.iter_mut().find(|game| game.id == item.game_id) {
                    game.save_path = Some(item.target_path.clone());
                    game.save_path_source = Some("imported".to_string());
                }
                changed.push(serde_json::json!({
                    "gameId": item.game_id,
                    "savePath": item.target_path,
                    "savePathSource": "imported",
                }));
                applied.ready.push(item);
            }
            Err(error) => applied.still_missing.push(missed(error)),
        }
    }

    if !changed.is_empty() {
        // One atomic write for the whole import, and the same event a relink
        // sends: several games' save folders changed in one pass.
        save_games_atomic(&app, &games)?;
        log::info!("Brought saves across for {} game(s)", changed.len());
        let _ = app.emit(
            "savefolders-relinked",
            serde_json::json!({ "count": changed.len(), "games": changed }),
        );
    }
    Ok(applied)
}

/// Determines how to launch a game on Linux based on the executable path.
/// Returns (command_name, args, working_dir_override)
#[cfg(not(target_os = "windows"))]
fn build_linux_game_command(exe_path: &str) -> (String, Vec<String>, Option<PathBuf>) {
    let path = Path::new(exe_path);

    // If it's a steam:// URL, use steam-run or xdg-open
    if exe_path.starts_with("steam://") {
        if Command::new("steam-run").arg("--version").output().is_ok() {
            return ("steam-run".to_string(), vec![exe_path.to_string()], None);
        }
        // Fallback to xdg-open for steam:// URLs
        return ("xdg-open".to_string(), vec![exe_path.to_string()], None);
    }

    // Check if it's an AppImage
    if exe_path.ends_with(".AppImage") || exe_path.ends_with(".appimage") {
        let args = vec![exe_path.to_string()];
        // Ensure it's executable
        let _ = fs::metadata(path).map(|m| {
            let mut perms = m.permissions();
            perms.set_mode(0o755);
            let _ = fs::set_permissions(path, perms);
        });
        return (exe_path.to_string(), args, path.parent().map(|p| p.to_path_buf()));
    }

    // Check if it's a shell script
    if exe_path.ends_with(".sh") {
        return (
            "bash".to_string(),
            vec![exe_path.to_string()],
            path.parent().map(|p| p.to_path_buf()),
        );
    }

    // Check if it's a Windows executable (.exe) - likely Wine/Proton
    if exe_path.ends_with(".exe") || exe_path.ends_with(".EXE") {
        // Check if it's in a Steam compatdata directory (Proton)
        let path_str = exe_path.to_string();
        if path_str.contains("compatdata") && path_str.contains("steamapps") {
            // This is a Steam Proton game - try to use steam-run or run via proton directly
            // Find the proton runtime
            if let Some(proton_dir) = find_proton_runtime(&path) {
                if let Some(proton_bin) = proton_dir.join("proton").to_str() {
                    let args = vec!["run".to_string(), exe_path.to_string()];
                    return (proton_bin.to_string(), args, path.parent().map(|p| p.to_path_buf()));
                }
            }
            // Fallback: try steam-run
            if Command::new("steam-run").arg("--version").output().is_ok() {
                return (
                    "steam-run".to_string(),
                    vec![exe_path.to_string()],
                    path.parent().map(|p| p.to_path_buf()),
                );
            }
            // Last resort: wine
            return (
                "wine".to_string(),
                vec![exe_path.to_string()],
                path.parent().map(|p| p.to_path_buf()),
            );
        }

        // Check if it's in a Wine prefix
        if path_str.contains(".wine") || path_str.contains("drive_c") {
            return (
                "wine".to_string(),
                vec![exe_path.to_string()],
                path.parent().map(|p| p.to_path_buf()),
            );
        }

        // Generic .exe - try wine
        return (
            "wine".to_string(),
            vec![exe_path.to_string()],
            path.parent().map(|p| p.to_path_buf()),
        );
    }

    // For native Linux binaries, check if it's actually an ELF binary
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.is_file() {
            // Check for ELF magic bytes
            if let Ok(mut file) = File::open(path) {
                let mut magic = [0u8; 4];
                if file.read_exact(&mut magic).is_ok() {
                    if &magic == b"\x7fELF" {
                        // Native Linux binary
                        return (exe_path.to_string(), vec![], path.parent().map(|p| p.to_path_buf()));
                    }
                    // Check for shebang
                    if &magic[..2] == b"#!" {
                        // Script with shebang - execute directly
                        let mut perms = metadata.permissions();
                        perms.set_mode(0o755);
                        let _ = fs::set_permissions(path, perms);
                        return (exe_path.to_string(), vec![], path.parent().map(|p| p.to_path_buf()));
                    }
                }
            }
        }
    }

    // Default: try to execute directly
    (exe_path.to_string(), vec![], path.parent().map(|p| p.to_path_buf()))
}

/// Try to find the Proton runtime for a Steam game
#[cfg(not(target_os = "windows"))]
fn find_proton_runtime(game_exe_path: &Path) -> Option<PathBuf> {
    // Walk up the path to find compatdata/<appid>/ and then look for proton in the steam installation
    let mut current = game_exe_path;
    while let Some(parent) = current.parent() {
        let parent_str = parent.to_string_lossy();
        if parent_str.contains("compatdata") {
            // Found compatdata directory, now look for Steam installation
            if let Ok(home) = std::env::var("HOME") {
                let steam_paths = vec![
                    PathBuf::from(&home).join(".local/share/Steam"),
                    PathBuf::from(&home).join(".steam/steam"),
                    PathBuf::from(&home).join(".var/app/com.valvesoftware.Steam/.local/share/Steam"),
                ];
                for steam_path in steam_paths {
                    let proton_dir = steam_path.join("steamapps/common/Proton");
                    if proton_dir.is_dir() {
                        // Find the latest Proton version
                        if let Ok(entries) = fs::read_dir(&proton_dir) {
                            let mut versions: Vec<_> = entries.filter_map(|e| e.ok()).collect();
                            versions.sort_by(|a, b| b.file_name().cmp(&a.file_name()));
                            for entry in versions {
                                let proton_bin = entry.path().join("proton");
                                if proton_bin.exists() {
                                    return Some(entry.path());
                                }
                            }
                        }
                    }
                }
            }
            break;
        }
        current = parent;
    }
    None
}

// ── Launcher-aware process tracking ────────────────────────────────────────
// Games frequently start through a pre-launcher (REDprelauncher.exe for
// Cyberpunk 2077,launcher-chain up to the real game binary. SILO spawns the
// configured exe and records every PID in the whole spawned subtree; a save
// write counts as belonging to this session when its locking PID's parent chain
// reaches any of those PIDs — even after intermediate launchers have exited.

/// Collect `pid` plus every process that has it (transitively) as an ancestor.
fn collect_subtree_pids(system: &sysinfo::System, root_pid: u32) -> HashSet<u32> {
    let mut known: HashSet<u32> = HashSet::new();
    known.insert(root_pid);
    // Children maps each parent PID to its live children. sysinfo does not
    // expose a reverse lookup, so build one from the full snapshot.
    let mut children: std::collections::HashMap<u32, Vec<u32>> = std::collections::HashMap::new();
    for (pid, proc) in system.processes() {
        if let Some(parent) = proc.parent() {
            children.entry(parent.as_u32()).or_default().push(pid.as_u32());
        }
    }
    let mut queue: Vec<u32> = vec![root_pid];
    while let Some(current) = queue.pop() {
        if let Some(kids) = children.get(&current) {
            for kid in kids {
                if known.insert(*kid) {
                    queue.push(*kid);
                }
            }
        }
    }
    known
}

const LAUNCHER_NAME_HINTS: &[&str] = &[
    "launcher",
    "redprelauncher",
    "prelauncher",
    "bootstrap",
    "starter",
    "splash",
    "epicgameslauncher",
    "galaxyclient",
    "steam",
];

/// True when an exe name smells like a launcher rather than a game binary.
fn is_launcher_like(name_lower: &str) -> bool {
    LAUNCHER_NAME_HINTS.iter().any(|hint| name_lower.contains(hint))
}

/// Folders inside a game install that never hold the game binary. Pruning them
/// keeps the search quick and stops redistributables from out-ranking the game.
const INSTALL_NOISE_DIRS: &[&str] = &[
    "redist",
    "redistributables",
    "_commonredist",
    "support",
    "docs",
    "documentation",
    "installers",
    "mods",
    "cache",
    "tools",
    "crashreporter",
    "dxsetup",
    "directx",
    "dotnet",
];

/// How far below the configured exe the real game binary may sit. One level
/// covers `bin\Game.exe`; two covers `bin\x64\Game.exe`, which is where
/// Cyberpunk 2077 keeps Cyberpunk2077.exe under REDprelauncher.exe.
const MAX_GAME_EXE_DEPTH: usize = 3;

/// A candidate's name-relatedness to the install folder, folded so that
/// "Cyberpunk2077.exe" still scores against the "Cyberpunk 2077" folder.
fn game_exe_name_bonus(stem: &str, install_dir: &str) -> i32 {
    let stem_key = crate::saveguard::normalize_key(stem);
    let dir_key = crate::saveguard::normalize_key(install_dir);
    if stem_key.is_empty() || dir_key.is_empty() {
        0
    } else if stem_key == dir_key {
        60
    } else if stem_key.len() >= 4 && (dir_key.contains(&stem_key) || stem_key.contains(&dir_key)) {
        40
    } else {
        0
    }
}

fn is_exe(path: &Path) -> bool {
    path.is_file() && path.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false)
}

/// Return the exe that most likely is the real game when the configured exe is
/// a launcher. Searches the launcher's own folder first, then a few levels
/// below it, because launchers often sit beside the game (sibling .exe) or
/// above it (`REDprelauncher.exe` next to `bin\x64\Cyberpunk2077.exe`).
/// Signals: name-relatedness to the install folder, shallowness, and size —
/// with the biggest plausible binary as a last resort so a direct-to-game
/// option still exists when no name lines up.
fn find_real_game_exe(configured: &Path) -> Option<PathBuf> {
    use walkdir::WalkDir;

    let dir = configured.parent()?;
    let configured_lower = configured.file_name()?.to_string_lossy().to_lowercase();
    if !is_launcher_like(&configured_lower) {
        return None;
    }
    let dir_name = dir.file_name()?.to_string_lossy().to_lowercase();

    // Named matches win outright; the biggest binary is only a fallback and has
    // to be plausibly a game (a redistributable is never 20MB or more).
    let mut named: Option<(i32, PathBuf)> = None;
    let mut biggest: Option<(u64, PathBuf)> = None;

    let mut consider = |path: &Path, depth: usize| {
        if !is_exe(path) {
            return;
        }
        let name_lower = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        if name_lower == configured_lower || is_blacklisted_exe(&name_lower) || is_launcher_like(&name_lower) {
            return;
        }
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
        let size = fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        let bonus = game_exe_name_bonus(&stem, &dir_name);
        if bonus > 0 {
            let score = bonus + score_exe_candidate(&stem, &dir_name, depth, size);
            if named.as_ref().map_or(true, |(s, _)| score > *s) {
                named = Some((score, path.to_path_buf()));
            }
        }
        if depth <= 2 && size >= 20 * 1024 * 1024 && biggest.as_ref().map_or(true, |(s, _)| size > *s) {
            biggest = Some((size, path.to_path_buf()));
        }
    };

    // Siblings of the launcher.
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten() {
            consider(&entry.path(), 0);
        }
    }

    // And the install's subfolders, pruning the noise ones wholesale.
    let noise = |name: &str| INSTALL_NOISE_DIRS.contains(&name);
    for entry in WalkDir::new(dir)
        .min_depth(1)
        .max_depth(MAX_GAME_EXE_DEPTH)
        .into_iter()
        .filter_entry(|e| !noise(&e.file_name().to_string_lossy().to_lowercase()))
        .flatten()
    {
        consider(entry.path(), entry.depth());
    }

    named.map(|(_, path)| path).or_else(|| biggest.map(|(_, path)| path))
}

/// True when the found binary's name lines up with the install folder — i.e.
/// the launcher genuinely sits above its own game (`REDprelauncher.exe` above
/// `bin\x64\Cyberpunk2077.exe`) rather than us guessing from file size. "Auto"
/// only bypasses a launcher on this stronger signal, so a launcher that is
/// required (account checks, DRM) is never skipped on a hunch.
fn is_confident_game_exe(candidate: &Path, configured: &Path) -> bool {
    let (Some(stem), Some(install)) = (
        candidate.file_stem().map(|s| s.to_string_lossy().to_lowercase()),
        configured
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().to_lowercase()),
    ) else {
        return false;
    };
    game_exe_name_bonus(&stem, &install) > 0
}

/// Names a game's save folders may carry: the library name, the game binary's
/// stem, and the install folder — folded for comparison by
/// [`crate::saveguard::name_matches`]. These are what let SaveGuard find the
/// saves of a game that a launcher started, and what let it resolve publisher
/// layouts like `Saved Games\CD Projekt Red\Cyberpunk 2077` to the game's own
/// folder instead of the publisher's.
fn save_name_keys(game_name: &str, game_exe: &Path) -> Vec<String> {
    let mut keys: Vec<String> = vec![game_name.to_string()];
    if let Some(stem) = game_exe.file_stem().map(|s| s.to_string_lossy().to_lowercase()) {
        if !stem.is_empty() && !is_launcher_like(&stem) {
            keys.push(stem);
        }
    }
    if let Some(folder) = game_exe
        .parent()
        .and_then(|p| p.file_name())
        .map(|n| n.to_string_lossy().to_lowercase())
    {
        // Short folder names ("Games", "bin") match too much to be useful.
        if folder.chars().filter(|c| c.is_alphanumeric()).count() >= 4 {
            keys.push(folder);
        }
    }
    keys.retain(|key| !crate::saveguard::normalize_key(key).is_empty());
    keys
}

#[tauri::command]
pub async fn launch_game(app: AppHandle, game_id: String) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let configured_exe = game.exe_path.clone().ok_or("No executable set")?;
    // Launcher-aware launch target, the Edit modal's Auto / Launcher / Game:
    //   launcher → always start the configured exe (the launcher itself)
    //   game     → start the detected game binary, falling back to the exe above
    //   auto/–   → start the game binary directly when SILO is *confident* it
    //              found it below the launcher, otherwise the configured exe
    // Save detection works through any of these: see name_keys below.
    let configured_path = Path::new(&configured_exe);
    let real_game_exe = find_real_game_exe(configured_path);
    let exe_path = match game.launch_target.as_deref() {
        Some("launcher") => configured_exe.clone(),
        Some("game") => real_game_exe
            .as_ref()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| configured_exe.clone()),
        _ => match real_game_exe.as_ref() {
            Some(candidate) if is_confident_game_exe(candidate, configured_path) => {
                candidate.to_string_lossy().into_owned()
            }
            _ => configured_exe.clone(),
        },
    };
    // Every name this game's save folder might carry, so the saves are found
    // whichever exe we ended up starting (and through publisher nesting).
    let name_keys = save_name_keys(&game.name, Path::new(&exe_path));

    let game_id_clone = game_id.clone();
    let save_path_clone = game.save_path.clone();
    let backup_count_clone = game.backup_count.unwrap_or(5);

    tauri::async_runtime::spawn(async move {
        let window = app.get_webview_window("main");

        let working_dir = std::path::Path::new(&exe_path)
            .parent()
            .unwrap_or(std::path::Path::new(""));

        #[cfg(target_os = "windows")]
        let mut cmd = {
            let mut c = Command::new("cmd");
            c.creation_flags(0x08000000);
            c.current_dir(working_dir);
            c.raw_arg(format!("/C start \"\" /HIGH /WAIT \"{}\"", exe_path));
            c
        };

        #[cfg(not(target_os = "windows"))]
        let mut cmd = {
            let (cmd_name, args, work_dir_override) = build_linux_game_command(&exe_path);
            let mut c = Command::new(&cmd_name);
            c.args(&args);
            c.current_dir(work_dir_override.unwrap_or_else(|| working_dir.to_path_buf()));
            c
        };

        if let Ok(mut child) = cmd.spawn() {
            // Hide SILO's window for the whole session (both modes) so the OS/GPU compositor
            // doesn't keep compositing a full-screen launcher surface — lowers compositor and
            // GPU memory while the game runs. Restored when the game exits.
            if let Some(win) = &window {
                let _ = win.hide();
            }
            // Lower our priority for the whole session so the game gets the memory and
            // SILO's pages become the OS's first reclaim candidate. Restored on exit.
            set_launcher_background(true);
            let exe_name = Path::new(&exe_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            // Watch for save writes when we have no usable folder yet — or when the stored
            // folder has vanished (game moved/reinstalled) so a fresh location is re-found
            // instead of silently never backing up again.
            let run_detection = save_detection_needed(save_path_clone.as_deref());
            // A folder that exists can still be the wrong one: an older build could
            // have stored the publisher folder instead of the game's own, which then
            // suppresses detection forever. Keep looking in that case — but only a
            // folder that names the game may replace it.
            let stored_is_suspect = save_path_clone
                .as_deref()
                .map(|p| Path::new(p).is_dir() && !save_path_names_game(p, &name_keys))
                .unwrap_or(false);
            let watch_for_saves = run_detection || stored_is_suspect;
            let mut detected_path: Option<PathBuf> = None;
            let mut watcher_opt = None;
            let mut rx_opt = None;

            if watch_for_saves {
                let (tx, rx) = channel();
                let watcher_res = notify::recommended_watcher(move |res| {
                    if let Ok(event) = res {
                        let _ = tx.send(event);
                    }
                });

                if let Ok(mut watcher) = watcher_res {
                    for path in get_watch_directories() {
                        let _ = watcher.watch(&path, RecursiveMode::Recursive);
                    }
                    watcher_opt = Some(watcher);
                    rx_opt = Some(rx);
                }
            }

            let spawned_child_pid = child.id();
            let mut system = sysinfo::System::new();
            let mut game_pid: Option<u32> = None;
            let mut game_started_at: Option<Instant> = None;
            let mut is_running = true;
            let mut found_process = false;
            let mut game_is_ours = false; // true only when game_pid is a descendant of our spawned cmd
            let check_start = Instant::now();
            let watch_dirs = get_watch_directories();
            let mut pending_paths: HashSet<PathBuf> = HashSet::new();
            let mut detected_roots: HashSet<PathBuf> = HashSet::new();
            let mut last_detection_check: Option<Instant> = None;
            // Wall-clock moment the game process was first seen, used for the session log.
            let mut game_started_wall: Option<String> = None;
            // Same moment as a wall clock, for the end-of-session save sweep.
            let mut game_started_sys: Option<std::time::SystemTime> = None;
            // Every PID in the subtree SILO spawned (launcher chain + game + helpers).
            // Keeps launcher handoffs linked even after intermediate processes exit.
            let mut known_pids: HashSet<u32> = HashSet::new();
            known_pids.insert(spawned_child_pid);

            while is_running {
                if game_pid.is_none() {
                    // Hunt for the real game PID. First keep the spawned subtree fresh so
                    // launcher handoffs (launcher exits, game spawns) stay attributed to
                    // this session. Then look for a game-named process INSIDE the subtree.
                    // This is what makes save detection work when the configured exe is a
                    // launcher like REDprelauncher.exe: the game spawns under it, and the
                    // subtree keeps the lineage even once the launcher process is gone.
                    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                    known_pids = collect_subtree_pids(&system, spawned_child_pid);

                    let mut name_match: Option<u32> = None;
                    // Best candidate inside our subtree even when names differ: when the
                    // configured exe is a launcher (REDprelauncher.exe), the real game has
                    // a different name and must still be pinned. A large binary that is
                    // NOT launcher-like and lives in (or under) the game install dir is
                    // almost certainly the game itself.
                    let mut subtree_best_score: i32 = -1;
                    let mut subtree_best_score_pid: Option<u32> = None;
                    for (pid, proc) in system.processes() {
                        let proc_name = proc.name().to_string_lossy().to_lowercase();
                        let candidate_pid = pid.as_u32();
                        let in_subtree = known_pids.contains(&candidate_pid);

                        if proc_name == exe_name.to_lowercase() {
                            // Prefer a process spawned inside our tracked subtree (avoids
                            // matching a second instance of the same game already running).
                            if in_subtree {
                                game_pid = Some(candidate_pid);
                                game_is_ours = true;
                                break;
                            }
                            if name_match.is_none() {
                                name_match = Some(candidate_pid);
                            }
                            continue;
                        }

                        // Launcher-handoff detection: score every process inside the
                        // subtree that isn't the launcher itself.
                        if in_subtree
                            && game_pid.is_none()
                            && !is_launcher_like(&proc_name)
                            && proc_name != "cmd.exe"
                            && candidate_pid != spawned_child_pid
                        {
                            let score = {
                                let exe_path: Option<&std::path::Path> = proc.exe();
                                let size: u64 = exe_path
                                    .as_ref()
                                    .and_then(|p| std::fs::metadata(p).ok())
                                    .map(|m| m.len())
                                    .unwrap_or(0);
                                (size / (10 * 1024 * 1024)).min(50) as i32
                            };
                            if score > subtree_best_score {
                                subtree_best_score = score;
                                subtree_best_score_pid = Some(candidate_pid);
                            }
                        }
                    }
                    if game_pid.is_none() && subtree_best_score_pid.is_some() && check_start.elapsed().as_secs() >= 5 {
                        // The launcher handed off to a differently-named game binary.
                        game_pid = subtree_best_score_pid;
                        game_is_ours = true;
                    }
                    // Fall back to name-only matching after a grace period (some games
                    // launch through an out-of-tree service rather than our cmd).
                    if game_pid.is_none() && name_match.is_some() && check_start.elapsed().as_secs() >= 3 {
                        game_pid = name_match;
                        game_is_ours = false;
                    }
                    if game_pid.is_some() {
                        game_started_at = Some(Instant::now());
                        game_started_wall = Some(chrono::Local::now().to_rfc3339());
                        game_started_sys = Some(std::time::SystemTime::now());
                        // Free the full-process snapshot: once pinned we only need the
                        // subtree set, so drop the rest of the table to keep session RAM low.
                        system = sysinfo::System::new();
                    }
                } else if let Some(game_pid) = game_pid {
                    // Keep the whole subtree refreshed: launcher-to-game handoffs happen
                    // AFTER the initial pin, so new children (the real game process) must
                    // keep entering known_pids or save attribution would silently miss them.
                    let root_alive = system
                        .process(sysinfo::Pid::from_u32(spawned_child_pid))
                        .is_some();
                    if root_alive {
                        system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                        known_pids = collect_subtree_pids(&system, spawned_child_pid);
                    } else {
                        // Spawned root is gone (launcher exited after handing off). Refresh
                        // only the pinned PID so we know when the game itself ends.
                        system.refresh_processes(
                            sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(game_pid)]),
                            true,
                        );
                    }
                }

                // Drop the save watcher the moment a root is found (nothing left to watch);
                // otherwise keep it alive for the whole session so late autosaves are caught.
                if detected_path.is_some() {
                    if let Some(w) = watcher_opt.take() {
                        drop(w);
                    }
                    rx_opt = None;
                }
                if let Some(rx) = rx_opt.as_ref() {
                    while let Ok(event) = rx.try_recv() {
                        let is_write = matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_));

                        // Accumulate changed paths and skip ones under an already-detected
                        // save root so Restart Manager isn't queried on every Modify event.
                        if is_write && detected_path.is_none() {
                            for path in event.paths {
                                if detected_roots.iter().any(|root| path.starts_with(root)) {
                                    continue;
                                }
                                pending_paths.insert(path);
                            }
                        }
                    }

                    // Coalesce detection runs to at most once every ~750ms.
                    let due = match last_detection_check {
                        Some(last) => last.elapsed().as_millis() >= 750,
                        None => !pending_paths.is_empty(),
                    };
                    if due && detected_path.is_none() && !pending_paths.is_empty() {
                        last_detection_check = Some(Instant::now());
                        {
                            let paths: Vec<PathBuf> = pending_paths.drain().collect();
                            for path in paths {
                                if detected_roots.iter().any(|root| path.starts_with(root)) {
                                    continue;
                                }
                                let pids = crate::saveguard::get_locking_pids(&path);
                                // Lock evidence only replaces a stored folder when that
                                // folder was already unusable; a suspect folder is only
                                // replaced by one that names the game.
                                let lock_pids: &[u32] = if stored_is_suspect { &[] } else { &pids };
                                for &locking_pid in lock_pids {
                                    // Attribute the write to this session when the locking
                                    // PID's parent chain reaches ANY PID SILO spawned. This
                                    // works through launchers: the game writes saves even if
                                    // the launcher that spawned it has already exited, because
                                    // the subtree kept every PID the chain passed through.
                                    if crate::saveguard::pid_chain_reaches(&system, locking_pid, &known_pids)
                                        || game_pid.is_some_and(|target| {
                                            crate::saveguard::pid_is_descendant_of(&mut system, locking_pid, target)
                                        })
                                    {
                                        if let Some(root) =
                                            crate::saveguard::get_save_root_for(&path, &watch_dirs, &name_keys)
                                        {
                                            log::info!("SaveGuard detected save root: {:?}", root);
                                            detected_path = Some(root.clone());
                                            detected_roots.insert(root);
                                            break;
                                        }
                                    }
                                }
                                // Lock-free fallback. A launcher hands the game off
                                // outside the tree SILO spawned, and many games save
                                // atomically (write, rename, close) so no process holds
                                // the file by the time Restart Manager is asked. The
                                // folder that names this game is evidence enough — and
                                // because a wrong folder here is visible and overridable,
                                // missing the saves entirely is the worse failure.
                                if detected_path.is_none() {
                                    if let Some(root) =
                                        crate::saveguard::save_root_by_name(&path, &watch_dirs, &name_keys)
                                    {
                                        log::info!("SaveGuard detected save root by name: {:?}", root);
                                        detected_path = Some(root.clone());
                                        detected_roots.insert(root);
                                    }
                                }
                                if detected_path.is_some() {
                                    break;
                                }
                            }
                        }
                    }
                }

                std::thread::sleep(std::time::Duration::from_millis(2500));

                let child_exited = matches!(child.try_wait(), Ok(Some(_)));

                if child_exited {
                    // Only keep the loop alive if the pinned PID is still running. Name-only
                    // fallback pins can grab an unrelated already-running instance of the
                    // same game; trusting those here would make this loop never terminate
                    // and the save backup never run.
                    let running_in_system = match game_pid {
                        Some(pid) => game_is_ours && system.process(sysinfo::Pid::from_u32(pid)).is_some(),
                        None => false,
                    };
                    if running_in_system {
                        found_process = true;
                    } else {
                        is_running = !found_process && check_start.elapsed().as_secs() < 12;
                    }
                } else {
                    found_process = true;
                    is_running = true;
                }
            }

            if let Some(watcher) = watcher_opt {
                drop(watcher);
            }

            // Game is over — restore normal priority before the backup/playtime writes.
            set_launcher_background(false);

            // Bring the launcher back BEFORE emitting save status events so the toasts
            // are actually visible when the game exits.
            if let Some(win) = &window {
                let _ = win.show();
                let _ = win.set_focus();
            }

            // Last resort before the backup: nothing was ever attributed to this
            // session — no handle to query (atomic saves), a launcher that started
            // the game outside our tree, or a session that never wrote a file.
            // Sweep the standard save locations for a folder that names the game
            // and was touched while it ran.
            if watch_for_saves && detected_path.is_none() && game_pid.is_some() {
                let since = game_started_sys.unwrap_or_else(std::time::SystemTime::now);
                let install_dir = Path::new(&exe_path).parent();
                if let Some(root) =
                    crate::saveguard::find_save_root_by_scan(&watch_dirs, &name_keys, since, install_dir)
                {
                    log::info!("SaveGuard found save root by scan: {:?}", root);
                    detected_roots.insert(root.clone());
                    detected_path = Some(root);
                }
            }

            let mut final_save_path = save_path_clone;

            if let Some(path) = detected_path {
                let path_str = path.to_string_lossy().into_owned();
                let mut games = get_games(app.clone());
                if let Some(g) = games.iter_mut().find(|g| g.id == game_id_clone) {
                    g.save_path = Some(path_str.clone());
                    g.save_path_source = Some("auto".to_string());
                    let _ = save_games_atomic(&app, &games);

                    let _ = app.emit(
                        "saveguard-path-detected",
                        serde_json::json!({
                            "gameId": game_id_clone,
                            "savePath": path_str
                        }),
                    );
                }
                final_save_path = Some(path_str);
            }

            // Auto-backup after a session — and surface every outcome instead of failing
            // silently. When a known folder has gone missing, the next launch re-runs
            // detection (save_detection_needed) to re-find it.
            match &final_save_path {
                Some(path_str) => {
                    let save_path = Path::new(path_str);
                    let backups_dir = get_backups_dir(&app, &game_id_clone);
                    if save_path.exists() {
                        match fs::create_dir_all(&backups_dir) {
                            Ok(()) => {
                                let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                                let backup_file = backups_dir.join(format!("auto_{}.zip", timestamp));
                                match zip_dir(save_path, &backup_file) {
                                    Ok(()) => {
                                        let _ = prune_backups(&backups_dir, backup_count_clone as usize);
                                        let _ = app.emit(
                                            "saveguard-backup-complete",
                                            serde_json::json!({
                                                "gameId": game_id_clone,
                                                "timestamp": timestamp
                                            }),
                                        );
                                    }
                                    Err(e) => {
                                        log::warn!("SaveGuard auto-backup failed for {}: {}", game_id_clone, e);
                                        let _ = app.emit(
                                            "saveguard-backup-failed",
                                            serde_json::json!({
                                                "gameId": game_id_clone,
                                                "reason": e
                                            }),
                                        );
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("SaveGuard could not create backups dir for {}: {}", game_id_clone, e);
                                let _ = app.emit(
                                    "saveguard-backup-failed",
                                    serde_json::json!({
                                        "gameId": game_id_clone,
                                        "reason": e.to_string()
                                    }),
                                );
                            }
                        }
                    } else {
                        log::warn!(
                            "SaveGuard known save folder missing at exit for {}: {:?}",
                            game_id_clone,
                            save_path
                        );
                        let _ = app.emit(
                            "saveguard-path-missing",
                            serde_json::json!({
                                "gameId": game_id_clone,
                                "savePath": path_str
                            }),
                        );
                    }
                }
                None => {
                    // A session ran with no usable save folder and SaveGuard could not
                    // attribute one — surface it so the user is never left wondering.
                    if run_detection {
                        log::info!("SaveGuard found no save folder for {} this session", game_id_clone);
                        let _ = app.emit(
                            "saveguard-not-found",
                            serde_json::json!({
                                "gameId": game_id_clone
                            }),
                        );
                    }
                }
            }

            // Playtime is measured from when the game process was first detected, not from spawn.
            let elapsed_secs = game_started_at.map_or(0, |started| started.elapsed().as_secs());
            let elapsed_minutes = if elapsed_secs > 0 { elapsed_secs.div_ceil(60) } else { 0 };

            let mut games = get_games(app.clone());
            if let Some(g) = games.iter_mut().find(|g| g.id == game_id_clone) {
                if elapsed_minutes > 0 {
                    g.playtime_minutes += elapsed_minutes as u32;
                }
                g.session_count += 1;
                g.last_played = Some(chrono::Local::now().to_rfc3339());
                let _ = app.emit("playtime-updated", g.clone());
            }
            let _ = save_games_atomic(&app, &games);

            // Record the session in the stats history. started_at is the moment we first
            // saw the game process (fallback: exit time for sub-minute/untracked starts),
            // and minutes uses the same ceiling as the playtime total above so they agree.
            let session_started = game_started_wall
                .clone()
                .unwrap_or_else(|| chrono::Local::now().to_rfc3339());
            record_session(&app, &game_id_clone, &session_started, elapsed_minutes as u32);
        }
    });

    Ok(())
}

#[tauri::command]
pub fn window_minimize(window: tauri::Window) {
    window.minimize().ok();
}

#[tauri::command]
pub fn window_maximize(window: tauri::Window) {
    if let Ok(is_max) = window.is_maximized() {
        if is_max {
            window.unmaximize().ok();
        } else {
            window.maximize().ok();
        }
    }
}

#[tauri::command]
pub fn window_close(window: tauri::Window) {
    window.close().ok();
}

#[tauri::command]
pub fn window_start_dragging(window: tauri::Window) {
    window.start_dragging().ok();
}

// Expanded blacklist of non-game executables, shared by scan_folder and library import.
pub const EXE_BLACKLIST: &[&str] = &[
    "unins",
    "setup",
    "crash",
    "redist",
    "dxwebsetup",
    "vcredist",
    "cef",
    "bootstrap",
    "dotnet",
    "directx",
    "dxsetup",
    "installer",
    "updater",
    "reporter",
    "helper",
    "service",
    "vc_redist",
    "oalinst",
    "physx",
    "easyanticheat",
    "battleye",
    "beclient",
    "beservice",
    "eac_launcher",
    "ue4prereqsetup",
    "unrealcefsubprocess",
    "steamwebhelper",
];

pub fn is_blacklisted_exe(lowercase_file_name: &str) -> bool {
    EXE_BLACKLIST.iter().any(|b| lowercase_file_name.contains(b))
}

// Scores a candidate game executable — higher is better. Shared by scan_folder and library import.
pub fn score_exe_candidate(stem: &str, game_dir: &str, depth: usize, size_bytes: u64) -> i32 {
    let mut score: i32 = 0;

    // Prefer exe name matching the game folder name
    if stem == game_dir || game_dir.contains(stem) || stem.contains(game_dir) {
        score += 50;
    }

    // Prefer shallower depth
    score -= (depth as i32) * 5;

    // Deprioritize variant suffixes
    let deprioritize = [
        "_be",
        "_dx12",
        "_dx11",
        "_vulkan",
        "_debug",
        "_server",
        "_shipping",
        "_launcher",
        "-win64",
        "-shipping",
        "dedicated",
    ];
    for suffix in &deprioritize {
        if stem.contains(suffix) {
            score -= 30;
        }
    }

    // Prefer larger files (likely the main game binary)
    score += (size_bytes / (10 * 1024 * 1024)) as i32; // +1 per 10MB

    score
}

#[derive(serde::Serialize)]
pub struct ScannedGame {
    pub name: String,
    pub exe_path: String,
}

#[tauri::command]
pub async fn scan_folder(folder_path: String) -> Result<Vec<ScannedGame>, String> {
    use walkdir::WalkDir;
    use std::collections::HashMap;

    let folder = std::path::Path::new(&folder_path);

    if !folder.exists() || !folder.is_dir() {
        return Err("Invalid folder path".into());
    }

    // Collect all candidate exes
    struct ExeCandidate {
        path: PathBuf,
        depth: usize,
        game_dir: String,     // top-level subfolder name (the "game folder")
        display_name: String, // cleaned up name for display
    }

    let mut candidates: Vec<ExeCandidate> = Vec::new();

    for entry in WalkDir::new(&folder_path)
        .max_depth(5)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let ext = match path.extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => continue,
        };
        if ext != "exe" {
            continue;
        }

        let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
        let file_stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();

        // Skip blacklisted names
        if is_blacklisted_exe(&file_name) {
            continue;
        }

        // Skip very small exes (< 500KB) — usually launchers or stubs
        if let Ok(meta) = std::fs::metadata(path) {
            if meta.len() < 500_000 {
                continue;
            }
        }

        // Determine the top-level game directory relative to the scanned folder
        let relative = match path.strip_prefix(folder) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let components: Vec<_> = relative.components().collect();
        if components.is_empty() {
            continue;
        }

        // If the exe is directly in the scanned folder (no subfolder), use exe stem as game dir
        let game_dir = if components.len() == 1 {
            file_stem.to_string()
        } else {
            components[0].as_os_str().to_string_lossy().to_string()
        };

        let depth = entry.depth();

        // Build display name from the game directory, cleaning up common folder patterns
        let parent_name = path
            .parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| game_dir.clone());

        let display_name = {
            let p_lower = parent_name.to_lowercase();
            if [
                "bin", "win64", "win32", "binaries", "system32", "x64", "x86", "game", "shipping",
            ]
            .contains(&p_lower.as_str())
            {
                // Walk up to find a better name
                path.parent()
                    .and_then(|p| p.parent())
                    .and_then(|p| p.file_name())
                    .map(|n| n.to_string_lossy().into_owned())
                    .unwrap_or(game_dir.clone())
            } else {
                parent_name
            }
        };

        candidates.push(ExeCandidate {
            path: path.to_path_buf(),
            depth,
            game_dir: game_dir.to_lowercase(),
            display_name,
        });
    }

    // Group by game directory and pick the best exe per group
    let mut groups: HashMap<String, Vec<ExeCandidate>> = HashMap::new();
    for c in candidates {
        groups.entry(c.game_dir.clone()).or_default().push(c);
    }

    let mut results = Vec::new();
    for (_game_dir, mut exes) in groups {
        // Score each exe — higher is better
        exes.sort_by(|a, b| {
            let score = |c: &ExeCandidate| -> i32 {
                let stem = c.path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
                let size = std::fs::metadata(&c.path).map(|m| m.len()).unwrap_or(0);
                score_exe_candidate(&stem, &c.game_dir, c.depth, size)
            };

            score(b).cmp(&score(a))
        });

        if let Some(best) = exes.into_iter().next() {
            results.push(ScannedGame {
                name: best.display_name,
                exe_path: best.path.to_string_lossy().into_owned(),
            });
        }
    }

    // Sort results alphabetically by name
    results.sort_by_key(|a| a.name.to_lowercase());

    Ok(results)
}

#[derive(serde::Serialize)]
pub struct SteamMetadata {
    pub name: String,
    pub app_id: String,
    pub wallpaper: String,
    pub logo: String,
}

#[tauri::command]
pub async fn fetch_steam_metadata(name: String) -> Result<Option<SteamMetadata>, String> {
    let url = format!(
        "https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US",
        urlencoding::encode(&name)
    );

    let client = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .build()
    {
        Ok(c) => c,
        Err(e) => return Err(e.to_string()),
    };

    match client.get(&url).send().await {
        Ok(response) => {
            if let Ok(json) = response.json::<serde_json::Value>().await {
                if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                    if let Some(first_item) = items.first() {
                        if let (Some(id), Some(game_name)) = (first_item.get("id"), first_item.get("name")) {
                            let id_str = id.as_i64().unwrap_or(0).to_string();
                            let game_name_str = game_name.as_str().unwrap_or(&name).to_string();

                            let logo_candidates = [
                                format!(
                                    "https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/logo.png",
                                    id_str
                                ),
                                format!("https://cdn.cloudflare.steamstatic.com/steam/apps/{}/logo.png", id_str),
                            ];
                            let mut logo = String::new();
                            for cand in &logo_candidates {
                                if let Ok(res) = client.get(cand).send().await {
                                    if res.status().is_success() {
                                        logo = cand.clone();
                                        break;
                                    }
                                }
                            }

                            let wallpaper_candidates = [
                                format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", id_str),
                                format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/page_bg_generated_v6.jpg", id_str),
                                format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/header.jpg", id_str),
                            ];
                            let mut wallpaper = String::new();
                            for cand in &wallpaper_candidates {
                                if let Ok(res) = client.get(cand).send().await {
                                    if res.status().is_success() {
                                        wallpaper = cand.clone();
                                        break;
                                    }
                                }
                            }

                            return Ok(Some(SteamMetadata {
                                name: game_name_str,
                                app_id: id_str,
                                wallpaper,
                                logo,
                            }));
                        }
                    }
                }
            }
            Ok(None)
        }
        Err(e) => Err(format!("Failed to fetch: {}", e)),
    }
}

// Helper functions and commands for SaveGuard
fn get_backups_dir(app: &AppHandle, game_id: &str) -> PathBuf {
    app.path().app_data_dir().unwrap().join("backups").join(game_id)
}

// ── Session history (stats) ─────────────────────────────────────────────────
// One row per completed play session, written at game exit in launch_game right
// where playtime totals are updated, so the log and the aggregates always agree.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionRecord {
    #[serde(rename = "gameId")]
    pub game_id: String,
    #[serde(rename = "startedAt")]
    pub started_at: String,
    // Whole minutes, same ceiling used to increment playtimeMinutes.
    pub minutes: u32,
}

fn get_sessions_path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("sessions.json")
}

fn read_sessions(app: &AppHandle) -> Vec<SessionRecord> {
    fs::read_to_string(get_sessions_path(app))
        .ok()
        .and_then(|data| serde_json::from_str::<Vec<SessionRecord>>(&data).ok())
        .unwrap_or_default()
}

fn save_sessions_atomic(app: &AppHandle, sessions: &[SessionRecord]) -> Result<(), String> {
    let path = get_sessions_path(app);
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json_str = serde_json::to_string_pretty(sessions).map_err(|e| e.to_string())?;
    let tmp_path = path.with_extension("json.tmp");
    {
        let mut file = File::create(&tmp_path).map_err(|e| e.to_string())?;
        file.write_all(json_str.as_bytes()).map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
    }
    fs::rename(&tmp_path, &path).map_err(|e| e.to_string())?;
    Ok(())
}

pub fn record_session(app: &AppHandle, game_id: &str, started_at: &str, minutes: u32) {
    let mut sessions = read_sessions(app);
    sessions.push(SessionRecord {
        game_id: game_id.to_string(),
        started_at: started_at.to_string(),
        minutes,
    });
    if let Err(e) = save_sessions_atomic(app, &sessions) {
        log::warn!("Failed to record session for {}: {}", game_id, e);
    }
}

#[tauri::command]
pub fn list_sessions(app: AppHandle) -> Vec<SessionRecord> {
    let mut sessions = read_sessions(&app);
    // Newest first, bounded so a long-lived library stays light to transfer.
    sessions.sort_by(|a, b| {
        let ts = |s: &str| {
            chrono::DateTime::parse_from_rfc3339(s)
                .map(|d| d.timestamp_millis())
                .unwrap_or(0)
        };
        ts(&b.started_at).cmp(&ts(&a.started_at))
    });
    sessions.truncate(5000);
    sessions
}





fn get_watch_directories() -> Vec<PathBuf> {
    let mut paths = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(roaming) = std::env::var("APPDATA") {
            paths.push(PathBuf::from(roaming));
        }
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            let local_dir = PathBuf::from(&local);
            paths.push(local_dir.clone());
            paths.push(local_dir.join("LocalLow"));
        }
        if let Ok(userprofile) = std::env::var("USERPROFILE") {
            let userpath = PathBuf::from(&userprofile);
            paths.push(userpath.join("Saved Games"));
            if let Some(docs) = windows_documents_dir() {
                paths.push(docs);
            } else {
                paths.push(userpath.join("Documents"));
            }
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(&home);
            // XDG standard user data/config dirs
            let xdg_data = std::env::var("XDG_DATA_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| h.join(".local/share"));
            let xdg_config = std::env::var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .unwrap_or_else(|_| h.join(".config"));
            paths.push(xdg_data.clone());
            paths.push(xdg_config);
            paths.push(h.join("Documents"));
            paths.push(h.join("Saved Games"));
            // Proton/Wine compatdata (Steam games running through Proton store saves here)
            let compatdata = xdg_data.join("Steam/steamapps/compatdata");
            if compatdata.is_dir() {
                paths.push(compatdata);
            }
            let flatpak_compat = h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/compatdata");
            if flatpak_compat.is_dir() {
                paths.push(flatpak_compat);
            }
            // Wine prefix
            let wine_prefix = h.join(".wine/drive_c/users");
            if wine_prefix.is_dir() {
                paths.push(wine_prefix);
            }
        }
    }

    paths.into_iter().filter(|p| p.exists() && p.is_dir()).collect()
}

#[cfg(target_os = "windows")]
fn windows_documents_dir() -> Option<PathBuf> {
    use winreg::enums::*;
    use winreg::RegKey;
    let shell = RegKey::predef(HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Explorer\User Shell Folders")
        .ok()?;
    let personal: String = shell.get_value("Personal").ok()?;
    let expanded = expand_env_vars(&personal);
    let path = PathBuf::from(expanded);
    if path.is_dir() {
        Some(path)
    } else {
        None
    }
}

// Expands the %VAR% tokens used by "User Shell Folders" values (e.g. %USERPROFILE%).
#[cfg(target_os = "windows")]
fn expand_env_vars(input: &str) -> String {
    let mut out = String::new();
    let bytes = input.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%' {
            if let Some(relative_end) = input[i + 1..].find('%') {
                let key = &input[i + 1..i + 1 + relative_end];
                if !key.is_empty() {
                    if let Ok(value) = std::env::var(key) {
                        out.push_str(&value);
                        i += relative_end + 2;
                        continue;
                    }
                }
            }
        }
        out.push(bytes[i] as char);
        i += 1;
    }
    out
}

/// Library exports skip nested archives — the per-game snapshots under
/// `backups/` and `saves/` — so the export stays a library + settings bundle
/// instead of ballooning into a zip of zips. Note this means save snapshots do
/// not travel with an exported library; the games, their settings, cover art and
/// session history do.
fn is_nested_archive(rel_path: &Path) -> bool {
    let mut components = rel_path.components();
    let Some(first) = components.next() else {
        return false;
    };
    let folder = first.as_os_str().to_string_lossy().to_lowercase();
    (folder == "backups" || folder == "saves")
        && rel_path
            .extension()
            .and_then(|e| e.to_str())
            .map(|e| e.eq_ignore_ascii_case("zip"))
            .unwrap_or(false)
}

fn zip_dir(src_dir: &Path, dest_zip: &Path) -> Result<(), String> {
    zip_dir_filtered(src_dir, dest_zip, |_| false)
}

fn zip_dir_filtered<F>(src_dir: &Path, dest_zip: &Path, exclude: F) -> Result<(), String>
where
    F: Fn(&Path) -> bool,
{
    let file = File::create(dest_zip).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default().compression_method(zip::CompressionMethod::Deflated);

    let walkdir = WalkDir::new(src_dir);
    let mut files_added: u32 = 0;
    let mut skipped: u32 = 0;
    for entry in walkdir.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = match path.strip_prefix(src_dir) {
            Ok(n) => n,
            Err(_) => continue,
        };

        if exclude(name) {
            continue;
        }

        let zip_name = name.to_string_lossy().replace("\\", "/");

        if path.is_file() {
            // Try to read the file; if locked or inaccessible, skip it silently
            // so the rest of the backup can still be created.
            if let Ok(mut f) = File::open(path) {
                if zip.start_file(&zip_name, options).is_ok() {
                    let mut buffer = Vec::new();
                    if f.read_to_end(&mut buffer).is_ok() {
                        if zip.write_all(&buffer).is_ok() {
                            files_added += 1;
                        } else {
                            skipped += 1;
                        }
                    } else {
                        skipped += 1;
                    }
                } else {
                    skipped += 1;
                }
            } else {
                skipped += 1;
            }
        } else if !name.as_os_str().is_empty() {
            let _ = zip.add_directory(zip_name, options);
        }
    }
    log::info!("Zip completed: {} files added, {} skipped", files_added, skipped);
    if files_added == 0 {
        return Err(
            "No files could be read from the save directory - it may be empty or all files are locked".to_string(),
        );
    }
    zip.finish().map_err(|e| e.to_string())?;
    Ok(())
}

fn find_common_root_prefix(archive: &mut zip::ZipArchive<File>) -> Option<PathBuf> {
    let mut common_root: Option<PathBuf> = None;

    for i in 0..archive.len() {
        let file = match archive.by_index(i) {
            Ok(f) => f,
            Err(_) => return None,
        };

        let raw_name = file.name().replace('\\', "/");
        let path = Path::new(&raw_name);

        let mut components = path.components();
        let first = match components.next() {
            Some(Component::Normal(c)) => PathBuf::from(c),
            _ => return None,
        };

        // If this entry is at the top level (only 1 component) and is NOT a directory,
        // then it is a top-level file, so there is no single wrapping root directory.
        if components.next().is_none() && !raw_name.ends_with('/') {
            return None;
        }

        match &common_root {
            None => common_root = Some(first),
            Some(root) => {
                if root != &first {
                    return None;
                }
            }
        }
    }

    common_root
}

fn unzip_file(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    let common_prefix = find_common_root_prefix(&mut archive);

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;

        // Older SILO backups may use Windows separators, so normalize them before checking.
        let file_name = file.name().replace('\\', "/");
        let archive_path = Path::new(&file_name);
        if archive_path.as_os_str().is_empty()
            || archive_path.is_absolute()
            || archive_path.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err(format!("Backup contains an unsafe path: {}", file.name()));
        }

        let relative_path = match &common_prefix {
            Some(prefix) => match archive_path.strip_prefix(prefix) {
                Ok(p) => p,
                Err(_) => archive_path,
            },
            None => archive_path,
        };

        // If stripping the common prefix leaves an empty path (i.e. the root directory entry itself), skip it.
        if relative_path.as_os_str().is_empty() {
            continue;
        }

        let outpath = dest_dir.join(relative_path);

        if file_name.ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(p).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}

fn temporary_sibling(path: &Path, purpose: &str) -> Result<PathBuf, String> {
    let parent = path.parent().ok_or("Save folder has no parent directory")?;
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or("Save folder name is invalid")?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    Ok(parent.join(format!(".{}_silo_{}_{}", name, purpose, timestamp)))
}

fn restore_save_transactionally(backup_file: &Path, save_path: &Path, backups_dir: &Path) -> Result<(), String> {
    stage_and_swap(save_path, backups_dir, "restore", |staging_dir| {
        unzip_file(backup_file, staging_dir).map_err(|error| format!("Backup could not be extracted safely: {}", error))
    })
}

/// Build the folder a save path will hold in a staging directory beside it, then
/// swap it into place: the previous contents are snapshotted into `backups_dir`
/// first, and nothing replaces the live folder until the staged one is complete.
/// `populate` fills the empty staging folder — unzipping a backup, or copying a
/// folder in from another machine.
fn stage_and_swap(
    save_path: &Path,
    backups_dir: &Path,
    purpose: &str,
    populate: impl FnOnce(&Path) -> Result<(), String>,
) -> Result<(), String> {
    let parent = save_path.parent().ok_or("Save folder has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("Could not prepare save folder: {}", e))?;

    let staging_dir = temporary_sibling(save_path, &format!("{}_staging", purpose))?;
    fs::create_dir(&staging_dir).map_err(|e| format!("Could not create {} staging folder: {}", purpose, e))?;

    if let Err(error) = populate(&staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(error);
    }

    if save_path.exists() {
        fs::create_dir_all(backups_dir).map_err(|e| format!("Could not prepare backup folder: {}", e))?;
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let safety_snapshot = backups_dir.join(format!("manual_{}_Before%20{}.zip", timestamp, purpose));
        if let Err(error) = zip_dir(save_path, &safety_snapshot) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(format!("Could not create the pre-{} safety backup: {}", purpose, error));
        }
    }

    let previous_dir = temporary_sibling(save_path, &format!("{}_previous", purpose))?;
    let had_existing_save = save_path.exists();
    if had_existing_save {
        if let Err(error) = fs::rename(save_path, &previous_dir) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(format!(
                "Could not prepare current save for {}. Is the game still running? {}",
                purpose, error
            ));
        }
    }

    if let Err(error) = fs::rename(&staging_dir, save_path) {
        if had_existing_save {
            let _ = fs::rename(&previous_dir, save_path);
        }
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(format!("Could not apply {}d save: {}", purpose, error));
    }

    if had_existing_save {
        if let Err(error) = fs::remove_dir_all(&previous_dir) {
            log::warn!(
                "{}d save successfully but could not remove the temporary previous save at {:?}: {}",
                purpose,
                previous_dir,
                error
            );
        }
    }

    Ok(())
}

/// Copy a save folder's contents into the empty folder prepared for it. Only
/// files and folders come across: a save folder is data, not a filesystem graph,
/// so links are left behind rather than followed somewhere unexpected.
fn copy_tree(from: &Path, into: &Path) -> Result<(), String> {
    let mut copied = 0usize;
    for entry in walkdir::WalkDir::new(from).min_depth(1) {
        let entry = entry.map_err(|e| format!("Could not read {}: {}", from.display(), e))?;
        let relative = entry
            .path()
            .strip_prefix(from)
            .map_err(|e| format!("Could not place {}: {}", entry.path().display(), e))?;
        let target = into.join(relative);
        if entry.file_type().is_dir() {
            fs::create_dir_all(&target).map_err(|e| format!("Could not create {}: {}", target.display(), e))?;
        } else if entry.file_type().is_file() {
            if let Some(parent) = target.parent() {
                fs::create_dir_all(parent).map_err(|e| format!("Could not create {}: {}", parent.display(), e))?;
            }
            fs::copy(entry.path(), &target).map_err(|e| format!("Could not copy {}: {}", entry.path().display(), e))?;
            copied += 1;
        }
    }
    if copied == 0 {
        // An empty folder is not saves: better to say so than to leave a game
        // pointed at a folder that holds nothing.
        return Err(format!("{} holds no files to bring across", from.display()));
    }
    Ok(())
}

fn prune_backups(backups_dir: &Path, max_backups: usize) -> Result<(), String> {
    if !backups_dir.exists() {
        return Ok(());
    }
    let mut entries = Vec::new();
    for entry in fs::read_dir(backups_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip") {
            let file_name = path.file_name().and_then(|s| s.to_str()).unwrap_or("");
            if file_name.starts_with("auto_") {
                if let Ok(metadata) = entry.metadata() {
                    if let Ok(modified) = metadata.modified() {
                        entries.push((path, modified));
                    }
                }
            }
        }
    }

    entries.sort_by_key(|e| e.1);

    if entries.len() > max_backups {
        let remove_count = entries.len() - max_backups;
        for entry in entries.iter().take(remove_count) {
            let _ = fs::remove_file(&entry.0);
        }
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BackupSnapshot {
    pub name: String,
    pub timestamp: String,
    pub size_bytes: u64,
    pub is_auto: bool,
    pub custom_name: Option<String>,
}

// Pure helper: parses a backup filename into its snapshot metadata. The caller fills
// in size_bytes from filesystem metadata. Used by get_game_backups and get_all_backups.
pub fn parse_backup_snapshot(filename: &str) -> Option<BackupSnapshot> {
    if !filename.ends_with(".zip") {
        return None;
    }

    // Parse format: auto_YYYYMMDD_HHMMSS.zip or manual_YYYYMMDD_HHMMSS_CustomName.zip
    let is_auto = filename.starts_with("auto_");
    let mut custom_name = None;

    let name_without_ext = filename.strip_suffix(".zip").unwrap_or(filename);
    let parts: Vec<&str> = name_without_ext.splitn(4, '_').collect();

    let timestamp_str = if parts.len() >= 3 {
        format!("{}_{}", parts[1], parts[2])
    } else if name_without_ext.starts_with("backup_") {
        name_without_ext.replace("backup_", "") // Legacy backups
    } else {
        "Unknown Time".to_string()
    };

    let mut formatted_time = timestamp_str.clone();
    if let Ok(dt) = chrono::NaiveDateTime::parse_from_str(&timestamp_str, "%Y%m%d_%H%M%S") {
        formatted_time = dt.format("%Y-%m-%d %H:%M:%S").to_string();
    }

    if parts.len() == 4 && !is_auto {
        // Try to urldecode the custom name
        let decoded = urlencoding::decode(parts[3]).unwrap_or_else(|_| std::borrow::Cow::Borrowed(parts[3]));
        custom_name = Some(decoded.into_owned());
    }

    Some(BackupSnapshot {
        name: filename.to_string(),
        timestamp: formatted_time,
        size_bytes: 0,
        is_auto,
        custom_name,
    })
}

#[tauri::command]
pub fn get_game_backups(app: AppHandle, game_id: String) -> Result<Vec<BackupSnapshot>, String> {
    let backups_dir = get_backups_dir(&app, &game_id);
    if !backups_dir.exists() {
        return Ok(vec![]);
    }

    let mut snapshots = Vec::new();
    for entry in fs::read_dir(backups_dir).map_err(|e| e.to_string())? {
        let entry = entry.map_err(|e| e.to_string())?;
        let path = entry.path();
        if path.is_file() && path.extension().and_then(|s| s.to_str()) == Some("zip") {
            if let Ok(metadata) = entry.metadata() {
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                if let Some(mut snapshot) = parse_backup_snapshot(&name) {
                    snapshot.size_bytes = metadata.len();
                    snapshots.push(snapshot);
                }
            }
        }
    }

    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));

    Ok(snapshots)
}

fn validate_backup_name(app: &AppHandle, game_id: &str, backup_name: &str) -> Result<PathBuf, String> {
    if backup_name.contains('/') || backup_name.contains('\\') || backup_name.contains("..") {
        return Err("Invalid backup name: path traversal characters detected".to_string());
    }

    let backups_dir = get_backups_dir(app, game_id);
    let target_path = backups_dir.join(backup_name);

    if !backups_dir.exists() {
        return Err("Backups directory does not exist".to_string());
    }

    let canonical_backups = backups_dir
        .canonicalize()
        .map_err(|e| format!("Invalid backups directory: {}", e))?;

    if target_path.exists() {
        let canonical_target = target_path
            .canonicalize()
            .map_err(|e| format!("Invalid backup file: {}", e))?;
        if !canonical_target.starts_with(&canonical_backups) {
            return Err("Invalid backup name: path outside backup directory".to_string());
        }
    } else if let Some(parent) = target_path.parent() {
        if let Ok(canonical_parent) = parent.canonicalize() {
            if !canonical_parent.starts_with(&canonical_backups) {
                return Err("Invalid backup name: path outside backup directory".to_string());
            }
        }
    }

    Ok(target_path)
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, game_id: String, backup_name: String) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let save_path_str = game.save_path.clone().ok_or("No save path configured for this game")?;

    let backup_file = validate_backup_name(&app, &game_id, &backup_name)?;
    if !backup_file.exists() {
        return Err("Backup file does not exist".to_string());
    }

    // Resolve the destination before touching disk: a folder left behind by another
    // machine must never be resurrected just because the library still remembers it,
    // or the saves land where the game will never look.
    let target = resolve_restore_target(&game, &save_path_str)?;

    let backups_dir = get_backups_dir(&app, &game_id);
    restore_save_transactionally(&backup_file, &target.path, &backups_dir)?;

    if target.rediscovered {
        // Remember where the saves actually went, so the next backup and restore of
        // this game agree with each other.
        let path_str = target.path.to_string_lossy().into_owned();
        let mut games = get_games(app.clone());
        if let Some(g) = games.iter_mut().find(|g| g.id == game_id) {
            g.save_path = Some(path_str.clone());
            g.save_path_source = Some("auto".to_string());
            if let Err(e) = save_games_atomic(&app, &games) {
                log::warn!(
                    "Restored {} but could not persist the re-detected save folder: {}",
                    game_id,
                    e
                );
            }
        }
        let _ = app.emit(
            "saveguard-path-detected",
            serde_json::json!({ "gameId": game_id, "savePath": path_str }),
        );
    }

    Ok(())
}

#[tauri::command]
pub fn delete_backup(app: AppHandle, game_id: String, backup_name: String) -> Result<(), String> {
    let backup_file = validate_backup_name(&app, &game_id, &backup_name)?;
    if backup_file.exists() {
        fs::remove_file(backup_file).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[derive(Debug, Clone, Serialize)]
pub struct BackupSummary {
    #[serde(rename = "gameId")]
    pub game_id: String,
    #[serde(rename = "gameName")]
    pub game_name: String,
    pub name: String,
    pub timestamp: String,
    #[serde(rename = "sizeBytes")]
    pub size_bytes: u64,
    #[serde(rename = "isAuto")]
    pub is_auto: bool,
    #[serde(rename = "customName")]
    pub custom_name: Option<String>,
}

#[tauri::command]
pub fn get_all_backups(app: AppHandle) -> Result<Vec<BackupSummary>, String> {
    let backups_root = app.path().app_data_dir().unwrap().join("backups");
    let games = get_games(app.clone());
    let mut summaries = Vec::new();

    if backups_root.is_dir() {
        for entry in fs::read_dir(&backups_root).map_err(|e| e.to_string())? {
            let entry = entry.map_err(|e| e.to_string())?;
            let game_dir = entry.path();
            if !game_dir.is_dir() {
                continue;
            }
            let game_id = entry.file_name().to_string_lossy().to_string();
            let game_name = games
                .iter()
                .find(|g| g.id == game_id)
                .map(|g| g.name.clone())
                .unwrap_or_else(|| game_id.clone());
            if let Ok(snapshots) = get_game_backups(app.clone(), game_id.clone()) {
                for snapshot in snapshots {
                    summaries.push(BackupSummary {
                        game_id: game_id.clone(),
                        game_name: game_name.clone(),
                        name: snapshot.name,
                        timestamp: snapshot.timestamp,
                        size_bytes: snapshot.size_bytes,
                        is_auto: snapshot.is_auto,
                        custom_name: snapshot.custom_name,
                    });
                }
            }
        }
    }

    summaries.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    Ok(summaries)
}

#[tauri::command]
pub fn delete_game_backups(app: AppHandle, game_id: String) -> Result<(), String> {
    if game_id.contains('/') || game_id.contains('\\') || game_id.contains("..") {
        return Err("Invalid game id: path traversal characters detected".to_string());
    }
    let backups_dir = get_backups_dir(&app, &game_id);
    if backups_dir.exists() {
        fs::remove_dir_all(&backups_dir).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn backup_game_now(app: AppHandle, game_id: String, custom_name: Option<String>) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let install_present = recorded_install_present(&game);
    let save_path_str = game.save_path.ok_or("No save path configured for this game")?;
    let save_path = Path::new(&save_path_str);

    if !save_path.exists() {
        return Err(save_folder_missing_message(install_present, &save_path_str));
    }

    let backups_dir = get_backups_dir(&app, &game_id);
    fs::create_dir_all(&backups_dir).map_err(|e| e.to_string())?;

    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();

    let filename = if let Some(name) = custom_name {
        if name.trim().is_empty() {
            format!("manual_{}.zip", timestamp)
        } else {
            let encoded = urlencoding::encode(name.trim());
            format!("manual_{}_{}.zip", timestamp, encoded)
        }
    } else {
        format!("manual_{}.zip", timestamp)
    };

    let backup_file = backups_dir.join(filename);

    zip_dir(save_path, &backup_file)?;

    let max_backups = game.backup_count.unwrap_or(5) as usize;
    let _ = prune_backups(&backups_dir, max_backups);

    Ok(())
}

#[tauri::command]
pub fn check_uninstaller(app: AppHandle, game_id: String) -> Result<Option<String>, String> {
    let games = get_games(app);
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let exe_path_str = game.exe_path.unwrap_or_default();
    let exe_path = Path::new(&exe_path_str);
    if let Some(parent) = exe_path.parent() {
        #[cfg(target_os = "windows")]
        {
            let unins = parent.join("unins000.exe");
            if unins.exists() {
                return Ok(Some(unins.to_string_lossy().to_string()));
            }
            let unins = parent.join("uninstall.exe");
            if unins.exists() {
                return Ok(Some(unins.to_string_lossy().to_string()));
            }
        }

        #[cfg(not(target_os = "windows"))]
        {
            let exe_path_str = exe_path_str.to_lowercase();

            // Check for Wine/Proton uninstallers
            let unins = parent.join("unins000.exe");
            if unins.exists() {
                return Ok(Some(unins.to_string_lossy().to_string()));
            }
            let unins = parent.join("uninstall.exe");
            if unins.exists() {
                return Ok(Some(unins.to_string_lossy().to_string()));
            }

            // Check for Steam game (in compatdata or steamapps)
            if exe_path_str.contains("steamapps") || exe_path_str.contains("compatdata") {
                // Try to find the appmanifest to get app ID
                let mut current = parent;
                while let Some(p) = current.parent() {
                    if p.file_name().and_then(|n| n.to_str()) == Some("steamapps") {
                        // Look for appmanifest_*.acf files
                        if let Ok(entries) = fs::read_dir(p) {
                            for entry in entries.filter_map(|e| e.ok()) {
                                let path = entry.path();
                                let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
                                if name.starts_with("appmanifest_") && name.ends_with(".acf") {
                                    if let Ok(data) = fs::read_to_string(&path) {
                                        for line in data.lines() {
                                            let t = line.trim();
                                            if t.starts_with("\"appid\"") {
                                                let parts: Vec<&str> = t.splitn(4, '"').collect();
                                                if parts.len() >= 4 {
                                                    let app_id = parts[3];
                                                    return Ok(Some(format!("steam://uninstall/{}", app_id)));
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        break;
                    }
                    current = p;
                }
                // Fallback: just return a generic steam uninstall hint
                return Ok(Some("steam://open/minigameslist".to_string()));
            }

            // Check for Flatpak
            if exe_path_str.contains(".var/app/") {
                // Extract flatpak app ID from path
                let parts: Vec<&str> = exe_path_str.split(".var/app/").collect();
                if parts.len() > 1 {
                    let after = parts[1];
                    let app_id = after.split('/').next().unwrap_or("");
                    if !app_id.is_empty() {
                        return Ok(Some(format!("flatpak uninstall {}", app_id)));
                    }
                }
            }

            // Check for native Linux uninstall scripts
            let uninstall_scripts = ["uninstall.sh", "uninstall", "uninstall.bin"];
            for script in uninstall_scripts {
                let script_path = parent.join(script);
                if script_path.exists() {
                    return Ok(Some(script_path.to_string_lossy().to_string()));
                }
            }
        }
    }
    Ok(None)
}

fn is_protected_dir(path: &Path) -> bool {
    if path.parent().is_none() {
        return true;
    }

    let canonical = match path.canonicalize() {
        Ok(c) => c,
        Err(_) => return false,
    };

    let mut protected = Vec::new();

    #[cfg(target_os = "windows")]
    {
        if let Ok(home) = std::env::var("USERPROFILE") {
            let h = PathBuf::from(home);
            protected.push(h.clone());
            protected.push(h.join("Desktop"));
            protected.push(h.join("Documents"));
            protected.push(h.join("Downloads"));
            protected.push(h.join("Videos"));
            protected.push(h.join("Pictures"));
            protected.push(h.join("Music"));
        }
        if let Ok(pf) = std::env::var("ProgramFiles") {
            protected.push(PathBuf::from(pf));
        }
        if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
            protected.push(PathBuf::from(pf86));
        }
        if let Ok(sys) = std::env::var("SystemRoot") {
            protected.push(PathBuf::from(sys));
        }
    }

    #[cfg(not(target_os = "windows"))]
    {
        // Always protect Linux system directories
        for sys_dir in [
            "/", "/etc", "/usr", "/var", "/bin", "/sbin", "/lib", "/lib64", "/boot", "/proc", "/sys", "/dev", "/run",
            "/tmp",
        ] {
            protected.push(PathBuf::from(sys_dir));
        }
        if let Ok(home) = std::env::var("HOME") {
            let h = PathBuf::from(home);
            protected.push(h.clone());
            protected.push(h.join("Desktop"));
            protected.push(h.join("Documents"));
            protected.push(h.join("Downloads"));
            protected.push(h.join("Videos"));
            protected.push(h.join("Pictures"));
            protected.push(h.join("Music"));
        }
    }

    for p in protected {
        if let Ok(c) = p.canonicalize() {
            if canonical == c {
                return true;
            }
        }
    }

    false
}

#[tauri::command]
pub fn delete_game_folder(app: AppHandle, game_id: String) -> Result<(), String> {
    let games = get_games(app);
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let exe_path_str = game.exe_path.unwrap_or_default();
    let exe_path = Path::new(&exe_path_str);
    if let Some(parent) = exe_path.parent() {
        if is_protected_dir(parent) {
            return Err("Cannot delete protected system or root directory".to_string());
        }
        if parent.exists() {
            fs::remove_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    Ok(())
}


#[tauri::command]
pub fn run_uninstaller(uninstaller_path: String) -> Result<(), String> {
    std::process::Command::new(uninstaller_path)
        .spawn()
        .map_err(|e| e.to_string())?;
    Ok(())
}

#[tauri::command]
pub fn open_logs_folder(app: AppHandle) -> Result<(), String> {
    let logs_dir = app
        .path()
        .app_log_dir()
        .unwrap_or_else(|_| app.path().app_data_dir().unwrap().join("logs"));
    if let Err(e) = fs::create_dir_all(&logs_dir) {
        return Err(format!("Failed to create logs folder: {}", e));
    }
    #[cfg(target_os = "windows")]
    {
        std::process::Command::new("explorer.exe")
            .arg(&logs_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(not(target_os = "windows"))]
    {
        std::process::Command::new("xdg-open")
            .arg(&logs_dir)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
pub async fn backup_library(app: AppHandle) -> Result<String, String> {
    let file_path = app
        .dialog()
        .file()
        .add_filter("Zip Archive", &["zip"])
        .set_file_name("silo_backup.zip")
        .blocking_save_file();
    if let Some(dest_path) = file_path {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|_| "Failed to get AppData directory")?;
        log::info!(
            "Starting library backup: app_data_dir={:?}, dest={:?}",
            app_data_dir,
            dest_path
        );
        let result = zip_dir_filtered(
            &app_data_dir,
            std::path::Path::new(&dest_path.to_string()),
            is_nested_archive,
        );
        match &result {
            Ok(()) => log::info!("Library backup completed successfully"),
            Err(e) => log::error!("Library backup failed: {}", e),
        }
        result?;
        return Ok(dest_path.to_string());
    }
    Err("Backup cancelled".into())
}

#[tauri::command]
pub async fn restore_library(app: AppHandle) -> Result<String, String> {
    let file_path = app
        .dialog()
        .file()
        .add_filter("Zip Archive", &["zip"])
        .blocking_pick_file();
    if let Some(src_path) = file_path {
        let app_data_dir = app
            .path()
            .app_data_dir()
            .map_err(|_| "Failed to get AppData directory")?;
        log::info!("Starting library restore: src={:?}, dest={:?}", src_path, app_data_dir);
        let result = unzip_file(std::path::Path::new(&src_path.to_string()), &app_data_dir);
        match &result {
            Ok(()) => log::info!("Library restore completed successfully"),
            Err(e) => log::error!("Library restore failed: {}", e),
        }
        result?;
        return Ok("Library restored successfully".into());
    }
    Err("Restore cancelled".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TempDir(PathBuf);

    impl TempDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!(
                "silo_cmd_test_{}_{}_{}",
                tag,
                std::process::id(),
                std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            ));
            fs::create_dir_all(&dir).unwrap();
            TempDir(dir)
        }

        fn path(&self) -> &Path {
            &self.0
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn relative_paths(root: &Path) -> Vec<String> {
        let mut paths: Vec<String> = WalkDir::new(root)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.path()
                    .strip_prefix(root)
                    .ok()
                    .filter(|p| !p.as_os_str().is_empty())
                    .map(|p| p.to_string_lossy().replace('\\', "/"))
            })
            .collect();
        paths.sort();
        paths
    }

    // ----- parse_backup_snapshot -----

    #[test]
    fn parse_backup_snapshot_auto() {
        let snap = parse_backup_snapshot("auto_20240101_120000.zip").unwrap();
        assert_eq!(snap.name, "auto_20240101_120000.zip");
        assert_eq!(snap.timestamp, "2024-01-01 12:00:00");
        assert!(snap.is_auto);
        assert_eq!(snap.custom_name, None);
        assert_eq!(snap.size_bytes, 0);
    }

    #[test]
    fn parse_backup_snapshot_manual_urldecodes_custom_name() {
        let snap = parse_backup_snapshot("manual_20240101_120000_My%20Save.zip").unwrap();
        assert_eq!(snap.timestamp, "2024-01-01 12:00:00");
        assert!(!snap.is_auto);
        assert_eq!(snap.custom_name.as_deref(), Some("My Save"));
    }

    #[test]
    fn parse_backup_snapshot_manual_without_custom_name() {
        let snap = parse_backup_snapshot("manual_20240101_120000.zip").unwrap();
        assert_eq!(snap.timestamp, "2024-01-01 12:00:00");
        assert!(!snap.is_auto);
        assert_eq!(snap.custom_name, None);
    }

    #[test]
    fn parse_backup_snapshot_legacy() {
        let snap = parse_backup_snapshot("backup_20240101.zip").unwrap();
        assert_eq!(snap.name, "backup_20240101.zip");
        assert_eq!(snap.timestamp, "20240101");
        assert!(!snap.is_auto);
        assert_eq!(snap.custom_name, None);
    }

    #[test]
    fn parse_backup_snapshot_rejects_non_zip() {
        assert!(parse_backup_snapshot("auto_20240101_120000.txt").is_none());
        assert!(parse_backup_snapshot("backup_20240101").is_none());
        assert!(parse_backup_snapshot("").is_none());
    }

    #[test]
    fn parse_backup_snapshot_garbage_gets_unknown_time() {
        let snap = parse_backup_snapshot("random_thing.zip").unwrap();
        assert_eq!(snap.timestamp, "Unknown Time");
        assert!(!snap.is_auto);
        assert_eq!(snap.custom_name, None);
    }

    #[test]
    fn parse_backup_snapshot_auto_ignores_extra_part() {
        let snap = parse_backup_snapshot("auto_20240101_120000_Extra.zip").unwrap();
        assert!(snap.is_auto);
        assert_eq!(snap.timestamp, "2024-01-01 12:00:00");
        assert_eq!(snap.custom_name, None);
    }

    // ----- is_blacklisted_exe -----

    #[test]
    fn is_blacklisted_exe_matches_known_tools() {
        for name in [
            "unins000.exe",
            "vcredist_x64.exe",
            "ue4prereqsetup.exe",
            "battleye_launcher.exe",
            "setup.exe",
            "steamwebhelper.exe",
        ] {
            assert!(is_blacklisted_exe(name), "expected {} blacklisted", name);
        }
    }

    #[test]
    fn is_blacklisted_exe_allows_real_games() {
        for name in ["witcher3.exe", "RDR2.exe", "borderlands3.exe", ""] {
            assert!(!is_blacklisted_exe(name), "expected {} not blacklisted", name);
        }
    }

    // ----- score_exe_candidate -----

    #[test]
    fn score_prefers_matching_stem() {
        let a = score_exe_candidate("witcher3", "witcher3", 1, 100_000_000);
        let b = score_exe_candidate("witcher3", "somethingelse", 1, 100_000_000);
        assert!(a > b, "matching stem should score higher ({} vs {})", a, b);
    }

    #[test]
    fn score_prefers_shallower_depth() {
        let a = score_exe_candidate("game", "game", 1, 100_000_000);
        let b = score_exe_candidate("game", "game", 4, 100_000_000);
        assert!(a > b, "shallower depth should score higher ({} vs {})", a, b);
    }

    #[test]
    fn score_prefers_larger_files() {
        let a = score_exe_candidate("game", "game", 1, 200_000_000);
        let b = score_exe_candidate("game", "game", 1, 100_000_000);
        assert!(a > b, "larger file should score higher ({} vs {})", a, b);
    }

    #[test]
    fn score_deprioritizes_variant_suffixes() {
        let base = score_exe_candidate("game", "game", 1, 100_000_000);
        for suffix in ["_be", "_launcher", "_server"] {
            let variant = score_exe_candidate(&format!("game{}", suffix), "game", 1, 100_000_000);
            assert!(base > variant, "{} should score lower", suffix);
        }
    }

    // ----- apply_optional / normalize_save_path -----

    #[test]
    fn apply_optional_clears_on_null_sets_on_string() {
        let mut t = Some("keep".to_string());
        apply_optional(Some(&serde_json::Value::Null), &mut t);
        assert_eq!(t, None);

        apply_optional(Some(&serde_json::json!("v")), &mut t);
        assert_eq!(t.as_deref(), Some("v"));

        let mut untouched = Some("x".to_string());
        apply_optional(None, &mut untouched);
        assert_eq!(untouched.as_deref(), Some("x"));
    }

    #[test]
    fn normalize_save_path_accepts_existing_dir() {
        let tmp = TempDir::new("normsave");
        let res = normalize_save_path(Some(tmp.path().to_str().unwrap())).unwrap();
        let p = res.expect("existing dir should normalize");
        assert!(Path::new(&p).is_dir());
    }

    #[test]
    fn normalize_save_path_rejects_missing_or_file() {
        let tmp = TempDir::new("normsave2");
        let missing = tmp.path().join("nope");
        assert!(normalize_save_path(Some(missing.to_str().unwrap())).is_err());

        let file = tmp.path().join("f.txt");
        fs::write(&file, "x").unwrap();
        assert!(normalize_save_path(Some(file.to_str().unwrap())).is_err());

        assert_eq!(normalize_save_path(Some("   ")), Ok(None));
        assert_eq!(normalize_save_path(None), Ok(None));
    }

    // ----- zip_dir / unzip_file round trip -----

    #[test]
    fn zip_unzip_round_trip_preserves_tree() {
        let src = TempDir::new("roundtrip_src");
        let zip_out = TempDir::new("roundtrip_zip");
        let dst = TempDir::new("roundtrip_dst");

        fs::create_dir_all(src.path().join("sub/nested")).unwrap();
        fs::create_dir_all(src.path().join("empty")).unwrap();
        fs::write(src.path().join("root.txt"), "root file").unwrap();
        fs::write(src.path().join("sub/one.bin"), vec![0u8, 1, 2, 3]).unwrap();
        fs::write(src.path().join("sub/nested/two.txt"), "nested").unwrap();

        let zip_path = zip_out.path().join("out.zip");
        zip_dir(src.path(), &zip_path).unwrap();

        let dest = dst.path().join("extracted");
        unzip_file(&zip_path, &dest).unwrap();

        assert_eq!(relative_paths(src.path()), relative_paths(&dest));
        assert_eq!(fs::read(dest.join("root.txt")).unwrap(), b"root file");
        assert_eq!(fs::read(dest.join("sub/one.bin")).unwrap(), vec![0u8, 1, 2, 3]);
        assert_eq!(fs::read(dest.join("sub/nested/two.txt")).unwrap(), b"nested");
        assert!(dest.join("empty").is_dir());
    }

    // ----- zip-slip security -----

    fn write_zip_with_entry(zip_path: &Path, name: &str, content: &[u8]) {
        let file = File::create(zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        writer.start_file(name, zip::write::FileOptions::default()).unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn unzip_rejects_parent_dir_traversal() {
        let zip_dir = TempDir::new("zipslip_zip");
        let dst = TempDir::new("zipslip_dst");
        let zip_path = zip_dir.path().join("evil.zip");
        write_zip_with_entry(&zip_path, "../../evil.txt", b"pwned");

        let err = unzip_file(&zip_path, dst.path()).unwrap_err();
        assert!(err.contains("unsafe path"), "unexpected error: {}", err);
        assert!(relative_paths(dst.path()).is_empty(), "dest should be untouched");
    }

    #[test]
    fn unzip_rejects_backslash_traversal() {
        let zip_dir = TempDir::new("zipslip_zip2");
        let dst = TempDir::new("zipslip_dst2");
        let zip_path = zip_dir.path().join("evil2.zip");
        write_zip_with_entry(&zip_path, "..\\evil.txt", b"pwned");

        assert!(unzip_file(&zip_path, dst.path()).is_err());
        assert!(relative_paths(dst.path()).is_empty());
    }

    #[test]
    fn unzip_strips_single_common_root_folder() {
        let zip_tmp = TempDir::new("unzip_root_zip");
        let dst = TempDir::new("unzip_root_dst");
        let zip_path = zip_tmp.path().join("nested.zip");

        let file = File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::FileOptions::default();

        writer.add_directory("DeathStranding2/", opts).unwrap();
        writer.start_file("DeathStranding2/savegame.dat", opts).unwrap();
        writer.write_all(b"save_data_123").unwrap();
        writer.start_file("DeathStranding2/sub/config.ini", opts).unwrap();
        writer.write_all(b"res=1080").unwrap();
        writer.finish().unwrap();

        unzip_file(&zip_path, dst.path()).unwrap();

        assert_eq!(
            relative_paths(dst.path()),
            vec!["savegame.dat", "sub", "sub/config.ini"]
        );
        assert_eq!(fs::read(dst.path().join("savegame.dat")).unwrap(), b"save_data_123");
        assert_eq!(fs::read(dst.path().join("sub/config.ini")).unwrap(), b"res=1080");
    }

    #[test]
    fn unzip_does_not_strip_when_top_level_has_mixed_files_and_folders() {
        let zip_tmp = TempDir::new("unzip_mixed_zip");
        let dst = TempDir::new("unzip_mixed_dst");
        let zip_path = zip_tmp.path().join("mixed.zip");

        let file = File::create(&zip_path).unwrap();
        let mut writer = zip::ZipWriter::new(file);
        let opts = zip::write::FileOptions::default();

        writer.start_file("root_save.dat", opts).unwrap();
        writer.write_all(b"root").unwrap();
        writer.start_file("sub/other.dat", opts).unwrap();
        writer.write_all(b"other").unwrap();
        writer.finish().unwrap();

        unzip_file(&zip_path, dst.path()).unwrap();

        assert_eq!(
            relative_paths(dst.path()),
            vec!["root_save.dat", "sub", "sub/other.dat"]
        );
    }

    #[test]
    fn save_detection_needed_without_path() {
        // No known folder -> always watch while the game runs.
        assert!(save_detection_needed(None));
        assert!(save_detection_needed(Some("")));
    }

    #[test]
    fn save_detection_needed_when_stored_folder_vanished() {
        let tmp = TempDir::new("savedetect");
        let dir = tmp.path().join("SaveGame");
        fs::create_dir_all(&dir).unwrap();

        // Folder exists -> no need to watch; plain exit backup covers it.
        assert!(!save_detection_needed(Some(&dir.to_string_lossy())));

        fs::remove_dir_all(&dir).unwrap();
        // Folder gone (game moved / reinstalled / wiped) -> must re-detect.
        assert!(save_detection_needed(Some(&dir.to_string_lossy())));
    }

    // ----- find_real_game_exe (launcher-aware "go straight to the game") -----

    /// Sparse file of the given length: `set_len` avoids writing real bytes, so
    /// the size thresholds these tests exercise cost nothing to set up.
    fn write_exe(path: &Path, size: u64) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::File::create(path).unwrap().set_len(size).unwrap();
    }

    // ----- library export / import (Settings -> Library Backup) -----

    #[test]
    fn library_export_then_import_restores_games_and_settings() {
        // An app-data dir shaped like the real one: library, settings, per-game
        // save snapshots, session history.
        let src = TempDir::new("lib_export");
        fs::create_dir_all(src.path().join("backups").join("game-1")).unwrap();
        fs::create_dir_all(src.path().join("saves").join("game-1")).unwrap();
        fs::create_dir_all(src.path().join("wallpapers")).unwrap();
        fs::write(
            src.path().join("games.json"),
            br#"[{"id":"game-1","name":"Cyberpunk 2077"}]"#,
        )
        .unwrap();
        fs::write(src.path().join("settings.json"), br#"{"showGameTitles":true}"#).unwrap();
        fs::write(src.path().join("sessions.json"), br#"[{"gameId":"game-1"}]"#).unwrap();
        fs::write(src.path().join("wallpapers").join("game-1.png"), b"png").unwrap();
        fs::write(
            src.path()
                .join("backups")
                .join("game-1")
                .join("auto_20240101_120000.zip"),
            b"snapshot",
        )
        .unwrap();
        fs::write(src.path().join("saves").join("game-1").join("notes.txt"), b"keep me").unwrap();

        let zip_path = TempDir::new("lib_export_zip");
        let archive_path = zip_path.path().join("silo_backup.zip");
        zip_dir_filtered(src.path(), &archive_path, is_nested_archive).unwrap();

        // Importing is exactly `restore_library`: unzip into a (fresh) data dir.
        let restored = TempDir::new("lib_import");
        unzip_file(&archive_path, restored.path()).unwrap();

        assert_eq!(
            fs::read_to_string(restored.path().join("games.json")).unwrap(),
            r#"[{"id":"game-1","name":"Cyberpunk 2077"}]"#
        );
        assert_eq!(
            fs::read_to_string(restored.path().join("settings.json")).unwrap(),
            r#"{"showGameTitles":true}"#
        );
        assert_eq!(
            fs::read_to_string(restored.path().join("sessions.json")).unwrap(),
            r#"[{"gameId":"game-1"}]"#
        );
        assert!(restored.path().join("wallpapers").join("game-1.png").exists());
        assert_eq!(
            fs::read_to_string(restored.path().join("saves").join("game-1").join("notes.txt")).unwrap(),
            "keep me"
        );
        // Save snapshots are deliberately left out of the library bundle.
        assert!(!restored
            .path()
            .join("backups")
            .join("game-1")
            .join("auto_20240101_120000.zip")
            .exists());
        assert!(is_nested_archive(Path::new("backups/game-1/auto_20240101_120000.zip")));
        assert!(is_nested_archive(Path::new("saves/game-1/auto_20240101_120000.ZIP")));
        assert!(!is_nested_archive(Path::new("games.json")));
        assert!(!is_nested_archive(Path::new("saves/game-1/notes.txt")));
    }

    #[test]
    fn empty_library_export_fails_loudly_instead_of_writing_an_empty_zip() {
        let src = TempDir::new("lib_export_empty");
        let archive = TempDir::new("lib_export_empty_zip");
        let err = zip_dir_filtered(src.path(), &archive.path().join("empty.zip"), is_nested_archive).unwrap_err();
        assert!(err.contains("No files could be read"), "unexpected error: {err}");
    }

    #[test]
    fn save_path_names_game_spots_broad_or_unrelated_folders() {
        let keys = save_name_keys(
            "Cyberpunk 2077",
            Path::new("C:\\Games\\Cyberpunk 2077\\bin\\x64\\Cyberpunk2077.exe"),
        );
        // The game's own folder, and a publisher folder that happens to be the
        // same depth — only the first is the game's.
        assert!(save_path_names_game(
            "C:\\Users\\Test\\Saved Games\\CD Projekt Red\\Cyberpunk 2077",
            &keys
        ));
        assert!(!save_path_names_game(
            "C:\\Users\\Test\\Saved Games\\CD Projekt Red",
            &keys
        ));
        assert!(!save_path_names_game("C:\\Users\\Test\\AppData\\Local\\Temp", &keys));
        assert!(!save_path_names_game("C:\\Users\\Test\\Documents\\Other Game", &keys));
    }

    #[test]
    fn save_name_keys_cover_name_exe_stem_and_install_folder() {
        let keys = save_name_keys(
            "Cyberpunk 2077",
            Path::new("C:\\Games\\Cyberpunk 2077\\bin\\x64\\Cyberpunk2077.exe"),
        );
        assert!(keys.iter().any(|k| crate::saveguard::name_matches("Cyberpunk 2077", k)));
        assert!(crate::saveguard::name_matches("Cyberpunk2077", &keys[1]));
        // The install folder is picked up too, so a game launched through its
        // launcher (whose own name is noise) still matches by folder.
        assert!(keys
            .iter()
            .any(|k| crate::saveguard::normalize_key(k) == "cyberpunk2077"));
        // A launcher's own name never becomes a key.
        let launcher_keys = save_name_keys(
            "Cyberpunk 2077",
            Path::new("C:\\Games\\Cyberpunk 2077\\REDprelauncher.exe"),
        );
        assert!(!launcher_keys.iter().any(|k| k.to_lowercase().contains("prelauncher")));
    }

    #[test]
    fn find_real_game_exe_finds_the_game_nested_below_a_prelauncher() {
        // The Cyberpunk 2077 layout: REDprelauncher.exe at the install root and
        // the actual game two levels down in bin\x64.
        let tmp = TempDir::new("launcher_nested");
        let install = tmp.path().join("Cyberpunk 2077");
        let prelauncher = install.join("REDprelauncher.exe");
        write_exe(&prelauncher, 1_024);
        let game = install.join("bin").join("x64").join("Cyberpunk2077.exe");
        write_exe(&game, 30 * 1024 * 1024);
        // Decoys that must never win: a bigger crash handler and a bigger
        // redistributable, both deeper than the game.
        write_exe(
            &install.join("bin").join("x64").join("CrashReporter.exe"),
            40 * 1024 * 1024,
        );
        write_exe(&install.join("redist").join("vc_redist.x64.exe"), 45 * 1024 * 1024);

        assert_eq!(find_real_game_exe(&prelauncher), Some(game));
    }

    #[test]
    fn find_real_game_exe_prefers_a_sibling_over_a_deeper_binary() {
        let tmp = TempDir::new("launcher_sibling");
        let install = tmp.path().join("Some Game");
        let launcher = install.join("SomeGameLauncher.exe");
        write_exe(&launcher, 1_024);
        let game = install.join("SomeGame.exe");
        write_exe(&game, 25 * 1024 * 1024);
        let helper = install.join("sub").join("SomeGameTool.exe");
        write_exe(&helper, 60 * 1024 * 1024);

        // The shallower, name-related binary wins over the bigger helper.
        assert_eq!(find_real_game_exe(&launcher), Some(game));
    }

    #[test]
    fn find_real_game_exe_falls_back_to_the_biggest_binary_without_a_name_match() {
        let tmp = TempDir::new("launcher_fallback");
        let install = tmp.path().join("install");
        let launcher = install.join("ThingLauncher.exe");
        write_exe(&launcher, 1_024);
        let game = install.join("bin").join("TotallyDifferentName.exe");
        write_exe(&game, 40 * 1024 * 1024);

        assert_eq!(find_real_game_exe(&launcher), Some(game));
    }

    #[test]
    fn find_real_game_exe_returns_none_for_a_normal_game_exe() {
        // Not launcher-like: nothing to redirect, so "auto" must keep using it.
        let tmp = TempDir::new("launcher_plain");
        let game = tmp.path().join("SomeGame.exe");
        write_exe(&game, 25 * 1024 * 1024);

        assert_eq!(find_real_game_exe(&game), None);
    }

    #[test]
    fn find_real_game_exe_ignores_redistributables_only() {
        let tmp = TempDir::new("launcher_redist");
        let install = tmp.path().join("install");
        let launcher = install.join("GameLauncher.exe");
        write_exe(&launcher, 1_024);
        write_exe(&install.join("redist").join("huge_setup.exe"), 45 * 1024 * 1024);
        write_exe(&install.join("Support").join("CrashReporter.exe"), 45 * 1024 * 1024);

        assert_eq!(find_real_game_exe(&launcher), None);
    }

    // ----- install_status_for (install-status classification) -----

    fn install_job_for(exe: Option<PathBuf>, launch_target: &str) -> InstallJob {
        InstallJob {
            game_id: "g1".to_string(),
            exe,
            launch_target: launch_target.to_string(),
        }
    }

    #[test]
    fn install_status_without_a_stored_path_is_installed() {
        // Nothing to verify, so SILO must never nag about it.
        assert_eq!(install_status_for(&install_job_for(None, "auto")), INSTALL_INSTALLED);
    }

    #[test]
    fn install_status_existing_exe_is_installed() {
        let tmp = TempDir::new("install_present");
        let exe = tmp.path().join("Game.exe");
        write_exe(&exe, 1024);
        assert_eq!(
            install_status_for(&install_job_for(Some(exe), "auto")),
            INSTALL_INSTALLED
        );
    }

    #[test]
    fn install_status_missing_exe_on_a_live_drive_is_missing() {
        // A path that does not exist while its drive/root is still reachable.
        let tmp = TempDir::new("install_missing");
        let exe = tmp.path().join("Gone").join("gone.exe");
        assert_eq!(install_status_for(&install_job_for(Some(exe), "auto")), INSTALL_MISSING);
    }

    #[test]
    fn install_status_absent_exe_on_a_dead_drive_is_unavailable() {
        // Windows: a rooted path with no drive prefix makes drive_available()
        // probe the path itself, which never exists. Unix: a path under an
        // automount root whose volume is not mounted.
        #[cfg(target_os = "windows")]
        let exe = PathBuf::from(r"\SiloNoSuchVolume\gone.exe");
        #[cfg(not(target_os = "windows"))]
        let exe = PathBuf::from("/mnt/__silo_no_such_volume__/gone.exe");
        assert_eq!(
            install_status_for(&install_job_for(Some(exe), "auto")),
            INSTALL_UNAVAILABLE
        );
    }

    // ----- restoring onto this machine -----

    #[cfg(target_os = "windows")]
    #[test]
    fn split_at_profile_keeps_the_profile_and_what_is_below_it() {
        let (prefix, tail) = split_at_profile(Path::new(r"C:\Users\Old\Documents\My Games\Foo")).unwrap();
        assert_eq!(prefix, PathBuf::from(r"C:\Users\Old"));
        assert_eq!(tail, PathBuf::from(r"Documents\My Games\Foo"));
    }

    #[cfg(not(target_os = "windows"))]
    #[test]
    fn split_at_profile_keeps_the_profile_and_what_is_below_it() {
        let (prefix, tail) = split_at_profile(Path::new("/home/old/Documents/My Games/Foo")).unwrap();
        assert_eq!(prefix, PathBuf::from("/home/old"));
        assert_eq!(tail, PathBuf::from("Documents/My Games/Foo"));
    }

    #[test]
    fn split_at_profile_ignores_paths_without_a_profile() {
        #[cfg(target_os = "windows")]
        let path = Path::new(r"D:\Games\Saves\Foo");
        #[cfg(not(target_os = "windows"))]
        let path = Path::new("/srv/saves/Foo");
        assert!(split_at_profile(path).is_none());
    }

    #[test]
    fn remap_moves_a_path_onto_this_accounts_profile() {
        #[cfg(target_os = "windows")]
        let (stored, profile, expected) = (
            Path::new(r"C:\Users\Old\Documents\My Games\Foo"),
            Path::new(r"C:\Users\New"),
            PathBuf::from(r"C:\Users\New\Documents\My Games\Foo"),
        );
        #[cfg(not(target_os = "windows"))]
        let (stored, profile, expected) = (
            Path::new("/home/old/Documents/My Games/Foo"),
            Path::new("/home/new"),
            PathBuf::from("/home/new/Documents/My Games/Foo"),
        );
        assert_eq!(remap_onto_profile(stored, profile), Some(expected));
    }

    #[test]
    fn verdict_accepts_this_accounts_profile_before_it_exists() {
        // Same account name on a fresh machine: the standard location is still
        // the right place to put the saves back.
        let Some(profile) = current_profile_dir() else {
            return;
        };
        let path = profile.join("Documents").join("__silo_missing_folder__");
        assert_eq!(
            stored_path_verdict(&path, &current_profile_name()),
            StoredSavePath::Usable
        );
    }

    #[test]
    fn verdict_flags_another_account_whose_folder_is_absent() {
        let Some(profile) = current_profile_dir() else {
            return;
        };
        let Some(parent) = profile.parent() else {
            return;
        };
        let path = parent.join("__silo_no_such_account__").join("Documents").join("Saves");
        if split_at_profile(&path).is_none() {
            return; // e.g. HOME=/root: no profile-shaped parent to test against
        }
        assert_eq!(
            stored_path_verdict(&path, &current_profile_name()),
            StoredSavePath::ForeignProfile("__silo_no_such_account__".to_string())
        );
    }

    #[test]
    fn verdict_accepts_another_account_whose_folder_really_exists() {
        // A shared profile, or a second account that really is on this machine,
        // stays usable: only an *absent* profile folder marks a path as having
        // come from somewhere else.
        let Some(profile) = current_profile_dir() else {
            return;
        };
        let path = profile.join("Documents").join("__silo_probe__");
        assert_eq!(stored_path_verdict(&path, "some_other_account"), StoredSavePath::Usable);
    }

    #[test]
    fn verdict_rejects_a_relative_path() {
        assert_eq!(
            stored_path_verdict(Path::new("Saves/Foo"), "me"),
            StoredSavePath::NotAbsolute
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn verdict_reports_a_drive_this_machine_does_not_have() {
        // Pick a letter that is genuinely absent so the assertion means something.
        let Some(root) = ('D'..='Z')
            .map(|letter| format!("{letter}:\\"))
            .find(|root| !Path::new(root).exists())
        else {
            return;
        };
        let path = PathBuf::from(root).join("Games").join("Saves");
        assert_eq!(stored_path_verdict(&path, "me"), StoredSavePath::RootUnavailable);
    }

    /// A minimal `Game`, built through serde so it cannot drift from the struct.
    fn restore_test_game(name: &str) -> Game {
        serde_json::from_value(serde_json::json!({ "id": "g1", "name": name }))
            .expect("a minimal Game should deserialize")
    }

    /// The current profile and its parent (`C:\Users` or `/home`), when the
    /// machine reports them.
    fn profile_and_parent() -> Option<(PathBuf, PathBuf)> {
        let profile = current_profile_dir()?;
        let parent = profile.parent()?.to_path_buf();
        Some((profile, parent))
    }

    #[test]
    fn resolve_restore_target_refuses_a_folder_from_another_machine() {
        // The reported bug: a library imported on a new PC still remembers the old
        // account's save folder, and a restore used to create that folder and unpack
        // the saves into a place the game will never read.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_no_such_game__");
        let game = restore_test_game("__silo_no_such_game__");

        let error = resolve_restore_target(&game, foreign.to_str().unwrap()).unwrap_err();
        assert!(error.starts_with("Cannot restore yet"), "unexpected error: {error}");
        assert!(
            error.contains("__silo_no_such_account__"),
            "the error should name the foreign account: {error}"
        );
        // The whole point: it must not resurrect the other machine's folder tree.
        assert!(!foreign.exists(), "the foreign save folder must never be created");
    }

    #[test]
    fn resolve_restore_target_keeps_a_folder_this_account_owns() {
        let Some((profile, _parent)) = profile_and_parent() else {
            return;
        };
        let stored = profile.join("Documents").join("__silo_probe__");
        let game = restore_test_game("__silo_no_such_game__");

        let target = resolve_restore_target(&game, stored.to_str().unwrap())
            .expect("a folder under this account's profile is usable as-is");
        assert_eq!(target.path, stored);
        assert!(!target.rediscovered);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn resolve_restore_target_remaps_onto_this_account_when_the_folder_is_there() {
        // A path from another account is accepted only when the same layout already
        // exists here, so the rewrite is evidence rather than a guess. The rewritten
        // location is this account's Temp folder: the only directory created by this
        // test lives inside it, and the guard removes it even if an assertion fails.
        let Some((profile, parent)) = profile_and_parent() else {
            return;
        };
        let leaf = format!("silo_remap_probe_{}", std::process::id());
        let tail = PathBuf::from("AppData").join("Local").join("Temp").join(&leaf);
        let remapped = profile.join(&tail);
        let _guard = TempDir(remapped.clone());
        fs::create_dir_all(&remapped).unwrap();

        let foreign = parent.join("__silo_no_such_account__").join(&tail);
        let game = restore_test_game("__silo_no_such_game__");
        let target = resolve_restore_target(&game, foreign.to_str().unwrap()).unwrap();

        assert_eq!(target.path, remapped);
        assert!(target.rediscovered);
        assert!(!foreign.exists(), "the foreign save folder must never be created");
    }

    // ----- relinking a library that came from another PC -----

    /// A `Game` carrying just what the survey reads.
    fn survey_game(id: &str, name: &str, save_path: Option<&str>) -> Game {
        let mut value = serde_json::json!({ "id": id, "name": name });
        if let Some(path) = save_path {
            value["savePath"] = serde_json::json!(path);
        }
        serde_json::from_value(value).expect("a minimal Game should deserialize")
    }

    /// A path on a volume this machine really does not have mounted, so the
    /// verdict has to name the drive rather than the folder.
    fn path_on_a_missing_drive() -> Option<String> {
        #[cfg(target_os = "windows")]
        {
            (b'D'..=b'Z')
                .map(|letter| format!("{}:\\", letter as char))
                .find(|root| !Path::new(root).exists())
                .map(|root| format!("{root}__silo_no_such_drive__\\Saves\\__silo_no_such_game__"))
        }
        #[cfg(not(target_os = "windows"))]
        {
            Some("/mnt/__silo_no_such_volume__/Saves/__silo_no_such_game__".to_string())
        }
    }

    /// A rooted path on a volume this machine does not have.
    fn path_on_a_missing_volume() -> &'static str {
        #[cfg(target_os = "windows")]
        let path = r"\SiloNoSuchVolume\Saves\__silo_no_such_game__";
        #[cfg(not(target_os = "windows"))]
        let path = "/mnt/__silo_no_such_volume__/Saves/__silo_no_such_game__";
        path
    }

    #[test]
    fn survey_has_nothing_to_do_for_a_library_recorded_here() {
        let Some((profile, _parent)) = profile_and_parent() else {
            return;
        };
        let stored = profile.join("Documents").join("__silo_probe__");
        let games = vec![
            survey_game("g1", "Own Folder", Some(stored.to_str().unwrap())),
            survey_game("g2", "Never Launched", None),
        ];

        let survey = survey_save_folders_of(&games);
        assert_eq!(survey.games_with_save_folder, 1, "only g1 records a save folder");
        assert_eq!(survey.usable, 1);
        assert!(survey.relinkable.is_empty());
        assert!(survey.unresolved.is_empty());
    }

    #[test]
    fn survey_never_calls_another_accounts_folder_usable() {
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_no_such_game__");
        let games = vec![survey_game(
            "g1",
            "__silo_no_such_game__",
            Some(foreign.to_str().unwrap()),
        )];

        let survey = survey_save_folders_of(&games);
        assert_eq!(
            survey.usable, 0,
            "a folder left by another PC must never count as usable"
        );
        assert_eq!(survey.games_with_save_folder, 1);
        let issue = survey
            .relinkable
            .first()
            .or_else(|| survey.unresolved.first())
            .expect("the game has to be reported one way or the other");
        assert_eq!(issue.game_id, "g1");
        assert_eq!(issue.recorded_path, foreign.to_str().unwrap());
        assert!(!issue.reason.is_empty(), "an issue has to explain itself");
        assert!(
            !foreign.exists(),
            "surveying must never create the other machine's folder"
        );
    }

    #[test]
    fn survey_places_each_game_in_exactly_one_bucket() {
        let Some((profile, parent)) = profile_and_parent() else {
            return;
        };
        let mine = profile.join("Documents").join("__silo_probe__");
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_no_such_game__");
        let dead = path_on_a_missing_volume();
        let games = vec![
            survey_game("g1", "Own Folder", Some(mine.to_str().unwrap())),
            survey_game("g2", "__silo_no_such_game__", Some(foreign.to_str().unwrap())),
            survey_game("g3", "__silo_no_such_game__", Some(dead)),
            survey_game("g4", "Never Launched", None),
        ];

        let survey = survey_save_folders_of(&games);
        assert_eq!(survey.games_with_save_folder, 3, "g4 records no save folder at all");
        assert_eq!(survey.usable, 1, "only the folder under this account works here");
        assert_eq!(survey.relinkable.len() + survey.unresolved.len(), 2);
        // A game the survey cannot place has to say so rather than be dropped, and
        // nothing may be invented for a game that records no folder.
        assert!(survey
            .relinkable
            .iter()
            .chain(survey.unresolved.iter())
            .all(|issue| !issue.reason.is_empty()));
        assert!(survey.relinkable.iter().all(|issue| issue.found_path.is_some()));
        assert!(survey.unresolved.iter().all(|issue| issue.found_path.is_none()));
        assert!(!survey.unresolved.iter().any(|issue| issue.game_id == "g4"));
        assert!(!survey.relinkable.iter().any(|issue| issue.game_id == "g4"));
        let unplugged = survey
            .unresolved
            .iter()
            .find(|issue| issue.game_id == "g3")
            .expect("the volume that is not here cannot be resolved");
        assert_eq!(unplugged.recorded_path, dead);
    }

    #[test]
    fn survey_gives_the_same_answer_twice() {
        let Some((profile, _parent)) = profile_and_parent() else {
            return;
        };
        let mine = profile.join("Documents").join("__silo_probe__");
        let games = vec![
            survey_game("g1", "Own Folder", Some(mine.to_str().unwrap())),
            survey_game("g2", "__silo_no_such_game__", Some(path_on_a_missing_volume())),
            survey_game("g3", "Never Launched", None),
        ];

        let first = survey_save_folders_of(&games);
        let second = survey_save_folders_of(&games);
        assert_eq!(first.usable, second.usable);
        assert_eq!(first.games_with_save_folder, second.games_with_save_folder);
        assert_eq!(first.relinkable.len(), second.relinkable.len());
        assert_eq!(first.unresolved.len(), second.unresolved.len());
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn survey_relinks_onto_this_account_and_then_has_nothing_left_to_do() {
        let Some((profile, parent)) = profile_and_parent() else {
            return;
        };
        let leaf = format!("__silo_survey_probe_{}__", std::process::id());
        let tail = PathBuf::from("Documents").join(&leaf);
        let mirrored = profile.join(&tail);
        let _guard = TempDir(mirrored.clone());
        fs::create_dir_all(&mirrored).unwrap();

        let foreign = parent.join("__silo_no_such_account__").join(&tail);
        let mut games = vec![survey_game(
            "g1",
            "__silo_no_such_game__",
            Some(foreign.to_str().unwrap()),
        )];

        let survey = survey_save_folders_of(&games);
        assert_eq!(survey.usable, 0);
        assert_eq!(survey.relinkable.len(), 1, "the mirrored layout only exists here");
        assert_eq!(survey.relinkable[0].found_path.as_deref(), mirrored.to_str());
        assert!(survey.unresolved.is_empty());
        assert!(!foreign.exists(), "the other machine's folder must never be created");

        // Applying the proposal is a fixed point: re-surveying finds nothing left.
        games[0].save_path = survey.relinkable[0].found_path.clone();
        let after = survey_save_folders_of(&games);
        assert_eq!(after.usable, 1);
        assert!(
            after.relinkable.is_empty(),
            "a second survey must have nothing left to do"
        );
        assert!(after.unresolved.is_empty());
    }

    #[test]
    fn survey_sends_a_game_that_is_not_here_to_the_old_pc_for_its_saves() {
        // The library came from another PC and this game was never reinstalled:
        // neither its files nor its saves exist here, so there is nothing to
        // relink to and the user has to bring the saves across.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let old_pc = parent.join("__silo_no_such_account__");
        let foreign = old_pc.join("Documents").join("__silo_no_such_game__");
        let mut game = survey_game("g1", "__silo_no_such_game__", Some(foreign.to_str().unwrap()));
        game.exe_path = Some(
            old_pc
                .join("Games")
                .join("__silo_no_such_game__.exe")
                .to_string_lossy()
                .into_owned(),
        );

        let survey = survey_save_folders_of(&[game]);
        assert_eq!(survey.usable, 0);
        assert_eq!(survey.relinkable.len() + survey.unresolved.len(), 1);
        let issue = survey
            .unresolved
            .first()
            .or_else(|| survey.relinkable.first())
            .expect("the game has to be reported");
        assert!(!issue.install_present, "its executable is not on this machine");
        assert!(
            !issue.reason.contains("Launch the game once"),
            "a game that is not installed here cannot be launched to re-find anything: {}",
            issue.reason
        );
        assert!(
            issue.reason.contains("other PC"),
            "the reason has to say where the saves actually are: {}",
            issue.reason
        );
        assert!(!foreign.exists(), "surveying must never create the other PC's folder");
    }

    #[test]
    fn survey_tells_a_game_that_is_here_to_launch_itself_once() {
        // The game is installed here and only its save folder moved, so letting
        // SaveGuard re-detect it really is the fastest route.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_no_such_game__");
        let mut game = survey_game("g1", "__silo_no_such_game__", Some(foreign.to_str().unwrap()));
        game.exe_path = std::env::current_exe()
            .ok()
            .map(|exe| exe.to_string_lossy().into_owned());

        let survey = survey_save_folders_of(&[game]);
        assert_eq!(survey.relinkable.len() + survey.unresolved.len(), 1);
        let issue = survey
            .relinkable
            .first()
            .or_else(|| survey.unresolved.first())
            .expect("the game has to be reported");
        assert!(issue.install_present, "the executable exists on this machine");
        assert!(
            issue.reason.contains("Launch the game once"),
            "the game is here, so launching it is the advice that applies: {}",
            issue.reason
        );
        assert!(
            !issue.reason.contains("other PC"),
            "only the folder moved, so the saves are not on another PC: {}",
            issue.reason
        );
    }

    #[test]
    fn survey_tells_a_game_on_an_unplugged_drive_to_reconnect_it() {
        // Same "this path does not work here", different cause: the saves are on
        // a drive that is not mounted, so bringing them from another PC is wrong.
        let Some((_profile, _parent)) = profile_and_parent() else {
            return;
        };
        let Some(dead) = path_on_a_missing_drive() else {
            return;
        };
        let game = survey_game("g1", "Unplugged", Some(dead.as_str()));

        let survey = survey_save_folders_of(&[game]);
        let issue = survey
            .relinkable
            .first()
            .or_else(|| survey.unresolved.first())
            .expect("the game has to be reported");
        assert!(!issue.install_present, "its executable is on the missing drive");
        assert!(
            issue.reason.contains("Reconnect the drive"),
            "the advice has to match the cause: {}",
            issue.reason
        );
        assert!(
            !issue.reason.contains("other PC"),
            "the saves are on that drive, not another PC: {}",
            issue.reason
        );
    }

    #[test]
    fn redetection_never_keys_on_the_old_machines_install_folder() {
        // Found on this machine while building this: the old PC's install folder
        // was `…\Games\`, so the key "games" matched unrelated folders here —
        // Epic Games, Rockstar Games, AppData\Roaming\game-save-manager — and a
        // game that is not on this machine at all was about to be relinked onto
        // one of them.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let mut gone = survey_game("g1", "Cyberpunk 2077", None);
        gone.exe_path = Some(
            parent
                .join("__silo_no_such_account__")
                .join("Games")
                .join("Cyberpunk2077.exe")
                .to_string_lossy()
                .into_owned(),
        );
        let keys = redetect_name_keys(&gone);
        assert_eq!(keys, vec!["Cyberpunk 2077".to_string()]);
        assert!(
            !keys.iter().any(|key| crate::saveguard::normalize_key(key) == "games"),
            "the old machine's folder name must never be matched here: {keys:?}"
        );

        // Once the executable is on this machine, its own folder is evidence too.
        let dir = TempDir::new("redetect");
        let exe = dir.path().join("Cyberpunk2077.exe");
        fs::write(&exe, b"").unwrap();
        let mut here = survey_game("g1", "Cyberpunk 2077", None);
        here.exe_path = Some(exe.to_string_lossy().into_owned());
        let keys = redetect_name_keys(&here);
        assert!(keys.iter().any(|key| key == "cyberpunk2077"), "{keys:?}");
        assert!(keys.len() > 2, "this machine's own layout is usable evidence: {keys:?}");
    }

    #[test]
    fn survey_serializes_the_field_names_the_renderer_reads() {
        // The modal reads camelCase keys straight off the IPC payload, and a
        // rename would quietly make every game look "not installed here".
        let survey = SaveFolderSurvey {
            games_with_save_folder: 1,
            usable: 0,
            relinkable: Vec::new(),
            unresolved: vec![SaveFolderIssue {
                game_id: "g1".to_string(),
                game_name: "Game".to_string(),
                recorded_path: "C:/old/saves".to_string(),
                found_path: None,
                confidence: Some(MatchConfidence::Guess),
                confidence_score: Some(MatchConfidence::Guess.score()),
                install_present: false,
                reason: "because".to_string(),
            }],
        };

        let json = serde_json::to_value(&survey).expect("the survey must serialize");
        assert_eq!(json["gamesWithSaveFolder"], 1);
        let issue = &json["unresolved"][0];
        assert_eq!(issue["gameId"], "g1");
        assert_eq!(issue["gameName"], "Game");
        assert_eq!(issue["recordedPath"], "C:/old/saves");
        assert_eq!(issue["installPresent"], false);
        assert!(
            issue.get("foundPath").is_none(),
            "a game with nowhere to point must not carry a found path"
        );
        // The category is used for safety policy; the numeric score is for UI
        // explanation only.
        assert_eq!(issue["confidence"], "guess");
        assert_eq!(issue["confidenceScore"], 25);
    }

    #[test]
    fn a_confirmed_match_serializes_its_confidence() {
        let survey = SaveFolderSurvey {
            games_with_save_folder: 1,
            usable: 0,
            relinkable: vec![SaveFolderIssue {
                game_id: "g1".to_string(),
                game_name: "Game".to_string(),
                recorded_path: "C:/old/saves".to_string(),
                found_path: Some("C:/new/saves".to_string()),
                confidence: Some(MatchConfidence::Confirmed),
                confidence_score: Some(MatchConfidence::Confirmed.score()),
                install_present: true,
                reason: "because".to_string(),
            }],
            unresolved: Vec::new(),
        };

        let json = serde_json::to_value(&survey).expect("the survey must serialize");
        assert_eq!(json["relinkable"][0]["confidence"], "confirmed");
        assert_eq!(json["relinkable"][0]["confidenceScore"], 90);
    }

    /// A recorded path and this machine's profile, in the flavour of the host.
    fn recorded_and_profile(leaf: &str) -> (String, &'static str) {
        #[cfg(target_os = "windows")]
        let (recorded, profile) = (format!(r"C:\Users\Old\Documents\My Games\{leaf}"), r"C:\Users\New");
        #[cfg(not(target_os = "windows"))]
        let (recorded, profile) = (format!("/home/old/Documents/My Games/{leaf}"), "/home/new");
        (recorded, profile)
    }

    /// The same recorded path written by the other operating system.
    fn recorded_on_the_other_os(leaf: &str) -> String {
        #[cfg(target_os = "windows")]
        let recorded = format!("/home/old/Documents/My Games/{leaf}");
        #[cfg(not(target_os = "windows"))]
        let recorded = format!(r"C:\Users\Old\Documents\My Games\{leaf}");
        recorded
    }

    #[test]
    fn import_destination_translates_this_machines_own_layout_only() {
        let (recorded, profile) = recorded_and_profile("X");
        let translated = import_destination(&recorded, Some(Path::new(profile)));
        assert_eq!(
            translated.as_deref(),
            remap_onto_profile(Path::new(&recorded), Path::new(profile)).as_deref(),
            "the destination is the recorded path on this account"
        );

        // A path from the other OS only *looks* like a profile path to this
        // parser; translating it would invent somewhere the game never looks.
        assert_eq!(
            import_destination(&recorded_on_the_other_os("X"), Some(Path::new(profile))),
            None
        );
        // Nothing to map a path without a profile in it onto, and no profile at
        // all means no destination either.
        #[cfg(target_os = "windows")]
        let no_profile = r"E:\Saves\X";
        #[cfg(not(target_os = "windows"))]
        let no_profile = "/mnt/old/Saves/X";
        assert_eq!(import_destination(no_profile, Some(Path::new(profile))), None);
        assert_eq!(import_destination(&recorded, None), None);
    }

    #[test]
    fn save_import_plan_takes_the_folder_the_user_picked() {
        let profile = TempDir::new("import_profile");
        let source = TempDir::new("import_source");
        let (recorded, _) = recorded_and_profile("__silo_no_such_game__");
        let found = source.path().join("__silo_no_such_game__");
        fs::create_dir_all(&found).unwrap();
        fs::write(found.join("save.dat"), b"progress").unwrap();
        let games = vec![survey_game("g1", "__silo_no_such_game__", Some(&recorded))];

        let plan = save_import_plan(&games, Some(profile.path()), Some(source.path()), &HashMap::new());

        assert_eq!(plan.ready.len(), 1, "still missing: {:?}", plan.still_missing);
        assert!(plan.still_missing.is_empty());
        let item = &plan.ready[0];
        assert_eq!(item.source, "folder");
        assert_eq!(Path::new(&item.source_path), found);
        assert_eq!(item.recorded_path, recorded);
        let target = profile
            .path()
            .join("Documents")
            .join("My Games")
            .join("__silo_no_such_game__");
        assert_eq!(Path::new(&item.target_path), target);
        assert!(!target.exists(), "planning must not create the folder it proposes");
    }

    #[test]
    fn save_import_plan_falls_back_to_the_games_own_backup() {
        let profile = TempDir::new("import_profile");
        let source = TempDir::new("import_source");
        let backups = TempDir::new("import_archives");
        let archive = backups.path().join("manual_20240101_120000.zip");
        fs::write(&archive, b"not a real zip, only the plan looks at it").unwrap();
        let (recorded, _) = recorded_and_profile("__silo_no_such_game__");
        let games = vec![survey_game("g1", "__silo_no_such_game__", Some(&recorded))];
        let mut newest_backup = HashMap::new();
        newest_backup.insert("g1".to_string(), archive.clone());

        // A picked folder that holds nothing for this game: the library's own
        // backup is what is left.
        let plan = save_import_plan(&games, Some(profile.path()), Some(source.path()), &newest_backup);
        assert_eq!(plan.ready.len(), 1, "still missing: {:?}", plan.still_missing);
        assert_eq!(plan.ready[0].source, "backup");
        assert_eq!(Path::new(&plan.ready[0].source_path), archive);

        // With neither a folder nor a backup there is nothing to bring across,
        // and the reason has to say so.
        let bare = save_import_plan(&games, Some(profile.path()), Some(source.path()), &HashMap::new());
        assert!(bare.ready.is_empty());
        assert_eq!(bare.still_missing.len(), 1);
        assert!(
            bare.still_missing[0]
                .reason
                .contains("No saves for it were found in the folder you picked"),
            "{}",
            bare.still_missing[0].reason
        );

        // Once the picked folder does hold this game, it is the live copy and
        // the older archive is not what gets restored over it.
        fs::create_dir_all(source.path().join("__silo_no_such_game__")).unwrap();
        fs::write(source.path().join("__silo_no_such_game__/save.dat"), b"now").unwrap();
        let live = save_import_plan(&games, Some(profile.path()), Some(source.path()), &newest_backup);
        assert_eq!(live.ready.len(), 1, "still missing: {:?}", live.still_missing);
        assert_eq!(live.ready[0].source, "folder");
        assert_eq!(
            Path::new(&live.ready[0].source_path),
            source.path().join("__silo_no_such_game__")
        );
    }

    #[test]
    fn save_import_plan_leaves_a_folder_that_already_holds_saves() {
        let profile = TempDir::new("import_profile");
        let (recorded, _) = recorded_and_profile("__silo_no_such_game__");
        let games = vec![survey_game("g1", "__silo_no_such_game__", Some(&recorded))];
        // The destination is already there with saves in it, which may well be
        // newer than anything being brought across.
        let target = profile
            .path()
            .join("Documents")
            .join("My Games")
            .join("__silo_no_such_game__");
        fs::create_dir_all(&target).unwrap();
        fs::write(target.join("newer.dat"), b"played here since").unwrap();

        let plan = save_import_plan(&games, Some(profile.path()), None, &HashMap::new());
        assert!(plan.ready.is_empty(), "nothing may be planned over it");
        assert_eq!(plan.still_missing.len(), 1);
        assert!(
            plan.still_missing[0].reason.contains("already holds saves"),
            "{}",
            plan.still_missing[0].reason
        );
    }

    #[test]
    fn save_import_plan_says_when_it_has_no_destination() {
        let profile = TempDir::new("import_profile");
        let source = TempDir::new("import_source");
        let leaf = "__silo_no_such_game__";
        let cross_os = recorded_on_the_other_os(leaf);
        // The saves are right there in the picked folder, but there is nowhere on
        // this machine that a path from the other OS translates to.
        let found = source.path().join(leaf);
        fs::create_dir_all(&found).unwrap();
        fs::write(found.join("save.dat"), b"progress").unwrap();
        let games = vec![survey_game("g1", leaf, Some(&cross_os))];

        let plan = save_import_plan(&games, Some(profile.path()), Some(source.path()), &HashMap::new());
        assert!(plan.ready.is_empty());
        assert_eq!(plan.still_missing.len(), 1);
        assert!(
            plan.still_missing[0]
                .reason
                .contains("has nowhere on this machine to put its saves"),
            "{}",
            plan.still_missing[0].reason
        );
        assert_eq!(plan.still_missing[0].game_id, "g1");
    }

    #[test]
    fn save_import_plan_skips_the_games_this_machine_already_handles() {
        let profile = TempDir::new("import_profile");
        let source = TempDir::new("import_source");
        let (recorded, _) = recorded_and_profile("__silo_missing__");
        // A folder under this account, so this game is already handled here.
        let mine = profile.path().join("__silo_usable__");
        fs::create_dir_all(&mine).unwrap();
        fs::write(mine.join("save.dat"), b"played here").unwrap();
        let games = vec![
            survey_game("g1", "__silo_usable__", Some(mine.to_str().unwrap())),
            // Nothing recorded to work from: the relink check's business, and it
            // would find the same folder this plan can only guess at.
            survey_game("g2", "__silo_never_launched__", None),
            survey_game("g3", "__silo_missing__", Some(&recorded)),
        ];

        let plan = save_import_plan(&games, Some(profile.path()), Some(source.path()), &HashMap::new());

        assert!(plan.ready.is_empty(), "nothing can be brought across");
        let missing: Vec<&str> = plan.still_missing.iter().map(|issue| issue.game_id.as_str()).collect();
        assert_eq!(
            missing,
            vec!["g3"],
            "only the game with nowhere to point is part of the import"
        );
    }

    #[test]
    fn bringing_a_folder_across_lands_it_in_place_and_leaves_nothing_behind() {
        let parent = TempDir::new("import_parent");
        let backups = TempDir::new("import_backups");
        let from = TempDir::new("import_from");
        fs::create_dir_all(from.path().join("nested")).unwrap();
        fs::write(from.path().join("save.dat"), b"progress").unwrap();
        fs::write(from.path().join("nested").join("slot2.dat"), b"more").unwrap();
        let destination = parent.path().join("Saves");

        stage_and_swap(&destination, backups.path(), "import", |staging| {
            copy_tree(from.path(), staging)
        })
        .unwrap();

        assert_eq!(fs::read(destination.join("save.dat")).unwrap(), b"progress");
        assert_eq!(fs::read(destination.join("nested").join("slot2.dat")).unwrap(), b"more");
        let leftovers: Vec<String> = fs::read_dir(parent.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|name| name.contains("_silo_import"))
            .collect();
        assert!(leftovers.is_empty(), "staging was left behind: {leftovers:?}");

        // A folder with nothing in it is not saves, and saying so beats pointing
        // the game at an empty folder.
        let empty = TempDir::new("import_empty");
        let error = copy_tree(empty.path(), &parent.path().join("Nowhere")).unwrap_err();
        assert!(error.contains("holds no files"), "{error}");
    }

    #[test]
    fn replacing_a_save_folder_keeps_what_was_there() {
        let parent = TempDir::new("swap_parent");
        let backups = TempDir::new("swap_backups");
        let fresh = TempDir::new("swap_fresh");
        fs::write(fresh.path().join("new.dat"), b"newer progress").unwrap();
        let destination = parent.path().join("Saves");
        fs::create_dir_all(&destination).unwrap();
        fs::write(destination.join("old.dat"), b"older progress").unwrap();

        stage_and_swap(&destination, backups.path(), "import", |staging| {
            copy_tree(fresh.path(), staging)
        })
        .unwrap();

        assert!(destination.join("new.dat").exists());
        assert!(!destination.join("old.dat").exists());
        let snapshots: Vec<String> = fs::read_dir(backups.path())
            .unwrap()
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(snapshots.len(), 1, "the replaced saves are kept: {snapshots:?}");
        assert!(snapshots[0].contains("Before%20import"), "{snapshots:?}");
        let extracted = TempDir::new("swap_extract");
        unzip_file(&backups.path().join(&snapshots[0]), extracted.path()).unwrap();
        assert_eq!(fs::read(extracted.path().join("old.dat")).unwrap(), b"older progress");
    }

    #[test]
    fn a_backup_named_with_a_time_outranks_one_without() {
        let dated = backup_recency_key("manual_20240101_120000_Slot%201.zip", None);
        let later = backup_recency_key("auto_20240301_090000.zip", None);
        let undated = backup_recency_key(
            "random_thing.zip",
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(9_999)),
        );
        // A name carrying a time is keyed by it; one carrying nothing usable is
        // keyed by the file's own time, in a form that still compares in order.
        assert_eq!(dated, (1, "2024-01-01 12:00:00".to_string()));
        assert_eq!(undated, (0, "00000000000009999000".to_string()));

        assert!(later > dated, "the later time in the name wins");
        assert!(dated > undated, "a name with no usable time never outranks a dated one");
        // Not even when the file itself was written long after the dated backup
        // was made: that is exactly what the Backup Manager's plain string sort
        // gets wrong.
        let written_lately = backup_recency_key(
            "random_thing.zip",
            Some(std::time::UNIX_EPOCH + std::time::Duration::from_secs(4_000_000_000)),
        );
        assert!(dated > written_lately, "the dated name is still the newer one");
        let older = backup_recency_key("random_thing.zip", Some(std::time::UNIX_EPOCH));
        assert!(undated > older, "two undated names still compare by file time");
    }

    #[test]
    fn import_plan_serializes_the_field_names_the_renderer_reads() {
        let plan = SaveImportPlan {
            ready: vec![SaveImportItem {
                game_id: "g1".to_string(),
                game_name: "Game".to_string(),
                source: "folder".to_string(),
                source_path: "E:/Old/Game".to_string(),
                target_path: "C:/Users/New/Documents/My Games/Game".to_string(),
                recorded_path: "C:/Users/Old/Documents/My Games/Game".to_string(),
            }],
            still_missing: Vec::new(),
        };

        let json = serde_json::to_value(&plan).expect("the plan must serialize");
        let item = &json["ready"][0];
        assert_eq!(item["gameId"], "g1");
        assert_eq!(item["gameName"], "Game");
        assert_eq!(item["source"], "folder");
        assert_eq!(item["sourcePath"], "E:/Old/Game");
        assert_eq!(item["targetPath"], "C:/Users/New/Documents/My Games/Game");
        assert_eq!(item["recordedPath"], "C:/Users/Old/Documents/My Games/Game");
        assert!(json["stillMissing"].is_array(), "{json}");
    }

    #[test]
    fn a_path_from_the_other_operating_system_is_named_as_such() {
        // One machine cannot run the other's path code, so the host is named
        // explicitly here and both directions are checked from either side.
        assert_eq!(
            foreign_os_shape(r"C:\Users\Old\Documents\My Games\X", OsFamily::Unix),
            Some(OsFamily::Windows)
        );
        assert_eq!(
            foreign_os_shape(r"\\OLDPC\Saves\X", OsFamily::Unix),
            Some(OsFamily::Windows)
        );
        assert_eq!(
            foreign_os_shape(r"Documents\My Games\X", OsFamily::Unix),
            Some(OsFamily::Windows)
        );
        assert_eq!(
            foreign_os_shape("C:/Users/Old/Documents/My Games/X", OsFamily::Unix),
            Some(OsFamily::Windows),
            "a Windows path written with forward slashes is still Windows"
        );
        assert_eq!(
            foreign_os_shape("/home/old/.local/share/X", OsFamily::Unix),
            None,
            "a path this OS could have written itself is not foreign"
        );
        assert_eq!(
            foreign_os_shape("/home/old/.local/share/X", OsFamily::Windows),
            Some(OsFamily::Unix)
        );
        assert_eq!(foreign_os_shape(r"C:\Users\Old\Documents\X", OsFamily::Windows), None);
        assert_eq!(
            foreign_os_shape(r"\SiloNoSuchVolume\Saves\X", OsFamily::Windows),
            None,
            "rooted without a drive is this machine's own bad path, not another OS"
        );
        assert_eq!(
            foreign_os_shape("//server/share/X", OsFamily::Windows),
            None,
            "a POSIX path that starts with two slashes is still POSIX"
        );
    }

    #[test]
    fn verdict_names_the_other_os_before_it_calls_a_path_relative() {
        // Without the OS check both of these land in `NotAbsolute`, which says
        // nothing about why they can never work here.
        assert_eq!(
            stored_path_verdict_of(Path::new(r"C:\Users\Old\Documents\My Games\X"), "me", OsFamily::Unix),
            StoredSavePath::ForeignOs(OsFamily::Windows)
        );
        assert_eq!(
            stored_path_verdict_of(Path::new("/home/old/.local/share/X"), "me", OsFamily::Windows),
            StoredSavePath::ForeignOs(OsFamily::Unix)
        );
        // A genuinely relative path stays relative, whichever host reads it.
        assert_eq!(
            stored_path_verdict_of(Path::new("Saves/Foo"), "me", OsFamily::Unix),
            StoredSavePath::NotAbsolute
        );
    }

    #[test]
    fn windows_tails_drop_the_drive_and_the_old_user() {
        assert_eq!(
            windows_path_tail(r"C:\Users\Old\Documents\My Games\X"),
            Some(PathBuf::from("Documents").join("My Games").join("X"))
        );
        assert_eq!(
            windows_path_tail(r"C:\Games\Saves\X"),
            Some(PathBuf::from("Games").join("Saves").join("X"))
        );
        assert_eq!(
            windows_path_tail(r"C:\Users\Old"),
            None,
            "the profile directory itself is not a save folder"
        );
        assert_eq!(windows_path_tail(r"C:\"), None);
    }

    #[test]
    fn wine_prefix_lookup_maps_a_windows_tree_onto_a_prefix() {
        // A Windows move onto Linux: the same tree, one level inside the prefix.
        let drive_c = TempDir::new("wine_drive_c");
        let saves = drive_c
            .path()
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("My Games")
            .join("Cyberpunk 2077");
        fs::create_dir_all(&saves).unwrap();
        let prefixes = [WinePrefix {
            drive_c: drive_c.path().to_path_buf(),
            app_id: Some("1091500".to_string()),
        }];
        let recorded = r"C:\Users\Old\Documents\My Games\Cyberpunk 2077";

        // The prefix in question is the one Steam names after the app id in the
        // recorded path, so the folder being there settles it.
        assert_eq!(
            find_in_wine_prefixes(&prefixes, recorded, Some("1091500")),
            Some((saves.clone(), MatchConfidence::Confirmed)),
            "the game's own prefix copy of the same tree is the folder to relink to"
        );
        // Asked without the identity, the very same folder is only an existence
        // match: any other game's prefix could have held it just as well, which is
        // why the relink refuses to act on it.
        assert_eq!(
            find_in_wine_prefixes(&prefixes, recorded, None),
            Some((saves.clone(), MatchConfidence::Likely))
        );
        assert_eq!(
            find_in_wine_prefixes(&prefixes, recorded, Some("292030")),
            Some((saves.clone(), MatchConfidence::Likely)),
            "the folder is there, but not in the prefix that belongs to this game"
        );
        assert_eq!(
            find_in_wine_prefixes(
                &prefixes,
                r"C:\Users\Old\Documents\My Games\Other Game",
                Some("1091500")
            ),
            None,
            "a folder the prefix does not have must not be invented"
        );
        assert_eq!(
            find_in_wine_prefixes(&[], recorded, Some("1091500")),
            None,
            "with no prefix on the machine there is nothing to map onto"
        );
    }

    #[test]
    fn restore_resolves_a_windows_folder_inside_a_prefix_on_this_machine() {
        // Windows → Linux: the saves are already here, one level inside the
        // game's prefix, so a restore has to use them rather than the recorded
        // path or a guess by name.
        let drive_c = TempDir::new("wine_restore");
        let saves = drive_c
            .path()
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("My Games")
            .join("__silo_no_such_game__");
        fs::create_dir_all(&saves).unwrap();
        // A prefix with no Steam identity: the game's own prefix this may well be,
        // but nothing here says so.
        let machine = MachinePaths {
            prefixes: vec![WinePrefix {
                drive_c: drive_c.path().to_path_buf(),
                app_id: None,
            }],
            steamapps: Vec::new(),
            search_dirs: Vec::new(),
        };
        let stored = r"C:\Users\Old\Documents\My Games\__silo_no_such_game__";
        let game = restore_test_game("__silo_no_such_game__");

        let target = resolve_restore_target_with(&game, stored, &machine)
            .expect("the prefix holds the folder, so the restore can resolve");
        assert_eq!(target.path, saves);
        assert!(target.rediscovered, "the recorded path is not the one in use");
        assert_eq!(
            target.confidence,
            MatchConfidence::Likely,
            "the folder is really here, but nothing ties the prefix to this game"
        );
        assert!(
            !target.confidence.is_enough_for_a_relink(),
            "an existence check is not evidence enough to rewrite a save path"
        );

        // With no prefix at all there is nothing on this machine to use, and the
        // refusal has to say so rather than pick a folder at random.
        let bare = MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: Vec::new(),
        };
        let missed = resolve_restore_target_with(&game, stored, &bare)
            .expect_err("nothing here matches, so the restore must refuse");
        assert!(missed.starts_with("Cannot restore yet"), "{missed}");
    }

    #[test]
    fn proton_prefixes_are_found_where_steam_and_wine_put_them() {
        // Only prefixes that are really on the machine count, so the known
        // layouts are listed and everything missing is skipped.
        let home = TempDir::new("wine_home");
        let data = home.path().join(".local/share");
        let steam_drive_c = data
            .join("Steam/steamapps/compatdata")
            .join("1091500")
            .join("pfx/drive_c");
        fs::create_dir_all(&steam_drive_c).unwrap();
        let flatpak_drive_c = home
            .path()
            .join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/compatdata")
            .join("292030")
            .join("pfx/drive_c");
        fs::create_dir_all(&flatpak_drive_c).unwrap();
        let wine_drive_c = home.path().join(".wine/drive_c");
        fs::create_dir_all(&wine_drive_c).unwrap();
        // A compatdata directory Steam made but Proton has not used yet.
        fs::create_dir_all(data.join("Steam/steamapps/compatdata/292030")).unwrap();

        let prefixes = wine_prefixes_in(home.path(), &data);
        let steam = WinePrefix {
            drive_c: steam_drive_c.clone(),
            app_id: Some("1091500".to_string()),
        };
        let flatpak = WinePrefix {
            drive_c: flatpak_drive_c.clone(),
            app_id: Some("292030".to_string()),
        };
        assert!(prefixes.contains(&steam), "the Steam prefix: {prefixes:?}");
        assert!(prefixes.contains(&flatpak), "the Flatpak one: {prefixes:?}");
        assert!(
            prefixes.contains(&WinePrefix {
                drive_c: wine_drive_c.clone(),
                app_id: None
            }),
            "and the plain Wine prefix, which belongs to no single game: {prefixes:?}"
        );
        assert_eq!(
            prefixes.len(),
            3,
            "a compatdata directory without a prefix is not one: {prefixes:?}"
        );

        // Discovery feeds the relink: the prefix the app id names is where a
        // Windows save folder recorded under Documents turns up on this machine,
        // and that identity is what makes the match evidence.
        let saves = steam_drive_c
            .join("users")
            .join("steamuser")
            .join("Documents")
            .join("My Games")
            .join("Cyberpunk 2077");
        fs::create_dir_all(&saves).unwrap();
        assert_eq!(
            find_in_wine_prefixes(
                &prefixes,
                r"C:\Users\Old\Documents\My Games\Cyberpunk 2077",
                Some("1091500")
            ),
            Some((saves, MatchConfidence::Confirmed))
        );

        // A compatdata directory whose name is not an app id still holds a prefix
        // worth looking in, but it names no game, so it can never be one's own.
        let odd = data
            .join("Steam")
            .join("steamapps")
            .join("compatdata")
            .join("not-an-app-id")
            .join("pfx")
            .join("drive_c");
        fs::create_dir_all(&odd).unwrap();
        let with_odd = wine_prefixes_in(home.path(), &data);
        assert!(
            with_odd.contains(&WinePrefix {
                drive_c: odd,
                app_id: None
            }),
            "the prefix is still found: {with_odd:?}"
        );

        // Nothing to find means nothing to look in.
        let empty = TempDir::new("wine_home_empty");
        assert!(wine_prefixes_in(empty.path(), &empty.path().join("data")).is_empty());
    }

    #[test]
    fn survey_explains_a_save_folder_recorded_on_the_other_operating_system() {
        // A PC move between Windows and Linux/macOS: the shape of the path gives
        // the cause away, and the advice has to match the direction it came from.
        let Some((_profile, _parent)) = profile_and_parent() else {
            return;
        };
        // Each direction gets its own advice: a Windows folder on Linux may well
        // be sitting in a prefix already, while a Unix folder on Windows is not
        // here at all.
        #[cfg(target_os = "windows")]
        let (recorded, other_os, advice) = (
            "/home/old/.local/share/__silo_no_such_game__",
            "a Linux or macOS machine",
            "Install the game here and launch it once",
        );
        #[cfg(not(target_os = "windows"))]
        let (recorded, other_os, advice) = (
            r"C:\Users\Old\Documents\My Games\__silo_no_such_game__",
            "a Windows machine",
            "through Proton or Wine its saves live inside",
        );

        let game = survey_game("g1", "__silo_no_such_game__", Some(recorded));
        let survey = survey_save_folders_of(&[game]);
        assert_eq!(survey.usable, 0, "the other system's path is never usable here");
        assert_eq!(survey.relinkable.len() + survey.unresolved.len(), 1);
        let issue = survey
            .relinkable
            .first()
            .or_else(|| survey.unresolved.first())
            .expect("the game has to be reported");
        assert!(
            issue.reason.contains(other_os),
            "the reason has to name the system it was recorded on: {}",
            issue.reason
        );
        assert!(
            !issue.reason.contains("does not exist on this machine"),
            "the cause is the other OS, not a missing folder: {}",
            issue.reason
        );
        assert!(
            issue.reason.contains(advice),
            "the advice has to fit the direction it came from: {}",
            issue.reason
        );
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn survey_never_relinks_a_game_onto_another_games_save_folder() {
        let Some((profile, parent)) = profile_and_parent() else {
            return;
        };
        let leaf = format!("__silo_shared_probe_{}__", std::process::id());
        let tail = PathBuf::from("Documents").join(&leaf);

        // One game saves here for real, in a folder that exists.
        let mine = profile.join(&tail);
        let _guard = TempDir(mine.clone());
        fs::create_dir_all(&mine).unwrap();
        let owner = survey_game("g1", "__silo_shared_probe__", Some(mine.to_str().unwrap()));

        // A second game arrived from another PC under the same name, and matching
        // by name alone lands on exactly that folder.
        let foreign = parent.join("__silo_no_such_account__").join(&tail);
        let mut guest = survey_game("g2", "__silo_shared_probe__", Some(foreign.to_str().unwrap()));
        guest.exe_path = Some(
            parent
                .join("__silo_no_such_account__")
                .join("Games")
                .join("guest.exe")
                .to_string_lossy()
                .into_owned(),
        );

        let survey = survey_save_folders_of(&[owner, guest]);
        assert_eq!(survey.usable, 1, "only the game that owns the folder is usable here");
        assert!(
            survey.relinkable.is_empty(),
            "the guest must not be pointed at the owner's saves: {:?}",
            survey.relinkable
        );
        let issue = survey
            .unresolved
            .first()
            .expect("the guest has to be reported instead of silently dropped");
        assert_eq!(issue.game_id, "g2");
        assert!(
            issue.reason.contains("__silo_shared_probe__"),
            "the reason has to name the game that owns the folder: {}",
            issue.reason
        );
        assert!(!issue.install_present);
    }

    // ----- how strongly a save folder was matched -----

    #[test]
    fn a_folder_matched_only_by_name_is_the_weakest_confidence() {
        // The game was never reinstalled here and its recorded folder is gone, so
        // the only thing on this machine that looks like its save folder shares its
        // name. That is a guess rather than evidence, and it has to be labelled as
        // one, because the relink decides what to do from the label.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let search = TempDir::new("name_only_search");
        let candidate = search.path().join("__silo_name_only_game__");
        fs::create_dir_all(&candidate).unwrap();
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_name_only_game__");
        let game = restore_test_game("__silo_name_only_game__");

        let machine = MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: vec![search.path().to_path_buf()],
        };
        let target = resolve_restore_target_with(&game, foreign.to_str().unwrap(), &machine)
            .expect("a folder named after the game is still a candidate");

        assert_eq!(target.path, candidate);
        assert!(target.rediscovered, "the recorded path is not the one in use");
        assert_eq!(
            target.confidence,
            MatchConfidence::Guess,
            "nothing but the folder's name ties it to this game"
        );
        assert!(
            !target.confidence.is_enough_for_a_relink(),
            "the relink must not act on a name"
        );
        // The bar itself: a layout that translated onto a folder that exists, or a
        // prefix that really holds it, is enough; a name is not.
        assert!(MatchConfidence::Confirmed.is_enough_for_a_relink());
        assert!(MatchConfidence::Exact.is_enough_for_a_relink());
    }

    #[test]
    fn the_survey_reports_a_name_only_match_instead_of_proposing_it() {
        // The same thing end to end: the game gets one issue, in the bucket the
        // user has to act on, naming the folder SILO refused to claim.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let search = TempDir::new("name_only_survey");
        let candidate = search.path().join("__silo_name_only_game__");
        fs::create_dir_all(&candidate).unwrap();
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_name_only_game__");
        let games = vec![survey_game(
            "g1",
            "__silo_name_only_game__",
            Some(foreign.to_str().unwrap()),
        )];

        let machine = MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: vec![search.path().to_path_buf()],
        };
        let survey = survey_save_folders_with(&games, &machine);

        assert_eq!(survey.games_with_save_folder, 1);
        assert_eq!(survey.usable, 0);
        assert!(
            survey.relinkable.is_empty(),
            "a name-only match must never be proposed for the relink: {:?}",
            survey.relinkable
        );
        let issue = survey
            .unresolved
            .first()
            .expect("the game still has to be reported rather than dropped");
        assert_eq!(issue.confidence, Some(MatchConfidence::Guess));
        assert!(issue.found_path.is_none(), "and must not claim the folder");
        assert!(
            issue.reason.contains("__silo_name_only_game__"),
            "the reason has to name the folder that only matched by name: {}",
            issue.reason
        );
        assert!(
            issue.reason.contains("set in Edit"),
            "and say what the user can do about it: {}",
            issue.reason
        );
        assert!(
            !foreign.exists(),
            "surveying must never create the other machine's folder"
        );
    }

    #[test]
    fn a_proposed_relink_without_evidence_is_never_written() {
        // The survey never proposes one of these, but the rule is enforced where
        // the library is actually written, so even a proposal that somehow carried
        // a name-only match could not reach a game's save path.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_name_only_game__");
        let mut games = vec![survey_game(
            "g1",
            "__silo_name_only_game__",
            Some(foreign.to_str().unwrap()),
        )];
        let proposal = |found: Option<&str>, confidence: Option<MatchConfidence>| SaveFolderIssue {
            game_id: "g1".to_string(),
            game_name: "__silo_name_only_game__".to_string(),
            recorded_path: foreign.to_string_lossy().into_owned(),
            found_path: found.map(str::to_string),
            confidence,
            confidence_score: confidence.map(MatchConfidence::score),
            install_present: false,
            reason: String::new(),
        };

        let guessed = proposal(
            Some("C:/somewhere/__silo_name_only_game__"),
            Some(MatchConfidence::Guess),
        );
        assert!(
            apply_relink_proposals(&mut games, &[guessed]).is_empty(),
            "a name-only match must not be applied"
        );
        assert_eq!(games[0].save_path.as_deref(), Some(foreign.to_str().unwrap()));
        assert_eq!(games[0].save_path_source, None);

        // One that says nothing about how it matched is refused the same way, and a
        // proposal for a game this library does not have changes nothing either.
        let blind = proposal(Some("C:/somewhere/__silo_name_only_game__"), None);
        let unknown = proposal(
            Some("C:/here/__silo_name_only_game__"),
            Some(MatchConfidence::Confirmed),
        );
        assert!(apply_relink_proposals(&mut games, &[blind]).is_empty());
        assert_eq!(games[0].save_path_source, None);

        // The evidence-backed proposal still lands, and reports itself as auto.
        let confirmed = proposal(
            Some("C:/here/__silo_name_only_game__"),
            Some(MatchConfidence::Confirmed),
        );
        let payload = apply_relink_proposals(&mut games, &[confirmed]);
        assert_eq!(payload.len(), 1);
        assert_eq!(payload[0]["savePath"], "C:/here/__silo_name_only_game__");
        assert_eq!(payload[0]["savePathSource"], "auto");
        assert_eq!(games[0].save_path.as_deref(), Some("C:/here/__silo_name_only_game__"));
        assert_eq!(games[0].save_path_source.as_deref(), Some("auto"));

        let mut other_library: Vec<Game> = Vec::new();
        assert!(
            apply_relink_proposals(&mut other_library, &[unknown]).is_empty(),
            "a proposal for a game that is not in the library changes nothing"
        );
    }

    // ----- which prefix belongs to which game -----

    #[test]
    fn steam_identity_comes_from_the_paths_steam_writes() {
        // The two shapes that name a game's Steam identity: its own prefix, and the
        // install directory Steam keeps the same on every platform.
        assert_eq!(
            steam_app_id_in("/home/me/.steam/steam/steamapps/compatdata/1091500/pfx/drive_c"),
            Some("1091500".to_string())
        );
        assert_eq!(
            steam_app_id_in(r"C:\Program Files (x86)\Steam\steamapps\compatdata\1091500\pfx\drive_c"),
            Some("1091500".to_string())
        );
        assert_eq!(
            steam_app_id_in("/home/me/.steam/steam/steamapps/compatdata/not-an-app-id/pfx/drive_c"),
            None,
            "only a directory Steam names after an app id is an identity"
        );
        assert_eq!(steam_app_id_in("/home/me/Documents/Saves"), None);

        assert_eq!(
            steam_install_dir_in(r"C:\Program Files (x86)\Steam\steamapps\common\Cyberpunk 2077\bin\x64\game.exe"),
            Some("Cyberpunk 2077".to_string())
        );
        assert_eq!(
            steam_install_dir_in("/home/me/.steam/steam/steamapps/common/Cyberpunk 2077/bin/game"),
            Some("Cyberpunk 2077".to_string()),
            "the install directory keeps its name on Linux, which is the whole point"
        );
        assert_eq!(
            steam_install_dir_in(r"C:\Games\Cyberpunk2077\bin\game.exe"),
            None,
            "a layout that is not Steam's must not name an app id"
        );
        assert_eq!(steam_install_dir_in(r"C:\Program Files (x86)\Steam\steamapps"), None);
    }

    /// The Valve key-value text an `appmanifest_<appid>.acf` holds, with the two
    /// values SILO reads out of it and one it ignores.
    fn app_manifest(app_id: &str, install_dir: &str) -> String {
        format!(
            "\"AppState\"\n{{\n\t\"appid\"\t\t\"{app_id}\"\n\t\"installdir\"\t\t\"{install_dir}\"\n\t\"StateFlags\"\t\t\"4\"\n}}\n"
        )
    }

    /// The `drive_c` of the prefix Steam keeps for one app id, as the discovery
    /// code spells it: joined one component at a time, so the test's expectations
    /// are the same string the survey reports on every platform.
    fn compatdata_prefix(steamapps: &Path, app_id: &str) -> PathBuf {
        steamapps.join("compatdata").join(app_id).join("pfx").join("drive_c")
    }

    /// A Steam install under `home` with two Proton prefixes whose user trees hold
    /// a folder of the same name: the one belonging to app id 1091500, whose
    /// manifest names `leaf` as its install directory, and a decoy under another
    /// game's app id that an existence check alone could not tell apart.
    fn steam_home_with_two_prefixes(home: &Path, leaf: &str) -> (PathBuf, PathBuf, PathBuf) {
        let steamapps = home.join(".steam/steam/steamapps");
        let tail = ["users", "steamuser", "Documents", "My Games", leaf];
        let own = tail
            .iter()
            .fold(compatdata_prefix(&steamapps, "1091500"), |path, part| path.join(part));
        let decoy = tail
            .iter()
            .fold(compatdata_prefix(&steamapps, "292030"), |path, part| path.join(part));
        fs::create_dir_all(&own).unwrap();
        fs::create_dir_all(&decoy).unwrap();
        fs::write(steamapps.join("appmanifest_1091500.acf"), app_manifest("1091500", leaf)).unwrap();
        fs::write(
            steamapps.join("appmanifest_292030.acf"),
            app_manifest("292030", "The Witcher 3"),
        )
        .unwrap();
        (steamapps, own, decoy)
    }

    #[test]
    fn the_games_own_proton_prefix_is_named_by_its_steam_app_id() {
        // Windows → Linux with the game really installed here. Steam keeps both the
        // install directory name and the app id, so the path the library recorded on
        // the old machine says which prefix belongs to this game — which turns the
        // lookup into evidence instead of a search of every prefix that happens to
        // hold a folder with that name.
        let home = TempDir::new("steam_app_id");
        let (steamapps, own, decoy) = steam_home_with_two_prefixes(home.path(), "__silo_prefix_probe__");
        let machine = machine_paths_in(home.path(), &home.path().join(".local/share"), Vec::new());
        let stored = r"C:\Users\Old\Documents\My Games\__silo_prefix_probe__";
        let mut game = restore_test_game("__silo_prefix_probe__");
        game.exe_path =
            Some(r"C:\Program Files (x86)\Steam\steamapps\common\__silo_prefix_probe__\game.exe".to_string());

        assert_eq!(
            steam_app_id_of(&game, stored, &machine.steamapps),
            Some("1091500".to_string()),
            "the install directory in the recorded path names the app id"
        );
        let target =
            resolve_restore_target_with(&game, stored, &machine).expect("the game's own prefix holds the folder");
        assert_eq!(
            target.path, own,
            "the game's own prefix, not the other one that also has such a folder"
        );
        assert_eq!(target.confidence, MatchConfidence::Confirmed);
        assert!(target.confidence.is_enough_for_a_relink());
        assert!(decoy.is_dir(), "the prefix that is not this game's was left alone");
        assert!(steamapps.join("appmanifest_1091500.acf").is_file());
    }

    #[test]
    fn the_survey_relinks_into_the_prefix_steams_app_id_names() {
        // The same decision as the relink sees it: a library from another PC, the
        // game installed here, and two prefixes holding a folder of the same name.
        // The app id is what tells them apart, so the game is offered the right one
        // rather than merely reported.
        let home = TempDir::new("steam_survey");
        let (_steamapps, own, decoy) = steam_home_with_two_prefixes(home.path(), "__silo_prefix_probe__");
        let machine = machine_paths_in(home.path(), &home.path().join(".local/share"), Vec::new());
        let mut game = survey_game(
            "g1",
            "__silo_prefix_probe__",
            Some(r"C:\Users\OldPC\Documents\My Games\__silo_prefix_probe__"),
        );
        game.exe_path =
            Some(r"C:\Program Files (x86)\Steam\steamapps\common\__silo_prefix_probe__\game.exe".to_string());

        let survey = survey_save_folders_with(&[game], &machine);
        assert_eq!(
            survey.relinkable.len(),
            1,
            "the app id is evidence: {:?}",
            survey.unresolved
        );
        let issue = &survey.relinkable[0];
        assert_eq!(issue.found_path.as_deref(), own.to_str());
        assert_eq!(issue.confidence, Some(MatchConfidence::Confirmed));
        assert!(decoy.is_dir());
    }

    #[test]
    fn the_survey_reports_a_prefix_match_it_cannot_tie_to_the_game() {
        // The folder really is inside a prefix on this machine, but nothing says
        // that prefix is this game's, so the relink reports it instead of applying
        // it — and the reason has to say that the prefix is what is missing.
        let drive_c = TempDir::new("prefix_survey");
        let found = drive_c
            .path()
            .join("users/steamuser/Documents/My Games/__silo_prefix_probe__");
        fs::create_dir_all(&found).unwrap();
        let machine = MachinePaths {
            prefixes: vec![WinePrefix {
                drive_c: drive_c.path().to_path_buf(),
                app_id: None,
            }],
            steamapps: Vec::new(),
            search_dirs: Vec::new(),
        };
        let games = vec![survey_game(
            "g1",
            "__silo_prefix_probe__",
            Some(r"C:\Users\OldPC\Documents\My Games\__silo_prefix_probe__"),
        )];

        let survey = survey_save_folders_with(&games, &machine);
        assert!(
            survey.relinkable.is_empty(),
            "no identity, so no relink: {:?}",
            survey.relinkable
        );
        let issue = survey.unresolved.first().expect("the game still has to be reported");
        assert_eq!(issue.confidence, Some(MatchConfidence::Likely));
        assert!(issue.found_path.is_none(), "and must not claim the folder");
        assert!(
            issue.reason.contains("prefix"),
            "the reason has to say it was a prefix match: {}",
            issue.reason
        );
        assert!(issue.reason.contains("__silo_prefix_probe__"), "{}", issue.reason);
        assert!(issue.reason.contains("set in Edit"), "{}", issue.reason);
    }

    #[test]
    fn the_edit_proposal_hands_back_the_name_only_match_the_bulk_relink_refuses() {
        // The two views of the same game. The relink never acts on a name, so the
        // survey reports it; but the user asked about this one game from Edit and
        // will see the path before saving, so the candidate is offered — with the
        // same weakness recorded, so the dialog can say why it needs checking.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let search = TempDir::new("proposal_name_only");
        let candidate = search.path().join("__silo_proposal_probe__");
        fs::create_dir_all(&candidate).unwrap();
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_proposal_probe__");
        let game = survey_game("g1", "__silo_proposal_probe__", Some(foreign.to_str().unwrap()));
        let machine = MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: vec![search.path().to_path_buf()],
        };

        let survey = survey_save_folders_with(std::slice::from_ref(&game), &machine);
        assert!(survey.relinkable.is_empty(), "the bulk relink still refuses a name");

        let proposal = save_folder_proposal(&game, &machine).expect("the one game is offered the candidate");
        assert_eq!(proposal.found_path.as_deref(), candidate.to_str());
        assert_eq!(
            proposal.confidence,
            Some(MatchConfidence::Guess),
            "the weakness is carried through, not hidden"
        );
        assert!(
            !proposal.confidence.is_some_and(MatchConfidence::is_enough_for_a_relink),
            "and it is still not enough evidence for the automatic relink"
        );
        assert_eq!(proposal.recorded_path, foreign.to_string_lossy());
        assert!(!foreign.exists(), "proposing must not create the other machine's folder");
    }

    #[test]
    fn the_edit_proposal_is_silent_when_there_is_nothing_to_change() {
        // Two kinds of nothing: a folder that already works here (so there is
        // nothing to propose), and a game with no save folder recorded at all.
        let machine = MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: Vec::new(),
        };
        let Some(profile) = current_profile_dir() else {
            return;
        };
        let current = profile.join("__silo_proposal_usable__");
        fs::create_dir_all(&current).unwrap();
        let usable = survey_game("g1", "__silo_proposal_usable__", Some(current.to_str().unwrap()));
        assert!(
            save_folder_proposal(&usable, &machine).is_none(),
            "a folder that already works here is not proposed for a change"
        );
        fs::remove_dir_all(&current).ok();

        let empty = survey_game("g2", "__silo_proposal_empty__", None);
        assert!(
            save_folder_proposal(&empty, &machine).is_none(),
            "a game with no save folder recorded has nothing to resolve"
        );
    }

    #[test]
    fn the_edit_proposal_still_reports_a_game_it_cannot_place() {
        // Nothing on this machine matches, but the user is still told why, so Edit
        // can say what to do instead of silently finding nothing.
        let Some((_profile, parent)) = profile_and_parent() else {
            return;
        };
        let foreign = parent
            .join("__silo_no_such_account__")
            .join("Documents")
            .join("__silo_proposal_absent__");
        let game = survey_game("g1", "__silo_proposal_absent__", Some(foreign.to_str().unwrap()));
        let machine = MachinePaths {
            prefixes: Vec::new(),
            steamapps: Vec::new(),
            search_dirs: Vec::new(),
        };

        let proposal = save_folder_proposal(&game, &machine).expect("the user still gets an answer");
        assert!(proposal.found_path.is_none(), "there is no candidate to offer");
        assert!(proposal.confidence.is_none());
        assert!(
            proposal.reason.contains("__silo_proposal_absent__"),
            "the reason names the recorded folder: {}",
            proposal.reason
        );
        assert!(
            proposal.reason.contains("different PC") || proposal.reason.contains("does not exist"),
            "and says why it does not work here: {}",
            proposal.reason
        );
    }
}

