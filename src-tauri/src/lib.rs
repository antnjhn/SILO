mod commands;
pub mod saveguard;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
  tauri::Builder::default()
    .plugin(tauri_plugin_shell::init())
    .plugin(tauri_plugin_fs::init())
    .plugin(tauri_plugin_dialog::init())
    .invoke_handler(tauri::generate_handler![
        commands::get_games,
        commands::add_game,
        commands::update_game,
        commands::delete_game,
        commands::get_system_fonts,
        commands::launch_game,
        commands::pick_exe,
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
        commands::backup_now,
        commands::check_uninstaller,
        commands::delete_game_folder,
        commands::run_uninstaller,
        commands::backup_library,
        commands::restore_library
    ])
    .setup(|app| {
      if cfg!(debug_assertions) {
        // app.handle().plugin(tauri_plugin_log::Builder::default().build())?;
      }
      Ok(())
    })
    .run(tauri::generate_context!())
    .expect("error while running tauri application");
}
