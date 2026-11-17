/* ── settings.js — settings modal + preferences plumbing ──────────────────── */
import { showGameTitles, setShowGameTitles, setAllGames } from './state.js';
import { showToast } from './utils.js';
import { applyMotionPref, syncMotionToggle, setForceMotion, setDynamicAccent, syncDynamicAccentToggle, applyCategoryFilter } from './library.js';
import { showPage } from './pages.js';
import { listen } from './api.js';

/* ── Settings & Backup ─────────────────────────────────────────────────────*/
let cachedSettings = null;

// Applies the 'Show game titles' preference to the live UI (body class drives CSS).
export function applyShowGameTitles(syncCheckbox = true) {
  document.body.classList.toggle('show-game-titles', !!showGameTitles);
  const track = document.querySelector('label[for="input-show-game-titles"] .toggle-track');
  if (track) track.classList.toggle('on', !!showGameTitles);
  if (syncCheckbox) {
    const cb = document.getElementById('input-show-game-titles');
    if (cb) cb.checked = !!showGameTitles;
  }
}

async function openSettingsModal() {
  showPage('settings');
  try {
    const s = await window.vault.getSettings();
    cachedSettings = s;
    setShowGameTitles(!!s.showGameTitles);
    document.getElementById('input-sgdb-key').value = s.sgdbApiKey || '';
    const closeBehaviourIds = { askEveryTime: 'ask', minimiseToTray: 'tray', exitApplication: 'exit' };
    const radio = document.getElementById('close-behaviour-' + closeBehaviourIds[s.closeBehaviour]);
    if (radio) radio.checked = true;
    applyShowGameTitles();
    syncMotionToggle();
    syncDynamicAccentToggle();
  } catch (err) {
    console.error(err);
  }
  await loadDiscovery();
  await refreshArtStatus();
  await loadSnapshots();
}

/* ── Library snapshots ─────────────────────────────────────────────────────
   games.json is written through a temp file and a .bak, and hourly copies are
   kept beside it. Until now nothing could use them. This is the way back: the
   copies are listed, and one can be put back without touching a file by hand. */
async function loadSnapshots() {
  const select = document.getElementById('snapshot-select');
  const button = document.getElementById('btn-restore-snapshot');
  if (!select || !button) return;
  select.innerHTML = '';
  let snapshots = [];
  try {
    snapshots = (await window.vault.listLibrarySnapshots()) || [];
  } catch {
    snapshots = [];
  }
  if (!snapshots.length) {
    const option = document.createElement('option');
    option.textContent = 'No snapshots yet';
    option.value = '';
    select.append(option);
    select.disabled = true;
    button.disabled = true;
    return;
  }
  for (const snapshot of snapshots) {
    const option = document.createElement('option');
    option.value = snapshot.name;
    option.textContent = `${snapshot.savedAt} · ${snapshot.games} game${snapshot.games === 1 ? '' : 's'}`;
    select.append(option);
  }
  select.disabled = false;
  button.disabled = false;
}

/* ── Artwork status ────────────────────────────────────────────────────────
   The sweep is the only thing that fetches art, and when it fails it fails
   quietly. This row is where the result lands: how many games still have none,
   and a retry that kicks the same sweep off again. */
async function refreshArtStatus() {
  const text = document.getElementById('art-status-text');
  const retry = document.getElementById('btn-art-retry');
  if (!text || !retry) return;

  let status;
  try {
    status = await window.vault.artStatus();
  } catch {
    text.textContent = 'Artwork status is unavailable.';
    retry.hidden = true;
    return;
  }

  if (!status || !status.total) {
    text.textContent = 'No games yet, so there is nothing to fetch.';
    retry.hidden = true;
    return;
  }
  if (status.missing === 0) {
    text.textContent = `All ${status.total} games have their artwork.`;
    retry.hidden = true;
    return;
  }
  text.textContent = status.hasKey
    ? `${status.missing} of ${status.total} games still need artwork.`
    : `${status.missing} of ${status.total} games need artwork. Add a key to fetch it.`;
  retry.hidden = !status.hasKey;
}

document.getElementById('btn-restore-snapshot')?.addEventListener('click', async () => {
  const select = document.getElementById('snapshot-select');
  const button = document.getElementById('btn-restore-snapshot');
  const name = select?.value;
  if (!name || !button || button.disabled) return;
  const label = select.options[select.selectedIndex]?.textContent || name;
  const { ask } = window.__TAURI__.dialog;
  const yes = await ask(
    `Restore the library from ${label}? The library you have now is saved first, so this can be undone.`,
    { title: 'Restore library', kind: 'warning' },
  );
  if (!yes) return;
  button.disabled = true;
  try {
    const count = await window.vault.restoreLibrarySnapshot(name);
    setAllGames(await window.vault.getGames());
    applyCategoryFilter(false);
    showToast('success', 'Library Restored', `${count} game${count === 1 ? '' : 's'} restored.`);
  } catch (err) {
    showToast('error', 'Restore Failed', err);
  } finally {
    button.disabled = false;
    loadSnapshots();
  }
});

