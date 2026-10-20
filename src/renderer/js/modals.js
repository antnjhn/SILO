/* ── modals.js — add/edit, metadata + art pickers, scan + import flows ─────── */
import { allGames, games, selectedIndex, setSelectedIndex } from './state.js';
import { showToast, esc, parseTags } from './utils.js';
import { getFileSrc } from './api.js';
import { showPage } from './pages.js';
import {
  applyCategoryFilter, renderGameList, renderDetails, updateGameListSelection,
  centerActiveItem, syncRailInstant, crossfadeWallpaper, detailsOpen,
} from './library.js';

// ── Modal-local state ──
let editingGameId = null;
let modalWallpaperPath = null;
let modalLogoPath = null;
let pendingLaunchTarget = null;   // Edit-modal selection: 'auto' | 'launcher' | 'game'

/* ── Add / Edit modal ──────────────────────────────────────────────────────*/
// Launch Target segmented control: 'auto' | 'launcher' | 'game'. SILO inspects
// the chosen exe and shows what it detected; the user can pin the choice.
const LAUNCH_TARGET_IDS = { auto: 'lt-auto', launcher: 'lt-launcher', game: 'lt-game' };

function setLaunchTargetUI(value, hint) {
  Object.entries(LAUNCH_TARGET_IDS).forEach(([val, id]) => {
    document.getElementById(id)?.classList.toggle('active', val === value);
  });
  const hintEl = document.getElementById('launch-target-hint');
  if (hintEl) hintEl.textContent = hint || '';
}

function setLaunchTarget(value) {
  pendingLaunchTarget = value;
  setLaunchTargetUI(value, null);
  refreshLaunchTargetHint();
}

// Ask the backend what the exe looks like and render the guidance line.
async function refreshLaunchTargetHint() {
  const exe = document.getElementById('input-exe').value.trim();
  if (!exe) {
    setLaunchTargetUI(pendingLaunchTarget || 'auto', '');
    return;
  }
  try {
    const info = await window.vault.detectLaunchTarget(exe);
    let hint;
    if (info.launcherLike && info.realGamePath) {
      // Show the path relative to the install folder — "x64/Cyberpunk2077.exe"
      // says more than just the file name.
      const parts = info.realGamePath.replace(/\\/g, '/').split('/');
      const gameExe = parts.slice(-2).join('/');
      hint = `Launcher detected — game found at ${gameExe}. Auto and Game start that binary directly; Launcher starts the exe above. Saves are found either way.`;
    } else if (info.launcherLike) {
      hint = 'Launcher detected, but no game binary was found below it. SILO starts the exe above; saves are still detected by name after the session.';
    } else {
      hint = 'This looks like the game itself. Saves are auto-detected on launch.';
    }
    setLaunchTargetUI(pendingLaunchTarget || 'auto', hint);
  } catch (e) {
    setLaunchTargetUI(pendingLaunchTarget || 'auto', '');
  }
}

function bindLaunchTargetButtons() {
  Object.entries(LAUNCH_TARGET_IDS).forEach(([val, id]) => {
    document.getElementById(id)?.addEventListener('click', () => setLaunchTarget(val));
  });
}
bindLaunchTargetButtons();

export function openAddModal() {
  showPage('import');
  editingGameId = null; modalWallpaperPath = null; modalLogoPath = null;
  document.getElementById('modal-title').textContent = 'Add game';
  document.getElementById('scan-folder-container').style.display = 'flex';
  document.getElementById('import-container').style.display = 'flex';
  document.getElementById('input-name').value = '';
  document.getElementById('input-font').value = '';
  document.getElementById('input-color').value = '#ffffff';
  document.getElementById('color-preview-text').textContent = '#ffffff';
  document.getElementById('input-exe').value = '';
  document.getElementById('input-save-path').value = '';
  // No game to look up yet, so the per-game save lookup has nothing to ask about.
  document.getElementById('btn-find-save-path').style.display = 'none';
  setSaveFolderHint(SAVE_FOLDER_HINT);
  document.getElementById('input-tags').value = '';
  document.getElementById('backup-count-group').style.display = 'none';
  document.getElementById('input-backup-count').value = 5;
  pendingLaunchTarget = 'auto';
  setLaunchTargetUI('auto', '');

  document.getElementById('edit-wallpaper-preview-img').style.display = 'none';
  document.getElementById('edit-logo-preview-img').style.display = 'none';
  updateTitlePreview();

  showPage('import');
  setTimeout(() => document.getElementById('input-name').focus(), 50);
}

