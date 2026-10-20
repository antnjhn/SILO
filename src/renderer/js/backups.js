/* ── backups.js — restore modal, backup manager, library export/import ─────── */
import { showToast, esc, escJs } from './utils.js';
import { showPage } from './pages.js';

/* ── Launch ────────────────────────────────────────────────────────────────*/
window.doDeleteBackup = async (gameId, backupName) => {
  const { ask } = window.__TAURI__.dialog;
  const yes = await ask('Are you sure you want to delete this backup?', { title: 'Delete Backup', kind: 'warning' });
  if (yes) {
    try {
      await window.__TAURI__.core.invoke('delete_backup', { gameId, backupName });
      showToast('success', 'Deleted', 'Backup deleted.');
      
      // Refresh the backup list if it's currently open
      const btn = document.getElementById('btn-restore-save');
      if (btn && btn.onclick) {
        btn.onclick();
      }
    } catch (err) {
      showToast('error', 'Delete Failed', err);
    }
  }
};

document.getElementById('btn-restore-cancel')?.addEventListener('click', () => {
  document.getElementById('restore-overlay').classList.add('hidden');
});

window.doRestore = async (gameId, backupName) => {
  showToast('info', 'Restoring', 'Restoring save file from backup...');
  document.getElementById('restore-overlay').classList.add('hidden');
  try {
    await window.vault.restoreBackup(gameId, backupName);
    showToast('success', 'Restore Complete', 'Save files have been restored.');
  } catch (err) {
    showToast('error', 'Restore Failed', err);
  }
};

/* ── Backup manager ─────────────────────────────────────────────────────────*/
document.getElementById('btn-backup-manager').addEventListener('click', () => {
  showPage('home');
  openBackupManager();
});
document.getElementById('btn-backup-manager-close').addEventListener('click', () => {
  document.getElementById('backup-manager-overlay').classList.add('hidden');
});

async function openBackupManager() {
  const overlay = document.getElementById('backup-manager-overlay');
  const listEl = document.getElementById('backup-manager-list');
  overlay.classList.remove('hidden');
  listEl.innerHTML = '';

  let backups;
  try {
    backups = await window.vault.getAllBackups();
  } catch (err) {
    showToast('error', 'Load Failed', err);
    return;
  }

  if (!backups.length) {
    listEl.innerHTML = '<div style="color: rgba(255,255,255,0.5); font-size: 13px; text-align: center; padding: 20px;">No backups yet. Launch a game to start saving backups.</div>';
    return;
  }

  const groups = new Map();
  backups.forEach(b => {
    if (!groups.has(b.gameId)) groups.set(b.gameId, { gameName: b.gameName, backups: [] });
    groups.get(b.gameId).backups.push(b);
  });

  let html = '';
  groups.forEach((grp, gameId) => {
    html += `
      <div>
        <div style="display: flex; align-items: center; justify-content: space-between; margin-bottom: 8px;">
          <div style="font-weight: 700; color: #fff; font-size: 14px;">${esc(grp.gameName)}</div>
          <button class="glass-btn small" style="border-color: #ff5555; color: #ff5555;" onclick="deleteAllGameBackups('${escJs(gameId)}', '${escJs(grp.gameName)}')">Delete All</button>
        </div>
        <div style="display: flex; flex-direction: column; gap: 6px;">
          ${grp.backups.map(b => managedBackupRow(gameId, b)).join('')}
        </div>
      </div>`;
  });
  listEl.innerHTML = html;
}

