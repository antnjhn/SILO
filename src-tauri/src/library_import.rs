use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ImportCandidate {
  pub name: String,
  pub source: String,
  #[serde(rename = "appId")]
  pub app_id: Option<String>,
  #[serde(rename = "exePath")]
  pub exe_path: Option<String>,
}

#[tauri::command]
pub async fn import_steam_library() -> Vec<ImportCandidate> {
  #[cfg(target_os = "windows")]
  { imp::steam_library() }
  #[cfg(not(target_os = "windows"))]
  { linux_imp::steam_library() }
}

#[tauri::command]
pub async fn import_epic_library() -> Vec<ImportCandidate> {
  #[cfg(target_os = "windows")]
  { imp::epic_library() }
  #[cfg(not(target_os = "windows"))]
  { linux_imp::heroic_epic_library() }
}

#[tauri::command]
pub async fn import_gog_library() -> Vec<ImportCandidate> {
  #[cfg(target_os = "windows")]
  { imp::gog_library() }
  #[cfg(not(target_os = "windows"))]
  { linux_imp::heroic_gog_library() }
}

#[cfg(target_os = "windows")]
mod imp {
  use super::ImportCandidate;
  use std::collections::HashSet;
  use std::fs;
  use std::path::{Path, PathBuf};

  // ----- Steam -----