export function openEditModal(g) {
  editingGameId = g.id;
  modalWallpaperPath = g.wallpaper || null;
  modalLogoPath = g.logoPath || null;
  document.getElementById('modal-title').textContent = 'Edit game';
  document.getElementById('scan-folder-container').style.display = 'none';
  document.getElementById('import-container').style.display = 'none';
  document.getElementById('input-name').value = g.name;
  document.getElementById('input-font').value = g.fontFamily || '';
  document.getElementById('input-color').value = g.fontColor || '#ffffff';
  document.getElementById('color-preview-text').textContent = g.fontColor || '#ffffff';
  document.getElementById('input-exe').value = g.exePath || '';
  document.getElementById('input-save-path').value = g.savePath || '';
  document.getElementById('btn-find-save-path').style.display = '';
  setSaveFolderHint(SAVE_FOLDER_HINT);
  document.getElementById('input-tags').value = (g.tags || []).join(', ');
  document.getElementById('backup-count-group').style.display = 'block';
  document.getElementById('input-backup-count').value = g.backupCount || 5;
  pendingLaunchTarget = g.launchTarget || 'auto';
  setLaunchTargetUI(pendingLaunchTarget, '');
  refreshLaunchTargetHint();

  const wImg = document.getElementById('edit-wallpaper-preview-img');
  if (modalWallpaperPath) {
    getFileSrc(modalWallpaperPath, wImg);
    wImg.style.display = 'block';
  } else { wImg.style.display = 'none'; }
  
  const lImg = document.getElementById('edit-logo-preview-img');
  if (modalLogoPath) {
    getFileSrc(modalLogoPath, lImg);
    lImg.style.display = 'block';
  } else { lImg.style.display = 'none'; }
  updateTitlePreview();

  showPage('import');
  setTimeout(() => document.getElementById('input-name').focus(), 50);
}

// Color picker hex preview
document.getElementById('input-color').addEventListener('input', (e) => {
  document.getElementById('color-preview-text').textContent = e.target.value;
});

// Live game-name preview for the Title Style field: renders the current game
// name in the chosen font + color as the user changes them.
function updateTitlePreview() {
  const el = document.getElementById('title-preview');
  if (!el) return;
  const name = document.getElementById('input-name').value.trim() || 'Game Title';
  const font = document.getElementById('input-font').value;
  const color = document.getElementById('input-color').value;
  el.textContent = name;
  el.style.fontFamily = font ? `'${font}', sans-serif` : '';
  el.style.color = color;
}
document.getElementById('input-font').addEventListener('change', updateTitlePreview);
document.getElementById('input-color').addEventListener('input', updateTitlePreview);
document.getElementById('input-name').addEventListener('input', updateTitlePreview);

document.getElementById('btn-add-game').addEventListener('click', openAddModal);

document.getElementById('input-name').addEventListener('input', (e) => {
  const val = e.target.value.trim();
  document.getElementById('btn-autofill').style.display = val.length > 2 ? 'inline-block' : 'none';
});

document.getElementById('btn-autofill').addEventListener('click', async () => {
  const name = document.getElementById('input-name').value.trim();
  if (!name) return;

  const btn = document.getElementById('btn-autofill');
  btn.textContent = '⏳ Searching...';

  try {
    const res = await window.vault.fetchMetadata(name);
    const candidates = (res && res.candidates) || [];
    if (!candidates.length) {
      showToast('error', 'No Metadata Found', `No metadata found for "${name}". You can still add it manually.`);
      return;
    }
    const best = candidates[0];
    if (typeof best.confidence === 'number' && best.confidence >= 70) {
      applyMetadata(best);
      return;
    }
    openMetadataPicker(candidates);
  } catch (err) {
    showToast('error', 'API Error', 'Failed to contact metadata service');
  } finally {
    btn.textContent = 'Auto-Fill Metadata';
  }
});