function managedBackupRow(gameId, b) {
  const typeTag = b.isAuto
    ? '<span style="color: #a0a0a0; font-size: 10px; border: 1px solid #555; padding: 2px 4px; border-radius: 4px; margin-right: 6px;">AUTO</span>'
    : '<span style="color: #66ccff; font-size: 10px; border: 1px solid #3388aa; padding: 2px 4px; border-radius: 4px; margin-right: 6px;">MANUAL</span>';
  const name = b.customName
    ? `<span style="font-weight: bold; color: #fff;">${esc(b.customName)}</span>`
    : '<span style="color: rgba(255,255,255,0.5);">(auto)</span>';
  const size = b.sizeBytes >= 1048576
    ? `${(b.sizeBytes / 1048576).toFixed(2)} MB`
    : `${(b.sizeBytes / 1024).toFixed(1)} KB`;
  return `
    <div style="display: flex; align-items: center; justify-content: space-between; gap: 8px; padding: 10px; background: rgba(0,0,0,0.2); border-radius: 8px;">
      <div style="display: flex; align-items: center; gap: 8px; min-width: 0; flex: 1;">
        ${typeTag}
        <div style="min-width: 0; flex: 1;">
          <div style="font-size: 12px; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${name}</div>
          <div style="font-size: 10px; color: rgba(255,255,255,0.5);">${esc(b.timestamp)} · ${size}</div>
        </div>
      </div>
      <div style="display: flex; gap: 6px; flex-shrink: 0;">
        <button class="glass-btn small" onclick="restoreManagedBackup('${escJs(gameId)}', '${escJs(b.name)}')">Restore</button>
        <button class="glass-btn small" style="border-color: #ff5555; color: #ff5555;" onclick="deleteManagedBackup('${escJs(gameId)}', '${escJs(b.name)}')">Delete</button>
      </div>
    </div>`;
}

window.restoreManagedBackup = async (gameId, backupName) => {
  const { ask } = window.__TAURI__.dialog;
  const yes = await ask('Restore this backup? It will overwrite the current save files.', { title: 'Restore Backup', kind: 'warning' });
  if (!yes) return;
  try {
    await window.vault.restoreBackup(gameId, backupName);
    showToast('success', 'Restore Complete', 'Save files have been restored.');
  } catch (err) {
    showToast('error', 'Restore Failed', err);
  }
};

window.deleteManagedBackup = async (gameId, backupName) => {
  const { ask } = window.__TAURI__.dialog;
  const yes = await ask('Delete this backup?', { title: 'Delete Backup', kind: 'warning' });
  if (!yes) return;
  try {
    await window.__TAURI__.core.invoke('delete_backup', { gameId, backupName });
    showToast('success', 'Deleted', 'Backup deleted.');
    openBackupManager();
  } catch (err) {
    showToast('error', 'Delete Failed', err);
  }
};

import { markSetupPending } from './migrate.js';

window.deleteAllGameBackups = async (gameId, gameName) => {
  const { ask } = window.__TAURI__.dialog;
  const yes = await ask(`Delete ALL backups for ${gameName}? This cannot be undone.`, { title: 'Delete All Backups', kind: 'warning' });
  if (!yes) return;
  try {
    await window.vault.deleteGameBackups(gameId);
    showToast('success', 'Deleted', `All backups for ${gameName} deleted.`);
    openBackupManager();
  } catch (err) {
    showToast('error', 'Delete Failed', err);
  }
};

document.getElementById('btn-export-backup').addEventListener('click', async () => {
  try {
    const dest = await window.vault.backupLibrary();
    if (dest) {
      showToast('success', 'Export Complete', `Library backed up to ${dest}`);
      showPage('home');
    }
  } catch (e) {
    if (e !== 'Backup cancelled') showToast('error', 'Export Failed', e);
  }
});

document.getElementById('btn-import-backup').addEventListener('click', async () => {
  const { ask } = window.__TAURI__.dialog;
  const confirmed = await ask("Restoring a backup will overwrite your current library and settings. Proceed?", { title: 'Import Backup', kind: 'warning' });
  if (!confirmed) return;
  
  try {
    const res = await window.vault.restoreLibrary();
    if (res) {
      // The restored library carries another PC's paths. Flag the setup step so
      // the reload lands on it instead of a rail full of folders that are gone.
      markSetupPending();
      showToast('success', 'Import Complete', `Library restored. Reloading...`);
      setTimeout(() => window.location.reload(), 1500);
    }
  } catch (e) {
    if (e !== 'Restore cancelled') showToast('error', 'Import Failed', e);
  }
});

