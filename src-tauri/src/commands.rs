use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager, Emitter};
use std::process::Command;
use std::os::windows::process::CommandExt;
use std::sync::mpsc::channel;
use notify::{Watcher, RecursiveMode, EventKind};
use walkdir::WalkDir;
use std::fs::File;
use std::io::{Read, Write};
use std::time::{SystemTime, Instant};
use std::collections::HashSet;


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
    #[serde(rename = "backupCount", default)]
    pub backup_count: Option<u32>,
    #[serde(rename = "isInstalled", default)]
    pub is_installed: Option<bool>,
}

fn get_data_path(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("games.json")
}

fn get_wallpapers_dir(app: &AppHandle) -> PathBuf {
    app.path().app_data_dir().unwrap().join("wallpapers")
}

#[tauri::command]
pub fn get_games(app: AppHandle) -> Vec<Game> {
    let data_path = get_data_path(&app);
    println!("DATA PATH IS: {:?}", data_path);
    if !data_path.exists() {
        println!("DATA PATH DOES NOT EXIST");
        return vec![];
    }
    let data = fs::read_to_string(data_path).unwrap_or_default();
    match serde_json::from_str::<Vec<Game>>(&data) {
        Ok(mut games) => {
            // Check installed status dynamically
            for game in &mut games {
                if let Some(ref path) = game.exe_path {
                    game.is_installed = Some(Path::new(path).exists());
                } else {
                    game.is_installed = Some(false);
                }
            }
            games
        },
        Err(e) => {
            println!("Failed to parse games.json: {}", e);
            vec![]
        }
    }
}

#[tauri::command]
pub fn add_game(app: AppHandle, name: String, exe_path: Option<String>, wallpaper: Option<String>, logo_path: Option<String>, font_family: Option<String>, font_color: Option<String>) -> Game {
    let mut games = get_games(app.clone());
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
        status: Some("Playing".to_string()),
        added_at: format!("{:?}", std::time::SystemTime::now()), // simple placeholder
        save_path: None,
        backup_count: Some(5),
    };
    games.push(new_game.clone());
    
    let data_path = get_data_path(&app);
    fs::create_dir_all(data_path.parent().unwrap()).ok();
    fs::write(data_path, serde_json::to_string_pretty(&games).unwrap()).ok();
    new_game
}

#[tauri::command]
pub fn update_game(app: AppHandle, id: String, updates: serde_json::Value) -> Option<Game> {
    let mut games = get_games(app.clone());
    if let Some(game) = games.iter_mut().find(|g| g.id == id) {
        if let Some(name) = updates.get("name").and_then(|v| v.as_str()) { game.name = name.to_string(); }
        if let Some(exe_path) = updates.get("exePath").and_then(|v| v.as_str()) { game.exe_path = Some(exe_path.to_string()); }
        if let Some(wallpaper) = updates.get("wallpaper").and_then(|v| v.as_str()) { game.wallpaper = Some(wallpaper.to_string()); }
        if let Some(logo_path) = updates.get("logoPath").and_then(|v| v.as_str()) { game.logo_path = Some(logo_path.to_string()); }
        if let Some(font_family) = updates.get("fontFamily").and_then(|v| v.as_str()) { game.font_family = Some(font_family.to_string()); }
        if let Some(font_color) = updates.get("fontColor").and_then(|v| v.as_str()) { game.font_color = Some(font_color.to_string()); }
        if let Some(status) = updates.get("status").and_then(|v| v.as_str()) { game.status = Some(status.to_string()); }
        
        if let Some(save_path) = updates.get("savePath") {
            if save_path.is_null() {
                game.save_path = None;
            } else if let Some(s) = save_path.as_str() {
                game.save_path = if s.trim().is_empty() { None } else { Some(s.to_string()) };
            }
        }
        if let Some(backup_count) = updates.get("backupCount").and_then(|v| v.as_u64()) {
            game.backup_count = Some(backup_count as u32);
        }
        
        let updated = game.clone();
        fs::write(get_data_path(&app), serde_json::to_string_pretty(&games).unwrap()).ok();
        Some(updated)
    } else {
        None
    }
}

#[tauri::command]
pub fn delete_game(app: AppHandle, id: String) -> bool {
    let mut games = get_games(app.clone());
    games.retain(|g| g.id != id);
    fs::write(get_data_path(&app), serde_json::to_string_pretty(&games).unwrap()).ok();
    true
}

