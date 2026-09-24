use std::collections::HashSet;
use std::fs::{self, File};
use std::path::{Path, PathBuf};
use std::sync::mpsc::channel;
use std::time::{Duration, Instant};
use notify::{Watcher, RecursiveMode, EventKind};
use sysinfo::{System, Pid};
use tauri::{AppHandle, Manager};
#[cfg(target_os = "windows")]
use windows::core::PCWSTR;
#[cfg(target_os = "windows")]
use windows::Win32::System::RestartManager::*;
#[cfg(target_os = "windows")]
use windows::Win32::Foundation::*;
use zip::write::FileOptions;
use std::io::{Write, Read};

#[cfg(target_os = "windows")]
pub fn get_locking_pids(path: &Path) -> Vec<u32> {
    let mut pids = Vec::new();
    unsafe {
        let mut session_handle: u32 = 0;
        let mut session_key: [u16; CCH_RM_SESSION_KEY as usize + 1] = [0; CCH_RM_SESSION_KEY as usize + 1];
        
        let res = RmStartSession(&mut session_handle, 0, windows::core::PWSTR(session_key.as_mut_ptr()));
        if res != ERROR_SUCCESS {
            return pids;
        }

        let path_wide: Vec<u16> = path.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect();
        let pcwstr = PCWSTR(path_wide.as_ptr());
        let resources = [pcwstr];

        let res = RmRegisterResources(session_handle, Some(&resources), None, None);
        if res == ERROR_SUCCESS {
            let mut proc_info_needed = 0;
            let mut proc_info_count = 0;
            let mut reason: u32 = 0;
            
            let res = RmGetList(
                session_handle,
                &mut proc_info_needed,
                &mut proc_info_count,
                None,
                &mut reason,
            );

            if res == ERROR_MORE_DATA {
                proc_info_count = proc_info_needed;
                let mut proc_infos: Vec<RM_PROCESS_INFO> = vec![std::mem::zeroed(); proc_info_count as usize];
                
                let res = RmGetList(
                    session_handle,
                    &mut proc_info_needed,
                    &mut proc_info_count,
                    Some(proc_infos.as_mut_ptr()),
                    &mut reason,
                );

                if res == ERROR_SUCCESS {
                    for i in 0..proc_info_count {
                        pids.push(proc_infos[i as usize].Process.dwProcessId);
                    }
                }
            }
        }
        
        let _ = RmEndSession(session_handle);
    }
    pids
}

#[cfg(not(target_os = "windows"))]
pub fn get_locking_pids(path: &Path) -> Vec<u32> {
    let mut pids = Vec::new();
    let canonical_path = path.canonicalize().unwrap_or_else(|_| path.to_path_buf());
    let proc_dir = Path::new("/proc");
    if let Ok(entries) = fs::read_dir(proc_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(file_name) = entry.file_name().into_string() {
                if let Ok(pid) = file_name.parse::<u32>() {
                    // Check open file descriptors
                    let fd_dir = entry.path().join("fd");
                    if let Ok(fd_entries) = fs::read_dir(fd_dir) {
                        for fd in fd_entries.filter_map(|e| e.ok()) {
                            if let Ok(target) = fs::read_link(fd.path()) {
                                if let Ok(canonical_target) = target.canonicalize() {
                                    if canonical_target.starts_with(&canonical_path) || canonical_path.starts_with(&canonical_target) {
                                        pids.push(pid);
                                        break;
                                    }
                                } else if target.starts_with(path) || path.starts_with(&target) {
                                    pids.push(pid);
                                    break;
                                }
                            }
                        }
                    }
                    
                    // Also check memory-mapped files (for memory-mapped saves)
                    let map_dir = entry.path().join("map_files");
                    if let Ok(map_entries) = fs::read_dir(map_dir) {
                        for map_entry in map_entries.filter_map(|e| e.ok()) {
                            if let Ok(target) = fs::read_link(map_entry.path()) {
                                if let Ok(canonical_target) = target.canonicalize() {
                                    if canonical_target.starts_with(&canonical_path) || canonical_path.starts_with(&canonical_target) {
                                        pids.push(pid);
                                        break;
                                    }
                                } else if target.starts_with(path) || path.starts_with(&target) {
                                    pids.push(pid);
                                    break;
                                }
                            }
                        }
                    }
                }
            }
        }
    }
    pids
}

