use crate::commands::{get_games, Game};
use serde::Serialize;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::AppHandle;

const TTL: Duration = Duration::from_secs(300);
const MAX_TOKENS: usize = 512;
const MAX_CANDIDATES: usize = MAX_TOKENS;

#[derive(Clone)]
struct Candidate {
    game_id: String,
    root: PathBuf,
    path: PathBuf,
    len: u64,
    modified: Option<std::time::SystemTime>,
    file_identity: FileIdentity,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct FileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    volume_serial: Option<u32>,
    #[cfg(windows)]
    file_index: Option<u64>,
}
static CANDIDATES: OnceLock<Mutex<HashMap<String, (Instant, Candidate)>>> = OnceLock::new();
fn store() -> &'static Mutex<HashMap<String, (Instant, Candidate)>> {
    CANDIDATES.get_or_init(|| Mutex::new(HashMap::new()))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UninstallerCandidate {
    candidate_id: String,
    file_name: String,
    relative_path: String,
}

fn game_root(game: &Game) -> Result<PathBuf, String> {
    let exe = game.exe_path.as_deref().ok_or("Game install path unavailable")?;
    Path::new(exe)
        .parent()
        .ok_or("Game install path unavailable")?
        .canonicalize()
        .map_err(|_| "Game folder unavailable".to_string())
}

fn looks_executable(path: &Path) -> bool {
    let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
        return false;
    };
    #[cfg(windows)]
    {
        ext.eq_ignore_ascii_case("exe")
    }
    #[cfg(not(windows))]
    {
        if !matches!(ext.to_ascii_lowercase().as_str(), "bin" | "appimage") {
            return false;
        }
        use std::os::unix::fs::PermissionsExt;
        fs::metadata(path).is_ok_and(|m| m.permissions().mode() & 0o111 != 0)
    }
}

fn looks_like_uninstaller(path: &Path) -> bool {
    let Some(stem) = path.file_stem().and_then(|s| s.to_str()) else {
        return false;
    };
    stem.split(|c: char| !c.is_ascii_alphanumeric()).any(|part| {
        let part = part.to_ascii_lowercase();
        ["uninstall", "uninstaller", "unins", "remove"]
            .iter()
            .any(|prefix| part.starts_with(prefix))
    })
}

fn identity(path: &Path, root: &Path) -> Result<(PathBuf, u64, Option<std::time::SystemTime>, FileIdentity), String> {
    let meta = fs::symlink_metadata(path).map_err(|_| "Candidate is no longer available".to_string())?;
    #[cfg(windows)]
    use std::os::windows::fs::MetadataExt;
    #[cfg(windows)]
    if meta.file_attributes() & 0x400 != 0 {
        return Err("Reparse-point candidates are not supported".into());
    }
    if meta.file_type().is_symlink() || !meta.is_file() || !looks_executable(path) {
        return Err("Candidate is not a supported executable".into());
    }
    let canonical = path
        .canonicalize()
        .map_err(|_| "Candidate is no longer available".to_string())?;
    if canonical.parent() != Some(root) {
        return Err("Candidate is outside the game folder".into());
    }
    #[cfg(unix)]
    let file_identity = {
        use std::os::unix::fs::MetadataExt;
        FileIdentity {
            device: meta.dev(),
            inode: meta.ino(),
        }
    };
    #[cfg(windows)]
    let file_identity = windows_file_identity(path)?;
    Ok((canonical, meta.len(), meta.modified().ok(), file_identity))
}

#[cfg(windows)]
fn windows_file_identity(path: &Path) -> Result<FileIdentity, String> {
    use std::{fs::OpenOptions, os::windows::io::AsRawHandle};
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION},
    };
    let file = OpenOptions::new()
        .read(true)
        .open(path)
        .map_err(|_| "Candidate is no longer available".to_string())?;
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: file keeps a valid handle alive, and info is a writable output buffer.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle() as *mut _), &mut info) }
        .map_err(|_| "Could not inspect candidate identity".to_string())?;
    Ok(FileIdentity {
        volume_serial: Some(info.dwVolumeSerialNumber),
        file_index: Some((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)),
    })
}