/* ── Metadata picker overlay ────────────────────────────────────────────────*/
function applyMetadata(c) {
  document.getElementById('input-name').value = c.name;
  const lImg = document.getElementById('edit-logo-preview-img');
  if (c.logo) {
    modalLogoPath = c.logo;
    getFileSrc(modalLogoPath, lImg);
    lImg.style.display = 'block';
  }
  const wImg = document.getElementById('edit-wallpaper-preview-img');
  if (c.wallpaper) {
    modalWallpaperPath = c.wallpaper;
    getFileSrc(modalWallpaperPath, wImg);
    wImg.style.display = 'block';
  }
  showToast('success', 'Metadata Applied', `Applied ${c.source || 'metadata'} metadata for ${c.name}`);
}

function openMetadataPicker(candidates) {
  const listEl = document.getElementById('metadata-list');
  listEl.innerHTML = candidates.map((c, i) => {
    const hasThumb = c.logo || c.wallpaper;
    const meta = [];
    if (typeof c.confidence === 'number') meta.push(`Confidence ${Math.round(c.confidence)}%`);
    if (c.year) meta.push(esc(String(c.year)));
    if (c.rating) meta.push(`★ ${esc(String(c.rating))}`);
    return `
    <div class="scan-result-item" data-metadata-index="${i}" style="display: flex; align-items: center; gap: 12px; padding: 10px; background: rgba(0,0,0,0.2); border-radius: 8px; cursor: pointer;">
      ${hasThumb
        ? `<img data-metadata-thumb="${i}" alt="" style="width: 64px; height: 64px; object-fit: contain; border-radius: 6px; background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.1); flex-shrink: 0;" />`
        : `<div style="width: 64px; height: 64px; flex-shrink: 0; display: flex; align-items: center; justify-content: center; font-size: 24px; font-weight: 900; color: rgba(255,255,255,0.35); border-radius: 6px; background: rgba(255,255,255,0.04); border: 1px solid rgba(255,255,255,0.1);">${esc(String(c.name || '?').charAt(0).toUpperCase())}</div>`}
      <div style="flex: 1; min-width: 0;">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-weight: bold; font-size: 14px; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${esc(c.name)}</span>
          <span style="flex-shrink: 0; font-size: 9px; font-weight: 700; letter-spacing: 1px; color: #66ccff; border: 1px solid #3388aa; padding: 2px 6px; border-radius: 4px;">${esc(c.source)}</span>
        </div>
        <div style="font-size: 11px; color: rgba(255,255,255,0.5); margin-top: 2px;">${meta.join(' · ')}</div>
      </div>
    </div>`;
  }).join('');

  listEl.querySelectorAll('[data-metadata-thumb]').forEach(img => {
    const c = candidates[parseInt(img.dataset.metadataThumb, 10)];
    if (!c) return;
    const src = c.logo || c.wallpaper;
    if (src) getFileSrc(src, img);
    else img.style.display = 'none';
  });

  listEl.querySelectorAll('[data-metadata-index]').forEach(el => {
    el.addEventListener('click', () => {
      const c = candidates[parseInt(el.dataset.metadataIndex, 10)];
      if (!c) return;
      applyMetadata(c);
      closeMetadataPicker();
    });
  });

  document.getElementById('metadata-overlay').classList.remove('hidden');
}

export function closeMetadataPicker() {
  document.getElementById('metadata-overlay').classList.add('hidden');
}

document.getElementById('btn-metadata-cancel').addEventListener('click', closeMetadataPicker);

/* ── Online art picker overlay (logo / wallpaper from Steam + SteamGridDB) ──*/
let onlineArtMode = 'logo'; // 'logo' | 'wallpaper'

function showArtNotice(msg) {
  const el = document.getElementById('online-art-notice');
  if (!el) return;
  if (msg) { el.textContent = msg; el.style.display = 'block'; }
  else { el.style.display = 'none'; }
}

