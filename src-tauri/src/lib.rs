mod commands;
pub mod library_import;
pub mod metadata;
pub mod saveguard;
pub mod settings;

use tauri_plugin_log::{RotationStrategy, Target, TargetKind};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_dialog::init())
    .plugin(
      tauri_plugin_log::Builder::new()
        .targets([
          Target::new(TargetKind::Stdout),
          Target::new(TargetKind::LogDir {
            file_name: Some("silo.log".into()),
          }),
        ])
        .max_file_size(5_000_000)
        .rotation_strategy(RotationStrategy::KeepAll)
        .build(),
    )
    .plugin(tauri_plugin_updater::Builder::new().build())
    .invoke_handler(tauri::generate_handler![
      commands::get_games,
      commands::add_game,
      commands::update_game,
      commands::delete_game,
      commands::get_system_fonts,
      commands::launch_game,
      commands::pick_exe,
      commands::pick_save_folder,
      commands::pick_wallpaper,
      commands::pick_logo,
      commands::window_minimize,
      commands::window_maximize,
      commands::window_close,
      commands::window_start_dragging,
      commands::scan_folder,
      commands::fetch_steam_metadata,
      commands::get_game_backups,
      commands::restore_backup,
      commands::delete_backup,
      commands::backup_game_now,
      commands::check_uninstaller,
      commands::delete_game_folder,
      commands::run_uninstaller,
      commands::backup_library,
      commands::restore_library,
      settings::get_settings,
      settings::set_settings,
      commands::open_logs_folder,
      commands::get_all_backups,
      commands::delete_game_backups,
      metadata::fetch_metadata,
      library_import::import_steam_library,
      library_import::import_epic_library,
      library_import::import_gog_library
    ])
    .setup(|_app| {
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