#[tauri::command]
pub fn get_system_fonts() -> Vec<String> {
    const CREATE_NO_WINDOW: u32 = 0x08000000;
    let output = Command::new("powershell")
        .args(["-NoProfile", "-Command", "Add-Type -AssemblyName PresentationCore; [System.Windows.Media.Fonts]::SystemFontFamilies | Select-Object -ExpandProperty Source"])
        .creation_flags(CREATE_NO_WINDOW)
        .output();
    
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut fonts: Vec<String> = stdout.lines().map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
        fonts.sort();
        fonts
    } else {
        vec!["Arial".to_string()]
    }
}



use tauri_plugin_dialog::DialogExt;

#[tauri::command]
pub async fn pick_exe(app: AppHandle) -> Option<String> {
    let file_path = app.dialog().file().add_filter("Executables", &["exe"]).blocking_pick_file();
    file_path.map(|p| p.to_string())
}

#[tauri::command]
pub async fn pick_wallpaper(app: AppHandle, game_id: String) -> Option<String> {
    let file_path = app.dialog().file().add_filter("Images", &["jpg", "jpeg", "png", "webp", "gif"]).blocking_pick_file();
    if let Some(src) = file_path {
        let src_str = src.to_string();
        let ext = std::path::Path::new(&src_str).extension().and_then(|s| s.to_str()).unwrap_or("png");
        let wallpapers_dir = get_wallpapers_dir(&app);
        fs::create_dir_all(&wallpapers_dir).ok();
        let dest = wallpapers_dir.join(format!("{}.{}", game_id, ext));
        if fs::copy(src.to_string(), &dest).is_ok() {
            return Some(dest.to_string_lossy().to_string());
        }
    }
    None
}