function openOnlineArtPicker(mode) {
  onlineArtMode = mode;
  const sub = document.getElementById('online-art-sub');
  const searchInput = document.getElementById('online-art-search');
  if (sub) {
    sub.textContent = mode === 'logo'
      ? 'Search a game to pick its logo. Click an image to use it.'
      : 'Search a game to pick its wallpaper. Click an image to use it.';
  }
  searchInput.value = document.getElementById('input-name').value.trim() || '';
  document.getElementById('online-art-list').innerHTML = '';
  document.getElementById('online-art-empty').style.display = 'none';
  showArtNotice(null);
  document.getElementById('online-art-overlay').classList.remove('hidden');
  searchInput.focus();
  if (searchInput.value.length >= 2) runOnlineArtSearch();
}

async function runOnlineArtSearch() {
  const query = document.getElementById('online-art-search').value.trim();
  const listEl = document.getElementById('online-art-list');
  const emptyEl = document.getElementById('online-art-empty');
  if (!query) return;
  listEl.innerHTML = '<div style="text-align:center; padding:20px; color:rgba(255,255,255,0.5);">Searching…</div>';
  emptyEl.style.display = 'none';
  showArtNotice(null);

  let res;
  try {
    res = await window.vault.fetchMetadata(query);
  } catch (err) {
    listEl.innerHTML = '';
    emptyEl.style.display = 'block';
    showToast('error', 'Search Failed', 'Could not reach the art service. Check your connection.');
    return;
  }

  const candidates = (res && res.candidates) || [];
  if (!candidates.length) {
    listEl.innerHTML = '';
    emptyEl.style.display = 'block';
    return;
  }

  listEl.innerHTML = candidates.map((c) => {
    const arts = onlineArtMode === 'logo' ? (c.logos || []) : (c.wallpapers || []);
    const badge = `<span style="flex-shrink:0; font-size:9px; font-weight:700; letter-spacing:1px; color:#66ccff; border:1px solid #3388aa; padding:2px 6px; border-radius:4px;">${esc(c.source)}</span>`;
    const thumbs = arts.length
      ? arts.map((u) => `<img class="online-art-thumb" data-url="${esc(u)}" alt="" title="${esc(u)}" />`).join('')
      : (onlineArtMode === 'logo'
          ? '<div style="font-size:11px; color:rgba(255,255,255,0.35); padding:8px 0;">No logo available</div>'
          : '<div style="font-size:11px; color:rgba(255,255,255,0.35); padding:8px 0;">No wallpaper available</div>');
    return `
    <div style="display:flex; align-items:center; gap:12px; padding:10px; background:rgba(0,0,0,0.2); border-radius:8px;">
      <div style="flex:1; min-width:0; display:flex; flex-direction:column; gap:6px;">
        <div style="display:flex; align-items:center; gap:8px;">
          <span style="font-weight:bold; font-size:14px; color:#fff; white-space:nowrap; overflow:hidden; text-overflow:ellipsis;">${esc(c.name)}</span>
          ${badge}
        </div>
        <div style="display:flex; gap:8px; flex-wrap:wrap;">${thumbs}</div>
      </div>
    </div>`;
  }).join('');

  listEl.querySelectorAll('.online-art-thumb').forEach(img => {
    getFileSrc(img.dataset.url, img);
    img.addEventListener('click', () => applyOnlineArt(img.dataset.url));
  });

  // Surface SteamGridDB status so a missing source is never a silent mystery.
  if (res.hasSgdb && res.sgdbCount === 0) {
    showArtNotice('SteamGridDB is enabled but returned no art for this search. Check Settings → Open Logs Folder.');
  } else if (!res.hasSgdb) {
    showArtNotice('SteamGridDB off - add your free key in Settings to see custom art.');
  }
}

