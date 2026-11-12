/* ── modals.js — add/edit, metadata + art pickers, scan + import flows ─────── */
import { allGames, games, selectedIndex, setSelectedIndex } from './state.js';
import { showToast, esc, parseTags } from './utils.js';
import { getFileSrc } from './api.js';
import { initArtPicker, openArtPicker, closeArtPicker } from './artpicker.js';
import { showPage, currentPage } from './pages.js';
import {
  applyCategoryFilter, renderGameList, renderDetails, updateGameListSelection,
  centerActiveItem, syncRailInstant, crossfadeWallpaper, detailsOpen,
  refreshInstallStatus,
} from './library.js';

// ── Modal-local state ──
let editingGameId = null;
let modalWallpaperPath = null;
let modalLogoPath = null;
let stagedGifPolicy = null;
let pendingLaunchTarget = null;   // Edit-modal selection: 'auto' | 'launcher' | 'game'

/* The one way an artwork preview is set. `.shown` is the reveal rule in
   style.css; no inline display is written, so opening a different modal can
   never inherit the previous game's image (and a cleared field clears both the
   class and the source). */
/* Small status line under the artwork boxes: says what SILO is doing while it
   fetches artwork for a brand-new game, and why nothing arrived when it could
   not find any. */
function setArtworkHint(text) {
  const hint = document.getElementById('artwork-hint');
  if (!hint) return;
  hint.textContent = text || '';
  hint.hidden = !text;
}

