// Tauri/vault stub for the SILO renderer harness (browser, no Rust backend).
// Mirrors the shapes app.js expects from window.__TAURI__ and the commands.

const RED_WALLPAPER = 'data:image/svg+xml;utf8,' + encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#d64545"/></svg>');
const WHITE_WALLPAPER = 'data:image/svg+xml;utf8,' + encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#f2f2f0"/></svg>');
const DARK_WALLPAPER = 'data:image/svg+xml;utf8,' + encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#141821"/></svg>');
const YELLOW_WALLPAPER = 'data:image/svg+xml;utf8,' + encodeURIComponent(
  '<svg xmlns="http://www.w3.org/2000/svg" width="8" height="8"><rect width="8" height="8" fill="#e8d31a"/></svg>');

// Save folders for the migration survey: one recorded under another PC's
// account, one on the unplugged drive, and one that is already right here.
const MINE = 'C:/Users/Current/Documents/My Games';
const FROM_ANOTHER_PC = 'C:/Users/OldPC/AppData/Local/CD Projekt Red/Cyberpunk 2077';
const ON_THE_UNPLUGGED_DRIVE = 'E:/Saves/Unplugged Drive Game';
// From the old PC and never reinstalled here, so nothing on this machine names
// it and its saves have to be copied over.
const OLD_PC_ONLY_GAME = 'C:/Users/OldPC/Documents/My Games/Old PC Only Game';
// Recorded on Linux and read by this Windows machine: the path shape itself is
// the diagnosis, and the advice is the direction from Unix to Windows.
const FROM_LINUX = '/home/old/.local/share/Old Linux Game';
// An old-PC game with a look-alike folder here that shares its name and nothing
// else, so SILO reports it rather than pointing the game at it.
const NAME_ONLY_GAME = 'C:/Users/OldPC/Documents/My Games/Name Only Game';
const NAME_ONLY_CANDIDATE = 'C:/Users/Current/Documents/My Games/Name Only Game';
// An old-PC game whose folder really is inside a Proton prefix here, but no app id
// ties that prefix to it, so SILO reports it instead of claiming it.
const PROTON_PREFIX_GAME = 'C:/Users/OldPC/Documents/My Games/Proton Prefix Game';
const PROTON_PREFIX_CANDIDATE = '/home/me/.steam/steam/steamapps/compatdata/292030/pfx/drive_c/users/steamuser/Documents/My Games/Proton Prefix Game';
const RELINKED_TO = MINE + '/Cyberpunk 2077';
// An old-PC game whose AppData folder never came across and is not on the old
// drive either, so only the SILO backup the library came with can supply it.
const OLD_PC_APPDATA_GAME = 'C:/Users/OldPC/AppData/Local/Bought Again Game';
const FROM_BACKUP_TO = 'C:/Users/Current/AppData/Local/Bought Again Game';
// The picked old drive, and the one game whose saves are on it. The library
// backs up every game, so the backup only gets used when the drive does not.
const OLD_DRIVE = 'E:/OldPC';
const ON_THE_OLD_DRIVE = { '6': 'Documents/My Games/Old PC Only Game' };
const FROM_DRIVE_TO = 'C:/Users/Current/Documents/My Games/Old PC Only Game';
const SILO_BACKUP = { '8': 'C:/Users/Current/AppData/Local/SILO/backups/8/manual_20240901_101500.zip' };