function applyOnlineArt(url) {
  if (!url) return;
  if (onlineArtMode === 'logo') {
    modalLogoPath = url;
    const lImg = document.getElementById('edit-logo-preview-img');
    getFileSrc(url, lImg);
    lImg.style.display = 'block';
    showToast('success', 'Logo Applied', 'This image will be used as the game logo.');
  } else {
    modalWallpaperPath = url;
    const wImg = document.getElementById('edit-wallpaper-preview-img');
    getFileSrc(url, wImg);
    wImg.style.display = 'block';
    showToast('success', 'Wallpaper Applied', 'This image will be used as the background.');
  }
  closeOnlineArtPicker();
}

export function closeOnlineArtPicker() {
  document.getElementById('online-art-overlay').classList.add('hidden');
}

document.getElementById('btn-online-logo').addEventListener('click', () => openOnlineArtPicker('logo'));
document.getElementById('btn-online-wallpaper').addEventListener('click', () => openOnlineArtPicker('wallpaper'));
document.getElementById('btn-online-art-go').addEventListener('click', runOnlineArtSearch);
document.getElementById('btn-online-art-cancel').addEventListener('click', closeOnlineArtPicker);
document.getElementById('online-art-search').addEventListener('keydown', (e) => {
  if (e.key === 'Enter') { e.preventDefault(); runOnlineArtSearch(); }
});

// Browse for .exe — pick a single file and auto-fill the form
document.getElementById('btn-browse-exe').addEventListener('click', async () => {
  const p = await window.vault.pickExe();
  if (p) {
    document.getElementById('input-exe').value = p;
    // Auto-fill game name from parent folder or exe stem
    const parts = p.replace(/\\/g, '/').split('/');
    const exeFile = parts[parts.length - 1] || '';
    const parentFolder = parts[parts.length - 2] || '';
    // Use parent folder name unless it's a generic folder like 'bin', 'win64', etc.
    const genericFolders = ['bin', 'win64', 'win32', 'binaries', 'x64', 'x86', 'game', 'shipping'];
    let gameName = parentFolder;
    if (genericFolders.includes(parentFolder.toLowerCase())) {
      gameName = parts[parts.length - 3] || exeFile.replace(/\.exe$/i, '');
    }
    if (!gameName) gameName = exeFile.replace(/\.exe$/i, '');
    
    const nameInput = document.getElementById('input-name');
    if (!nameInput.value.trim()) {
      nameInput.value = gameName;
      // Trigger autofill button visibility
      nameInput.dispatchEvent(new Event('input'));
    }
  }
});

// Scan folder — show checklist overlay for selection
let pendingScanResults = [];
// Import candidates (Steam / Epic / GOG) awaiting confirmation
let pendingImportResults = [];

document.getElementById('btn-scan-folder').addEventListener('click', async () => {
  const { open } = window.__TAURI__.dialog;
  const { invoke } = window.__TAURI__.core;
  
  try {
    const selectedPath = await open({
      directory: true,
      multiple: false,
      title: 'Select Game Directory to Scan'
    });
    
    if (selectedPath) {
      const btn = document.getElementById('btn-scan-folder');
      const originalText = btn.innerHTML;
      btn.innerHTML = '⏳ Scanning...';
      
      const results = await invoke('scan_folder', { folderPath: selectedPath });
      btn.innerHTML = originalText;
      
      if (results && results.length > 0) {
        // Filter out games we already have
        const newResults = results.filter(r => !allGames.find(g => g.exePath === r.exe_path));
        
        if (newResults.length === 0) {
          showToast('info', 'Scan Complete', 'All games from that folder are already in your library.');
          return;
        }
        
        pendingScanResults = newResults;
        document.getElementById('scan-results-title').textContent = 'Scan results';

        // Build the checklist
        const listEl = document.getElementById('scan-results-list');
        listEl.innerHTML = newResults.map((r, i) => {
          const exeFile = r.exe_path.replace(/\\/g, '/').split('/').pop();
          return `
          <label class="scan-result-item" style="display: flex; align-items: center; gap: 10px; padding: 10px; background: rgba(0,0,0,0.2); border-radius: 8px; cursor: pointer;">
            <input type="checkbox" checked data-scan-index="${i}" style="accent-color: #66ccff; width: 18px; height: 18px; flex-shrink: 0;" />
            <div style="flex: 1; min-width: 0;">
              <div style="font-weight: bold; font-size: 14px; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${esc(r.name)}</div>
              <div style="font-size: 11px; color: rgba(255,255,255,0.4); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;" title="${esc(r.exe_path)}">${esc(exeFile)}</div>
            </div>
          </label>`;
        }).join('');
        
        updateScanCount();
        
        // Listen for checkbox changes to update count
        listEl.querySelectorAll('input[type="checkbox"]').forEach(cb => {
          cb.addEventListener('change', updateScanCount);
        });
        
        // Close the add modal and show scan results
        showPage('home');
        document.getElementById('scan-results-overlay').classList.remove('hidden');
      } else {
        showToast('error', 'Scan Failed', 'No valid game executables found in that folder.');
      }
    }
  } catch (err) {
    console.error(err);
    showToast('error', 'Scan Error', 'Something went wrong while scanning.');
  }
});