  fn steam_install_path() -> PathBuf {
    use winreg::enums::*;
    use winreg::RegKey;
    if let Ok(steam) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\WOW6432Node\Valve\Steam") {
      if let Ok(path) = steam.get_value::<String, _>("SteamPath") {
        return PathBuf::from(path);
      }
    }
    if let Ok(steam) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(r"SOFTWARE\Valve\Steam") {
      if let Ok(path) = steam.get_value::<String, _>("SteamPath") {
        return PathBuf::from(path);
      }
    }
    PathBuf::from(r"C:\Program Files (x86)\Steam")
  }

  fn steam_library_folders(steam_path: &Path) -> Vec<PathBuf> {
    let mut folders = Vec::new();
    let main = steam_path.join("steamapps");
    if main.is_dir() {
      folders.push(main);
    }
    let vdf_path = steam_path.join("steamapps").join("libraryfolders.vdf");
    if let Ok(data) = fs::read_to_string(&vdf_path) {
      if let Some(root) = vdf_parse(&data) {
        for (_, value) in &root {
          if let VdfValue::Map(indices) = value {
            for (_, index_value) in indices {
              if let VdfValue::Map(folder_entries) = index_value {
                if let Some(VdfValue::Str(path)) = vdf_get(folder_entries, "path") {
                  let library = PathBuf::from(path).join("steamapps");
                  if library.is_dir() && !folders.contains(&library) {
                    folders.push(library);
                  }
                }
              }
            }
          }
        }
      }
    }
    folders
  }

  pub fn steam_library() -> Vec<ImportCandidate> {
    let mut candidates = Vec::new();
    for steamapps in steam_library_folders(&steam_install_path()) {
      let Ok(entries) = fs::read_dir(&steamapps) else {
        continue;
      };
      for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()) else {
          continue;
        };
        if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") {
          continue;
        }
        let app_id = file_name.trim_start_matches("appmanifest_").trim_end_matches(".acf").to_string();
        let Ok(data) = fs::read_to_string(&path) else {
          continue;
        };
        let Some(root) = vdf_parse(&data) else {
          continue;
        };
        let Some(app_state) = root.iter().find_map(|(_, value)| match value {
          VdfValue::Map(entries) => Some(entries),
          VdfValue::Str(_) => None,
        }) else {
          continue;
        };
        let name = vdf_get(app_state, "name").and_then(|v| vdf_str(v)).unwrap_or_else(|| app_id.clone());
        let exe_path = vdf_get(app_state, "installdir")
          .and_then(|v| vdf_str(v))
          .and_then(|dir| find_best_game_exe(&steamapps.join("common").join(&dir)))
          .map(|p| p.to_string_lossy().into_owned());
        candidates.push(ImportCandidate {
          name,
          source: "Steam".to_string(),
          app_id: Some(app_id),
          exe_path,
        });
      }
    }
    candidates.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    candidates
  }

  // ----- Epic -----

  fn epic_manifests_dir() -> PathBuf {
    if let Ok(program_data) = std::env::var("PROGRAMDATA") {
      return PathBuf::from(program_data)
        .join("Epic")
        .join("EpicGamesLauncher")
        .join("Data")
        .join("Manifests");
    }
    PathBuf::new()
  }

  #[derive(serde::Deserialize)]
  struct EpicManifest {
    #[serde(rename = "DisplayName")]
    display_name: Option<String>,
    #[serde(rename = "AppName")]
    app_name: Option<String>,
    #[serde(rename = "InstallLocation")]
    install_location: Option<String>,
    #[serde(rename = "LaunchExecutable")]
    launch_executable: Option<String>,
    #[serde(rename = "MainGameExecutable")]
    main_game_executable: Option<String>,
  }

  pub fn epic_library() -> Vec<ImportCandidate> {
    let mut candidates = Vec::new();
    let manifest_dir = epic_manifests_dir();
    if !manifest_dir.is_dir() {
      return candidates;
    }
    let Ok(entries) = fs::read_dir(&manifest_dir) else {
      return candidates;
    };
    for entry in entries.filter_map(|e| e.ok()) {
      let path = entry.path();
      if path.extension().and_then(|e| e.to_str()) != Some("item") {
        continue;
      }
      let Ok(data) = fs::read_to_string(&path) else {
        continue;
      };
      let Ok(manifest) = serde_json::from_str::<EpicManifest>(&data) else {
        continue;
      };
      let Some(name) = manifest.display_name else {
        continue;
      };
      if name.trim().is_empty() {
        continue;
      }

      let mut exe_path = None;
      if let (Some(install), Some(launch)) = (&manifest.install_location, &manifest.launch_executable) {
        let candidate = Path::new(install).join(launch);
        if candidate.exists() {
          exe_path = Some(candidate.to_string_lossy().into_owned());
        }
      }
      if exe_path.is_none() {
        if let (Some(install), Some(main)) = (&manifest.install_location, &manifest.main_game_executable) {
          let candidate = Path::new(install).join(main);
          if candidate.exists() {
            exe_path = Some(candidate.to_string_lossy().into_owned());
          }
        }
      }

      candidates.push(ImportCandidate {
        name,
        source: "Epic".to_string(),
        app_id: manifest.app_name,
        exe_path,
      });
    }
    candidates.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    candidates
  }

  // ----- GOG -----

  fn gog_registry_candidates() -> Vec<ImportCandidate> {
    use winreg::enums::*;
    use winreg::RegKey;
    let mut candidates = Vec::new();
    let roots = [
      r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall",
      r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall",
    ];
    for root in roots {
      let Ok(key) = RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(root) else {
        continue;
      };
      for subkey_name in key.enum_keys().filter_map(|r| r.ok()) {
        let Ok(app) = key.open_subkey(&subkey_name) else {
          continue;
        };
        let Some(name) = app.get_value::<String, _>("DisplayName").ok() else {
          continue;
        };
        if name.trim().is_empty() {
          continue;
        }
        let publisher: Option<String> = app.get_value("Publisher").ok();
        let install_location: Option<String> = app.get_value("InstallLocation").ok();
        let display_icon: Option<String> = app.get_value("DisplayIcon").ok();

        let is_gog = publisher.as_deref().map(|p| p.to_lowercase().contains("gog")).unwrap_or(false)
          || name.to_lowercase().contains("gog")
          || install_location.as_deref().map(|p| p.to_lowercase().contains("gog games")).unwrap_or(false);
        if !is_gog {
          continue;
        }

        let mut exe_path = None;
        if let Some(icon) = &display_icon {
          let icon_path = icon.split(',').next().unwrap_or(icon);
          let icon = PathBuf::from(icon_path);
          if icon.extension().and_then(|e| e.to_str()) == Some("exe") && icon.exists() {
            exe_path = Some(icon.to_string_lossy().into_owned());
          }
        }
        if exe_path.is_none() {
          if let Some(loc) = &install_location {
            let loc = Path::new(loc);
            if loc.is_dir() {
              exe_path = find_best_game_exe(loc).map(|p| p.to_string_lossy().into_owned());
            }
          }
        }

        candidates.push(ImportCandidate {
          name,
          source: "GOG".to_string(),
          app_id: None,
          exe_path,
        });
      }
    }
    candidates
  }

  pub fn gog_library() -> Vec<ImportCandidate> {
    let mut candidates = gog_registry_candidates();

    let gog_root = PathBuf::from(r"C:\GOG Games");
    if gog_root.is_dir() {
      if let Ok(entries) = fs::read_dir(&gog_root) {
        for entry in entries.filter_map(|e| e.ok()) {
          let path = entry.path();
          if !path.is_dir() {
            continue;
          }
          let name = path.file_name().unwrap_or_default().to_string_lossy().to_string();
          if name.is_empty() {
            continue;
          }
          let exe_path = find_best_game_exe(&path).map(|p| p.to_string_lossy().into_owned());
          candidates.push(ImportCandidate {
            name,
            source: "GOG".to_string(),
            app_id: None,
            exe_path,
          });
        }
      }
    }

    // Deduplicate by resolved exe path.
    let mut seen = HashSet::new();
    candidates.retain(|c| match &c.exe_path {
      Some(path) => seen.insert(path.to_lowercase()),
      None => true,
    });
    candidates.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    candidates
  }

  // ----- shared exe scanning -----

  fn find_best_game_exe(game_dir: &Path) -> Option<PathBuf> {
    super::shared::find_best_game_exe(game_dir)
  }

  // ----- minimal Valve VDF parser -----

  enum VdfValue {
    Str(String),
    Map(Vec<(String, VdfValue)>),
  }

  fn vdf_str(value: &VdfValue) -> Option<String> {
    match value {
      VdfValue::Str(s) => Some(s.clone()),
      VdfValue::Map(_) => None,
    }
  }

  fn vdf_get<'a>(map: &'a [(String, VdfValue)], key: &str) -> Option<&'a VdfValue> {
    map.iter().find(|(k, _)| k == key).map(|(_, v)| v)
  }

  fn vdf_tokenize(input: &str) -> Vec<String> {
    let bytes = input.as_bytes();
    let mut tokens = Vec::new();
    let mut pos = 0;
    while pos < bytes.len() {
      let byte = bytes[pos];
      if byte.is_ascii_whitespace() {
        pos += 1;
      } else if byte == b'{' {
        tokens.push("{".to_string());
        pos += 1;
      } else if byte == b'}' {
        tokens.push("}".to_string());
        pos += 1;
      } else if byte == b'"' {
        pos += 1;
        let mut value = String::new();
        let mut closed = false;
        while pos < bytes.len() {
          let ch = bytes[pos];
          if ch == b'\\' {
            pos += 1;
            if pos < bytes.len() {
              match bytes[pos] {
                b'n' => value.push('\n'),
                b't' => value.push('\t'),
                b'r' => value.push('\r'),
                b'"' => value.push('"'),
                b'\\' => value.push('\\'),
                other => value.push(other as char),
              }
              pos += 1;
            }
          } else if ch == b'"' {
            pos += 1;
            closed = true;
            break;
          } else {
            value.push(ch as char);
            pos += 1;
          }
        }
        if closed {
          tokens.push(value);
        }
      } else {
        pos += 1; // Tolerate stray unquoted bytes.
      }
    }
    tokens
  }

  fn vdf_parse_map(tokens: &[String], pos: &mut usize) -> Option<(String, VdfValue)> {
    let key = tokens.get(*pos)?.clone();
    *pos += 1;
    if tokens.get(*pos).map(|t| t.as_str()) == Some("{") {
      *pos += 1;
      let mut entries = Vec::new();
      loop {
        match tokens.get(*pos).map(|t| t.as_str()) {
          Some("}") => {
            *pos += 1;
            break;
          }
          Some(_) => {
            let (k, v) = vdf_parse_map(tokens, pos)?;
            entries.push((k, v));
          }
          None => return None,
        }
      }
      Some((key, VdfValue::Map(entries)))
    } else {
      let value = tokens.get(*pos)?.clone();
      *pos += 1;
      Some((key, VdfValue::Str(value)))
    }
  }

  fn vdf_parse(input: &str) -> Option<Vec<(String, VdfValue)>> {
    let tokens = vdf_tokenize(input);
    if tokens.is_empty() {
      return None;
    }
    let mut pos = 0;
    let mut root = Vec::new();
    while pos < tokens.len() {
      let (k, v) = vdf_parse_map(&tokens, &mut pos)?;
      root.push((k, v));
    }
    Some(root)
  }

  #[cfg(test)]
  mod tests {
    use super::*;

    #[test]
    fn vdf_parse_reads_nested_acf() {
      let data = "\"AppState\"\n{\n\t\"appid\"\t\t\"1234\"\n\t\"name\"\t\t\"Witcher 3\"\n\t\"UserConfig\"\n\t{\n\t\t\"Language\"\t\t\"english\"\n\t}\n}\n";
      let root = vdf_parse(data).expect("valid acf should parse");
      let state = root.iter().find(|(k, _)| k == "AppState").expect("AppState key");
      let map = match &state.1 {
        VdfValue::Map(m) => m,
        _ => panic!("AppState should be a map"),
      };
      assert_eq!(vdf_str(vdf_get(map, "appid").unwrap()).unwrap(), "1234");
      assert_eq!(vdf_str(vdf_get(map, "name").unwrap()).unwrap(), "Witcher 3");
      match vdf_get(map, "UserConfig").unwrap() {
        VdfValue::Map(uc) => {
          assert_eq!(vdf_str(vdf_get(uc, "Language").unwrap()).unwrap(), "english");
        }
        _ => panic!("UserConfig should be a map"),
      }
    }

    #[test]
    fn vdf_parse_unbalanced_braces_returns_none() {
      // Missing closing brace.
      assert!(vdf_parse("\"AppState\" { \"name\" \"Game\"").is_none());
      // Extra trailing closing brace.
      assert!(vdf_parse("\"AppState\" { \"name\" \"Game\" } }").is_none());
      // Map opened but never closed.
      assert!(vdf_parse("\"AppState\" {").is_none());
    }

    #[test]
    fn vdf_parse_empty_input_returns_none() {
      assert!(vdf_parse("").is_none());
      assert!(vdf_parse("   \n\t ").is_none());
    }

    #[test]
    fn vdf_parse_flat_key_value_works() {
      let root = vdf_parse("\"key\" \"value\"").expect("flat kv should parse");
      let (k, v) = root.first().unwrap();
      assert_eq!(k, "key");
      assert_eq!(vdf_str(v).unwrap(), "value");
    }

    #[test]
    fn vdf_tokenize_handles_escapes() {
      assert_eq!(vdf_tokenize(r#""a\nb""#), vec!["a\nb"]);
      assert_eq!(vdf_tokenize(r#""a\tb""#), vec!["a\tb"]);
      assert_eq!(vdf_tokenize(r#""a\"b""#), vec!["a\"b"]);
      assert_eq!(vdf_tokenize(r#""a\\b""#), vec!["a\\b"]);
    }

    #[test]
    fn vdf_tokenize_drops_unterminated_strings() {
      assert_eq!(vdf_tokenize("\"unterminated"), Vec::<String>::new());
    }

    #[test]
    fn vdf_parse_libraryfolders_shape() {
      let data = "\"libraryfolders\"\n{\n\t\"1\"\n\t{\n\t\t\"path\"\t\t\"D:\\\\SteamLibrary\"\n\t}\n}";
      let root = vdf_parse(data).expect("libraryfolders should parse");
      let (_, folders) = root.iter().find(|(k, _)| k == "libraryfolders").unwrap();
      let entries = match folders {
        VdfValue::Map(m) => m,
        _ => panic!("libraryfolders should be a map"),
      };
      let one = entries.iter().find(|(k, _)| k == "1").expect("library 1");
      match &one.1 {
        VdfValue::Map(folder) => {
          assert_eq!(
            vdf_str(vdf_get(folder, "path").unwrap()).unwrap(),
            r"D:\SteamLibrary"
          );
        }
        _ => panic!("library 1 should be a map"),
      }
    }
  }
}

// ── Shared exe scanner (used by both Windows imp and Linux linux_imp) ────────
mod shared {
  use std::path::{Path, PathBuf};
  use std::fs;
  use walkdir::WalkDir;

  #[cfg(target_os = "windows")]
  const GAME_EXTS: &[&str] = &["exe"];
  #[cfg(not(target_os = "windows"))]
  const GAME_EXTS: &[&str] = &["exe", "sh", "x86_64", "bin", "AppImage", "appimage"];

  pub fn find_best_game_exe(game_dir: &Path) -> Option<PathBuf> {
    let dir_name = game_dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    let mut best: Option<(i32, PathBuf)> = None;
    for entry in WalkDir::new(game_dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
      let path = entry.path();
      if !path.is_file() { continue; }
      let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
      if !GAME_EXTS.contains(&ext.as_str()) { continue; }
      let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
      if crate::commands::is_blacklisted_exe(&file_name) { continue; }
      let Ok(metadata) = fs::metadata(path) else { continue; };
      // On Linux scripts can be small — skip tiny files only for Windows .exe
      #[cfg(target_os = "windows")]
      if metadata.len() < 500_000 { continue; }
      let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
      let score = crate::commands::score_exe_candidate(&stem, &dir_name, entry.depth(), metadata.len());
      if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
        best = Some((score, path.to_path_buf()));
      }
    }
    best.map(|(_, path)| path)
  }
}

// ── Linux library import ─────────────────────────────────────────────────────
#[cfg(not(target_os = "windows"))]
mod linux_imp {
  use super::{ImportCandidate, shared};
  use std::collections::HashSet;
  use std::fs;
  use std::path::PathBuf;

  fn steam_paths() -> Vec<PathBuf> {
    let mut paths = Vec::new();
    if let Ok(home) = std::env::var("HOME") {
      let h = PathBuf::from(&home);
      // Native Steam
      paths.push(h.join(".local/share/Steam"));
      paths.push(h.join(".steam/steam"));
      // Flatpak Steam
      paths.push(h.join(".var/app/com.valvesoftware.Steam/.local/share/Steam"));
    }
    // XDG_DATA_HOME override
    if let Ok(xdg) = std::env::var("XDG_DATA_HOME") {
      paths.push(PathBuf::from(xdg).join("Steam"));
    }
    paths
  }

  pub fn steam_library() -> Vec<ImportCandidate> {
    let mut candidates = Vec::new();
    let mut seen = HashSet::new();

    for steam_root in steam_paths() {
      let steamapps = steam_root.join("steamapps");
      if !steamapps.is_dir() { continue; }

      // Parse extra library folders from libraryfolders.vdf
      let mut lib_dirs = vec![steamapps.clone()];
      let vdf_path = steamapps.join("libraryfolders.vdf");
      if let Ok(data) = fs::read_to_string(&vdf_path) {
        // Simple key-value scan for "path" entries
        for line in data.lines() {
          let trimmed = line.trim();
          if trimmed.starts_with('"') {
            let parts: Vec<&str> = trimmed.splitn(4, '"').collect();
            if parts.len() >= 4 && parts[1] == "path" {
              let extra = PathBuf::from(parts[3]).join("steamapps");
              if extra.is_dir() && !lib_dirs.contains(&extra) {
                lib_dirs.push(extra);
              }
            }
          }
        }
      }

      for steamapps_dir in &lib_dirs {
        let Ok(entries) = fs::read_dir(steamapps_dir) else { continue; };
        for entry in entries.filter_map(|e| e.ok()) {
          let path = entry.path();
          let Some(file_name) = path.file_name().and_then(|n| n.to_str()).map(|s| s.to_string()) else { continue; };
          if !file_name.starts_with("appmanifest_") || !file_name.ends_with(".acf") { continue; }
          let app_id = file_name.trim_start_matches("appmanifest_").trim_end_matches(".acf").to_string();
          let Ok(data) = fs::read_to_string(&path) else { continue; };

          let mut name = app_id.clone();
          let mut install_dir = String::new();
          for line in data.lines() {
            let t = line.trim();
            let parts: Vec<&str> = t.splitn(4, '"').collect();
            if parts.len() >= 4 {
              match parts[1] {
                "name" => name = parts[3].to_string(),
                "installdir" => install_dir = parts[3].to_string(),
                _ => {}
              }
            }
          }
          if name.is_empty() || install_dir.is_empty() { continue; }

          let game_dir = steamapps_dir.join("common").join(&install_dir);
          let exe_path = shared::find_best_game_exe(&game_dir).map(|p| p.to_string_lossy().into_owned());
          if let Some(ref p) = exe_path {
            if !seen.insert(p.to_lowercase()) { continue; }
          }
          candidates.push(ImportCandidate {
            name,
            source: "Steam".to_string(),
            app_id: Some(app_id),
            exe_path,
          });
        }
      }
    }

    candidates.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    candidates
  }

  // Heroic stores Epic & GOG installs in ~/.config/heroic/
  fn heroic_config_dir() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("HOME") {
      let native = PathBuf::from(&home).join(".config/heroic");
      if native.is_dir() { return Some(native); }
      // Flatpak Heroic
      let flat = PathBuf::from(&home).join(".var/app/com.heroicgameslauncher.hgl/config/heroic");
      if flat.is_dir() { return Some(flat); }
    }
    None
  }

  #[derive(serde::Deserialize)]
  struct HeroicInstalled {
    app_name: Option<String>,
    title: Option<String>,
    install_path: Option<String>,
    executable: Option<String>,
    platform: Option<String>,
  }

  fn heroic_library(source: &str, runner: &str) -> Vec<ImportCandidate> {
    let mut candidates = Vec::new();
    let Some(heroic_dir) = heroic_config_dir() else { return candidates; };

    let installed_path = heroic_dir.join(runner).join("installed.json");
    let Ok(data) = fs::read_to_string(&installed_path) else { return candidates; };
    let Ok(list) = serde_json::from_str::<Vec<HeroicInstalled>>(&data) else { return candidates; };

    for entry in list {
      let name = entry.title.unwrap_or_default();
      if name.is_empty() { continue; }

      let exe_path = if let (Some(install), Some(exe)) = (&entry.install_path, &entry.executable) {
        let full = std::path::Path::new(install).join(exe);
        if full.exists() {
          Some(full.to_string_lossy().into_owned())
        } else {
          shared::find_best_game_exe(std::path::Path::new(install)).map(|p| p.to_string_lossy().into_owned())
        }
      } else if let Some(install) = &entry.install_path {
        shared::find_best_game_exe(std::path::Path::new(install)).map(|p| p.to_string_lossy().into_owned())
      } else {
        None
      };

      candidates.push(ImportCandidate {
        name,
        source: source.to_string(),
        app_id: entry.app_name,
        exe_path,
      });
    }
    candidates.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    candidates
  }

  pub fn heroic_epic_library() -> Vec<ImportCandidate> {
    heroic_library("Epic", "legendaryConfig/legendary")
  }

  pub fn heroic_gog_library() -> Vec<ImportCandidate> {
    heroic_library("GOG", "gog_store")
  }
}
