/* ── api.js — the renderer ↔ Rust IPC surface (all invoke/listen calls) ────── */

/* ── Tauri IPC Shim ────────────────────────────────────────────────────────*/
export const invoke = window.__TAURI__.core.invoke;
export const listen = window.__TAURI__.event.listen;
window.vault = {
  getGames: () => invoke('get_games'),
  addGame: (data) => invoke('add_game', { name: data.name, exePath: data.exePath || null, wallpaper: data.wallpaper || null, logoPath: data.logoPath || null, fontFamily: data.fontFamily || null, fontColor: data.fontColor || null, savePath: data.savePath || null, launchTarget: data.launchTarget || null }),
  updateGame: (id, updates) => invoke('update_game', { id, updates }),
  detectLaunchTarget: (exePath) => invoke('detect_launch_target', { exePath }),
  deleteGame: (id) => invoke('delete_game', { id }),
  pickExe: () => invoke('pick_exe'),
  pickSaveFolder: () => invoke('pick_save_folder'),
  pickWallpaper: (gameId) => invoke('pick_wallpaper', { gameId }),
  pickLogo: (gameId) => invoke('pick_logo', { gameId }),
  launchGame: (gameId) => invoke('launch_game', { gameId }),
  listSessions: () => invoke('list_sessions'),
  getSystemFonts: () => invoke('get_system_fonts'),
  onPlaytimeUpdated: async (cb) => await listen('playtime-updated', (event) => cb(event.payload)),
  windowMinimize: () => invoke('window_minimize'),
  windowMaximize: () => invoke('window_maximize'),
  windowClose: () => invoke('window_close'),
  windowStartDragging: () => invoke('window_start_dragging'),
  getGameBackups: (gameId) => invoke('get_game_backups', { gameId }),
  restoreBackup: (gameId, backupName) => invoke('restore_backup', { gameId, backupName }),
  surveySaveFolders: () => invoke('survey_save_folders'),
  proposeSaveFolder: (gameId) => invoke('propose_save_folder', { gameId }),
  relinkSaveFolders: () => invoke('relink_save_folders'),
  planSaveImport: (sourceDir) => invoke('plan_save_import', { sourceDir: sourceDir || null }),
  bringSavesAcross: (sourceDir) => invoke('bring_saves_across', { sourceDir: sourceDir || null }),
  backupLibrary: () => invoke('backup_library'),
  restoreLibrary: () => invoke('restore_library'),
  fetchMetadata: (name) => invoke('fetch_metadata', { name }),
  getSettings: () => invoke('get_settings'),
  setSettings: (data) => invoke('set_settings', { settings: data.settings }),
  openLogsFolder: () => invoke('open_logs_folder'),
  getAllBackups: () => invoke('get_all_backups'),
  deleteGameBackups: (gameId) => invoke('delete_game_backups', { gameId }),
  importSteamLibrary: () => invoke('import_steam_library'),
  importEpicLibrary: () => invoke('import_epic_library'),
  importGogLibrary: () => invoke('import_gog_library'),
};
export function getFileSrc(path, imgElement) {
  if (!path) return;
  if (path.startsWith('http://') || path.startsWith('https://')) {
    imgElement.src = path;
  } else {
    imgElement.src = window.__TAURI__.core.convertFileSrc(path);
  }
}