function updateScanCount() {
  const checked = document.querySelectorAll('#scan-results-list input[type="checkbox"]:checked').length;
  const total = document.querySelectorAll('#scan-results-list input[type="checkbox"]').length;
  document.getElementById('scan-selected-count').textContent = `${checked} of ${total} selected`;
  document.getElementById('btn-scan-add').disabled = checked === 0;
}

document.getElementById('btn-scan-cancel').addEventListener('click', () => {
  document.getElementById('scan-results-overlay').classList.add('hidden');
  showPage('import');
  pendingScanResults = [];
  pendingImportResults = [];
});

document.getElementById('btn-scan-add').addEventListener('click', async () => {
  const checkboxes = document.querySelectorAll('#scan-results-list input[type="checkbox"]:checked');
  const indices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.scanIndex));

  let addedCount = 0;
  if (pendingImportResults.length) {
    for (const idx of indices) {
      const game = pendingImportResults[idx];
      if (!game) continue;
      const added = await window.vault.addGame({ name: game.name, exePath: game.exePath || null });
      if (added) {
        allGames.push(added);
        addedCount++;
      }
    }
    pendingImportResults = [];
  } else {
    for (const idx of indices) {
      const game = pendingScanResults[idx];
      if (!game) continue;

      const newGame = {
        name: game.name,
        exePath: game.exe_path,
        fontColor: '#ffffff',
        fontFamily: '',
      };
      const added = await window.vault.addGame(newGame);
      if (added) {
        allGames.push(added);
        addedCount++;
      }
    }
    pendingScanResults = [];
  }

  document.getElementById('scan-results-overlay').classList.add('hidden');
  showPage('import');

  if (addedCount > 0) {
    applyCategoryFilter(false);
    renderGameList();
    showToast('success', 'Added', `${addedCount} game${addedCount > 1 ? 's' : ''} added to SILO!`);
  }
});

