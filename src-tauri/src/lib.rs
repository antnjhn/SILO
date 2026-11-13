mod commands;
pub mod discovery;
pub mod display;
pub mod game_launch;
pub mod gaming_lifecycle;
pub mod gaming_session_guard;
pub mod gaming_session_monitor;
pub mod gaming_session_persistence;
pub mod gaming_session_recovery;
pub mod gaming_session_tail;
pub mod gaming_window;
pub mod library_import;
pub mod metadata;
pub mod process_monitor;
pub mod saveguard;
pub mod settings;
pub mod tray;
mod uninstaller;
pub mod window_lifecycle;

use tauri::Emitter;
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
            commands::verify_library_installs,
            commands::verify_game_install,
            commands::add_game,
            commands::update_game,
            commands::detect_launch_target,
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
            commands::fetch_wallpaper_bytes,
            commands::get_game_backups,
            commands::restore_backup,
            commands::survey_save_folders,
            commands::propose_save_folder,
            commands::relink_save_folders,
            commands::plan_save_import,
            commands::bring_saves_across,
            commands::delete_backup,
            commands::backup_game_now,
            uninstaller::discover_uninstallers,
            uninstaller::run_uninstaller_candidate,
            commands::delete_game_folder,
            commands::backup_library,
            commands::restore_library,
            settings::get_settings,
            settings::set_settings,
            commands::confirm_close,
            commands::open_logs_folder,
            commands::get_all_backups,
            commands::delete_game_backups,
            commands::list_sessions,
            display::get_display_info,
            metadata::fetch_metadata,
            metadata::sgdb_search_games,
            metadata::sgdb_art,
            metadata::auto_art_for_game,
            metadata::preview_art,
            discovery::get_discovery_state,
            discovery::discovery_add_folder,
            discovery::discovery_remove_folder,
            discovery::discovery_set_enabled,
            discovery::discover_games,
            discovery::record_discovery_decision,
            discovery::clear_discovery_decision,
            discovery::reset_ignored_discoveries,
            library_import::import_steam_library,
            library_import::import_epic_library,
            library_import::import_gog_library
        ])
        .on_window_event(|window, event| {
            // A native close request (Alt+F4, taskbar, or `window.close()`) is
            // governed by the stored close behaviour: minimise to tray, exit, or
            // ask the user every time. The tray still owns the only explicit Quit,
            // and without a tray the close-and-exit behaviour is kept so a failed
            // tray install can never strand the process.
            if window.label() != "main" {
                return;
            }
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                let app = tauri::Manager::app_handle(window);
                let behaviour = crate::settings::load_settings(app).close_behaviour;
                match tray::close_decision(tray::is_installed(), gaming_lifecycle::is_exiting(), behaviour) {
                    tray::CloseDecision::StepAside => {
                        api.prevent_close();
                        tray::hide_to_tray(app);
                    }
                    tray::CloseDecision::AllowClose => {
                        // Let the close proceed. If a gaming session owns the app,
                        // `gaming_lifecycle` defers the exit until the session ends.
                    }
                    tray::CloseDecision::AskUser => {
                        // Defer the close and let the user choose. The frontend
                        // shows the confirmation dialog and answers through
                        // `confirm_close`.
                        api.prevent_close();
                        let _ = app.emit("close-confirm-requested", serde_json::Value::Null);
                    }
                }
            }
        })
        .setup(|app| {
            // The tray is what keeps SILO reachable while a session owns the
            // window, and it owns the explicit Quit. A failure is logged rather
            // than fatal; `window_close` falls back to closing the app.
            if let Err(error) = tray::install(app.handle()) {
                log::error!("SILO could not install its tray icon: {error}");
            }
            #[cfg(target_os = "windows")]
            {
                let handle = app.handle().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    commands::recover_gaming_sessions(handle);
                });
            }
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building the tauri application")
        .run(|_app, event| {
            // A quit requested while a gaming session owns the app is deferred
            // until the session has been committed, never dropped.
            if let tauri::RunEvent::ExitRequested { api, code, .. } = event {
                // Destroying the last window (Stage 6 releases the WebView for
                // the session) asks the app for permission to exit with no exit
                // code. That request is not a quit: preventing it must leave no
                // intent behind, so an ordinary session never ends in an exit.
                if window_lifecycle::exit_request_is_own_teardown(
                    code,
                    window_lifecycle::webview_is_released(),
                    gaming_session_guard::session_is_active(),
                ) {
                    api.prevent_exit();
                    log::info!(
                        "SILO exit prevented: the WebView is released for a Gaming Mode session, not a quit request"
                    );
                    return;
                }
                let decision = gaming_lifecycle::handle_exit_request(&api, gaming_session_guard::session_is_active());
                log::info!("SILO exit request: {decision:?}");
            }
        });
}