function setAssetPreview(img, path) {
  if (!img) return;
  img.style.display = '';
  if (!path) {
    img.removeAttribute('src');
    img.classList.remove('shown');
    return;
  }
  if (/\.gif(?:$|[?#])/i.test(path)) img.removeAttribute('src');
  else getFileSrc(path, img);
  img.classList.add('shown');
}

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
  stagedGifPolicy = null;
  showPage('import');
  editingGameId = null; modalWallpaperPath = null; modalLogoPath = null;
  document.getElementById('modal-title').textContent = 'Add game';
  document.getElementById('scan-folder-container').hidden = false;
  document.getElementById('import-container').hidden = false;
  document.getElementById('input-name').value = '';
  document.getElementById('input-font').value = '';
  document.getElementById('input-color').value = '#ffffff';
  document.getElementById('color-preview-text').textContent = '#ffffff';
  document.getElementById('input-exe').value = '';
  document.getElementById('input-save-path').value = '';
  // No game to look up yet, so the per-game save lookup has nothing to ask about.
  document.getElementById('btn-find-save-path').hidden = true;
  setSaveFolderHint(SAVE_FOLDER_HINT);
  document.getElementById('input-tags').value = '';
  document.getElementById('backup-count-group').style.display = 'none';
  document.getElementById('input-backup-count').value = 5;
  pendingLaunchTarget = 'auto';
  setLaunchTargetUI('auto', '');

  setAssetPreview(document.getElementById('edit-wallpaper-preview-img'), null);
  setAssetPreview(document.getElementById('edit-logo-preview-img'), null);
  setArtworkHint(null);
  updateTitlePreview();

  showPage('import');
  setTimeout(() => document.getElementById('input-name').focus(), 50);
}

export function openEditModal(g) {
  stagedGifPolicy = null;
  editingGameId = g.id;
  modalWallpaperPath = g.wallpaper || null;
  modalLogoPath = g.logoPath || null;
  document.getElementById('modal-title').textContent = 'Edit game';
  document.getElementById('scan-folder-container').hidden = true;
  document.getElementById('import-container').hidden = true;
  document.getElementById('input-name').value = g.name;
  document.getElementById('input-font').value = g.fontFamily || '';
  document.getElementById('input-color').value = g.fontColor || '#ffffff';
  document.getElementById('color-preview-text').textContent = g.fontColor || '#ffffff';
  document.getElementById('input-exe').value = g.exePath || '';
  document.getElementById('input-save-path').value = g.savePath || '';
  document.getElementById('btn-find-save-path').hidden = false;
  setSaveFolderHint(SAVE_FOLDER_HINT);
  document.getElementById('input-tags').value = (g.tags || []).join(', ');
  document.getElementById('backup-count-group').style.display = 'block';
  document.getElementById('input-backup-count').value = g.backupCount || 5;
  pendingLaunchTarget = g.launchTarget || 'auto';
  setLaunchTargetUI(pendingLaunchTarget, '');
  refreshLaunchTargetHint();

  // Both boxes are set from this game's own paths, so an empty field is empty
  // and a stored logo/wallpaper is visible.
  setAssetPreview(document.getElementById('edit-wallpaper-preview-img'), modalWallpaperPath);
  setAssetPreview(document.getElementById('edit-logo-preview-img'), modalLogoPath);
  setArtworkHint(null);
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
  document.getElementById('btn-autofill').hidden = val.length <= 2;
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
  if (c.logo) {
    modalLogoPath = c.logo;
    setAssetPreview(document.getElementById('edit-logo-preview-img'), modalLogoPath);
  }
  if (c.wallpaper) {
    modalWallpaperPath = c.wallpaper;
    setAssetPreview(document.getElementById('edit-wallpaper-preview-img'), modalWallpaperPath);
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

/* ── Online art picker overlay (SteamGridDB browse, see artpicker.js) ───────*/
// The picker owns its own state and rendering; this module only owns the form
// fields an applied image lands in.
let onlineArtMode = 'logo'; // 'logo' | 'wallpaper'

async function applyOnlineArt(url, art) {
  if (!url) return;
  if (onlineArtMode === 'logo') {
    const applied = await chooseLogo(url, Boolean(art?.animated) || /\.gif(?:$|[?#])/i.test(url));
    if (applied) {
      showToast('success', 'Logo Applied', 'This image will be used as the game logo.');
    }
  } else {
    modalWallpaperPath = url;
    setAssetPreview(document.getElementById('edit-wallpaper-preview-img'), url);
    showToast('success', 'Wallpaper Applied', 'This image will be used as the background.');
  }
  closeOnlineArtPicker();
}

export function closeOnlineArtPicker() {
  closeArtPicker();
}

document.getElementById('btn-online-logo').addEventListener('click', () => {
  onlineArtMode = 'logo';
  openArtPicker('logo', { apply: applyOnlineArt });
});
document.getElementById('btn-online-wallpaper').addEventListener('click', () => {
  onlineArtMode = 'wallpaper';
  openArtPicker('wallpaper', { apply: applyOnlineArt });
});
document.getElementById('btn-online-art-cancel').addEventListener('click', closeOnlineArtPicker);

initArtPicker();

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
  try {
    // While adding there is no id yet; the backend files artwork by folder name,
    // exactly as the wallpaper picker already does for a new game.
    const p = await window.vault.pickLogo(editingGameId || 'new_' + Date.now());
    if (p) await chooseLogo(p);
  } catch (err) {
    showToast('error', 'Logo Error', err);
  }
});

function setGifPolicy(gameId, path, policy) {
  try {
    const key = `${gameId || 'new'}:${path}`;
    const policies = JSON.parse(localStorage.getItem('silo.gifPolicies') || '{}');
    if (policy) policies[key] = policy;
    else delete policies[key];
    localStorage.setItem('silo.gifPolicies', JSON.stringify(policies));
  } catch { /* storage may be disabled */ }
}

async function chooseLogo(path, animated = /\.gif(?:$|[?#])/i.test(path)) {
  let policy = null;
  if (animated) {
    const decision = await new Promise((resolve) => {
      const prior = document.getElementById('btn-pick-logo');
      const overlay = document.getElementById('gif-policy-overlay');
      const dialog = overlay.querySelector('[role="dialog"]') || overlay;
      overlay.classList.remove('hidden');
      dialog.setAttribute('aria-modal', 'true');
      const close = (value) => {
        overlay.classList.add('hidden');
        document.removeEventListener('keydown', onKey);
        prior?.focus?.();
        resolve(value);
      };
      const onKey = (event) => {
        if (event.key === 'Escape') { event.preventDefault(); close(null); return; }
        if (event.key !== 'Tab') return;
        const controls = [...dialog.querySelectorAll('button:not([disabled]), [href], input:not([disabled]), [tabindex]:not([tabindex="-1"])')].filter(el => el.getClientRects().length);
        if (!controls.length) { event.preventDefault(); dialog.focus(); return; }
        const first = controls[0], last = controls[controls.length - 1];
        if (event.shiftKey && (document.activeElement === first || !dialog.contains(document.activeElement))) { event.preventDefault(); last.focus(); }
        else if (!event.shiftKey && (document.activeElement === last || !dialog.contains(document.activeElement))) { event.preventDefault(); first.focus(); }
      };
      document.addEventListener('keydown', onKey);
      overlay.querySelector('[data-policy="once"]').onclick = () => close('once');
      overlay.querySelector('[data-policy="loop"]').onclick = () => close('loop');
      overlay.querySelector('[data-policy="cancel"]').onclick = () => close(null);
      overlay.querySelector('[data-policy="once"]').focus();
    });
    if (!decision) return false;
    policy = decision;
  }
  modalLogoPath = path;
  stagedGifPolicy = { path, policy };
  setAssetPreview(document.getElementById('edit-logo-preview-img'), path);
  return true;
}

document.getElementById('btn-pick-wallpaper-modal').addEventListener('click', async () => {
  try {
    const p = await window.vault.pickWallpaper(editingGameId || 'new_' + Date.now());
    if (p) {
      modalWallpaperPath = p;
      setAssetPreview(document.getElementById('edit-wallpaper-preview-img'), modalWallpaperPath);
    }
  } catch (err) {
    showToast('error', 'Wallpaper Error', err);
  }
});

document.getElementById('btn-modal-cancel').addEventListener('click', () => {
  stagedGifPolicy = null;
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

  let created = null;
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
      const priorLogo = idx === -1 ? null : allGames[idx].logoPath;
      if (priorLogo && priorLogo !== modalLogoPath) setGifPolicy(editingGameId, priorLogo, null);
      if (stagedGifPolicy?.path === modalLogoPath) setGifPolicy(editingGameId, stagedGifPolicy.path, stagedGifPolicy.policy);
      stagedGifPolicy = null;
      if (!u) throw new Error('The game record was not found, so nothing was changed.');
      if (idx !== -1) allGames[idx] = u;
      applyCategoryFilter(false);
      // Repaint the panel from the stored record. The dock buttons and the Edit
      // modal read the object they were rendered with, so leaving the old one
      // in place made the next Edit start from the pre-edit values.
      const fresh = games.find(x => x.id === editingGameId) || u;
      if (detailsOpen && fresh) renderDetails(fresh);
      void refreshInstallStatus(editingGameId);
      showToast('success', 'Updated', `${name} saved`);
    } else {
      const g = await window.vault.addGame({ name, exePath, wallpaper: modalWallpaperPath, logoPath: modalLogoPath, fontFamily: fontFamily || undefined, fontColor: fontColor || undefined, savePath: savePath || null, launchTarget: pendingLaunchTarget || null });
      if (!g) throw new Error('Game was not added');
      if (stagedGifPolicy?.path === modalLogoPath) setGifPolicy(g.id, stagedGifPolicy.path, stagedGifPolicy.policy);
      stagedGifPolicy = null;
      if (g && tags.length) {
        try {
          const updated = await window.vault.updateGame(g.id, { tags });
          if (updated) g.tags = updated.tags;
        } catch (e) { /* tags are optional — ignore failures */ }
      }
      allGames.push(g);
      applyCategoryFilter(false);
      const addedIndex = games.findIndex(x => x.id === g.id);
      setSelectedIndex(addedIndex !== -1 ? addedIndex : Math.max(0, games.length - 1));
      // The new game is the focused one, so the panel must be rendered from it:
      // otherwise it keeps the previously selected game (and editing from it
      // would rename that game instead).
      if (detailsOpen && games[selectedIndex]) renderDetails(games[selectedIndex]);
      void refreshInstallStatus(g.id);
      showToast('success', 'Added', `${name} added to SILO`);
      created = g;
    }

    updateGameListSelection();
    centerActiveItem(true);
    syncRailInstant();
    crossfadeWallpaper();
    // A brand-new game goes straight into Edit so its artwork and details can be
    // finished in one pass; an edit closes back to the library as before.
    if (created) startGameOnboarding(created);
    else showPage('home');
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

/* ── New-game onboarding ────────────────────────────────────────────────────
   Add Game → the record is saved → Edit opens for that exact game → its first
   SteamGridDB logo and hero are fetched and shown in place. Fetching is best
   effort: a missing key, no match or a failed download leaves the field empty,
   shows the reason, and never undoes the newly created game.
   ───────────────────────────────────────────────────────────────────────── */
export async function startGameOnboarding(game) {
  if (!game) {
    showPage('home');
    return;
  }
  openEditModal(game);
  await applyAutomaticArtwork(game.id);
}

async function applyAutomaticArtwork(gameId) {
  setArtworkHint('Looking for a logo and wallpaper on SteamGridDB…');
  let result = null;
  try {
    result = await window.vault.autoArtForGame(gameId);
  } catch (err) {
    // The game exists either way: artwork is an extra, never a condition.
    console.warn('Automatic artwork failed', err);
    setArtworkHint(
      editingGameId === gameId && currentPage() === 'import'
        ? 'SteamGridDB could not be reached. Pick artwork manually below.'
        : null,
    );
    return;
  }

  // The record may have gained artwork on disk: keep the list entry identical
  // to what a restart would load, so the rail logo matches the Edit preview.
  if (result && result.game) {
    const idx = allGames.findIndex(g => g.id === gameId);
    if (idx !== -1) allGames[idx] = result.game;
    renderGameList();
    updateGameListSelection();
    syncRailInstant();
  }

  // The user may have cancelled or moved to another game while this ran; in
  // that case nothing is written into the modal they are now looking at.
  const stillEditingThisGame = editingGameId === gameId && currentPage() === 'import';
  if (!stillEditingThisGame) {
    setArtworkHint(null);
    return;
  }

  if (result?.logo) {
    modalLogoPath = result.logo;
    setAssetPreview(document.getElementById('edit-logo-preview-img'), result.logo);
  }
  if (result?.wallpaper) {
    modalWallpaperPath = result.wallpaper;
    setAssetPreview(document.getElementById('edit-wallpaper-preview-img'), result.wallpaper);
  }
  // What arrived is already on screen; the hint only explains what did not.
  setArtworkHint(result?.reason || null);
}