const GAMES = [
  { id: '1', name: 'Cyberpunk 2077', exePath: 'C:/Games/CP2077/bin/x64/Cyberpunk2077.exe', logoPath: null,
    wallpaper: YELLOW_WALLPAPER, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 7805, sessionCount: 62,
    lastPlayed: '2026-10-08T18:00:00Z', status: 'Playing', addedAt: '2026-01-01', savePath: FROM_ANOTHER_PC, favorite: true,
    tags: ['RPG', 'Open World'], isInstalled: true },
  { id: '2', name: 'A Great Big Long Game Name That Wraps', exePath: 'D:/Games/Long/Long.exe', logoPath: null,
    wallpaper: RED_WALLPAPER, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 130 * 60 + 5, sessionCount: 3,
    lastPlayed: '2026-09-20T10:00:00Z', status: 'On Hold', addedAt: '2026-02-01', savePath: null, favorite: false,
    tags: ['Indie'], isInstalled: true },
  { id: '3', name: 'Hollow Knight', exePath: 'C:/Games/HK/Hollow Knight.exe', logoPath: null,
    wallpaper: DARK_WALLPAPER, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 450, sessionCount: 12,
    lastPlayed: '2026-08-01T10:00:00Z', status: 'Backlog', addedAt: '2026-03-01', savePath: MINE + '/Hollow Knight', favorite: false,
    tags: ['Indie', 'Metroidvania'], isInstalled: true },
  { id: '4', name: 'Unplugged Drive Game', exePath: 'E:/Games/UD/ud.exe', logoPath: null,
    wallpaper: WHITE_WALLPAPER, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 90, sessionCount: 2,
    lastPlayed: '2026-07-01T10:00:00Z', status: 'Playing', addedAt: '2026-04-01', savePath: ON_THE_UNPLUGGED_DRIVE, favorite: false,
    tags: ['RPG'], isInstalled: true },
  { id: '5', name: 'Deleted Game', exePath: 'C:/Games/Gone/gone.exe', logoPath: null,
    wallpaper: null, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 0, sessionCount: 0,
    lastPlayed: null, status: 'Backlog', addedAt: '2026-05-01', savePath: null, favorite: false,
    tags: ['Backlog'], isInstalled: true },
  { id: '6', name: 'Old PC Only Game', exePath: 'C:/Users/OldPC/Games/OldPCOnly/old.exe', logoPath: null,
    wallpaper: null, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 0, sessionCount: 0,
    lastPlayed: null, status: 'Backlog', addedAt: '2026-06-01', savePath: OLD_PC_ONLY_GAME, favorite: false,
    tags: ['Backlog'], isInstalled: false },
  { id: '7', name: 'Old Linux Game', exePath: '/home/old/Games/old-linux-game/game', logoPath: null,
    wallpaper: null, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 0, sessionCount: 0,
    lastPlayed: null, status: 'Backlog', addedAt: '2026-06-15', savePath: FROM_LINUX, favorite: false,
    tags: ['Backlog'], isInstalled: false },
  { id: '8', name: 'Bought Again Game', exePath: null, logoPath: null,
    wallpaper: null, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 0, sessionCount: 0,
    lastPlayed: null, status: 'Backlog', addedAt: '2026-06-20', savePath: OLD_PC_APPDATA_GAME, favorite: false,
    tags: ['Backlog'], isInstalled: false },
  { id: '9', name: 'Name Only Game', exePath: 'C:/Users/OldPC/Games/name-only/nameonly.exe', logoPath: null,
    wallpaper: null, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 0, sessionCount: 0,
    lastPlayed: null, status: 'Backlog', addedAt: '2026-06-25', savePath: NAME_ONLY_GAME, favorite: false,
    tags: ['Backlog'], isInstalled: false },
  { id: '10', name: 'Proton Prefix Game', exePath: 'C:/Users/OldPC/Games/proton/prefix.exe', logoPath: null,
    wallpaper: null, fontFamily: null, fontColor: '#ffffff', playtimeMinutes: 0, sessionCount: 0,
    lastPlayed: null, status: 'Backlog', addedAt: '2026-06-30', savePath: PROTON_PREFIX_GAME, favorite: false,
    tags: ['Backlog'], isInstalled: false },
];

let settings = { sgdbApiKey: 'testkey123', checkUpdatesOnLaunch: false, showGameTitles: false };

// Live library state, so "Locate…" can genuinely fix a game.
const state = { games: JSON.parse(JSON.stringify(GAMES)) };
const RELOCATED = 'D:/Relocated/game.exe';

// id 4 is on an unplugged drive; id 5 is missing (drive present) until the user
// points SILO at a new executable, after which it verifies as installed.
function computeInstallResults() {
  return state.games.map(g => ({
    game_id: g.id,
    status: g.id === '4' ? 'unavailable'
      : (g.id === '5' || g.id === '6' || g.id === '7' ? (g.exePath === RELOCATED ? 'installed' : 'missing') : 'installed'),
  }));
}

