use serde::{Deserialize, Serialize};
use std::fs;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Manager};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
  #[serde(rename = "sgdbApiKey")]
  pub sgdb_api_key: Option<String>,
  #[serde(rename = "checkUpdatesOnLaunch")]
  pub check_updates_on_launch: bool,
}

impl Default for Settings {
  fn default() -> Self {
    Settings {
      sgdb_api_key: None,
      check_updates_on_launch: true,
    }
  }
}

fn get_settings_path(app: &AppHandle) -> PathBuf {
  app.path().app_data_dir().unwrap().join("settings.json")
}

// Pure path-based atomic writer, extracted so the tmp/bak/rename logic can be unit-tested
// without an AppHandle. Mirrors save_games_atomic: write to a temp sibling, keep a .bak,
// then rename into place.
fn write_json_atomic(data_path: &Path, json_str: &str) -> Result<(), String> {
  if let Some(parent) = data_path.parent() {
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
  }

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

fn save_settings_atomic(app: &AppHandle, settings: &Settings) -> Result<(), String> {
  let data_path = get_settings_path(app);
  let json_str = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
  write_json_atomic(&data_path, &json_str)
}

#[tauri::command]
pub fn get_settings(app: AppHandle) -> Settings {
  let data_path = get_settings_path(&app);
  if let Ok(data) = fs::read_to_string(&data_path) {
    if let Ok(settings) = serde_json::from_str::<Settings>(&data) {
      return settings;
    }
  }
  Settings::default()
}

// Whole-object update: the frontend sends the full Settings object.
#[tauri::command]
pub fn set_settings(app: AppHandle, settings: Settings) -> Result<(), String> {
  save_settings_atomic(&app, &settings)?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use std::io::Read;

  struct TempDir(PathBuf);

  impl TempDir {
    fn new(tag: &str) -> Self {
      let dir = std::env::temp_dir().join(format!(
        "silo_settings_test_{}_{}_{}",
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

    fn path(&self) -> PathBuf {
      self.0.clone()
    }
  }

  impl Drop for TempDir {
    fn drop(&mut self) {
      let _ = fs::remove_dir_all(&self.0);
    }
  }

  #[test]
  fn defaults_have_updates_on_launch_enabled() {
    let s = Settings::default();
    assert!(s.check_updates_on_launch);
    assert_eq!(s.sgdb_api_key, None);
  }

  #[test]
  fn settings_serde_round_trip() {
    let s = Settings {
      sgdb_api_key: Some("abc".to_string()),
      check_updates_on_launch: false,
      ..Default::default()
    };
    let json = serde_json::to_string(&s).unwrap();
    let back: Settings = serde_json::from_str(&json).unwrap();
    assert_eq!(back.sgdb_api_key.as_deref(), Some("abc"));
    assert!(!back.check_updates_on_launch);
  }

  #[test]
  fn missing_fields_deserialize_to_defaults() {
    let back: Settings = serde_json::from_str("{}").unwrap();
    assert_eq!(back.sgdb_api_key, None);
    assert!(back.check_updates_on_launch);
  }

  #[test]
  fn write_json_atomic_creates_file_without_tmp_leftover() {
    let tmp = TempDir::new("atomic");
    let path = tmp.path().join("settings.json");
    let json = serde_json::to_string_pretty(&Settings::default()).unwrap();

    write_json_atomic(&path, &json).unwrap();

    assert!(path.exists());
    assert!(!path.with_extension("json.tmp").exists());
    assert!(!path.with_extension("json.bak").exists());
    assert_eq!(fs::read_to_string(&path).unwrap(), json);
  }

  #[test]
  fn write_json_atomic_preserves_previous_version_in_bak() {
    let tmp = TempDir::new("atomic2");
    let path = tmp.path().join("settings.json");
    let first = serde_json::to_string_pretty(&Settings {
      sgdb_api_key: Some("k1".into()),
      check_updates_on_launch: true,
      ..Default::default()
    })
    .unwrap();
    let second = serde_json::to_string_pretty(&Settings {
      sgdb_api_key: Some("k2".into()),
      check_updates_on_launch: false,
      ..Default::default()
    })
    .unwrap();

    write_json_atomic(&path, &first).unwrap();
    write_json_atomic(&path, &second).unwrap();

    let bak = path.with_extension("json.bak");
    assert!(bak.exists(), "a .bak should exist after the second write");
    let mut bak_content = String::new();
    fs::File::open(&bak)
      .unwrap()
      .read_to_string(&mut bak_content)
      .unwrap();
    assert_eq!(bak_content, first, ".bak should hold the first version");
    assert_eq!(fs::read_to_string(&path).unwrap(), second);
  }
}