pub fn is_descendant(system: &System, target_pid: u32, ancestor_pid: u32) -> bool {
    if target_pid == ancestor_pid { return true; }
    
    let mut current_pid = target_pid;
    while let Some(proc) = system.process(Pid::from_u32(current_pid)) {
        if let Some(parent) = proc.parent() {
            let parent_pid = parent.as_u32();
            if parent_pid == ancestor_pid {
                return true;
            }
            current_pid = parent_pid;
        } else {
            break;
        }
    }
    false
}

/// True when `target_pid` is `ancestor_pid` or runs somewhere under its process tree.
/// Unlike [`is_descendant`], which only consults the already-loaded snapshot, this
/// refreshes each process as it climbs the parent chain — so save writes made by a
/// game's child/helper processes are attributed even though the session snapshot only
/// tracks the single pinned game PID. Refresh cost is one PID per step of the chain.
pub fn pid_is_descendant_of(system: &mut System, target_pid: u32, ancestor_pid: u32) -> bool {
    if target_pid == ancestor_pid { return true; }

    let mut current_pid = target_pid;
    for _ in 0..64 {
        system.refresh_processes(
            sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(current_pid)]),
            false,
        );
        let parent_pid = match system.process(Pid::from_u32(current_pid)).and_then(|proc| proc.parent()) {
            Some(parent) => parent.as_u32(),
            // Process is gone or its parent is not visible — treat as not a descendant.
            None => return false,
        };
        if parent_pid == ancestor_pid {
            return true;
        }
        if parent_pid == current_pid {
            return false; // safety valve: no meaningful parent
        }
        current_pid = parent_pid;
    }
    false
}

/// True when `target_pid` runs somewhere under a process tree whose PIDs are all
/// recorded in `known`. Unlike [`is_descendant`], the anchor doesn't have to be a
/// single live root: intermediate launchers may have already exited, and a PID
/// recorded in `known` while its chain was still intact keeps the subtree linked.
pub fn pid_chain_reaches(system: &System, target_pid: u32, known: &HashSet<u32>) -> bool {
    if known.contains(&target_pid) { return true; }

    let mut current_pid = target_pid;
    for _ in 0..64 {
        let Some(proc) = system.process(Pid::from_u32(current_pid)) else { return false; };
        let Some(parent) = proc.parent() else { return false; };
        let parent_pid = parent.as_u32();
        if known.contains(&parent_pid) { return true; }
        if parent_pid == current_pid { return false; } // safety valve: no meaningful parent
        current_pid = parent_pid;
    }
    false
}

pub fn is_save_root_excluded(folder_name: &str) -> bool {
    let exclusions = [
        "d3dscache",
        "temp",
        "crashpad",
        "crash_reports",
        "crashdumps",
        "logs",
        "cache",
        "nvidia",
        "amd",
        "cef",
        "webcache",
        "gpuconfig",
        "shadercache"
    ];
    exclusions.contains(&folder_name)
}

pub fn get_save_root(path: &Path, base_dirs: &[PathBuf]) -> Option<PathBuf> {
    for base_dir in base_dirs {
        if path.starts_with(base_dir) {
            if let Ok(rel_path) = path.strip_prefix(base_dir) {
                if let Some(first_component) = rel_path.components().next() {
                    let folder_name = first_component.as_os_str().to_string_lossy().to_lowercase();

                    if is_save_root_excluded(&folder_name) {
                        return None; // Ignore cache and temp folders
                    }

                    let mut root = base_dir.clone();
                    root.push(first_component);
                    return Some(root);
                }
            }
        }
    }
    None
}

// ── Name-aware save roots ───────────────────────────────────────────────────
// Save folders are not always the first folder under a watched location.
// Cyberpunk 2077 writes to `Saved Games\CD Projekt Red\Cyberpunk 2077`, so
// resolving to the publisher folder would sweep in every other game that
// publisher makes. Knowing the game's name lets us pick the game's own folder,
// and — because a game can be started through a launcher (REDprelauncher.exe)
// or close its file handle before Restart Manager is asked — it also lets a
// save write be attributed by *where* it landed instead of only by which
// process held it.

