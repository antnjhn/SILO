use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};
use tauri::{AppHandle, Manager, Emitter};
use std::process::Command;
#[cfg(target_os = "windows")]
use std::os::windows::process::CommandExt;
use std::sync::mpsc::channel;
use std::collections::HashSet;
use notify::{Watcher, RecursiveMode, EventKind};
use walkdir::WalkDir;
use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;


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
                        game.is_installed = Some(game.exe_path.as_ref().map_or(false, |p| Path::new(p).exists()));
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
                game.is_installed = Some(game.exe_path.as_ref().map_or(false, |p| Path::new(p).exists()));
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
                            game.is_installed = Some(game.exe_path.as_ref().map_or(false, |p| Path::new(p).exists()));
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

#[tauri::command]
pub fn add_game(app: AppHandle, name: String, exe_path: Option<String>, wallpaper: Option<String>, logo_path: Option<String>, font_family: Option<String>, font_color: Option<String>, save_path: Option<String>) -> Result<Game, String> {
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
fn set_launcher_background(_background: bool) {}

/// SaveGuard should watch for save writes while a game runs when there is no usable
/// folder yet — or when the stored folder no longer exists (game moved/reinstalled)
/// so a fresh location gets re-found instead of silently never backing up again.
fn save_detection_needed(save_path: Option<&str>) -> bool {
    match save_path {
        None => true,
        Some(path) => !Path::new(path).is_dir(),
    }
}

#[tauri::command]
pub async fn launch_game(app: AppHandle, game_id: String, xbox_mode: bool) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let exe_path = game.exe_path.clone().ok_or("No executable set")?;
    
    let game_id_clone = game_id.clone();
    let save_path_clone = game.save_path.clone();
    let backup_count_clone = game.backup_count.unwrap_or(5);
    
    tauri::async_runtime::spawn(async move {
        let window = app.get_webview_window("main");
        
        let working_dir = std::path::Path::new(&exe_path).parent().unwrap_or(std::path::Path::new(""));

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
            let mut c = Command::new(&exe_path);
            c.current_dir(working_dir);
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
            let mut detected_path: Option<PathBuf> = None;
            let mut watcher_opt = None;
            let mut rx_opt = None;
            
            if run_detection {
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

            while is_running {
                if game_pid.is_none() {
                    // Hunt for the real game PID (by name, preferring a descendant of the
                    // cmd we spawned) for as long as it takes — some games start through a
                    // launcher and only spawn their main process later. The 0.2.1 "perf" cap
                    // stopped this scan after 15s and silently broke save detection for those
                    // games, so it now runs until a PID is pinned (the full snapshot is freed
                    // the moment we pin).
                    system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                    let mut name_match: Option<u32> = None;
                    for (pid, proc) in system.processes() {
                        if proc.name().to_string_lossy().to_lowercase() != exe_name.to_lowercase() {
                            continue;
                        }
                        let candidate_pid = pid.as_u32();
                        // Prefer a process spawned by our cmd instance (avoids matching a
                        // second instance of the same game already running).
                        if crate::saveguard::is_descendant(&system, candidate_pid, spawned_child_pid) {
                            game_pid = Some(candidate_pid);
                            game_is_ours = true;
                            break;
                        }
                        if name_match.is_none() {
                            name_match = Some(candidate_pid);
                        }
                    }
                    // Fall back to name-only matching after a grace period (some games
                    // launch through a launcher rather than directly from our cmd).
                    if game_pid.is_none() && name_match.is_some() && check_start.elapsed().as_secs() >= 3 {
                        game_pid = name_match;
                        game_is_ours = false;
                    }
                    if game_pid.is_some() {
                        game_started_at = Some(Instant::now());
                        game_started_wall = Some(chrono::Local::now().to_rfc3339());
                        // Free the full-process snapshot: once pinned we only need this one
                        // process, so drop the rest of the table to keep session RAM low.
                        system = sysinfo::System::new();
                    }
                } else {
                    system.refresh_processes(
                        sysinfo::ProcessesToUpdate::Some(&[sysinfo::Pid::from_u32(game_pid.unwrap())]),
                        true,
                    );
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
                        let is_write = match event.kind {
                            EventKind::Modify(_) | EventKind::Create(_) => true,
                            _ => false,
                        };

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
                        if let Some(target_pid) = game_pid {
                            let paths: Vec<PathBuf> = pending_paths.drain().collect();
                            for path in paths {
                                if detected_roots.iter().any(|root| path.starts_with(root)) {
                                    continue;
                                }
                                let pids = crate::saveguard::get_locking_pids(&path);
                                for locking_pid in pids {
                                    // Refresh the writer's own ancestry so saves written by
                                    // child/helper processes of the game are attributed too
                                    // (the snapshot only tracks the pinned PID at this point).
                                    if crate::saveguard::pid_is_descendant_of(&mut system, locking_pid, target_pid) {
                                        if let Some(root) = crate::saveguard::get_save_root(&path, &watch_dirs) {
                                            log::info!("SaveGuard detected save root: {:?}", root);
                                            detected_path = Some(root.clone());
                                            detected_roots.insert(root);
                                            break;
                                        }
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
                
                let child_exited = match child.try_wait() {
                    Ok(Some(_)) => true,
                    _ => false,
                };
                
                if child_exited {
                    // Only keep the loop alive if the pinned PID is still running AND it is
                    // a descendant of the cmd we spawned. The name-match fallback can pin an
                    // unrelated already-running instance of the same game; trusting it here
                    // would make this loop never terminate and the save backup never run.
                    let running_in_system = match game_pid {
                        Some(pid) => game_is_ours && system.process(sysinfo::Pid::from_u32(pid)).is_some(),
                        None => false,
                    };
                    if running_in_system {
                        found_process = true;
                    } else {
                        if !found_process && check_start.elapsed().as_secs() < 12 {
                            is_running = true;
                        } else {
                            is_running = false;
                        }
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

            let mut final_save_path = save_path_clone;
            
            if let Some(path) = detected_path {
                let path_str = path.to_string_lossy().into_owned();
                let mut games = get_games(app.clone());
                if let Some(g) = games.iter_mut().find(|g| g.id == game_id_clone) {
                    g.save_path = Some(path_str.clone());
                    g.save_path_source = Some("auto".to_string());
                    let _ = save_games_atomic(&app, &games);
                    
                    let _ = app.emit("saveguard-path-detected", serde_json::json!({
                        "gameId": game_id_clone,
                        "savePath": path_str
                    }));
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
                                        let _ = app.emit("saveguard-backup-complete", serde_json::json!({
                                            "gameId": game_id_clone,
                                            "timestamp": timestamp
                                        }));
                                    }
                                    Err(e) => {
                                        log::warn!("SaveGuard auto-backup failed for {}: {}", game_id_clone, e);
                                        let _ = app.emit("saveguard-backup-failed", serde_json::json!({
                                            "gameId": game_id_clone,
                                            "reason": e
                                        }));
                                    }
                                }
                            }
                            Err(e) => {
                                log::warn!("SaveGuard could not create backups dir for {}: {}", game_id_clone, e);
                                let _ = app.emit("saveguard-backup-failed", serde_json::json!({
                                    "gameId": game_id_clone,
                                    "reason": e.to_string()
                                }));
                            }
                        }
                    } else {
                        log::warn!("SaveGuard known save folder missing at exit for {}: {:?}", game_id_clone, save_path);
                        let _ = app.emit("saveguard-path-missing", serde_json::json!({
                            "gameId": game_id_clone,
                            "savePath": path_str
                        }));
                    }
                }
                None => {
                    // A session ran with no usable save folder and SaveGuard could not
                    // attribute one — surface it so the user is never left wondering.
                    if run_detection {
                        log::info!("SaveGuard found no save folder for {} this session", game_id_clone);
                        let _ = app.emit("saveguard-not-found", serde_json::json!({
                            "gameId": game_id_clone
                        }));
                    }
                }
            }
            
            // Playtime is measured from when the game process was first detected, not from spawn.
            let elapsed_secs = game_started_at.map_or(0, |started| started.elapsed().as_secs());
            let elapsed_minutes = if elapsed_secs > 0 { (elapsed_secs + 59) / 60 } else { 0 };

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

    let _ = xbox_mode; // hide/show now applies to both modes
    Ok(())
}

#[tauri::command]
pub fn window_minimize(window: tauri::Window) {
    window.minimize().ok();
}

#[tauri::command]
pub fn window_maximize(window: tauri::Window) {
    if let Ok(is_max) = window.is_maximized() {
        if is_max { window.unmaximize().ok(); } else { window.maximize().ok(); }
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
    "unins", "setup", "crash", "redist", "dxwebsetup", "vcredist",
    "cef", "bootstrap", "dotnet", "directx", "dxsetup",
    "installer", "updater", "reporter", "helper", "service",
    "vc_redist", "oalinst", "physx", "easyanticheat",
    "battleye", "beclient", "beservice", "eac_launcher",
    "ue4prereqsetup", "unrealcefsubprocess", "steamwebhelper",
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
    let deprioritize = ["_be", "_dx12", "_dx11", "_vulkan", "_debug", "_server",
                        "_shipping", "_launcher", "-win64", "-shipping", "dedicated"];
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
        game_dir: String,      // top-level subfolder name (the "game folder")
        display_name: String,  // cleaned up name for display
    }

    let mut candidates: Vec<ExeCandidate> = Vec::new();

    for entry in WalkDir::new(&folder_path).max_depth(5).into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        if !path.is_file() { continue; }
        
        let ext = match path.extension() {
            Some(e) => e.to_string_lossy().to_lowercase(),
            None => continue,
        };
        if ext != "exe" { continue; }

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
        if components.is_empty() { continue; }

        // If the exe is directly in the scanned folder (no subfolder), use exe stem as game dir
        let game_dir = if components.len() == 1 {
            file_stem.to_string()
        } else {
            components[0].as_os_str().to_string_lossy().to_string()
        };

        let depth = entry.depth();

        // Build display name from the game directory, cleaning up common folder patterns
        let parent_name = path.parent()
            .and_then(|p| p.file_name())
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| game_dir.clone());

        let display_name = {
            let p_lower = parent_name.to_lowercase();
            if ["bin", "win64", "win32", "binaries", "system32", "x64", "x86", "game", "shipping"].contains(&p_lower.as_str()) {
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
    results.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));

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
    let url = format!("https://store.steampowered.com/api/storesearch/?term={}&l=english&cc=US", urlencoding::encode(&name));
    
    let client = match reqwest::Client::builder()
        .user_agent("Mozilla/5.0 (Windows NT 10.0; Win64; x64)")
        .build() {
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
                                format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/logo.png", id_str),
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
        },
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
    if path.is_dir() { Some(path) } else { None }
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

fn zip_dir(src_dir: &Path, dest_zip: &Path) -> Result<(), String> {
    zip_dir_filtered(src_dir, dest_zip, |_| false)
}

fn zip_dir_filtered<F>(src_dir: &Path, dest_zip: &Path, exclude: F) -> Result<(), String>
where
    F: Fn(&Path) -> bool,
{
    let file = File::create(dest_zip).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let walkdir = WalkDir::new(src_dir);
    for entry in walkdir.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path.strip_prefix(src_dir)
            .map_err(|e| e.to_string())?;

        if exclude(name) {
            continue;
        }

        let zip_name = name.to_string_lossy().replace("\\", "/");

        if path.is_file() {
            zip.start_file(zip_name, options)
                .map_err(|e| e.to_string())?;
            let mut f = File::open(path).map_err(|e| e.to_string())?;
            let mut buffer = Vec::new();
            f.read_to_end(&mut buffer).map_err(|e| e.to_string())?;
            zip.write_all(&buffer).map_err(|e| e.to_string())?;
        } else if !name.as_os_str().is_empty() {
            zip.add_directory(zip_name, options)
                .map_err(|e| e.to_string())?;
        }
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
                matches!(component, Component::ParentDir | Component::RootDir | Component::Prefix(_))
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
    let name = path.file_name().and_then(|name| name.to_str()).ok_or("Save folder name is invalid")?;
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S_%3f");
    Ok(parent.join(format!(".{}_silo_{}_{}", name, purpose, timestamp)))
}

fn restore_save_transactionally(backup_file: &Path, save_path: &Path, backups_dir: &Path) -> Result<(), String> {
    let parent = save_path.parent().ok_or("Save folder has no parent directory")?;
    fs::create_dir_all(parent).map_err(|e| format!("Could not prepare save folder: {}", e))?;

    let staging_dir = temporary_sibling(save_path, "restore_staging")?;
    fs::create_dir(&staging_dir).map_err(|e| format!("Could not create restore staging folder: {}", e))?;

    if let Err(error) = unzip_file(backup_file, &staging_dir) {
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(format!("Backup could not be extracted safely: {}", error));
    }

    if save_path.exists() {
        fs::create_dir_all(backups_dir).map_err(|e| format!("Could not prepare backup folder: {}", e))?;
        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S");
        let safety_snapshot = backups_dir.join(format!("manual_{}_Before%20restore.zip", timestamp));
        if let Err(error) = zip_dir(save_path, &safety_snapshot) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(format!("Could not create the pre-restore safety backup: {}", error));
        }
    }

    let previous_dir = temporary_sibling(save_path, "restore_previous")?;
    let had_existing_save = save_path.exists();
    if had_existing_save {
        if let Err(error) = fs::rename(save_path, &previous_dir) {
            let _ = fs::remove_dir_all(&staging_dir);
            return Err(format!("Could not prepare current save for restore. Is the game still running? {}", error));
        }
    }

    if let Err(error) = fs::rename(&staging_dir, save_path) {
        if had_existing_save {
            let _ = fs::rename(&previous_dir, save_path);
        }
        let _ = fs::remove_dir_all(&staging_dir);
        return Err(format!("Could not apply restored save: {}", error));
    }

    if had_existing_save {
        if let Err(error) = fs::remove_dir_all(&previous_dir) {
            log::warn!("Restored save successfully but could not remove temporary previous save at {:?}: {}", previous_dir, error);
        }
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
        for i in 0..remove_count {
            let _ = fs::remove_file(&entries[i].0);
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

    let canonical_backups = backups_dir.canonicalize().map_err(|e| format!("Invalid backups directory: {}", e))?;
    
    if target_path.exists() {
        let canonical_target = target_path.canonicalize().map_err(|e| format!("Invalid backup file: {}", e))?;
        if !canonical_target.starts_with(&canonical_backups) {
            return Err("Invalid backup name: path outside backup directory".to_string());
        }
    } else {
        if let Some(parent) = target_path.parent() {
            if let Ok(canonical_parent) = parent.canonicalize() {
                if !canonical_parent.starts_with(&canonical_backups) {
                    return Err("Invalid backup name: path outside backup directory".to_string());
                }
            }
        }
    }

    Ok(target_path)
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, game_id: String, backup_name: String) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let save_path_str = game.save_path.ok_or("No save path configured for this game")?;
    let save_path = Path::new(&save_path_str);
    
    let backup_file = validate_backup_name(&app, &game_id, &backup_name)?;
    if !backup_file.exists() {
        return Err("Backup file does not exist".to_string());
    }
    
    let backups_dir = get_backups_dir(&app, &game_id);
    restore_save_transactionally(&backup_file, save_path, &backups_dir)?;
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
            let game_name = games.iter().find(|g| g.id == game_id).map(|g| g.name.clone()).unwrap_or_else(|| game_id.clone());
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
    let save_path_str = game.save_path.ok_or("No save path configured for this game")?;
    let save_path = Path::new(&save_path_str);
    
    if !save_path.exists() {
        return Err("Save directory does not exist".to_string());
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
        let unins = parent.join("unins000.exe");
        if unins.exists() { return Ok(Some(unins.to_string_lossy().to_string())); }
        let unins = parent.join("uninstall.exe");
        if unins.exists() { return Ok(Some(unins.to_string_lossy().to_string())); }
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
        for sys_dir in ["/", "/etc", "/usr", "/var", "/bin", "/sbin", "/lib", "/lib64",
                        "/boot", "/proc", "/sys", "/dev", "/run", "/tmp"] {
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
    let logs_dir = app.path().app_log_dir().unwrap_or_else(|_| app.path().app_data_dir().unwrap().join("logs"));
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
    let file_path = app.dialog().file().add_filter("Zip Archive", &["zip"]).set_file_name("silo_backup.zip").blocking_save_file();
    if let Some(dest_path) = file_path {
        let app_data_dir = app.path().app_data_dir().map_err(|_| "Failed to get AppData directory")?;
        // Do not recurse existing backup/save archives (avoids zip-of-zip bloat).
        let exclude = |rel_path: &Path| {
            let mut components = rel_path.components();
            if let Some(first) = components.next() {
                let folder = first.as_os_str().to_string_lossy().to_lowercase();
                if (folder == "backups" || folder == "saves")
                    && rel_path.extension().and_then(|e| e.to_str()) == Some("zip") {
                    return true;
                }
            }
            false
        };
        zip_dir_filtered(&app_data_dir, std::path::Path::new(&dest_path.to_string()), exclude)?;
        return Ok(dest_path.to_string());
    }
    Err("Backup cancelled".into())
}

#[tauri::command]
pub async fn restore_library(app: AppHandle) -> Result<String, String> {
    let file_path = app.dialog().file().add_filter("Zip Archive", &["zip"]).blocking_pick_file();
    if let Some(src_path) = file_path {
        let app_data_dir = app.path().app_data_dir().map_err(|_| "Failed to get AppData directory")?;
        unzip_file(std::path::Path::new(&src_path.to_string()), &app_data_dir)?;
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
        writer
            .start_file(name, zip::write::FileOptions::default())
            .unwrap();
        writer.write_all(content).unwrap();
        writer.finish().unwrap();
    }

    #[test]
    fn unzip_rejects_parent_dir_traversal() {
        let zip_dir = TempDir::new("zipslip_zip");
        let dst = TempDir::new("zipslip_dst");
        let zip_path = zip_dir.path().join("evil.zip");
        write_zip_with_entry(&zip_path, "../../evil.txt", b"pwned");

        let err = unzip_file(&zip_path, &dst.path()).unwrap_err();
        assert!(err.contains("unsafe path"), "unexpected error: {}", err);
        assert!(
            relative_paths(&dst.path()).is_empty(),
            "dest should be untouched"
        );
    }

    #[test]
    fn unzip_rejects_backslash_traversal() {
        let zip_dir = TempDir::new("zipslip_zip2");
        let dst = TempDir::new("zipslip_dst2");
        let zip_path = zip_dir.path().join("evil2.zip");
        write_zip_with_entry(&zip_path, "..\\evil.txt", b"pwned");

        assert!(unzip_file(&zip_path, &dst.path()).is_err());
        assert!(relative_paths(&dst.path()).is_empty());
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
}