/* ── Library imports (Steam / Epic / GOG) ───────────────────────────────────*/
async function runImport(source) {
  const btn = document.getElementById('btn-import-' + source.toLowerCase());
  let candidates;
  try {
    if (btn) btn.textContent = '⏳ Importing...';
    candidates = await window.vault['import' + source + 'Library']();
  } catch (err) {
    showToast('error', 'Import Failed', err);
    return;
  } finally {
    if (btn) btn.textContent = 'Import ' + source;
  }

  const knownExes = new Set(allGames.map(g => g.exePath).filter(Boolean));
  const fresh = (candidates || []).filter(c => {
    if (!c.exePath) return true; // importable, but exe not found
    return !knownExes.has(c.exePath);
  });

  if (fresh.length === 0) {
    showToast('info', 'Nothing New', `No new ${source} games found in your library.`);
    return;
  }

  pendingImportResults = fresh;
  document.getElementById('scan-results-title').textContent = `${source} import`;
  const listEl = document.getElementById('scan-results-list');
  listEl.innerHTML = fresh.map((c, i) => {
    const exeLine = c.exePath
      ? `<div style="font-size: 11px; color: rgba(255,255,255,0.4); white-space: nowrap; overflow: hidden; text-overflow: ellipsis;" title="${esc(c.exePath)}">${esc(c.exePath)}</div>`
      : `<div style="font-size: 11px; color: rgba(255,255,255,0.25); font-style: italic;">(no exe found)</div>`;
    return `
    <label class="scan-result-item" style="display: flex; align-items: center; gap: 10px; padding: 10px; background: rgba(0,0,0,0.2); border-radius: 8px; cursor: pointer;">
      <input type="checkbox" checked data-scan-index="${i}" style="accent-color: #66ccff; width: 18px; height: 18px; flex-shrink: 0;" />
      <div style="flex: 1; min-width: 0;">
        <div style="display: flex; align-items: center; gap: 8px;">
          <span style="font-weight: bold; font-size: 14px; color: #fff; white-space: nowrap; overflow: hidden; text-overflow: ellipsis;">${esc(c.name)}</span>
          <span style="flex-shrink: 0; font-size: 9px; font-weight: 700; letter-spacing: 1px; color: #66ccff; border: 1px solid #3388aa; padding: 2px 6px; border-radius: 4px;">${esc(source)}</span>
        </div>
        ${exeLine}
      </div>
    </label>`;
  }).join('');

  updateScanCount();
  listEl.querySelectorAll('input[type="checkbox"]').forEach(cb => {
    cb.addEventListener('change', updateScanCount);
  });

  showPage('home');
  document.getElementById('scan-results-overlay').classList.remove('hidden');
}

document.getElementById('btn-import-steam').addEventListener('click', () => runImport('Steam'));
document.getElementById('btn-import-epic').addEventListener('click', () => runImport('Epic'));
document.getElementById('btn-import-gog').addEventListener('click', () => runImport('GOG'));

/* ── Save folder pickers in add/edit modal ──────────────────────────────────*/
// The default line, restored every time the dialog opens so a previous lookup's
// result never lingers over a different game's folder.
const SAVE_FOLDER_HINT = 'An auto-detected save path shows here once the game is launched. If the library came from another PC, use Find on this PC.';
// How much to trust a folder SILO proposes for one game, in the user's words.
// The bulk relink refuses the first two outright; here the user decides.
const SAVE_FOLDER_CONFIDENCE = {
  guess: "it matches this game's name alone, so check it holds this game's saves",
  likely: 'it sits in a Proton or Wine prefix SILO cannot tie to this game, so check it is the right folder',
};

function setSaveFolderHint(text) {
  const hint = document.getElementById('save-folder-hint');
  if (hint) hint.textContent = text || '';
}

// "Find on this PC" — the per-game counterpart to the relink. The bulk relink
// will not act on a match it cannot tie to the game, but this is one game the
// user asked about and the path is shown before Save, so the candidate is
// offered with its strength spelled out rather than withheld.
document.getElementById('btn-find-save-path')?.addEventListener('click', async () => {
  if (!editingGameId) return;
  const btn = document.getElementById('btn-find-save-path');
  const original = btn.textContent;
  btn.disabled = true;
  btn.textContent = 'Searching…';
  try {
    const proposal = await window.vault.proposeSaveFolder(editingGameId);
    if (!proposal) {
      setSaveFolderHint(document.getElementById('input-save-path').value.trim()
        ? 'That save folder already works on this machine.'
        : 'No save folder is recorded yet — use Browse to pick one.');
      return;
    }
    if (!proposal.foundPath) {
      setSaveFolderHint(proposal.reason);
      return;
    }
    document.getElementById('input-save-path').value = proposal.foundPath;
    const note = SAVE_FOLDER_CONFIDENCE[proposal.confidence];
    setSaveFolderHint(note
      ? `Found ${proposal.foundPath} — ${note}. Save to use it.`
      : `Found ${proposal.foundPath}. Save to use it.`);
  } catch (err) {
    showToast('error', 'Search Failed', err);
  } finally {
    btn.disabled = false;
    btn.textContent = original;
  }
});

