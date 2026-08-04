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
  {
    imp::steam_library()
  }
  #[cfg(not(target_os = "windows"))]
  {
    Vec::new()
  }
}

#[tauri::command]
pub async fn import_epic_library() -> Vec<ImportCandidate> {
  #[cfg(target_os = "windows")]
  {
    imp::epic_library()
  }
  #[cfg(not(target_os = "windows"))]
  {
    Vec::new()
  }
}

#[tauri::command]
pub async fn import_gog_library() -> Vec<ImportCandidate> {
  #[cfg(target_os = "windows")]
  {
    imp::gog_library()
  }
  #[cfg(not(target_os = "windows"))]
  {
    Vec::new()
  }
}

#[cfg(target_os = "windows")]
mod imp {
  use super::ImportCandidate;
  use std::collections::HashSet;
  use std::fs;
  use std::path::{Path, PathBuf};
  use walkdir::WalkDir;

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
    let dir_name = game_dir.file_name().and_then(|n| n.to_str()).unwrap_or("").to_lowercase();
    let mut best: Option<(i32, PathBuf)> = None;
    // 3 levels covers common layouts like <game>/bin/x64/game.exe.
    for entry in WalkDir::new(game_dir).max_depth(3).into_iter().filter_map(|e| e.ok()) {
      let path = entry.path();
      if !path.is_file() {
        continue;
      }
      if path.extension().and_then(|e| e.to_str()) != Some("exe") {
        continue;
      }
      let file_name = path.file_name().unwrap_or_default().to_string_lossy().to_lowercase();
      if crate::commands::is_blacklisted_exe(&file_name) {
        continue;
      }
      let Ok(metadata) = fs::metadata(path) else {
        continue;
      };
      if metadata.len() < 500_000 {
        continue;
      }
      let stem = path.file_stem().unwrap_or_default().to_string_lossy().to_lowercase();
      let score = crate::commands::score_exe_candidate(&stem, &dir_name, entry.depth(), metadata.len());
      if best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
        best = Some((score, path.to_path_buf()));
      }
    }
    best.map(|(_, path)| path)
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