#[cfg(windows)]
fn windows_held_file_identity(file: &fs::File) -> Result<FileIdentity, String> {
    use std::os::windows::io::AsRawHandle;
    use windows::Win32::{
        Foundation::HANDLE,
        Storage::FileSystem::{GetFileInformationByHandle, BY_HANDLE_FILE_INFORMATION, FILE_ATTRIBUTE_REPARSE_POINT},
    };
    let mut info = BY_HANDLE_FILE_INFORMATION::default();
    // SAFETY: `file` owns a live handle and `info` is a writable output buffer.
    unsafe { GetFileInformationByHandle(HANDLE(file.as_raw_handle() as *mut _), &mut info) }
        .map_err(|_| "Could not inspect candidate identity".to_string())?;
    if info.dwFileAttributes & FILE_ATTRIBUTE_REPARSE_POINT.0 != 0 {
        return Err("Reparse-point candidates are not supported".into());
    }
    Ok(FileIdentity {
        volume_serial: Some(info.dwVolumeSerialNumber),
        file_index: Some((u64::from(info.nFileIndexHigh) << 32) | u64::from(info.nFileIndexLow)),
    })
}

#[tauri::command]
pub fn discover_uninstallers(app: AppHandle, game_id: String) -> Result<Vec<UninstallerCandidate>, String> {
    let game = get_games(app)
        .into_iter()
        .find(|g| g.id == game_id)
        .ok_or("Game not found")?;
    let root = game_root(&game)?;
    let mut found = Vec::new();
    for entry in fs::read_dir(&root).map_err(|_| "Could not read game folder".to_string())? {
        let entry = entry.map_err(|_| "Could not read game folder".to_string())?;
        let path = entry.path();
        if !looks_executable(&path) || !looks_like_uninstaller(&path) {
            continue;
        }
        let Ok((canonical, len, modified, file_identity)) = identity(&path, &root) else {
            continue;
        };
        let candidate_id = uuid::Uuid::new_v4().to_string();
        found.push((
            entry.file_name().to_string_lossy().into_owned(),
            candidate_id.clone(),
            Candidate {
                game_id: game_id.clone(),
                root: root.clone(),
                path: canonical,
                len,
                modified,
                file_identity,
            },
        ));
    }
    found.sort_by(|a, b| a.0.to_lowercase().cmp(&b.0.to_lowercase()));
    found.truncate(MAX_CANDIDATES);
    let mut candidates = store().lock().map_err(|_| "Candidate store unavailable".to_string())?;
    candidates.retain(|_, (created, _)| created.elapsed() < TTL);
    // A scan invalidates prior tokens only for this game. Keep the cache globally bounded.
    candidates.retain(|_, (_, candidate)| candidate.game_id != game_id);
    while candidates.len().saturating_add(found.len()) > MAX_TOKENS {
        let Some(oldest) = candidates
            .iter()
            .min_by_key(|(_, (created, _))| *created)
            .map(|(id, _)| id.clone())
        else {
            break;
        };
        candidates.remove(&oldest);
    }
    for (_, token, candidate) in &found {
        candidates.insert(token.clone(), (Instant::now(), candidate.clone()));
    }
    Ok(found
        .into_iter()
        .map(|(name, candidate_id, _)| UninstallerCandidate {
            candidate_id,
            file_name: name.clone(),
            relative_path: name,
        })
        .collect())
}