document.getElementById('btn-pick-save-path').addEventListener('click', async () => {
  try {
    const p = await window.vault.pickSaveFolder();
    if (p) document.getElementById('input-save-path').value = p;
  } catch (err) {
    showToast('error', 'Folder Error', err);
  }
});

document.getElementById('btn-clear-save-path').addEventListener('click', () => {
  document.getElementById('input-save-path').value = '';
});

document.getElementById('btn-pick-exe').addEventListener('click', async () => {
  const p = await window.vault.pickExe();
  if (p) {
    document.getElementById('input-exe').value = p;
    refreshLaunchTargetHint();
  }
});
document.getElementById('input-exe')?.addEventListener('change', () => refreshLaunchTargetHint());

document.getElementById('btn-pick-logo').addEventListener('click', async () => {
  if (!editingGameId) return;
  try {
    const p = await window.vault.pickLogo(editingGameId);
    if (p) {
      modalLogoPath = p;
      const img = document.getElementById('edit-logo-preview-img');
      getFileSrc(modalLogoPath, img);
      img.style.display = 'block';
    }
  } catch (err) {
    showToast('error', 'Logo Error', err);
  }
});

document.getElementById('btn-pick-wallpaper-modal').addEventListener('click', async () => {
  try {
    const p = await window.vault.pickWallpaper(editingGameId || 'new_' + Date.now());
    if (p) { 
      modalWallpaperPath = p; 
      const img = document.getElementById('edit-wallpaper-preview-img');
      getFileSrc(modalWallpaperPath, img);
      img.style.display = 'block';
    }
  } catch (err) {
    showToast('error', 'Wallpaper Error', err);
  }
});

document.getElementById('btn-modal-cancel').addEventListener('click', () => {
  showPage('home');
});

document.getElementById('btn-modal-save').addEventListener('click', async () => {
  const name = document.getElementById('input-name').value.trim();
  const exePath = document.getElementById('input-exe').value.trim();
  const fontFamily = document.getElementById('input-font').value;
  const fontColor = document.getElementById('input-color').value;
  const savePath = document.getElementById('input-save-path').value.trim();
  const tags = parseTags(document.getElementById('input-tags').value);
  const backupCount = parseInt(document.getElementById('input-backup-count').value, 10);
  const inp = document.getElementById('input-name');
  if (!name) { inp.style.borderColor = 'rgba(255,100,100,0.6)'; setTimeout(() => inp.style.borderColor = '', 1500); inp.focus(); return; }

  try {
    if (editingGameId) {
      const updates = {
        name, exePath: exePath || undefined,
        wallpaper: modalWallpaperPath || undefined,
        logoPath: modalLogoPath || undefined,
        fontFamily: fontFamily || undefined,
        fontColor: fontColor || undefined,
        savePath: savePath || null,
        launchTarget: pendingLaunchTarget || null,
        tags,
      };
      if (!isNaN(backupCount) && backupCount >= 1 && backupCount <= 50) updates.backupCount = backupCount;
      const u = await window.vault.updateGame(editingGameId, updates);
      const idx = allGames.findIndex(g => g.id === editingGameId);
      if (idx !== -1 && u) allGames[idx] = u;
      applyCategoryFilter(false);
      if (detailsOpen && games[selectedIndex]?.id === editingGameId) renderDetails(u);
      showToast('success', 'Updated', `${name} saved`);
    } else {
      const g = await window.vault.addGame({ name, exePath, wallpaper: modalWallpaperPath, logoPath: modalLogoPath, fontFamily: fontFamily || undefined, fontColor: fontColor || undefined, savePath: savePath || null, launchTarget: pendingLaunchTarget || null });
      if (g && tags.length) {
        try {
          const updated = await window.vault.updateGame(g.id, { tags });
          if (updated) g.tags = updated.tags;
        } catch (e) { /* tags are optional — ignore failures */ }
      }
      allGames.push(g);
      applyCategoryFilter(false);
      setSelectedIndex(games.length - 1);
      showToast('success', 'Added', `${name} added to SILO`);
    }

    updateGameListSelection();
    centerActiveItem(true);
    syncRailInstant();
    crossfadeWallpaper();
    showPage('home');
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