#[tauri::command]
pub async fn pick_logo(app: AppHandle, game_id: String) -> Option<String> {
    let file_path = app.dialog().file().add_filter("Images", &["jpg", "jpeg", "png", "webp"]).blocking_pick_file();
    if let Some(src) = file_path {
        let src_str = src.to_string();
        let ext = std::path::Path::new(&src_str).extension().and_then(|s| s.to_str()).unwrap_or("png");
        let wallpapers_dir = get_wallpapers_dir(&app);
        fs::create_dir_all(&wallpapers_dir).ok();
        let dest = wallpapers_dir.join(format!("logo_{}.{}", game_id, ext));
        if fs::copy(src.to_string(), &dest).is_ok() {
            return Some(dest.to_string_lossy().to_string());
        }
    }
    None
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
        const CREATE_NO_WINDOW: u32 = 0x08000000;
        let start = std::time::Instant::now();
        #[cfg(target_os = "windows")]
        use std::os::windows::process::CommandExt;
        
        let window = app.get_webview_window("main");
        
        if xbox_mode {
            if let Some(win) = &window {
                let _ = win.hide();
            }
            #[cfg(target_os = "windows")]
            {
                // Kill bloatware apps and explorer (which also mutes notifications)
                // Note: Removed msedgewebview2.exe because that kills the launcher's own UI!
                for app in &["OneDrive.exe", "PhoneExperienceHost.exe", "explorer.exe"] {
                    let mut kill_cmd = std::process::Command::new("taskkill");
                    kill_cmd.creation_flags(0x08000000);
                    let _ = kill_cmd.args(&["/F", "/IM", app]).spawn();
                }
                
                // Stop services and track which were running
                let ps_script = r#"
                    $svcs = @('SysMain', 'WSearch')
                    $stopped = @()
                    foreach ($s in $svcs) {
                        $svc = Get-Service -Name $s -ErrorAction SilentlyContinue
                        if ($svc.Status -eq 'Running') {
                            Stop-Service -Name $s -Force -ErrorAction SilentlyContinue
                            $stopped += $s
                        }
                    }
                    $stopped -join ',' | Out-File "$env:TEMP\silo_svcs.txt"
                    
                    # Stop telemetry permanently for this session
                    Stop-Service -Name DiagTrack -Force -ErrorAction SilentlyContinue
                "#;
                let mut ps_cmd = std::process::Command::new("powershell");
                ps_cmd.creation_flags(0x08000000);
                let _ = ps_cmd.args(&["-NoProfile", "-Command", ps_script]).spawn();
            }
        }
        
        let mut cmd = Command::new("cmd");
        #[cfg(target_os = "windows")]
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
        
        let working_dir = std::path::Path::new(&exe_path).parent().unwrap_or(std::path::Path::new(""));
        
        cmd.current_dir(working_dir)
           .raw_arg(format!("/C start \"\" /HIGH /WAIT \"{}\"", exe_path));

        if let Ok(mut child) = cmd.spawn() {
            let exe_name = Path::new(&exe_path)
                .file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned();

            // Set up watcher channel and watch directories if save path is not set yet
            let run_detection = save_path_clone.is_none();
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
            
            let mut system = sysinfo::System::new();
            let mut game_pid: Option<u32> = None;
            let mut is_running = true;
            let mut found_process = false;
            let check_start = Instant::now();
            let watch_dirs = get_watch_directories();
            
            while is_running {
                system.refresh_processes(sysinfo::ProcessesToUpdate::All, true);
                
                // Try to find the game PID if we haven't yet
                if game_pid.is_none() {
                    for (pid, proc) in system.processes() {
                        if proc.name().to_string_lossy().to_lowercase() == exe_name.to_lowercase() {
                            game_pid = Some(pid.as_u32());
                            break;
                        }
                    }
                }
                
                // Poll watcher events if active
                if let Some(rx) = &rx_opt {
                    while let Ok(event) = rx.try_recv() {
                        let is_write = match event.kind {
                            EventKind::Modify(_) | EventKind::Create(_) => true,
                            _ => false,
                        };
                        
                        if is_write && detected_path.is_none() {
                            if let Some(target_pid) = game_pid {
                                for path in event.paths {
                                    let pids = crate::saveguard::get_locking_pids(&path);
                                    for locking_pid in pids {
                                        if crate::saveguard::is_descendant(&system, locking_pid, target_pid) {
                                            if let Some(root) = crate::saveguard::get_save_root(&path, &watch_dirs) {
                                                log::info!("SaveGuard detected save root: {:?}", root);
                                                detected_path = Some(root);
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                
                std::thread::sleep(std::time::Duration::from_millis(1000));
                
                let child_exited = match child.try_wait() {
                    Ok(Some(_)) => true,
                    _ => false,
                };
                
                if child_exited {
                    let running_in_system = is_process_running(&exe_name);
                    if running_in_system {
                        found_process = true;
                    } else {
                        // Startup tolerance: wait up to 12s for process to register on slow loads or launcher redirects
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
            
            // Cleanup watcher and process candidates
            if let Some(watcher) = watcher_opt {
                drop(watcher);
            }
            
            let mut final_save_path = save_path_clone;
            
            if let Some(path) = detected_path {
                let path_str = path.to_string_lossy().into_owned();
                let mut games = get_games(app.clone());
                if let Some(g) = games.iter_mut().find(|g| g.id == game_id_clone) {
                    g.save_path = Some(path_str.clone());
                    let _ = fs::write(get_data_path(&app), serde_json::to_string_pretty(&games).unwrap());
                    
                    let _ = app.emit("saveguard-path-detected", serde_json::json!({
                        "gameId": game_id_clone,
                        "savePath": path_str
                    }));
                }
                final_save_path = Some(path_str);
            }
            
            // Auto-backup on exit
            if let Some(ref path_str) = final_save_path {
                let save_path = Path::new(path_str);
                if save_path.exists() {
                    let backups_dir = get_backups_dir(&app, &game_id_clone);
                    if fs::create_dir_all(&backups_dir).is_ok() {
                        let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
                        let backup_file = backups_dir.join(format!("auto_{}.zip", timestamp));
                        
                        if zip_dir(save_path, &backup_file).is_ok() {
                            let _ = prune_backups(&backups_dir, backup_count_clone as usize);
                            let _ = app.emit("saveguard-backup-complete", serde_json::json!({
                                "gameId": game_id_clone,
                                "timestamp": timestamp
                            }));
                        }
                    }
                }
            }
            
            let elapsed = start.elapsed().as_secs() / 60;
            
            if xbox_mode {
                #[cfg(target_os = "windows")]
                {
                    // Restart explorer
                    let mut start_cmd = std::process::Command::new("explorer.exe");
                    start_cmd.creation_flags(0x08000000);
                    let _ = start_cmd.spawn();
                    
                    // Restart only the services that were previously running
                    let ps_script = r#"
                        $txt = "$env:TEMP\silo_svcs.txt"
                        if (Test-Path $txt) {
                            $stopped = Get-Content $txt
                            foreach ($s in ($stopped -split ',')) {
                                if ($s) { Start-Service -Name $s -ErrorAction SilentlyContinue }
                            }
                            Remove-Item $txt -ErrorAction SilentlyContinue
                        }
                    "#;
                    let mut ps_cmd = std::process::Command::new("powershell");
                    ps_cmd.creation_flags(0x08000000);
                    let _ = ps_cmd.args(&["-NoProfile", "-Command", ps_script]).spawn();
                }
                if let Some(win) = &window {
                    let _ = win.show();
                    let _ = win.set_focus();
                }
            }
            
            // Update playtime...
            let mut games = get_games(app.clone());
            if let Some(g) = games.iter_mut().find(|g| g.id == game_id_clone) {
                g.playtime_minutes += elapsed as u32;
                g.session_count += 1;
                // Emit event
                let _ = app.emit("playtime-updated", g.clone());
            }
            fs::write(get_data_path(&app), serde_json::to_string_pretty(&games).unwrap()).ok();
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

    // Expanded blacklist of non-game executables
    let blacklist = [
        "unins", "setup", "crash", "redist", "dxwebsetup", "vcredist",
        "cef", "bootstrap", "dotnet", "directx", "dxsetup",
        "installer", "updater", "reporter", "helper", "service",
        "vc_redist", "oalinst", "physx", "easyanticheat",
        "battleye", "beclient", "beservice", "eac_launcher",
        "ue4prereqsetup", "unrealcefsubprocess", "steamwebhelper",
    ];

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
        if blacklist.iter().any(|b| file_name.contains(b)) {
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
                let mut s: i32 = 0;

                // Prefer exe name matching the game folder name
                if stem == c.game_dir || c.game_dir.contains(&stem) || stem.contains(&c.game_dir) {
                    s += 50;
                }

                // Prefer shallower depth
                s -= (c.depth as i32) * 5;

                // Deprioritize variant suffixes
                let deprioritize = ["_be", "_dx12", "_dx11", "_vulkan", "_debug", "_server",
                                    "_shipping", "_launcher", "-win64", "-shipping", "dedicated"];
                for suffix in &deprioritize {
                    if stem.contains(suffix) {
                        s -= 30;
                    }
                }

                // Prefer larger files (likely the main game binary)
                if let Ok(meta) = std::fs::metadata(&c.path) {
                    s += (meta.len() / (10 * 1024 * 1024)) as i32; // +1 per 10MB
                }

                s
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
    
    let client = reqwest::Client::new();
    match client.get(&url).send().await {
        Ok(response) => {
            if let Ok(json) = response.json::<serde_json::Value>().await {
                if let Some(items) = json.get("items").and_then(|i| i.as_array()) {
                    if let Some(first_item) = items.first() {
                        if let (Some(id), Some(game_name)) = (first_item.get("id"), first_item.get("name")) {
                            let id_str = id.as_i64().unwrap_or(0).to_string();
                            let game_name_str = game_name.as_str().unwrap_or(&name).to_string();
                            
                            let wallpaper = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/library_hero.jpg", id_str);
                            let logo = format!("https://shared.akamai.steamstatic.com/store_item_assets/steam/apps/{}/logo.png", id_str);
                            
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

fn is_process_running(exe_name: &str) -> bool {
    let output = Command::new("tasklist")
        .args(&["/FI", &format!("IMAGENAME eq {}", exe_name), "/FO", "CSV", "/NH"])
        .creation_flags(0x08000000)
        .output();
    
    if let Ok(output) = output {
        let stdout = String::from_utf8_lossy(&output.stdout);
        stdout.to_lowercase().contains(&exe_name.to_lowercase())
    } else {
        false
    }
}



fn get_watch_directories() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(roaming) = std::env::var("APPDATA") {
        paths.push(PathBuf::from(roaming));
    }
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
        paths.push(PathBuf::from(local));
    }
    if let Ok(userprofile) = std::env::var("USERPROFILE") {
        let userpath = PathBuf::from(userprofile);
        paths.push(userpath.join("Saved Games"));
        paths.push(userpath.join("Documents"));
    }
    paths.into_iter().filter(|p| p.exists() && p.is_dir()).collect()
}

fn zip_dir(src_dir: &Path, dest_zip: &Path) -> Result<(), String> {
    let file = File::create(dest_zip).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipWriter::new(file);
    let options = zip::write::FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    let walkdir = WalkDir::new(src_dir);
    for entry in walkdir.into_iter().filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path.strip_prefix(src_dir)
            .map_err(|e| e.to_string())?;
            
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

fn unzip_file(zip_path: &Path, dest_dir: &Path) -> Result<(), String> {
    let file = File::open(zip_path).map_err(|e| e.to_string())?;
    let mut archive = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;

    for i in 0..archive.len() {
        let mut file = archive.by_index(i).map_err(|e| e.to_string())?;
        
        // Handle broken backups that used backslashes
        let file_name = file.name().replace("\\", "/");
        if file_name.contains("..") { continue; } // Basic path traversal protection
        
        let outpath = dest_dir.join(&file_name);

        if file_name.ends_with('/') {
            fs::create_dir_all(&outpath).map_err(|e| e.to_string())?;
        } else {
            if let Some(p) = outpath.parent() {
                if !p.exists() {
                    fs::create_dir_all(&p).map_err(|e| e.to_string())?;
                }
            }
            let mut outfile = File::create(&outpath).map_err(|e| e.to_string())?;
            std::io::copy(&mut file, &mut outfile).map_err(|e| e.to_string())?;
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
                let size_bytes = metadata.len();
                let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
                
                // Parse format: auto_YYYYMMDD_HHMMSS.zip or manual_YYYYMMDD_HHMMSS_CustomName.zip
                let is_auto = name.starts_with("auto_");
                let mut custom_name = None;
                
                let name_without_ext = name.strip_suffix(".zip").unwrap_or(&name);
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

                snapshots.push(BackupSnapshot {
                    name,
                    timestamp: formatted_time,
                    size_bytes,
                    is_auto,
                    custom_name,
                });
            }
        }
    }
    
    snapshots.sort_by(|a, b| b.timestamp.cmp(&a.timestamp));
    
    Ok(snapshots)
}

#[tauri::command]
pub fn backup_now(app: AppHandle, game_id: String) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.iter().find(|g| g.id == game_id).ok_or("Game not found")?;

    if let Some(save_path_str) = &game.save_path {
        let mut roots = std::collections::HashSet::new();
        roots.insert(PathBuf::from(save_path_str));
        crate::saveguard::backup_saves(&roots, &game_id, &app);
        Ok(())
    } else {
        Err("No save path recorded yet. Launch the game first to detect saves.".to_string())
    }
}

#[tauri::command]
pub async fn restore_backup(app: AppHandle, game_id: String, backup_name: String) -> Result<(), String> {
    let games = get_games(app.clone());
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let save_path_str = game.save_path.ok_or("No save path configured for this game")?;
    let save_path = Path::new(&save_path_str);
    
    let backup_file = get_backups_dir(&app, &game_id).join(&backup_name);
    if !backup_file.exists() {
        return Err("Backup file does not exist".to_string());
    }
    
    unzip_file(&backup_file, save_path)?;
    Ok(())
}

#[tauri::command]
pub fn delete_backup(app: AppHandle, game_id: String, backup_name: String) -> Result<(), String> {
    let backup_file = get_backups_dir(&app, &game_id).join(&backup_name);
    if backup_file.exists() {
        fs::remove_file(backup_file).map_err(|e| e.to_string())?;
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

#[tauri::command]
pub fn delete_game_folder(app: AppHandle, game_id: String) -> Result<(), String> {
    let games = get_games(app);
    let game = games.into_iter().find(|g| g.id == game_id).ok_or("Game not found")?;
    let exe_path_str = game.exe_path.unwrap_or_default();
    let exe_path = Path::new(&exe_path_str);
    if let Some(parent) = exe_path.parent() {
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
pub async fn backup_library(app: AppHandle) -> Result<String, String> {
    let file_path = app.dialog().file().add_filter("Zip Archive", &["zip"]).set_file_name("silo_backup.zip").blocking_save_file();
    if let Some(dest_path) = file_path {
        let app_data_dir = app.path().app_data_dir().map_err(|_| "Failed to get AppData directory")?;
        zip_dir(&app_data_dir, std::path::Path::new(&dest_path.to_string()))?;
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
