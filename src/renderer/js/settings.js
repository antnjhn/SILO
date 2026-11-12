/* ── settings.js — settings modal + preferences plumbing ──────────────────── */
import { showGameTitles, setShowGameTitles } from './state.js';
import { showToast } from './utils.js';
import { applyMotionPref, syncMotionToggle, setForceMotion, setDynamicAccent, syncDynamicAccentToggle } from './library.js';
import { showPage } from './pages.js';

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
    document.getElementById('input-check-updates').checked = !!s.checkUpdatesOnLaunch;
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
}

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
    checkUpdatesOnLaunch: document.getElementById('input-check-updates').checked,
    closeBehaviour: currentCloseBehaviour(),
  };
  base.showGameTitles = on;
  base.closeBehaviour = currentCloseBehaviour();
  try {
    await window.vault.setSettings({
      settings: {
        sgdbApiKey: base.sgdbApiKey || null,
        checkUpdatesOnLaunch: !!base.checkUpdatesOnLaunch,
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
      checkUpdatesOnLaunch: document.getElementById('input-check-updates').checked,
      closeBehaviour: currentCloseBehaviour(),
    };
    base.closeBehaviour = currentCloseBehaviour();
    try {
      await window.vault.setSettings({
        settings: {
          sgdbApiKey: base.sgdbApiKey || null,
          checkUpdatesOnLaunch: !!base.checkUpdatesOnLaunch,
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
// Advanced section collapse (settings page)
document.getElementById('settings-advanced-toggle')?.addEventListener('click', () => {
  const adv = document.getElementById('settings-advanced');
  if (adv) adv.classList.toggle('open');
});

document.getElementById('btn-save-settings').addEventListener('click', async () => {
  const sgdbApiKey = document.getElementById('input-sgdb-key').value.trim() || null;
  const checkUpdatesOnLaunch = document.getElementById('input-check-updates').checked;
  const closeBehaviour = currentCloseBehaviour();
  setShowGameTitles(document.getElementById('input-show-game-titles').checked);
  applyShowGameTitles();
  try {
    await window.vault.setSettings({ settings: { sgdbApiKey, checkUpdatesOnLaunch, showGameTitles, closeBehaviour } });
    cachedSettings = { sgdbApiKey, checkUpdatesOnLaunch, showGameTitles, closeBehaviour };
    showToast('success', 'Settings Saved', 'Settings have been saved.');
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

document.getElementById('btn-open-logs').addEventListener('click', async () => {
  try {
    await window.vault.openLogsFolder();
  } catch (err) {
    showToast('error', 'Logs Unavailable', 'Could not open the logs folder.');
  }
});

document.getElementById('btn-check-updates').addEventListener('click', async () => {
  try {
    const updater = window.__TAURI__?.updater;
    if (updater && updater.check) {
      const u = await updater.check();
      if (u) {
        if (u.updateAvailable) {
          showToast('info', 'Update Available', `Update ${u.version} is available`);
          await u.downloadAndInstall({
            mimeType: 'application/octet-stream',
            requestHeaders: [],
          });
        } else {
          showToast('info', 'Up to Date', 'You are up to date');
        }
      } else {
        showToast('info', 'Up to Date', 'You are up to date');
      }
    } else {
      showToast('info', 'Not Configured', 'Updates not configured');
    }
  } catch (e) {
    showToast('error', 'Update Unavailable', 'Update check unavailable');
  }
});
