/* ── settings.js — settings modal + preferences plumbing ──────────────────── */
import { showGameTitles, setShowGameTitles } from './state.js';
import { showToast } from './utils.js';
import { applyMotionPref, syncMotionToggle, setForceMotion } from './library.js';
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
    applyShowGameTitles();
    syncMotionToggle();
  } catch (err) {
    console.error(err);
  }
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
  };
  base.showGameTitles = on;
  try {
    await window.vault.setSettings({
      settings: {
        sgdbApiKey: base.sgdbApiKey || null,
        checkUpdatesOnLaunch: !!base.checkUpdatesOnLaunch,
        showGameTitles: on,
      },
    });
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});
// 'Always animate' — overrides the system reduce-motion setting for the launcher.
// Applies instantly (no Save required); stored locally, not in the settings file.
document.getElementById('input-force-motion')?.addEventListener('change', (e) => {
  setForceMotion(e.target.checked);
  applyMotionPref();
});

document.getElementById('btn-settings-close').addEventListener('click', () => {
  showPage('home');
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
  setShowGameTitles(document.getElementById('input-show-game-titles').checked);
  applyShowGameTitles();
  try {
    await window.vault.setSettings({ settings: { sgdbApiKey, checkUpdatesOnLaunch, showGameTitles } });
    cachedSettings = { sgdbApiKey, checkUpdatesOnLaunch, showGameTitles };
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