// Mirrors the Rust survey: the other PC's folder is relinkable to the matching
// folder here, the unplugged drive is not, a folder matched only by name is
// reported rather than relinked, everything else is already fine.
function computeSurvey() {
  const relinkable = [];
  const unresolved = [];
  let usable = 0;
  for (const g of state.games) {
    if (!g.savePath) continue;
    if (g.savePath === FROM_ANOTHER_PC) {
      relinkable.push({
        gameId: g.id, gameName: g.name, recordedPath: g.savePath, foundPath: RELINKED_TO,
        confidence: 'confirmed', installPresent: true,
        reason: `${g.savePath} sits under OldPC's profile, which does not exist on this machine — it was recorded on a different PC.`,
      });
    } else if (g.savePath === PROTON_PREFIX_GAME) {
      unresolved.push({
        gameId: g.id, gameName: g.name, recordedPath: g.savePath, installPresent: false,
        confidence: 'likely',
        reason: `${g.savePath} sits under OldPC's profile, which does not exist on this machine — it was recorded on a different PC. SILO did find ${PROTON_PREFIX_CANDIDATE} inside a Proton or Wine prefix on this machine, but not one it can tie to this game, so this game was left for you to set in Edit.`,
      });
    } else if (g.savePath === NAME_ONLY_GAME) {
      unresolved.push({
        gameId: g.id, gameName: g.name, recordedPath: g.savePath, installPresent: false,
        confidence: 'guess',
        reason: `${g.savePath} sits under OldPC's profile, which does not exist on this machine — it was recorded on a different PC. SILO did find ${NAME_ONLY_CANDIDATE}, but it matches the game's name alone and may hold another game's saves, so this game was left for you to set in Edit.`,
      });
    } else if (g.savePath === ON_THE_UNPLUGGED_DRIVE) {
      unresolved.push({
        gameId: g.id, gameName: g.name, recordedPath: g.savePath, installPresent: false,
        reason: `${g.savePath} is on a drive that is not available on this machine. Reconnect the drive that holds it, or set the folder in Edit, then try again.`,
      });
    } else if (g.savePath === FROM_LINUX) {
      unresolved.push({
        gameId: g.id, gameName: g.name, recordedPath: g.savePath, installPresent: false,
        reason: `${g.savePath} was recorded on a Linux or macOS machine, so that path cannot exist on this one. Install the game here and launch it once so SaveGuard re-finds the folder, or copy the saves across and set the folder in Edit.`,
      });
    } else if (g.savePath === OLD_PC_ONLY_GAME || g.savePath === OLD_PC_APPDATA_GAME) {
      unresolved.push({
        gameId: g.id, gameName: g.name, recordedPath: g.savePath, installPresent: false,
        reason: `${g.savePath} sits under OldPC's profile, which does not exist on this machine — it was recorded on a different PC. SILO cannot see this game's files on this machine either, so its saves are still on the other PC: copy them across (or restore a SILO backup of this game), then check again.`,
      });
    } else {
      usable++;
    }
  }
  return {
    gamesWithSaveFolder: usable + relinkable.length + unresolved.length,
    usable,
    relinkable,
    unresolved,
  };
}

// Mirrors the Rust plan: only the games the survey could not place are covered,
// the folder the user picked is preferred over the game's own backup, a game
// with no destination on this machine is left out with the reason why, and
// nothing is written until bring_saves_across runs.
function computeImportPlan(sourceDir) {
  const ready = [];
  const stillMissing = [];
  for (const issue of computeSurvey().unresolved) {
    const g = state.games.find(x => x.id === issue.gameId);
    if (!g) continue;
    const target = g.savePath === OLD_PC_ONLY_GAME ? FROM_DRIVE_TO
      : g.savePath === OLD_PC_APPDATA_GAME ? FROM_BACKUP_TO
      : null;
    const onDrive = sourceDir && ON_THE_OLD_DRIVE[g.id]
      ? `${sourceDir}/${ON_THE_OLD_DRIVE[g.id]}` : null;
    if (!target) {
      stillMissing.push({ ...issue, reason: `${issue.reason} SILO has no folder on this machine to put the saves in, so set one in Edit.` });
    } else if (onDrive) {
      ready.push({ gameId: g.id, gameName: g.name, source: 'folder', sourcePath: onDrive,
        targetPath: target, recordedPath: g.savePath });
    } else if (SILO_BACKUP[g.id]) {
      ready.push({ gameId: g.id, gameName: g.name, source: 'backup', sourcePath: SILO_BACKUP[g.id],
        targetPath: target, recordedPath: g.savePath });
    } else {
      stillMissing.push({ ...issue, reason: `${issue.reason} No saves for it were found in the folder you picked, and it has no SILO backup.` });
    }
  }
  return { ready, stillMissing };
}

window.__calls = [];
window.__installChecks = 0;
window.__toastLog = [];
window.__oldDrive = OLD_DRIVE;

// Watch the toast container from the very first frame so a toast that appears
// and vanishes during startup is still recorded.
document.addEventListener('DOMContentLoaded', () => {
  const c = document.getElementById('toast-container');
  if (!c) { window.__toastLog.push(['no-container']); return; }
  new MutationObserver((muts) => {
    const now = Math.round(performance.now());
    for (const m of muts) {
      m.addedNodes.forEach(n => { if (n.classList && n.classList.contains('toast')) window.__toastLog.push(['added', now, n.querySelector('.toast-title')?.textContent]); });
      m.removedNodes.forEach(n => { if (n.classList && n.classList.contains('toast')) window.__toastLog.push(['removed', now, n.querySelector('.toast-title')?.textContent]); });
      if (m.type === 'attributes') window.__toastLog.push(['attr', now, m.target.className, m.attributeName]);
    }
  }).observe(c, { childList: true, attributes: true });
  window.__toastLog.push(['observer-ready']);
});