/// How deep under a watched location a save folder may sit and still be named
/// after the game (one level of publisher nesting covers the common layouts).
const MAX_NAME_DEPTH: usize = 3;

/// Fold a name to lowercase alphanumerics, so "Cyberpunk 2077",
/// "cyberpunk2077" and "Cyberpunk2077.exe" all compare equal.
pub fn normalize_key(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// True when a folder name plausibly names the game. Folded equality always
/// counts; otherwise the shorter side (at least 4 characters) must appear
/// inside the longer one, so "The Witcher 3" matches
/// "The Witcher 3: Wild Hunt" while "CD Projekt Red" matches neither.
pub fn name_matches(folder: &str, key: &str) -> bool {
    let folder_key = normalize_key(folder);
    let name_key = normalize_key(key);
    if folder_key.is_empty() || name_key.is_empty() {
        return false;
    }
    if folder_key == name_key {
        return true;
    }
    let (short, long) = if folder_key.len() <= name_key.len() {
        (folder_key.as_str(), name_key.as_str())
    } else {
        (name_key.as_str(), folder_key.as_str())
    };
    short.len() >= 4 && long.contains(short)
}

/// Deepest level (1-based) among the first [`MAX_NAME_DEPTH`] components of
/// `rel` that names the game. Excluded folders (cache/temp/…) are never
/// candidates, but they don't disqualify a deeper match either — a save folder
/// may legitimately contain a `cache` subfolder.
fn deepest_name_match(rel: &Path, name_keys: &[String]) -> Option<usize> {
    let mut found = None;
    for (index, component) in rel.components().take(MAX_NAME_DEPTH).enumerate() {
        let name = component.as_os_str().to_string_lossy().to_lowercase();
        if is_save_root_excluded(&name) {
            continue;
        }
        if name_keys
            .iter()
            .any(|key| name_matches(&name, key))
        {
            found = Some(index + 1);
        }
    }
    found
}

fn push_components(base_dir: &Path, rel: &Path, depth: usize) -> PathBuf {
    let mut root = base_dir.to_path_buf();
    for component in rel.components().take(depth) {
        root.push(component.as_os_str());
    }
    root
}

/// Name-aware [`get_save_root`]: when a folder on the path names the game, that
/// folder is the root, so `Saved Games\CD Projekt Red\Cyberpunk 2077` backs up
/// Cyberpunk's saves instead of every CD Projekt Red game. Falls back to the
/// historical "first folder under the base" behaviour when the name is unknown
/// or nothing on the path matches.
pub fn get_save_root_for(path: &Path, base_dirs: &[PathBuf], name_keys: &[String]) -> Option<PathBuf> {
    for base_dir in base_dirs {
        let Ok(rel_path) = path.strip_prefix(base_dir) else {
            continue;
        };
        let Some(first_component) = rel_path.components().next() else {
            continue;
        };
        let first_name = first_component.as_os_str().to_string_lossy().to_lowercase();
        if is_save_root_excluded(&first_name) {
            continue; // Ignore cache and temp folders
        }
        let depth = deepest_name_match(rel_path, name_keys).unwrap_or(1);
        return Some(push_components(base_dir, rel_path, depth));
    }
    None
}

/// The deepest folder on a changed file's ancestry that names the game, used as
/// the lock-free fallback when Restart Manager cannot attribute a write.
/// Returns `None` unless a folder genuinely matches — a write by anything else
/// is never claimed.
pub fn save_root_by_name(path: &Path, base_dirs: &[PathBuf], name_keys: &[String]) -> Option<PathBuf> {
    if name_keys.is_empty() {
        return None;
    }
    for base_dir in base_dirs {
        let Ok(rel_path) = path.strip_prefix(base_dir) else {
            continue;
        };
        if let Some(depth) = deepest_name_match(rel_path, name_keys) {
            return Some(push_components(base_dir, rel_path, depth));
        }
    }
    None
}

pub fn start_watcher(pid: u32, game_id: String, app_handle: AppHandle) {
    std::thread::spawn(move || {
        let (tx, rx) = channel();
        let mut watcher = match notify::recommended_watcher(tx) {
            Ok(w) => w,
            Err(e) => {
                log::error!("Failed to create watcher: {}", e);
                return;
            }
        };
        
        let path_resolver = app_handle.path();
        let mut base_dirs = Vec::new();
        if let Ok(dir) = path_resolver.local_data_dir() { base_dirs.push(dir); }
        if let Ok(dir) = path_resolver.data_dir() { base_dirs.push(dir); }
        if let Ok(dir) = path_resolver.config_dir() { base_dirs.push(dir); }
        if let Ok(dir) = path_resolver.document_dir() {
            base_dirs.push(dir.clone());
            base_dirs.push(dir.join("My Games"));
        }
        if let Ok(dir) = path_resolver.home_dir() {
            base_dirs.push(dir.join("Saved Games"));
            #[cfg(not(target_os = "windows"))]
            {
                base_dirs.push(dir.join(".local/share/Steam/steamapps/compatdata"));
                base_dirs.push(dir.join(".steam/steam/steamapps/compatdata"));
                base_dirs.push(dir.join(".var/app/com.valvesoftware.Steam/.local/share/Steam/steamapps/compatdata"));
                base_dirs.push(dir.join(".wine/drive_c/users"));
            }
        }

        for dir in &base_dirs {
            if dir.exists() {
                let _ = watcher.watch(dir, RecursiveMode::Recursive);
            }
        }

        let mut detected_roots = HashSet::new();
        let mut pending_paths = HashSet::new();
        let mut last_detection_check: Option<Instant> = None;
        let mut system = System::new();

        loop {
            system.refresh_processes(sysinfo::ProcessesToUpdate::Some(&[Pid::from_u32(pid)]), true);
            if system.process(Pid::from_u32(pid)).is_none() {
                break; // Game exited
            }

            if let Ok(Ok(event)) = rx.recv_timeout(Duration::from_millis(1000)) {
                match event.kind {
                    EventKind::Create(_) | EventKind::Modify(_) => {
                        for path in event.paths {
                            // Skip paths already under an already-detected root, and defer
                            // detection so Restart Manager isn't queried on every event.
                            if detected_roots.iter().any(|root| path.starts_with(root)) {
                                continue;
                            }
                            pending_paths.insert(path);
                        }
                    }
                    _ => {}
                }
            }

            // Coalesce detection runs to at most once every ~750ms.
            let due = match last_detection_check {
                Some(last) => last.elapsed().as_millis() >= 750,
                None => !pending_paths.is_empty(),
            };
            if due && !pending_paths.is_empty() {
                last_detection_check = Some(Instant::now());
                let paths: Vec<PathBuf> = pending_paths.drain().collect();
                for path in paths {
                    if detected_roots.iter().any(|root| path.starts_with(root)) {
                        continue;
                    }
                    let pids = get_locking_pids(&path);
                    for locking_pid in pids {
                        if is_descendant(&system, locking_pid, pid) {
                            if let Some(root) = get_save_root(&path, &base_dirs) {
                                log::info!("SaveGuard detected save root: {:?}", root);
                                detected_roots.insert(root);
                            }
                        }
                    }
                }
            }
        }

        log::info!("Game process exited. Backing up saves: {:?}", detected_roots);
        backup_saves(&detected_roots, &game_id, &app_handle);
    });
}

// ── Last-resort sweep ───────────────────────────────────────────────────────
// Everything above reacts to a write. This is for the session where nothing was
// ever attributed: a launcher started the game outside SILO's process tree, the
// game saved atomically so no handle could be queried, or it saved into a
// folder we never saw a write for. It is the difference between "no saves
// found" and "found them anyway".

/// Levels below a watched location the sweep looks at. One covers
/// `Saved Games\<game>`; two covers `Documents\My Games\<game>` and
/// `Saved Games\CD Projekt Red\Cyberpunk 2077`.
const MAX_SCAN_DEPTH: usize = 2;

/// Files examined inside a matching folder before the recency check gives up —
/// save folders hold a handful of files, so this only guards against one that
/// happens to point at something huge.
const MAX_SAVE_FOLDER_SCAN: usize = 500;

fn folder_touched_since(dir: &Path, since: std::time::SystemTime) -> bool {
    walkdir::WalkDir::new(dir)
        .max_depth(2)
        .into_iter()
        .flatten()
        .take(MAX_SAVE_FOLDER_SCAN)
        .filter(|entry| entry.file_type().is_file())
        .any(|entry| {
            entry
                .metadata()
                .ok()
                .and_then(|m| m.modified().ok())
                .map_or(false, |modified| modified >= since)
        })
}

/// Sweep the watched locations for the game's save folder. Prefers, in order:
/// 1. a folder that names the game and holds a file modified since `since`
///    (the game was actually writing there this session),
/// 2. otherwise the first folder that names the game at all — a game that
///    saved nothing yet still owns that folder, and it is where the next save
///    will land.
/// `exclude` (the game's install folder) is never adopted: backing up an
/// install because it happens to live under a watched location would be much
/// worse than finding no saves at all.
pub fn find_save_root_by_scan(
    base_dirs: &[PathBuf],
    name_keys: &[String],
    since: std::time::SystemTime,
    exclude: Option<&Path>,
) -> Option<PathBuf> {
    if name_keys.is_empty() {
        return None;
    }
    let mut named_only: Option<PathBuf> = None;
    for base_dir in base_dirs {
        let walk = walkdir::WalkDir::new(base_dir)
            .min_depth(1)
            .max_depth(MAX_SCAN_DEPTH)
            .into_iter()
            .filter_entry(|entry| entry.file_type().is_dir())
            .flatten();
        for entry in walk {
            let path = entry.path();
            if exclude.map_or(false, |skip| path == skip || path.starts_with(skip)) {
                continue;
            }
            let name = entry.file_name().to_string_lossy().to_lowercase();
            if is_save_root_excluded(&name) {
                continue;
            }
            if !name_keys.iter().any(|key| name_matches(&name, key)) {
                continue;
            }
            if folder_touched_since(path, since) {
                return Some(path.to_path_buf());
            }
            if named_only.is_none() {
                named_only = Some(path.to_path_buf());
            }
        }
    }
    named_only
}

pub fn backup_saves(roots: &HashSet<PathBuf>, game_id: &str, app_handle: &AppHandle) {
    if roots.is_empty() { return; }
    
    let saves_dir = app_handle.path().app_data_dir().unwrap().join("saves").join(game_id);
    let _ = fs::create_dir_all(&saves_dir);
    
    let timestamp = chrono::Local::now().format("%Y%m%d_%H%M%S").to_string();
    let zip_path = saves_dir.join(format!("{}.zip", timestamp));
    
    let zip_file = match File::create(&zip_path) {
        Ok(f) => f,
        Err(e) => {
            log::error!("Failed to create zip file: {}", e);
            return;
        }
    };
    
    let mut zip = zip::ZipWriter::new(zip_file);
    let options = FileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .unix_permissions(0o755);

    for root in roots {
        let root_name = root.file_name().unwrap().to_string_lossy().to_string();
        for entry in walkdir::WalkDir::new(root) {
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            let path = entry.path();
            
            let rel_path = match path.strip_prefix(root) {
                Ok(p) => p,
                Err(_) => continue,
            };
            
            let zip_internal_path = format!("{}/{}", root_name, rel_path.to_string_lossy().replace("\\", "/"));
            
            if path.is_file() {
                if let Ok(mut f) = File::open(path) {
                    if zip.start_file(&zip_internal_path, options).is_ok() {
                        let mut buffer = Vec::new();
                        if f.read_to_end(&mut buffer).is_ok() {
                            let _ = zip.write_all(&buffer);
                        }
                    }
                }
            } else if path.is_dir() && !rel_path.as_os_str().is_empty() {
                let _ = zip.add_directory(&zip_internal_path, options);
            }
        }
    }
    
    let _ = zip.finish();
    log::info!("Backup created at {:?}", zip_path);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_save_root_excluded_covers_known_cache_folders() {
        for folder in ["temp", "cache", "crashpad", "crash_reports", "logs", "shadercache", "nvidia"] {
            assert!(is_save_root_excluded(folder), "expected {:?} excluded", folder);
        }
    }

    #[test]
    fn is_save_root_excluded_allows_game_folders() {
        assert!(!is_save_root_excluded("Witcher 3"));
        assert!(!is_save_root_excluded("Borderlands 3"));
        assert!(!is_save_root_excluded("My Games"));
    }

    #[test]
    fn get_save_root_returns_first_component_under_base() {
        #[cfg(target_os = "windows")]
        {
            let base = PathBuf::from(r"C:\Users\Test\Saved Games");
            let path = PathBuf::from(r"C:\Users\Test\Saved Games\Witcher 3\saves\game0");
            assert_eq!(
                get_save_root(&path, &[base.clone()]),
                Some(PathBuf::from(r"C:\Users\Test\Saved Games\Witcher 3"))
            );
        }
        #[cfg(not(target_os = "windows"))]
        {
            let base = PathBuf::from("/home/test/Saved Games");
            let path = PathBuf::from("/home/test/Saved Games/Witcher 3/saves/game0");
            assert_eq!(
                get_save_root(&path, &[base.clone()]),
                Some(PathBuf::from("/home/test/Saved Games/Witcher 3"))
            );
        }
    }

    #[test]
    fn get_save_root_returns_none_for_excluded_first_component() {
        #[cfg(target_os = "windows")]
        {
            let base = PathBuf::from(r"C:\Users\Test\AppData\Local");
            let path = PathBuf::from(r"C:\Users\Test\AppData\Local\Temp\save\game0");
            assert_eq!(get_save_root(&path, &[base]), None);
        }
        #[cfg(not(target_os = "windows"))]
        {
            let base = PathBuf::from("/home/test/.local/share");
            let path = PathBuf::from("/home/test/.local/share/Temp/save/game0");
            assert_eq!(get_save_root(&path, &[base]), None);
        }
    }

    #[test]
    fn get_save_root_returns_none_outside_any_base() {
        #[cfg(target_os = "windows")]
        {
            let base = PathBuf::from(r"C:\Users\Test\Saved Games");
            let path = PathBuf::from(r"D:\Games\Witcher 3\saves");
            assert_eq!(get_save_root(&path, &[base]), None);
        }
        #[cfg(not(target_os = "windows"))]
        {
            let base = PathBuf::from("/home/test/Saved Games");
            let path = PathBuf::from("/mnt/games/Witcher 3/saves");
            assert_eq!(get_save_root(&path, &[base]), None);
        }
    }

    #[test]
    fn name_matches_folds_case_spaces_and_extensions() {
        assert!(name_matches("Cyberpunk 2077", "Cyberpunk 2077"));
        assert!(name_matches("cyberpunk2077", "Cyberpunk 2077"));
        assert!(name_matches("Cyberpunk2077", "Cyberpunk2077.exe"));
        assert!(name_matches("The Witcher 3", "The Witcher 3: Wild Hunt"));
    }

    #[test]
    fn name_matches_rejects_publishers_and_short_noise() {
        assert!(!name_matches("CD Projekt Red", "Cyberpunk 2077"));
        assert!(!name_matches("Saved Games", "Cyberpunk 2077"));
        assert!(!name_matches("GOG", "Cyberpunk 2077"));
        // Too short to be a safe substring match.
        assert!(!name_matches("Games", "Cyberpunk 2077"));
        // Non-game folders are excluded outright.
        assert!(!name_matches("cache", "Cyberpunk 2077"));
        // Sequels share a prefix and should still be recognised as the game.
        assert!(name_matches("MyGame", "My Game 2"));
    }

    #[test]
    fn publisher_nested_layout_resolves_to_the_game_folder() {
        let keys = vec!["Cyberpunk 2077".to_string()];
        #[cfg(target_os = "windows")]
        let (base, path, expected) = (
            PathBuf::from(r"C:\Users\Test\Saved Games"),
            PathBuf::from(r"C:\Users\Test\Saved Games\CD Projekt Red\Cyberpunk 2077\ManualSave-1\sav.dat"),
            PathBuf::from(r"C:\Users\Test\Saved Games\CD Projekt Red\Cyberpunk 2077"),
        );
        #[cfg(not(target_os = "windows"))]
        let (base, path, expected) = (
            PathBuf::from("/home/test/Saved Games"),
            PathBuf::from("/home/test/Saved Games/CD Projekt Red/Cyberpunk 2077/ManualSave-1/sav.dat"),
            PathBuf::from("/home/test/Saved Games/CD Projekt Red/Cyberpunk 2077"),
        );
        assert_eq!(get_save_root_for(&path, &[base.clone()], &keys), Some(expected.clone()));
        assert_eq!(save_root_by_name(&path, &[base], &keys), Some(expected));
    }

    #[test]
    fn name_aware_root_still_falls_back_to_first_folder() {
        let keys = vec!["Cyberpunk 2077".to_string()];
        #[cfg(target_os = "windows")]
        let (base, path, expected) = (
            PathBuf::from(r"C:\Users\Test\Documents"),
            PathBuf::from(r"C:\Users\Test\Documents\SomeUnknownGame\saves\slot1.dat"),
            PathBuf::from(r"C:\Users\Test\Documents\SomeUnknownGame"),
        );
        #[cfg(not(target_os = "windows"))]
        let (base, path, expected) = (
            PathBuf::from("/home/test/Documents"),
            PathBuf::from("/home/test/Documents/SomeUnknownGame/saves/slot1.dat"),
            PathBuf::from("/home/test/Documents/SomeUnknownGame"),
        );
        assert_eq!(get_save_root_for(&path, &[base.clone()], &keys), Some(expected));
        // …but the lock-free fallback refuses to claim an unrelated game's write.
        assert_eq!(save_root_by_name(&path, &[base], &keys), None);
    }

    #[test]
    fn save_root_by_name_ignores_paths_outside_every_base() {
        let keys = vec!["Cyberpunk 2077".to_string()];
        let base = PathBuf::from(if cfg!(target_os = "windows") {
            r"C:\Users\Test\Saved Games"
        } else {
            "/home/test/Saved Games"
        });
        let path = PathBuf::from(if cfg!(target_os = "windows") {
            r"D:\Games\Cyberpunk 2077\saves\sav.dat"
        } else {
            "/mnt/games/Cyberpunk 2077/saves/sav.dat"
        });
        assert_eq!(save_root_by_name(&path, &[base], &keys), None);
    }

    #[test]
    fn save_root_by_name_needs_a_name_key() {
        let path = PathBuf::from(if cfg!(target_os = "windows") {
            r"C:\Users\Test\Saved Games\Cyberpunk 2077\sav.dat"
        } else {
            "/home/test/Saved Games/Cyberpunk 2077/sav.dat"
        });
        let base = PathBuf::from(if cfg!(target_os = "windows") {
            r"C:\Users\Test\Saved Games"
        } else {
            "/home/test/Saved Games"
        });
        assert_eq!(save_root_by_name(&path, &[base], &[]), None);
    }

    // ----- find_save_root_by_scan (the "nothing was attributed" sweep) -----

    struct TestDir(PathBuf);

    impl TestDir {
        fn new(tag: &str) -> Self {
            let dir = std::env::temp_dir().join(format!("silo_save_scan_{}_{}", tag, std::process::id()));
            let _ = fs::remove_dir_all(&dir);
            fs::create_dir_all(&dir).unwrap();
            TestDir(dir)
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

    fn write_file(path: &Path) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, b"save").unwrap();
    }

    #[test]
    fn sweep_finds_a_publisher_nested_save_folder_touched_this_session() {
        let tmp = TestDir::new("nested");
        let base = tmp.path().join("Saved Games");
        let game_folder = base.join("CD Projekt Red").join("Cyberpunk 2077");
        write_file(&game_folder.join("ManualSave-1").join("sav.dat"));

        let keys = vec!["Cyberpunk 2077".to_string()];
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(86_400);
        assert_eq!(find_save_root_by_scan(&[base.clone()], &keys, long_ago, None), Some(game_folder.clone()));
        // A future session start means "nothing was touched since" -> still returns
        // the named folder, because it is where the next save will land.
        let future = std::time::SystemTime::now() + Duration::from_secs(3_600);
        assert_eq!(find_save_root_by_scan(&[base], &keys, future, None), Some(game_folder));
    }

    #[test]
    fn sweep_prefers_the_folder_touched_since_the_session_started() {
        let tmp = TestDir::new("recent");
        let base = tmp.path().join("Documents");
        let stale = base.join("Cyberpunk 2077 (old)");
        let fresh = base.join("Cyberpunk 2077");
        write_file(&stale.join("sav.dat"));
        write_file(&fresh.join("sav.dat"));
        // Backdate the decoy so only the real folder counts as touched.
        let old = std::time::SystemTime::now() - Duration::from_secs(86_400);
        filetime_backdate(&stale.join("sav.dat"), old);

        let keys = vec!["Cyberpunk 2077".to_string()];
        let since = std::time::SystemTime::now() - Duration::from_secs(600);
        assert_eq!(find_save_root_by_scan(&[base], &keys, since, None), Some(fresh));
    }

    #[test]
    fn sweep_never_adopts_the_install_folder_or_unrelated_folders() {
        let tmp = TestDir::new("exclude");
        let base = tmp.path().join("Documents");
        let install = base.join("Cyberpunk 2077");
        write_file(&install.join("Cyberpunk2077.exe"));
        write_file(&base.join("Other Game").join("sav.dat"));

        let keys = vec!["Cyberpunk 2077".to_string()];
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(86_400);
        assert_eq!(find_save_root_by_scan(&[base.clone()], &keys, long_ago, Some(&install)), None);
        assert_eq!(find_save_root_by_scan(&[base], &keys, long_ago, None), Some(install));
    }

    #[test]
    fn sweep_without_name_keys_finds_nothing() {
        let tmp = TestDir::new("nokeys");
        let base = tmp.path().join("Saved Games");
        write_file(&base.join("Cyberpunk 2077").join("sav.dat"));
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(86_400);
        assert_eq!(find_save_root_by_scan(&[base], &[], long_ago, None), None);
    }

    /// Backdate a file so the recency check has something old to reject.
    fn filetime_backdate(path: &Path, when: std::time::SystemTime) {
        let file = fs::OpenOptions::new().write(true).open(path).unwrap();
        file.set_modified(when).unwrap();
    }

    #[test]
    #[ignore = "manual check: reads the real machine's save locations"]
    fn manual_check_real_machine() {
        let mut bases = Vec::new();
        if let Ok(p) = std::env::var("USERPROFILE") {
            bases.push(PathBuf::from(&p).join("Saved Games"));
        }
        if let Ok(p) = std::env::var("LOCALAPPDATA") {
            bases.push(PathBuf::from(&p));
        }
        let keys = vec!["Cyberpunk 2077".to_string()];
        let long_ago = std::time::SystemTime::now() - Duration::from_secs(86_400 * 365);
        println!("scan -> {:?}", find_save_root_by_scan(&bases, &keys, long_ago, None));
        let sample = bases[0]
            .join("CD Projekt Red")
            .join("Cyberpunk 2077")
            .join("AutoSave-0")
            .join("sav.dat");
        println!("by name -> {:?}", save_root_by_name(&sample, &bases, &keys));
        println!("old behaviour -> {:?}", get_save_root(&sample, &bases));
    }

    #[test]
    fn is_descendant_true_when_target_equals_ancestor() {
        let system = System::new();
        assert!(is_descendant(&system, 4321, 4321));
    }

    #[test]
    fn is_descendant_false_for_unknown_pids() {
        let system = System::new(); // no process table loaded -> process() returns None
        assert!(!is_descendant(&system, 99_999_999, 1));
        assert!(!is_descendant(&system, 1, 99_999_999));
    }

    #[test]
    fn pid_is_descendant_of_self_is_true() {
        let mut system = System::new();
        assert!(pid_is_descendant_of(&mut system, 4321, 4321));
    }

    #[test]
    fn pid_is_descendant_of_false_for_unknown_pids() {
        // Unknown PIDs can't be refreshed -> the climb stops -> not a descendant.
        let mut system = System::new();
        assert!(!pid_is_descendant_of(&mut system, 99_999_999, 1));
    }
}
