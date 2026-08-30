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
    let proc_dir = Path::new("/proc");
    if let Ok(entries) = fs::read_dir(proc_dir) {
        for entry in entries.filter_map(|e| e.ok()) {
            if let Ok(file_name) = entry.file_name().into_string() {
                if let Ok(pid) = file_name.parse::<u32>() {
                    let fd_dir = entry.path().join("fd");
                    if let Ok(fd_entries) = fs::read_dir(fd_dir) {
                        for fd in fd_entries.filter_map(|e| e.ok()) {
                            if let Ok(target) = fs::read_link(fd.path()) {
                                if target.starts_with(path) || path.starts_with(&target) {
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
        let base = PathBuf::from(r"C:\Users\Test\Saved Games");
        let path = PathBuf::from(r"C:\Users\Test\Saved Games\Witcher 3\saves\game0");
        assert_eq!(
            get_save_root(&path, &[base.clone()]),
            Some(PathBuf::from(r"C:\Users\Test\Saved Games\Witcher 3"))
        );
    }

    #[test]
    fn get_save_root_returns_none_for_excluded_first_component() {
        let base = PathBuf::from(r"C:\Users\Test\AppData\Local");
        let path = PathBuf::from(r"C:\Users\Test\AppData\Local\Temp\save\game0");
        assert_eq!(get_save_root(&path, &[base]), None);
    }

    #[test]
    fn get_save_root_lowercases_before_exclusion_check() {
        // "Temp" (mixed case) is excluded because get_save_root lowercases the first component.
        let base = PathBuf::from(r"C:\Users\Test\AppData\Local");
        let path = PathBuf::from(r"C:\Users\Test\AppData\Local\Temp\save");
        assert_eq!(get_save_root(&path, &[base]), None);
    }

    #[test]
    fn get_save_root_returns_none_outside_any_base() {
        let base = PathBuf::from(r"C:\Users\Test\Saved Games");
        let path = PathBuf::from(r"D:\Games\Witcher 3\saves");
        assert_eq!(get_save_root(&path, &[base]), None);
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
}