// Event plumbing so the renderer's listen() callbacks can actually fire.
const listeners = new Map();
window.__emit = (name, payload) => {
  for (const cb of listeners.get(name) || []) cb({ event: name, payload });
};

window.__TAURI__ = {
  dialog: {
    ask: async (message) => { window.__lastAsk = message; return true; },
    message: async (message, options) => { window.__lastMessage = message; window.__lastMessageOptions = options || null; return true; },
    // The renderer asks for a folder only to pick the old PC's saves; the
    // harness answers with the old drive so the flow can be driven end to end.
    open: async (options) => (options && options.directory ? window.__oldDrive : null),
  },
  core: {
    convertFileSrc: (p) => p,
    invoke: async (cmd, args) => {
      window.__calls.push(cmd);
      switch (cmd) {
        case 'get_games': return JSON.parse(JSON.stringify(state.games));
        case 'get_settings': return { ...settings };
        case 'set_settings': settings = { ...settings, ...(args?.settings || {}) }; return { ...settings };
        case 'verify_library_installs':
          window.__installChecks++;
          await new Promise(r => setTimeout(r, 350));   // pretend to hit the disk
          return computeInstallResults();
        case 'verify_game_install': {
          const hit = computeInstallResults().find(r => r.game_id === args?.gameId);
          return hit ? { ...hit } : { game_id: args?.gameId, status: 'installed' };
        }
        case 'list_sessions': return [];
        case 'get_all_backups': return [];
        case 'get_game_backups': return [];
        case 'pick_exe': return RELOCATED;
        case 'update_game': {
          const g = state.games.find(x => x.id === args?.id);
          if (!g) return null;
          Object.assign(g, args.updates || {});
          return JSON.parse(JSON.stringify(g));
        }
        case 'detect_launch_target': return { kind: 'game', confidence: 'high' };
        case 'survey_save_folders': return computeSurvey();
        case 'propose_save_folder': {
          const g = state.games.find(x => x.id === args?.gameId);
          if (!g || !g.savePath) return null;
          // A folder that already works here is not a change to propose.
          if (g.savePath.startsWith('C:/Users/Current')) return null;
          const issue = computeSurvey().relinkable.find(i => i.gameId === g.id)
            || computeSurvey().unresolved.find(i => i.gameId === g.id);
          if (!issue) return null;
          // The bulk relink refuses a weak match and hides its candidate inside
          // the reason; the per-game lookup offers it, so the stub digs it out.
          const found = issue.foundPath
            || (issue.confidence === 'guess' ? NAME_ONLY_CANDIDATE
              : issue.confidence === 'likely' ? PROTON_PREFIX_CANDIDATE : null);
          const proposal = { gameId: g.id, gameName: g.name, recordedPath: g.savePath,
            installPresent: issue.installPresent, reason: issue.reason };
          if (issue.confidence) proposal.confidence = issue.confidence;
          if (found) proposal.foundPath = found;
          return proposal;
        }
        case 'plan_save_import': return computeImportPlan(args?.sourceDir);
        case 'bring_saves_across': {
          const plan = computeImportPlan(args?.sourceDir);
          const brought = [];
          for (const item of plan.ready) {
            const g = state.games.find(x => x.id === item.gameId);
            if (!g) continue;
            g.savePath = item.targetPath;
            g.savePathSource = 'imported';
            brought.push({ gameId: g.id, savePath: g.savePath, savePathSource: 'imported' });
          }
          window.__emit('savefolders-relinked', { count: brought.length, games: brought });
          return { ready: plan.ready, stillMissing: plan.stillMissing };
        }
        case 'relink_save_folders': {
          const survey = computeSurvey();
          const games = [];
          for (const issue of survey.relinkable) {
            const g = state.games.find(x => x.id === issue.gameId);
            if (!g) continue;
            g.savePath = issue.foundPath;
            g.savePathSource = 'auto';
            games.push({ gameId: g.id, savePath: g.savePath, savePathSource: 'auto' });
          }
          window.__emit('savefolders-relinked', { count: games.length, games });
          return survey;
        }
        default: return null;
      }
    },
  },
  event: {
    listen: async (name, cb) => {
      if (!listeners.has(name)) listeners.set(name, []);
      listeners.get(name).push(cb);
      return () => {};
    },
  },
};