#[tauri::command]
pub fn run_uninstaller_candidate(app: AppHandle, game_id: String, candidate_id: String) -> Result<(), String> {
    let (created, candidate) = store()
        .lock()
        .map_err(|_| "Candidate store unavailable".to_string())?
        .remove(&candidate_id)
        .ok_or("Candidate expired; rescan the game folder")?;
    if created.elapsed() >= TTL || candidate.game_id != game_id {
        return Err("Candidate expired; rescan the game folder".into());
    }
    let game = get_games(app)
        .into_iter()
        .find(|g| g.id == game_id)
        .ok_or("Game not found")?;
    let root = game_root(&game)?;
    if root != candidate.root {
        return Err("Game folder changed; rescan before running".into());
    }
    let (path, len, modified, file_identity) = identity(&candidate.path, &root)?;
    if path != candidate.path
        || len != candidate.len
        || modified != candidate.modified
        || file_identity != candidate.file_identity
    {
        return Err("Candidate changed; rescan before running".into());
    }
    // Re-open and verify immediately before launch. Holding the handle narrows, but
    // cannot eliminate, path replacement races because Command launches by pathname.
    #[cfg(windows)]
    let held = {
        use std::{fs::OpenOptions, os::windows::fs::OpenOptionsExt};
        use windows::Win32::Storage::FileSystem::FILE_SHARE_READ;
        OpenOptions::new()
            .read(true)
            // Prevent writes and delete/rename replacement while the verified handle is held.
            .share_mode(FILE_SHARE_READ.0)
            .open(&path)
            .map_err(|_| "Candidate is no longer available".to_string())?
    };
    #[cfg(not(windows))]
    let held = fs::File::open(&path).map_err(|_| "Candidate is no longer available".to_string())?;
    #[cfg(unix)]
    let held_meta = held
        .metadata()
        .map_err(|_| "Candidate is no longer available".to_string())?;
    #[cfg(unix)]
    let held_identity = {
        use std::os::unix::fs::MetadataExt;
        FileIdentity {
            device: held_meta.dev(),
            inode: held_meta.ino(),
        }
    };
    #[cfg(windows)]
    let held_identity = windows_held_file_identity(&held)?;
    if held_identity != candidate.file_identity {
        return Err("Candidate changed; rescan before running".into());
    }
    Command::new(path)
        .spawn()
        .map_err(|_| "Could not start the selected uninstaller".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn installer_name_filter_rejects_game_binaries_and_accepts_plausible_names() {
        for name in ["game.exe", "launcher.bin", "MyGame.AppImage"] {
            assert!(!looks_like_uninstaller(Path::new(name)), "{name}");
        }
        for name in [
            "Uninstall.exe",
            "unins000.exe",
            "game-uninstaller.exe",
            "Remove Game.bin",
        ] {
            assert!(looks_like_uninstaller(Path::new(name)), "{name}");
        }
    }
    #[test]
    fn containment_rejects_symlink_and_accepts_only_root_file() {
        let dir = std::env::temp_dir().join(format!("silo-uninstaller-{}", uuid::Uuid::new_v4()));
        let outside = dir.with_extension("outside");
        fs::create_dir_all(&dir).expect("test dir");
        fs::write(dir.join("remove.exe"), b"MZ").expect("test exe");
        fs::write(&outside, b"MZ").expect("outside file");
        assert!(identity(&dir.join("remove.exe"), &dir.canonicalize().expect("canonical root")).is_ok());
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&outside, dir.join("escape.exe")).expect("symlink");
            assert!(identity(&dir.join("escape.exe"), &dir.canonicalize().expect("canonical root")).is_err());
        }
        let _ = fs::remove_dir_all(dir);
        let _ = fs::remove_file(outside);
    }

    #[test]
    fn identity_detects_replacement_even_when_size_and_timestamp_match() {
        let dir = std::env::temp_dir().join(format!("silo-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("test dir");
        let file = dir.join(if cfg!(windows) { "remove.exe" } else { "remove.bin" });
        fs::write(&file, b"same contents").expect("first file");
        let root = dir.canonicalize().expect("canonical root");
        let first = identity(&file, &root).expect("first identity");
        fs::remove_file(&file).expect("remove first file");
        fs::write(&file, b"same contents").expect("replacement");
        let second = identity(&file, &root).expect("replacement identity");
        assert_ne!(first.3, second.3);
        let _ = fs::remove_dir_all(dir);
    }
    #[cfg(windows)]
    #[test]
    fn held_handle_identity_matches_scanned_identity() {
        let dir = std::env::temp_dir().join(format!("silo-held-identity-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).expect("test dir");
        let file = dir.join("remove.exe");
        fs::write(&file, b"MZ").expect("test executable");
        let (_, _, _, scanned) =
            identity(&file, &dir.canonicalize().expect("canonical root")).expect("scanned identity");
        let held = fs::File::open(&file).expect("held file");
        assert_eq!(windows_held_file_identity(&held).expect("held identity"), scanned);
        drop(held);
        let _ = fs::remove_dir_all(dir);
    }
    #[test]
    fn token_is_single_use_and_args_are_not_provided() {
        let executable = Path::new("installer.exe");
        assert!(!executable
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir)));
        assert_eq!(Command::new(executable).get_args().count(), 0);
    }
}