document.getElementById('btn-art-retry')?.addEventListener('click', async () => {
  const button = document.getElementById('btn-art-retry');
  if (!button || button.disabled) return;
  button.disabled = true;
  button.textContent = 'Fetching...';
  try {
    await window.vault.sweepArt();
  } catch (err) {
    button.disabled = false;
    button.textContent = 'Retry';
    showToast('error', 'Artwork Failed', err);
  }
});

// The sweep reports through art-progress; when it finishes, the row tells the
// truth again and Retry becomes usable.
listen('art-progress', (event) => {
  if (event?.payload?.kind !== 'done') return;
  const button = document.getElementById('btn-art-retry');
  if (button) {
    button.disabled = false;
    button.textContent = 'Retry';
  }
  refreshArtStatus();
});

function currentCloseBehaviour() {
  const checked = document.querySelector('input[name="closeBehaviour"]:checked');
  return checked ? checked.value : 'askEveryTime';
}

document.getElementById('btn-settings').addEventListener('click', openSettingsModal);

// Toggle applies immediately (no Save required) and persists the whole settings object.
document.getElementById('input-show-game-titles')?.addEventListener('change', async (e) => {
  const on = e.target.checked;
  setShowGameTitles(on);
  applyShowGameTitles();
  const base = cachedSettings || {
    sgdbApiKey: document.getElementById('input-sgdb-key').value.trim() || null,
    closeBehaviour: currentCloseBehaviour(),
  };
  base.showGameTitles = on;
  base.closeBehaviour = currentCloseBehaviour();
  try {
    await window.vault.setSettings({
      settings: {
        sgdbApiKey: base.sgdbApiKey || null,
        showGameTitles: on,
        closeBehaviour: base.closeBehaviour,
      },
    });
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

// Close behaviour radios apply immediately and persist the whole settings object.
document.querySelectorAll('input[name="closeBehaviour"]').forEach((radio) => {
  radio.addEventListener('change', async () => {
    const base = cachedSettings || {
      sgdbApiKey: document.getElementById('input-sgdb-key').value.trim() || null,
      closeBehaviour: currentCloseBehaviour(),
    };
    base.closeBehaviour = currentCloseBehaviour();
    try {
      await window.vault.setSettings({
        settings: {
          sgdbApiKey: base.sgdbApiKey || null,
          showGameTitles: document.getElementById('input-show-game-titles').checked,
          closeBehaviour: base.closeBehaviour,
        },
      });
      showToast('success', 'Close Behaviour', 'Closing SILO will now: ' + base.closeBehaviour);
    } catch (err) {
      showToast('error', 'Save Failed', err);
    }
  });
});
// 'Always animate' — overrides the system reduce-motion setting for the launcher.
// Applies instantly (no Save required); stored locally, not in the settings file.
document.getElementById('input-force-motion')?.addEventListener('change', (e) => {
  setForceMotion(e.target.checked);
  applyMotionPref();
});

// 'Dynamic accent color' — same pattern as 'Always animate': applies instantly,
// stored locally. The Rust Settings struct has no field for it (unknown keys
// are dropped when serde deserializes), so settings.json can't persist it.
document.getElementById('input-dynamic-accent')?.addEventListener('change', (e) => {
  setDynamicAccent(e.target.checked);
});

document.getElementById('btn-settings-close').addEventListener('click', () => {
  showPage('home');
});

/* ── Automatic Game Discovery ───────────────────────────────────────────────
   Configured folders live in discovery.json (never in games.json). The section
   only ever writes that file, so saving settings cannot disturb the library. */

function discoveryRow(label, title, actionLabel, onAction) {
  const row = document.createElement('li');
  row.className = 'discovery-folder';
  const text = document.createElement('span');
  text.className = 'discovery-folder-path';
  text.textContent = label;
  text.title = title || label;
  const button = document.createElement('button');
  button.type = 'button';
  button.className = 'settings-ghost';
  button.textContent = actionLabel;
  button.addEventListener('click', onAction);
  row.append(text, button);
  return row;
}

function renderDiscovery(state) {
  if (!state) return;
  const enabled = document.getElementById('input-discovery-enabled');
  if (enabled) enabled.checked = !!state.enabled;

  const list = document.getElementById('discovery-folder-list');
  if (list) {
    list.replaceChildren();
    for (const folder of state.folders || []) {
      list.append(discoveryRow(folder, folder, 'Remove', async () => {
        try {
          renderDiscovery(await window.vault.discoveryRemoveFolder(folder));
          showToast('success', 'Folder Removed', 'SILO will not scan that folder again.');
        } catch (err) {
          showToast('error', 'Remove Failed', err);
        }
      }));
    }
    const empty = document.getElementById('discovery-empty');
    if (empty) empty.hidden = (state.folders || []).length > 0;
  }

  const ignoredWrap = document.getElementById('discovery-ignored-wrap');
  const ignoredList = document.getElementById('discovery-ignored-list');
  const ignored = state.ignored || [];
  if (ignoredWrap) ignoredWrap.hidden = ignored.length === 0;
  if (ignoredList) {
    ignoredList.replaceChildren();
    for (const entry of ignored) {
      const label = entry.name || entry.exePath || entry.key;
      ignoredList.append(discoveryRow(label, entry.exePath, 'Reconsider', async () => {
        try {
          renderDiscovery(await window.vault.clearDiscoveryDecision(entry.key));
          showToast('info', 'Reconsidered', 'SILO can offer that game again on the next start.');
        } catch (err) {
          showToast('error', 'Update Failed', err);
        }
      }));
    }
  }

  const sub = document.getElementById('discovery-status-sub');
  if (sub) {
    const count = (state.folders || []).length;
    sub.textContent = !state.enabled
      ? 'Scanning is off. Registered folders are kept but never scanned.'
      : count === 0
        ? 'SILO looks for installed games in the folders below and always asks before adding one.'
        : `Scanning ${count} folder${count === 1 ? '' : 's'} at startup. SILO always asks before adding a game.`;
  }
}

async function loadDiscovery() {
  try {
    renderDiscovery(await window.vault.getDiscoveryState());
  } catch (err) {
    console.warn('Could not load discovery settings', err);
  }
}

document.getElementById('input-discovery-enabled')?.addEventListener('change', async (e) => {
  try {
    renderDiscovery(await window.vault.discoverySetEnabled(e.target.checked));
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

document.getElementById('btn-discovery-add-folder')?.addEventListener('click', async () => {
  let folder = null;
  try {
    folder = await window.vault.pickSaveFolder();
  } catch (err) {
    showToast('error', 'Folder Error', err);
    return;
  }
  if (!folder) return;
  try {
    renderDiscovery(await window.vault.discoveryAddFolder(folder));
    showToast('success', 'Folder Added', 'SILO will look for new games there on the next start.');
  } catch (err) {
    showToast('error', 'Folder Not Added', err);
  }
});

document.getElementById('btn-discovery-reset-ignored')?.addEventListener('click', async () => {
  try {
    renderDiscovery(await window.vault.resetIgnoredDiscoveries());
    showToast('success', 'Ignored Games Reset', 'Every rejected game can be offered again.');
  } catch (err) {
    showToast('error', 'Reset Failed', err);
  }
});
// API key show/hide — visible eye-toggle instead of a permanently masked field.
document.getElementById('btn-toggle-key-visibility')?.addEventListener('click', () => {
  const key = document.getElementById('input-sgdb-key');
  const btn = document.getElementById('btn-toggle-key-visibility');
  if (!key || !btn) return;
  const show = key.type === 'password';
  key.type = show ? 'text' : 'password';
  btn.textContent = show ? 'HIDE' : 'SHOW';
  btn.title = show ? 'Hide key' : 'Show key';
});
document.getElementById('btn-save-settings').addEventListener('click', async () => {
  const sgdbApiKey = document.getElementById('input-sgdb-key').value.trim() || null;
  const closeBehaviour = currentCloseBehaviour();
  setShowGameTitles(document.getElementById('input-show-game-titles').checked);
  applyShowGameTitles();

  // A key is checked before it is stored, so a bad paste is caught here rather
  // than becoming games that never get artwork.
  if (sgdbApiKey) {
    try {
      await window.vault.validateSgdbKey(sgdbApiKey);
    } catch (err) {
      showToast('error', 'Key Not Accepted', err);
      return;
    }
  }

  try {
    await window.vault.setSettings({ settings: { sgdbApiKey, showGameTitles, closeBehaviour } });
    cachedSettings = { sgdbApiKey, showGameTitles, closeBehaviour };
    showToast('success', 'Settings Saved', 'Settings have been saved.');
    // A new key can make the retry button relevant straight away.
    refreshArtStatus();
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

/* ── Store libraries ─────────────────────────────────────────────────────────
   The three marks read the store the user already has installed. Only the
   store that was picked is read, and its games are added through the normal
   add path so artwork and the library refresh behave like any other add. */
async function importStoreLibrary(source, button) {
  const status = document.getElementById('set-import-status');
  const cards = Array.from(document.querySelectorAll('.set-store'));
  if (button.disabled) return;

  for (const card of cards) card.disabled = true;
  button.setAttribute('aria-pressed', 'true');
  if (status) {
    status.hidden = false;
    status.textContent = `Looking through your ${source} library...`;
  }

  let candidates = [];
  try {
    candidates = (await window.vault[`import${source}Library`]()) || [];
  } catch (err) {
    console.warn('Store import failed', err);
    if (status) status.textContent = `${source} could not be read. You can try again later.`;
    for (const card of cards) card.disabled = false;
    button.setAttribute('aria-pressed', 'false');
    return;
  }

  let known = new Set();
  let knownNames = new Set();
  try {
    const library = (await window.vault.getGames()) || [];
    known = new Set(library.map((game) => game.exePath).filter(Boolean));
    // The same game can come from a store and from a folder scan, and those
    // records do not always agree on the executable, so the name counts too.
    knownNames = new Set(library
      .map((game) => (game.name || '').trim().toLowerCase())
      .filter(Boolean));
  } catch {
    known = new Set();
    knownNames = new Set();
  }

  const fresh = candidates.filter((candidate) => candidate
    && candidate.exePath
    && !known.has(candidate.exePath)
    && !knownNames.has((candidate.name || '').trim().toLowerCase()));
  const alreadyThere = candidates.length - fresh.length;

  let added = 0;
  for (const candidate of fresh) {
    try {
      const game = await window.vault.addGame({ name: candidate.name, exePath: candidate.exePath });
      if (!game) continue;
      added += 1;
      try { await window.vault.autoArtForGame(game.id); } catch { /* artwork is optional */ }
    } catch (err) {
      console.warn(`Could not add ${candidate.name}`, err);
    }
  }

  if (status) {
    if (added) {
      status.textContent = `${added} game${added === 1 ? '' : 's'} added from ${source}.`;
    } else if (alreadyThere) {
      status.textContent = `Every game in your ${source} library is already here.`;
    } else {
      status.textContent = `Nothing new found in your ${source} library.`;
    }
  }
  try {
    setAllGames(await window.vault.getGames());
    applyCategoryFilter(false);
  } catch (err) {
    console.warn('Could not refresh the library after the import', err);
  }
  for (const card of cards) card.disabled = false;
  button.setAttribute('aria-pressed', 'false');
}

for (const card of document.querySelectorAll('.set-store')) {
  card.addEventListener('click', () => importStoreLibrary(card.dataset.source, card));
}

/* ── Advanced rows and the About block at the end of the same scroll ──────── */
const SGDB_KEY_URL = 'https://www.steamgriddb.com/profile/preferences/api';
const RELEASES_URL = 'https://github.com/antnjhn/SILO/releases';
const LICENSES_URL = 'https://github.com/antnjhn/SILO/blob/main/LICENSE';

document.getElementById('btn-open-sgdb')?.addEventListener('click', async () => {
  try {
    await window.vault.openExternalUrl(SGDB_KEY_URL);
  } catch (err) {
    showToast('error', 'Browser Unavailable', 'Could not open steamgriddb.com.');
  }
});

document.getElementById('btn-release-notes')?.addEventListener('click', async () => {
  try {
    await window.vault.openExternalUrl(RELEASES_URL);
  } catch (err) {
    showToast('error', 'Browser Unavailable', 'Could not open the release notes.');
  }
});

document.getElementById('btn-open-licenses')?.addEventListener('click', async () => {
  try {
    await window.vault.openExternalUrl(LICENSES_URL);
  } catch (err) {
    showToast('error', 'Browser Unavailable', 'Could not open the license page.');
  }
});

document.getElementById('btn-open-logs').addEventListener('click', async () => {
  try {
    await window.vault.openLogsFolder();
  } catch (err) {
    showToast('error', 'Logs Unavailable', 'Could not open the logs folder.');
  }
});

/* Asks GitHub for the newest published release. This needs no signing key and
   no updater endpoint: it is a read of the public releases list, and it sends
   the user to the download page rather than installing anything behind them. */
document.getElementById('btn-check-updates').addEventListener('click', async () => {
  const button = document.getElementById('btn-check-updates');
  if (button?.disabled) return;
  if (button) button.disabled = true;
  try {
    const update = await window.vault.checkForUpdate();
    if (!update) {
      const version = await window.__TAURI__.app.getVersion();
      showToast('info', 'Up to Date', `You are on ${version}.`);
      return;
    }
    const { ask } = window.__TAURI__.dialog;
    const yes = await ask(`Version ${update.latest} is available. Open the releases page?`, {
      title: 'Update Available',
      kind: 'info',
    });
    if (yes) await window.vault.openExternalUrl(update.url);
  } catch (err) {
    showToast('error', 'Update Unavailable', err);
  } finally {
    if (button) button.disabled = false;
  }
});
