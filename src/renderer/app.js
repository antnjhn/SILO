/* ── State ─────────────────────────────────────────────────────────────────*/
let allGames = [];
let games = [];
let categoriesOpen = false;
let currentCategory = 'ALL GAMES'; // 'ALL GAMES' | 'RECENTLY PLAYED'
let categorySelectedIndex = 0;
let selectedIndex = 0;
let detailsOpen = false;
let editingGameId = null;
let pendingLaunchGameId = null;
let modalWallpaperPath = null;
let modalLogoPath = null;
let searchQuery = '';
let sortMode = 'name'; // 'name' | 'playtime' | 'recent'

// Gamepad
const DEADZONE = 0.4;
const NAV_REPEAT = 175;
let lastNavTime = 0;
let prevAxes = { right: false, left: false, up: false, down: false };
let buttonWas = {};

function vibrate(left) {
  const pads = navigator.getGamepads?.() || [];
  const pad = Array.from(pads).find(p => p?.connected);
  if (pad && pad.vibrationActuator && pad.vibrationActuator.type === 'dual-rumble') {
    pad.vibrationActuator.playEffect('dual-rumble', {
      startDelay: 0,
      duration: 150,
      weakMagnitude: left ? 0.0 : 0.05,
      strongMagnitude: left ? 0.05 : 0.0
    });
  }
}

function vibrateVertical() {
  const pads = navigator.getGamepads?.() || [];
  const pad = Array.from(pads).find(p => p?.connected);
  if (pad && pad.vibrationActuator && pad.vibrationActuator.type === 'dual-rumble') {
    pad.vibrationActuator.playEffect('dual-rumble', {
      startDelay: 0,
      duration: 150,
      weakMagnitude: 0.1,
      strongMagnitude: 0.1
    });
  }
}

// Wallpaper crossfade state
let currentBgLayer = 'bg-wallpaper-1';
let currentWallpaperUrl = '';
let wallpaperGeneration = 0;

// Item height for centering (logo size + gap)
const ITEM_SIZE = 140;
const ITEM_GAP = 14;
const ITEM_STEP = ITEM_SIZE + ITEM_GAP;

/* ── Tauri IPC Shim ────────────────────────────────────────────────────────*/
const invoke = window.__TAURI__.core.invoke;
const listen = window.__TAURI__.event.listen;
window.vault = {
  getGames: () => invoke('get_games'),
  addGame: (data) => invoke('add_game', { name: data.name, exePath: data.exePath || null, wallpaper: data.wallpaper || null, logoPath: data.logoPath || null, fontFamily: data.fontFamily || null, fontColor: data.fontColor || null, savePath: data.savePath || null }),
  updateGame: (id, updates) => invoke('update_game', { id, updates }),
  deleteGame: (id) => invoke('delete_game', { id }),
  pickExe: () => invoke('pick_exe'),
  pickSaveFolder: () => invoke('pick_save_folder'),
  pickWallpaper: (gameId) => invoke('pick_wallpaper', { gameId }),
  pickLogo: (gameId) => invoke('pick_logo', { gameId }),
  launchGame: (gameId, xboxMode) => invoke('launch_game', { gameId, xboxMode }),
  getSystemFonts: () => invoke('get_system_fonts'),
  onPlaytimeUpdated: async (cb) => await listen('playtime-updated', (event) => cb(event.payload)),
  windowMinimize: () => invoke('window_minimize'),
  windowMaximize: () => invoke('window_maximize'),
  windowClose: () => invoke('window_close'),
  windowStartDragging: () => invoke('window_start_dragging'),
  getGameBackups: (gameId) => invoke('get_game_backups', { gameId }),
  restoreBackup: (gameId, backupName) => invoke('restore_backup', { gameId, backupName }),
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
function getFileSrc(path, imgElement) {
  if (!path) return;
  if (path.startsWith('http://') || path.startsWith('https://')) {
    imgElement.src = path;
  } else {
    imgElement.src = window.__TAURI__.core.convertFileSrc(path);
  }
}

/* ── Toast ─────────────────────────────────────────────────────────────────*/
function showToast(type, title, msg) {
  const container = document.getElementById('toast-container');
  const icons = { error: '⚠', success: '✓', info: 'ℹ' };
  const toast = document.createElement('div');
  toast.className = `toast ${type}`;
  toast.innerHTML = `
    <span class="toast-icon">${icons[type] || 'ℹ'}</span>
    <div class="toast-body">
      <div class="toast-title">${esc(title)}</div>
      <div class="toast-msg">${esc(msg)}</div>
    </div>
  `;
  container.appendChild(toast);
  setTimeout(() => toast.remove(), 5200);
  toast.addEventListener('click', () => toast.remove());
}

/* ── Init ──────────────────────────────────────────────────────────────────*/
async function init() {
  allGames = await window.vault.getGames();
  applyCategoryFilter();
  requestAnimationFrame(pollGamepad);

  listen('library-corrupt-recovered', (event) => {
    showToast('error', 'Library Recovered', event.payload);
  });
  listen('library-corrupt-failed', (event) => {
    showToast('error', 'Library Corrupted', event.payload);
  });

  window.vault.getSystemFonts().then(fonts => {
    const fontSelect = document.getElementById('input-font');
    if (fontSelect && fonts && fonts.length > 0) {
      fontSelect.innerHTML = '<option value="">Default Font</option>' + 
        fonts.map(f => `<option value="${esc(f)}" style="font-family: '${esc(f)}'">${esc(f)}</option>`).join('');
    }
  });

  window.vault.onPlaytimeUpdated((updated) => {
    const idx = allGames.findIndex(g => g.id === updated.id);
    if (idx !== -1) {
      allGames[idx] = updated;
      applyCategoryFilter(false);
      if (detailsOpen && games[selectedIndex]?.id === updated.id) renderDetails(updated);
    }
  });

  if (games.length === 0) {
    document.getElementById('details-panel').classList.add('hidden');
    document.getElementById('empty-state').style.display = 'flex';
  } else {
    document.getElementById('details-panel').classList.remove('hidden');
    document.getElementById('details-panel').classList.add('is-preview');
    document.getElementById('empty-state').style.display = 'none';
    if (!detailsOpen) renderDetails(games[selectedIndex]);
  }
}

/* ── Render game list ─────────────────────────────────────────────────────*/
function getCategories() {
  const tagSet = new Set();
  allGames.forEach(g => (g.tags || []).forEach(t => tagSet.add(t)));
  const tags = [...tagSet].sort((a, b) => String(a).localeCompare(String(b)));
  return ['ALL GAMES', 'RECENTLY PLAYED', 'FAVORITES', ...tags];
}

function rebuildCategoryDots() {
  const panel = document.getElementById('categories-panel');
  if (!panel) return;
  const cats = getCategories();
  let idx = cats.indexOf(currentCategory);
  if (idx === -1) idx = 0;
  categorySelectedIndex = idx;
  panel.innerHTML = cats.map((c, i) =>
    `<div class="category-dot${i === idx ? ' active' : ''}" data-index="${i}" data-label="${esc(c)}" title="${esc(c)}"></div>`
  ).join('');
  panel.querySelectorAll('.category-dot').forEach((el, i) => {
    el.addEventListener('click', () => {
      categorySelectedIndex = i;
      panel.querySelectorAll('.category-dot').forEach((e, idx2) => e.classList.toggle('active', idx2 === i));
      closeCategories(true);
    });
  });
}

function applyCategoryFilter(resetIndex = true) {
  const cats = getCategories();
  if (!cats.includes(currentCategory)) currentCategory = 'ALL GAMES';
  rebuildCategoryDots();

  const q = searchQuery;
  let filtered;
  if (q) {
    // Search overrides category filtering — search within all games
    filtered = allGames.filter(g => String(g.name || '').toLowerCase().includes(q));
  } else if (currentCategory === 'RECENTLY PLAYED') {
    filtered = [...allGames].sort((a, b) => {
      const aTime = a.lastPlayed ? new Date(a.lastPlayed).getTime() : 0;
      const bTime = b.lastPlayed ? new Date(b.lastPlayed).getTime() : 0;
      return bTime - aTime;
    }).slice(0, 4);
  } else if (currentCategory === 'FAVORITES') {
    filtered = allGames.filter(g => g.favorite);
  } else if (currentCategory !== 'ALL GAMES') {
    filtered = allGames.filter(g => (g.tags || []).includes(currentCategory));
  } else {
    filtered = [...allGames];
  }

  // Apply sort mode (RECENTLY PLAYED keeps its own ordering/slice)
  if (!(q === '' && currentCategory === 'RECENTLY PLAYED')) {
    if (sortMode === 'name') {
      filtered = [...filtered].sort((a, b) => String(a.name || '').localeCompare(String(b.name || '')));
    } else if (sortMode === 'playtime') {
      filtered = [...filtered].sort((a, b) => (b.playtimeMinutes || 0) - (a.playtimeMinutes || 0));
    } else if (sortMode === 'recent') {
      filtered = [...filtered].sort((a, b) => {
        const aTime = a.lastPlayed ? new Date(a.lastPlayed).getTime() : 0;
        const bTime = b.lastPlayed ? new Date(b.lastPlayed).getTime() : 0;
        return bTime - aTime;
      });
    }
  }

  games = filtered;

  if (resetIndex) {
    selectedIndex = 0;
  } else {
    if (selectedIndex >= games.length) selectedIndex = Math.max(0, games.length - 1);
  }

  renderGameList();
  centerActiveItem(false);
  crossfadeWallpaper();
}

function renderGameList() {
  const list = document.getElementById('game-list');
  const empty = document.getElementById('empty-state');

  empty.style.display = games.length === 0 ? 'flex' : 'none';

  list.innerHTML = games.map((g, i) => {
    const dist = Math.abs(i - selectedIndex);
    let cls = 'game-item';
    if (i === selectedIndex) cls += ' active';
    else if (dist === 1) cls += ' near';
    if (g.isInstalled === false) cls += ' uninstalled';

    return `
    <div class="${cls}" data-index="${i}" title="${esc(g.name)}">
      ${g.logoPath
        ? `<img data-logo-path="${esc(g.logoPath)}" alt="${esc(g.name)}"
             onerror="this.style.display='none'; this.nextElementSibling.style.display='flex';" />
           <span class="fallback-letter" style="display:none; color:${esc(g.fontColor||'')}; font-family:'${esc(g.fontFamily||'')}';">${esc(String(g.name).charAt(0))}</span>`
        : `<span class="fallback-letter" style="color:${esc(g.fontColor||'')}; font-family:'${esc(g.fontFamily||'')}';">${esc(String(g.name).charAt(0))}</span>`}
    </div>
  `}).join('');

  // Load sidebar logos via base64
  list.querySelectorAll('.game-item img[data-logo-path]').forEach(img => {
    getFileSrc(img.dataset.logoPath, img);
  });

  list.querySelectorAll('.game-item').forEach(el => {
    el.addEventListener('click', () => selectGame(parseInt(el.dataset.index)));
    el.addEventListener('dblclick', () => { selectGame(parseInt(el.dataset.index)); openDetails(); });
  });
}

function openCategories() {
  if (detailsOpen || (!games.length && allGames.length === 0)) return;
  categoriesOpen = true;
  document.body.classList.add('categories-open');
  vibrate(true); // Left side
}

function closeCategories(apply = false) {
  categoriesOpen = false;
  document.body.classList.remove('categories-open');
  if (apply) {
    const cats = getCategories();
    if (currentCategory !== cats[categorySelectedIndex]) {
      currentCategory = cats[categorySelectedIndex];
      applyCategoryFilter();
    }
  }
  vibrate(false); // Right side
}

function changeCategorySelection(delta) {
  const numCats = getCategories().length;
  categorySelectedIndex = (categorySelectedIndex + delta + numCats) % numCats;
  document.querySelectorAll('.category-dot').forEach((el, i) => {
    el.classList.toggle('active', i === categorySelectedIndex);
  });
}

// Search & sort controls in the sidebar
document.getElementById('search-input')?.addEventListener('input', (e) => {
  searchQuery = e.target.value.trim().toLowerCase();
  applyCategoryFilter();
});
document.getElementById('sort-select')?.addEventListener('change', (e) => {
  sortMode = e.target.value;
  applyCategoryFilter();
});

// Search toggle: 🔍 turns into ✕, reveals the inline search bar + filter bar,
// blurs the settings button, and filters the list live as you type.
let isSearchOpen = false;

const SEARCH_ICON = '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="m21 21-4.34-4.34"/><circle cx="11" cy="11" r="8"/></svg>';
const X_ICON = '<svg xmlns="http://www.w3.org/2000/svg" width="20" height="20" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><path d="M18 6 6 18"/><path d="m6 6 12 12"/></svg>';

function toggleSearch(forceOpen) {
  const panel = document.getElementById('game-list-panel');
  const toggle = document.getElementById('btn-search-toggle');
  const next = typeof forceOpen === 'boolean' ? forceOpen : !isSearchOpen;
  isSearchOpen = next;
  panel.classList.toggle('search-mode', next);
  toggle.innerHTML = next ? X_ICON : SEARCH_ICON;
  if (next) {
    document.getElementById('search-input').focus();
  } else {
    const inp = document.getElementById('search-input');
    if (inp.value) { inp.value = ''; searchQuery = ''; applyCategoryFilter(); }
    inp.blur();
  }
}

document.getElementById('btn-search-toggle')?.addEventListener('click', () => toggleSearch());

/* ── Center active item in viewport ───────────────────────────────────────*/
function centerActiveItem(animate = true) {
  const viewport = document.getElementById('game-list-viewport');
  const list = document.getElementById('game-list');
  if (!viewport || !list || !games.length) return;

  const viewportH = viewport.clientHeight;
  const centerY = viewportH / 2;
  // Position of active item center relative to list top
  const itemCenterY = selectedIndex * ITEM_STEP + ITEM_SIZE / 2;
  const translateY = centerY - itemCenterY;

  if (!animate) list.style.transition = 'none';
  list.style.transform = `translateY(${translateY}px)`;
  if (!animate) {
    // Force reflow then re-enable transitions
    list.offsetHeight;
    list.style.transition = '';
  }

  list.querySelectorAll('.game-item').forEach((el, i) => {
    const dist = Math.abs(i - selectedIndex);
    const xOffset = -1 * (dist * dist * 8);
    const scale = i === selectedIndex ? 1 : (dist === 1 ? 0.82 : 0.7);
    if (!animate) el.style.transition = 'none';
    el.style.transform = `translateX(${xOffset}px) scale(${scale})`;
    if (!animate) {
      el.offsetHeight;
      el.style.transition = '';
    }
  });
}

/* ── Select game — with transitions ───────────────────────────────────────*/
function selectGame(i) {
  if (i < 0 || i >= games.length || i === selectedIndex) return;
  selectedIndex = i;
  updateGameListSelection();
  centerActiveItem(true);
  crossfadeWallpaper();
  if (games[i]) renderDetails(games[i]);
}

function updateGameListSelection() {
  const items = document.querySelectorAll('#game-list .game-item');
  items.forEach((el, i) => {
    const dist = Math.abs(i - selectedIndex);
    el.className = 'game-item';
    if (i === selectedIndex) el.classList.add('active');
    else if (dist === 1) el.classList.add('near');
  });
}

/* ── Wallpaper crossfade between two layers ───────────────────────────────*/
function crossfadeWallpaper() {
  const g = games[selectedIndex];
  const url = g?.wallpaper || '';

  if (url === currentWallpaperUrl) return;
  currentWallpaperUrl = url;
  wallpaperGeneration++;
  const thisGen = wallpaperGeneration;

  const bg1 = document.getElementById('bg-wallpaper-1');
  const bg2 = document.getElementById('bg-wallpaper-2');
  
  const activeLayer = currentBgLayer === 'bg-wallpaper-1' ? bg1 : bg2;
  const nextLayer = currentBgLayer === 'bg-wallpaper-1' ? bg2 : bg1;
  const nextLayerId = currentBgLayer === 'bg-wallpaper-1' ? 'bg-wallpaper-2' : 'bg-wallpaper-1';

  if (!url) {
    setTimeout(() => {
      if (thisGen !== wallpaperGeneration) return;
      bg1.style.opacity = '0';
      bg2.style.opacity = '0';
    }, 100);
    return;
  }

  const preload = new Image();
  preload.onload = () => {
    if (thisGen !== wallpaperGeneration) return; // Stale load — abort
    nextLayer.style.backgroundImage = `url("${preload.src}")`;
    nextLayer.style.opacity = '0.35';
    activeLayer.style.opacity = '0';
    
    currentBgLayer = nextLayerId;
    
    const staleLayer = activeLayer;
    const staleGen = thisGen;
    setTimeout(() => {
      if (staleGen !== wallpaperGeneration) return;
      staleLayer.style.backgroundImage = 'none';
    }, 650);
  };
  preload.onerror = () => {
    if (thisGen !== wallpaperGeneration) return;
    showToast('error', 'Wallpaper Failed', `Could not load: ${url}`);
  };
  getFileSrc(url, preload);
}

/* ── Logo Transition — FLIP from sidebar center to details ────────────────*/
function openDetails() {
  if (!games.length || detailsOpen) return;
  vibrate(false);
  detailsOpen = true;
  document.getElementById('empty-state').style.display = 'none';
  renderDetails(games[selectedIndex]);
  const sidebar = document.getElementById('game-list-panel');
  const details = document.getElementById('details-panel');
  // The CSS transitions handle the crossfade: the sidebar slides/fades out while
  // the details panel (and its logo) fade in. No flying logo clone.
  details.classList.remove('hidden');
  details.classList.remove('is-preview');
  sidebar.classList.add('collapsed');
}

function closeDetails() {
  if (!detailsOpen) return;
  vibrate(true);
  detailsOpen = false;
  const sidebar = document.getElementById('game-list-panel');
  const details = document.getElementById('details-panel');
  // CSS transitions crossfade: the logo fades back to preview as the sidebar returns.
  sidebar.classList.remove('collapsed');
  details.classList.add('is-preview');
  if (!games.length) {
    details.classList.add('hidden');
    document.getElementById('empty-state').style.display = 'flex';
  }
}

function renderDetails(g) {
  const img = document.getElementById('details-logo-img');
  const fallback = document.getElementById('details-game-name-fallback');
  if (g.logoPath) {
    getFileSrc(g.logoPath, img);
    img.classList.add('visible');
    document.getElementById('details-panel').classList.add('has-logo');
    img.onerror = () => {
      img.classList.remove('visible');
      document.getElementById('details-panel').classList.remove('has-logo');
    };
  } else {
    img.classList.remove('visible');
    document.getElementById('details-panel').classList.remove('has-logo');
  }

  fallback.textContent = g.name;
  fallback.style.fontFamily = g.fontFamily || 'inherit';
  fallback.style.color = g.fontColor || 'inherit';
  if (g.fontFamily) fallback.classList.add('game-title-styled');
  else fallback.classList.remove('game-title-styled');

  document.getElementById('stat-playtime').textContent = fmtTime(g.playtimeMinutes);
  document.getElementById('stat-sessions').textContent = g.sessionCount || 0;
  document.getElementById('stat-last-played').textContent = fmtDate(g.lastPlayed);

  const statusSelect = document.getElementById('details-status-select');
  if (statusSelect) {
    statusSelect.value = g.status || 'Playing';
    statusSelect.onchange = async () => {
      const newStatus = statusSelect.value;
      const updated = await window.vault.updateGame(g.id, { status: newStatus });
      if (updated) {
        const idx = games.findIndex(x => x.id === g.id);
        if (idx !== -1) games[idx] = updated;
      }
    };
  }

  const favBtn = document.getElementById('btn-toggle-favorite');
  if (favBtn) {
    const cur = allGames.find(x => x.id === g.id) || g;
    favBtn.classList.toggle('fav-active', !!cur.favorite);
    favBtn.title = cur.favorite ? 'Remove from Favorites' : 'Add to Favorites';
    favBtn.onclick = async () => {
      const current = allGames.find(x => x.id === g.id) || g;
      const next = !current.favorite;
      try {
        const updated = await window.vault.updateGame(g.id, { favorite: next });
        if (updated) {
          const idx = allGames.findIndex(x => x.id === g.id);
          if (idx !== -1) allGames[idx] = updated;
          applyCategoryFilter(false);
          const sel = games.find(x => x.id === g.id) || games[selectedIndex];
          if (detailsOpen && sel) renderDetails(sel);
        }
      } catch (err) {
        showToast('error', 'Update Failed', err);
      }
    };
  }

  const restoreBtn = document.getElementById('btn-restore-save');
  const backupBtn = document.getElementById('btn-backup-save');
  
  if (restoreBtn && backupBtn) {
    restoreBtn.style.display = 'flex';
    backupBtn.style.display = 'flex';
    
    restoreBtn.onclick = async () => {
      if (!g.savePath) {
        showToast('error', 'Not Ready', 'Launch the game first to automatically detect its save folder.');
        return;
      }
      
      const backups = await window.vault.getGameBackups(g.id);
      
      if (backups.length === 0) {
        showToast('error', 'No Backups', 'There are no backups available for this game yet. Launch the game to create one, or click BACKUP.');
        return;
      }
      
      const listEl = document.getElementById('restore-list');
      listEl.innerHTML = backups.map(b => {
        const isAuto = b.isAuto !== undefined ? b.isAuto : b.is_auto;
        const customName = b.customName !== undefined ? b.customName : b.custom_name;
        const sizeBytes = b.sizeBytes !== undefined ? b.sizeBytes : b.size_bytes;
        const typeTag = isAuto ? '<span style="color: #a0a0a0; font-size: 10px; border: 1px solid #555; padding: 2px 4px; border-radius: 4px; margin-right: 6px;">AUTO</span>'
                               : '<span style="color: #66ccff; font-size: 10px; border: 1px solid #3388aa; padding: 2px 4px; border-radius: 4px; margin-right: 6px;">MANUAL</span>';
        const displayName = customName ? `<div style="font-size: 14px; font-weight: bold; color: #fff;">${esc(customName)}</div>` : '';

        return `
        <div class="restore-item" style="display: flex; justify-content: space-between; align-items: center; padding: 10px; background: rgba(0,0,0,0.2); border-radius: 8px;">
          <div style="display: flex; flex-direction: column; gap: 4px;">
            ${displayName}
            <div style="display: flex; align-items: center;">
              ${typeTag}
              <span style="font-weight: bold; font-size: 12px;">${esc(b.timestamp)}</span>
            </div>
            <div style="font-size: 11px; color: rgba(255,255,255,0.5);">${((sizeBytes || 0) / 1024).toFixed(1)} KB</div>
          </div>
          <div style="display: flex; gap: 6px; align-items: center;">
            <button class="glass-btn small" onclick="doRestore('${g.id}', '${escJs(b.name)}')">Restore</button>
            <button class="glass-btn small" style="border-color: #ff4444; color: #ff4444; padding: 4px 8px;" onclick="doDeleteBackup('${g.id}', '${escJs(b.name)}')">🗑</button>
          </div>
        </div>
        `;
      }).join('');
      
      document.getElementById('restore-overlay').classList.remove('hidden');
    };

    backupBtn.onclick = async () => {
      if (!g.savePath) {
        showToast('error', 'Not Ready', 'Launch the game first to automatically detect its save folder.');
        return;
      }
      
      const overlay = document.getElementById('backup-name-overlay');
      const input = document.getElementById('input-backup-name');
      const btnCancel = document.getElementById('btn-backup-name-cancel');
      const btnOk = document.getElementById('btn-backup-name-ok');
      
      input.value = '';
      overlay.classList.remove('hidden');
      input.focus();
      
      // We need to use a one-off promise or just handlers for the buttons
      const cleanup = () => {
        overlay.classList.add('hidden');
        btnCancel.onclick = null;
        btnOk.onclick = null;
      };
      
      btnCancel.onclick = () => cleanup();
      
      btnOk.onclick = async () => {
        const customName = input.value;
        cleanup();
        
        showToast('info', 'Backing Up', 'Creating a manual backup...');
        try {
          await window.__TAURI__.core.invoke('backup_game_now', { gameId: g.id, customName: customName || null });
          showToast('success', 'Backup Complete', 'Save files have been backed up successfully.');
        } catch (err) {
          showToast('error', 'Backup Failed', err);
        }
      };
    };
  }

  setupHoldButton('btn-launch-xbox', 5000, () => doLaunch(g.id, true));
  setupHoldButton('btn-edit-game', 5000, () => openEditModal(g));
  

  
  const deleteBtn = document.getElementById('btn-delete-game');
  deleteBtn.onmousedown = null; deleteBtn.onmouseup = null; deleteBtn.onmouseleave = null;
  deleteBtn._startGamepadHold = null; deleteBtn._resetGamepadHold = null;
  deleteBtn.classList.remove('holding');
  
  deleteBtn.onclick = async () => {
    // Check if uninstaller exists
    const uninstaller = await window.__TAURI__.core.invoke('check_uninstaller', { gameId: g.id }).catch(() => null);
    
    const uninstallOverlay = document.getElementById('uninstall-overlay');
    const runBtn = document.getElementById('btn-uninstall-run');
    
    if (uninstaller) {
      runBtn.style.display = 'block';
    } else {
      runBtn.style.display = 'none';
    }

    document.getElementById('btn-uninstall-delete').onclick = async () => {
      uninstallOverlay.classList.add('hidden');
      const { ask } = window.__TAURI__.dialog;
      const yes = await ask(`WARNING: This will permanently delete the entire game folder from your drive.\n\nAre you absolutely sure?`, { title: 'Delete Game Data', kind: 'warning' });
      if (yes) {
        showToast('info', 'Deleting', 'Deleting game folder...');
        try {
          await window.__TAURI__.core.invoke('delete_game_folder', { gameId: g.id });
          showToast('success', 'Deleted', 'Game folder deleted permanently.');
          const removeYes = await ask("The game files have been deleted. Do you also want to remove this game from your Silo library?", { title: 'Remove from Launcher?', kind: 'info' });
          if (removeYes) {
            await finishRemoval(g);
          } else {
            g.isInstalled = false;
            renderGameList();
            if (detailsOpen) renderDetails(g);
          }
        } catch (err) {
          showToast('error', 'Delete Failed', err);
        }
      }
    };

    document.getElementById('btn-uninstall-remove').onclick = async () => {
      uninstallOverlay.classList.add('hidden');
      await finishRemoval(g);
      showToast('success', 'Removed', `${g.name} has been removed from Launcher.`);
    };

    document.getElementById('btn-uninstall-cancel').onclick = () => {
      uninstallOverlay.classList.add('hidden');
    };

    if (uninstaller) {
      runBtn.onclick = async () => {
        uninstallOverlay.classList.add('hidden');
        try {
          await window.__TAURI__.core.invoke('run_uninstaller', { uninstallerPath: uninstaller });
          const { ask } = window.__TAURI__.dialog;
          const removeYes = await ask("The uninstaller has been launched. Do you also want to remove this game from your Silo library?", { title: 'Remove from Launcher?', kind: 'info' });
          if (removeYes) {
            await finishRemoval(g);
          } else {
            g.isInstalled = false;
            renderGameList();
            if (detailsOpen) renderDetails(g);
          }
        } catch (err) {
          showToast('error', 'Uninstaller Failed', err);
        }
      };
    }

    uninstallOverlay.classList.remove('hidden');
  };

  async function finishRemoval(game) {
    if (detailsOpen) closeDetails();
    const removed = await window.vault.deleteGame(game.id);
    if (removed) {
      games = games.filter(x => x.id !== game.id);
      allGames = allGames.filter(x => x.id !== game.id);
      renderGameList();
    }
  }
}

let actionHoldStart = 0;
let actionHoldTarget = null;
let actionHoldInterval = null;

function setupHoldButton(id, duration, onComplete) {
  const btn = document.getElementById(id);
  // Clear old listeners
  btn.onmousedown = null; btn.onmouseup = null; btn.onmouseleave = null; btn.onclick = null;
  
  const isDelete = id === 'btn-delete-game';
  
  if (!isDelete) {
    btn.onclick = onComplete;
  }
  
  const startHold = () => {
    if (actionHoldStart) return;
    actionHoldStart = Date.now();
    actionHoldTarget = id;
    btn.classList.add('holding');
    if (isDelete) triggerDeletePrompt();
    
    actionHoldInterval = setInterval(() => {
      const elapsed = Date.now() - actionHoldStart;
      if (isDelete) updateDeleteProgress(elapsed);
      if (elapsed >= duration) {
        clearInterval(actionHoldInterval);
        resetHold();
        onComplete();
      }
    }, 50);
  };
  
  const resetHold = () => {
    if (actionHoldTarget !== id) return;
    actionHoldStart = 0;
    actionHoldTarget = null;
    clearInterval(actionHoldInterval);
    btn.classList.remove('holding');
    if (isDelete) resetDeleteButton();
  };
  
  btn._startGamepadHold = startHold;
  btn._resetGamepadHold = resetHold;
  
  if (isDelete) {
    btn.onmousedown = startHold;
    btn.onmouseup = resetHold;
    btn.onmouseleave = resetHold;
  }
}

/* ── Launch ────────────────────────────────────────────────────────────────*/
function openLaunchModal(gameId) {
  pendingLaunchGameId = gameId;
  const g = games.find(g => g.id === gameId);
  const pWallImg = document.getElementById('launch-wallpaper-img');
  const pLogoImg = document.getElementById('launch-logo-img');
  const lName = document.getElementById('launch-game-name');

  if (g.wallpaper) {
    getFileSrc(g.wallpaper, pWallImg);
    pWallImg.style.display = 'block';
  } else {
    pWallImg.src = '';
    pWallImg.style.display = 'none';
  }

  if (g.logoPath) {
    getFileSrc(g.logoPath, pLogoImg);
    pLogoImg.style.display = 'block';
  } else {
    pLogoImg.src = '';
    pLogoImg.style.display = 'none';
  }
  lName.textContent = g?.name || '';
  document.getElementById('launch-overlay').classList.remove('hidden');
}

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

async function doLaunch(gameId, xboxMode) {
  const g = games.find(g => g.id === gameId);
  if (g && g.isInstalled === false) {
    showToast('error', 'Not Installed', 'The executable for this game could not be found.');
    return;
  }
  showToast('info', 'Launching', `${g?.name || 'Game'}${xboxMode ? ' in Xbox Mode...' : '...'}`);
  try {
    await window.vault.launchGame(gameId, xboxMode);
    showToast('success', 'Launched', `${g?.name || 'Game'} is running`);
  } catch (err) {
    showToast('error', 'Launch Failed', err);
  }
}

document.getElementById('btn-launch-mode-normal').addEventListener('click', () => {
  document.getElementById('launch-overlay').classList.add('hidden');
  if (pendingLaunchGameId) doLaunch(pendingLaunchGameId, false);
  pendingLaunchGameId = null;
});
document.getElementById('btn-launch-mode-xbox').addEventListener('click', () => {
  document.getElementById('launch-overlay').classList.add('hidden');
  if (pendingLaunchGameId) doLaunch(pendingLaunchGameId, true);
  pendingLaunchGameId = null;
});
document.getElementById('btn-launch-cancel').addEventListener('click', () => {
  document.getElementById('launch-overlay').classList.add('hidden');
  pendingLaunchGameId = null;
});

/* ── Add / Edit modal ──────────────────────────────────────────────────────*/
function openAddModal() {
  editingGameId = null; modalWallpaperPath = null; modalLogoPath = null;
  document.getElementById('modal-title').textContent = 'ADD GAME';
  document.getElementById('scan-folder-container').style.display = 'flex';
  document.getElementById('import-container').style.display = 'flex';
  document.getElementById('input-name').value = '';
  document.getElementById('input-font').value = '';
  document.getElementById('input-color').value = '#ffffff';
  document.getElementById('color-preview-text').textContent = '#ffffff';
  document.getElementById('input-exe').value = '';
  document.getElementById('input-save-path').value = '';
  document.getElementById('input-tags').value = '';
  document.getElementById('backup-count-group').style.display = 'none';
  document.getElementById('input-backup-count').value = 5;

  document.getElementById('edit-wallpaper-preview-img').style.display = 'none';
  document.getElementById('edit-logo-preview-img').style.display = 'none';
  updateTitlePreview();

  document.getElementById('modal-overlay').classList.remove('hidden');
  setTimeout(() => document.getElementById('input-name').focus(), 50);
}

function openEditModal(g) {
  editingGameId = g.id;
  modalWallpaperPath = g.wallpaper || null;
  modalLogoPath = g.logoPath || null;
  document.getElementById('modal-title').textContent = 'EDIT GAME';
  document.getElementById('scan-folder-container').style.display = 'none';
  document.getElementById('import-container').style.display = 'none';
  document.getElementById('input-name').value = g.name;
  document.getElementById('input-font').value = g.fontFamily || '';
  document.getElementById('input-color').value = g.fontColor || '#ffffff';
  document.getElementById('color-preview-text').textContent = g.fontColor || '#ffffff';
  document.getElementById('input-exe').value = g.exePath || '';
  document.getElementById('input-save-path').value = g.savePath || '';
  document.getElementById('input-tags').value = (g.tags || []).join(', ');
  document.getElementById('backup-count-group').style.display = 'block';
  document.getElementById('input-backup-count').value = g.backupCount || 5;

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

  document.getElementById('modal-overlay').classList.remove('hidden');
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
    btn.textContent = '✨ Auto-Fill Metadata';
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

function closeMetadataPicker() {
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
    showArtNotice('SteamGridDB off — add your free key in Settings to see custom art.');
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

function closeOnlineArtPicker() {
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
        document.getElementById('scan-results-title').textContent = 'SCAN RESULTS';

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
        document.getElementById('modal-overlay').classList.add('hidden');
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
  document.getElementById('scan-results-title').textContent = `${source.toUpperCase()} IMPORT`;
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

  document.getElementById('modal-overlay').classList.add('hidden');
  document.getElementById('scan-results-overlay').classList.remove('hidden');
}

document.getElementById('btn-import-steam').addEventListener('click', () => runImport('Steam'));
document.getElementById('btn-import-epic').addEventListener('click', () => runImport('Epic'));
document.getElementById('btn-import-gog').addEventListener('click', () => runImport('GOG'));

/* ── Save folder pickers in add/edit modal ──────────────────────────────────*/
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
  if (p) document.getElementById('input-exe').value = p;
});

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
  document.getElementById('modal-overlay').classList.add('hidden');
});

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
      const g = await window.vault.addGame({ name, exePath, wallpaper: modalWallpaperPath, logoPath: modalLogoPath, fontFamily: fontFamily || undefined, fontColor: fontColor || undefined, savePath: savePath || null });
      if (g && tags.length) {
        try {
          const updated = await window.vault.updateGame(g.id, { tags });
          if (updated) g.tags = updated.tags;
        } catch (e) { /* tags are optional — ignore failures */ }
      }
      allGames.push(g);
      applyCategoryFilter(false);
      selectedIndex = games.length - 1;
      showToast('success', 'Added', `${name} added to SILO`);
    }

    updateGameListSelection();
    centerActiveItem(true);
    crossfadeWallpaper();
    document.getElementById('modal-overlay').classList.add('hidden');
  } catch (err) {
    showToast('error', 'Save Failed', err);
  }
});

/* ── Delete ────────────────────────────────────────────────────────────────*/
function triggerDeletePrompt() {
  const btn = document.getElementById('btn-delete-game');
  btn.innerHTML = `<span style="font-size:12px; font-weight:700; color:#fff;">HOLD 10s</span><div id="delete-progress"></div>`;
}

function updateDeleteProgress(ms) {
  const prog = document.getElementById('delete-progress');
  if (prog) {
    const pct = Math.min(100, (ms / 10000) * 100);
    prog.style.width = pct + '%';
  }
}

function resetDeleteButton() {
  const btn = document.getElementById('btn-delete-game');
  btn.innerHTML = `<p>×</p><span></span><span></span><span></span><span></span>`;
}

document.getElementById('btn-close-details').addEventListener('click', closeDetails);

/* ── Window controls ───────────────────────────────────────────────────────*/
document.getElementById('btn-minimize').addEventListener('click', () => window.vault.windowMinimize());
document.getElementById('btn-maximize').addEventListener('click', () => window.vault.windowMaximize());
document.getElementById('btn-close').addEventListener('click', () => window.vault.windowClose());

document.getElementById('titlebar').addEventListener('mousedown', (e) => {
  if (e.target.closest('#titlebar-controls')) return;
  if (e.button === 0) window.vault.windowStartDragging();
});

/* ── Keyboard ──────────────────────────────────────────────────────────────*/
document.addEventListener('keydown', e => {
  // Metadata picker is opened while the add/edit modal is still visible, so it must be checked first
  if (!document.getElementById('metadata-overlay').classList.contains('hidden')) {
    if (e.key === 'Escape') closeMetadataPicker();
    return;
  }
  if (!document.getElementById('online-art-overlay').classList.contains('hidden')) {
    if (e.key === 'Escape') closeOnlineArtPicker();
    return;
  }
  if (!document.getElementById('modal-overlay').classList.contains('hidden')) return;
  if (!document.getElementById('launch-overlay').classList.contains('hidden')) {
    if (e.key === 'Escape') document.getElementById('btn-launch-cancel').click();
    return;
  }
  if (!document.getElementById('backup-manager-overlay').classList.contains('hidden')) {
    if (e.key === 'Escape') document.getElementById('btn-backup-manager-close').click();
    return;
  }
  if (!document.getElementById('settings-overlay').classList.contains('hidden')) {
    if (e.key === 'Escape') document.getElementById('btn-settings-close').click();
    return;
  }

  if (categoriesOpen) {
    switch (e.key) {
      case 'ArrowUp': e.preventDefault(); changeCategorySelection(-1); break;
      case 'ArrowDown': e.preventDefault(); changeCategorySelection(1); break;
      case 'ArrowRight':
      case 'Enter':
        e.preventDefault(); closeCategories(true); break;
      case 'Escape': e.preventDefault(); closeCategories(false); break;
    }
    return;
  }

  // If the user is typing in a text field, don't hijack keys
  const ae = document.activeElement;
  if (ae && (ae.tagName === 'INPUT' || ae.tagName === 'SELECT' || ae.tagName === 'TEXTAREA')) {
    if (e.key === 'Escape' && ae.id === 'search-input') {
      if (ae.value) { ae.value = ''; searchQuery = ''; applyCategoryFilter(); }
      toggleSearch(false);
      ae.blur();
    }
    return;
  }

  if (e.key === '/' || (e.ctrlKey && e.key.toLowerCase() === 'k')) {
    e.preventDefault();
    toggleSearch(true);
    return;
  }

  switch (e.key) {
    case 'ArrowUp':    e.preventDefault(); if (!detailsOpen && selectedIndex > 0) selectGame(selectedIndex - 1); break;
    case 'ArrowDown':  e.preventDefault(); if (!detailsOpen && selectedIndex < games.length - 1) selectGame(selectedIndex + 1); break;
    case 'ArrowRight': e.preventDefault(); if (!detailsOpen && games.length) openDetails(); break;
    case 'ArrowLeft':  e.preventDefault();
      if (detailsOpen) closeDetails();
      else openCategories();
      break;
    case 'Enter':      e.preventDefault();
      if (!detailsOpen && games.length) openDetails();
      else if (detailsOpen && games[selectedIndex]) openLaunchModal(games[selectedIndex].id);
      break;
    case 'Escape': if (detailsOpen) closeDetails(); break;
  }
});

/* ── Gamepad ───────────────────────────────────────────────────────────────*/
function pollGamepad() {
  const pads = navigator.getGamepads?.() || [];
  const pad = Array.from(pads).find(p => p?.connected);
  if (pad) {
    document.body.classList.add('gamepad-active');
    handlePad(pad);
  } else {
    document.body.classList.remove('gamepad-active');
  }
  requestAnimationFrame(pollGamepad);
}

function handlePad(pad) {
  const now = Date.now();
  const modalOpen = !document.getElementById('modal-overlay').classList.contains('hidden');
  const launchOpen = !document.getElementById('launch-overlay').classList.contains('hidden');
  const metadataOpen = !document.getElementById('metadata-overlay').classList.contains('hidden');
  const onlineArtOpen = !document.getElementById('online-art-overlay').classList.contains('hidden');
  const backupManagerOpen = !document.getElementById('backup-manager-overlay').classList.contains('hidden');
  const settingsOpen = !document.getElementById('settings-overlay').classList.contains('hidden');

  const ly = pad.axes[1];
  const lx = pad.axes[0];
  const dUp    = pad.buttons[12]?.pressed;
  const dDown  = pad.buttons[13]?.pressed;
  const dLeft  = pad.buttons[14]?.pressed;
  const dRight = pad.buttons[15]?.pressed;

  const up    = dUp    || ly < -DEADZONE;
  const down  = dDown  || ly >  DEADZONE;
  const right = dRight || lx >  DEADZONE;
  const left  = dLeft  || lx < -DEADZONE;

  if (launchOpen) {
    if (btnPressed(pad, 0, 'A')) document.getElementById('btn-launch-mode-normal').click();
    if (btnPressed(pad, 1, 'B')) document.getElementById('btn-launch-cancel').click();
    if (right && !prevAxes.right) document.getElementById('btn-launch-mode-xbox').focus();
    if (left && !prevAxes.left) document.getElementById('btn-launch-mode-normal').focus();
    prevAxes.right = right; prevAxes.left = left;
    savePad(pad); return;
  }
  if (modalOpen || metadataOpen || onlineArtOpen || backupManagerOpen || settingsOpen) { savePad(pad); return; }

  if (categoriesOpen) {
    if (up && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; changeCategorySelection(-1); vibrateVertical(); }
    if (down && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; changeCategorySelection(1); vibrateVertical(); }
    if ((right && !prevAxes.right) || btnPressed(pad, 0, 'A')) { closeCategories(true); }
    if (btnPressed(pad, 1, 'B')) { closeCategories(false); }
    prevAxes.right = right; prevAxes.left = left;
    savePad(pad); return;
  }

  if (up   && now - lastNavTime > NAV_REPEAT && selectedIndex > 0) { lastNavTime = now; if (!detailsOpen) selectGame(selectedIndex - 1); vibrateVertical(); }
  if (down && now - lastNavTime > NAV_REPEAT && selectedIndex < games.length - 1) { lastNavTime = now; if (!detailsOpen) selectGame(selectedIndex + 1); vibrateVertical(); }
  if (right && !prevAxes.right && !detailsOpen && games.length) openDetails();
  if (left  && !prevAxes.left) {
    if (detailsOpen) closeDetails();
    else openCategories();
  }

  prevAxes.right = right; prevAxes.left = left;

  const aPressed = !!pad.buttons[0]?.pressed;
  const bPressed = !!pad.buttons[1]?.pressed;
  const xPressed = !!pad.buttons[2]?.pressed;
  const yPressed = !!pad.buttons[3]?.pressed;

  if (detailsOpen && games[selectedIndex]) {
    const handleGamepadHold = (isPressed, btnId) => {
      const btn = document.getElementById(btnId);
      if (!btn) return;
      if (isPressed) {
        if (!actionHoldTarget && btn._startGamepadHold) btn._startGamepadHold();
      } else {
        if (actionHoldTarget === btnId && btn._resetGamepadHold) btn._resetGamepadHold();
      }
    };
    
    handleGamepadHold(aPressed, 'btn-launch-normal');
    handleGamepadHold(bPressed, 'btn-launch-xbox');
    if (btnPressed(pad, 2, 'X')) document.getElementById('btn-delete-game').click();
    handleGamepadHold(yPressed, 'btn-edit-game');
  } else {
    // If not in details, Y opens add
    if (btnPressed(pad, 3, 'Y') && !detailsOpen) openAddModal();
    if (btnPressed(pad, 0, 'A') && !detailsOpen && games.length) openDetails();
  }

  savePad(pad);
}

function btnPressed(pad, idx, key) {
  const cur = !!pad.buttons[idx]?.pressed;
  const prev = !!buttonWas[key];
  buttonWas[key] = cur;
  return cur && !prev;
}

function savePad(pad) {
  pad.buttons.forEach((b, i) => { buttonWas['b' + i] = b.pressed; });
}

/* ── Helpers ───────────────────────────────────────────────────────────────*/
function fmtTime(min) {
  if (!min || min < 1) return '0h 0m';
  const h = Math.floor(min / 60), m = min % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

function fmtDate(iso) {
  if (!iso) return 'Never';
  const d = new Date(iso), now = new Date();
  const days = Math.floor((now - d) / 86400000);
  if (days === 0) return 'Today';
  if (days === 1) return 'Yesterday';
  if (days < 7) return `${days}d ago`;
  if (days < 30) return `${Math.floor(days/7)}w ago`;
  return d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
}

function esc(s) {
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

// Escape for use inside a JS string literal embedded in an inline onclick handler
function escJs(s) {
  return esc(s).replace(/'/g, "\\'");
}

// Split a comma-separated tag string into a deduped, trimmed array
function parseTags(val) {
  return [...new Set(String(val || '').split(',').map(t => t.trim()).filter(Boolean))];
}

/* ── Settings & Backup ─────────────────────────────────────────────────────*/
document.getElementById('btn-settings').addEventListener('click', async () => {
  document.getElementById('settings-overlay').classList.remove('hidden');
  try {
    const s = await window.vault.getSettings();
    document.getElementById('input-sgdb-key').value = s.sgdbApiKey || '';
    document.getElementById('input-check-updates').checked = !!s.checkUpdatesOnLaunch;
  } catch (err) {
    console.error(err);
  }
});
document.getElementById('btn-settings-close').addEventListener('click', () => {
  document.getElementById('settings-overlay').classList.add('hidden');
});
// Advanced section collapse (settings page)
document.getElementById('settings-advanced-toggle')?.addEventListener('click', () => {
  const adv = document.getElementById('settings-advanced');
  if (adv) adv.classList.toggle('open');
});

document.getElementById('btn-save-settings').addEventListener('click', async () => {
  const sgdbApiKey = document.getElementById('input-sgdb-key').value.trim() || null;
  const checkUpdatesOnLaunch = document.getElementById('input-check-updates').checked;
  try {
    await window.vault.setSettings({ settings: { sgdbApiKey, checkUpdatesOnLaunch } });
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
        showToast('info', 'Update Available', `Update ${u.version} available`);
        const { ask } = window.__TAURI__.dialog;
        const yes = await ask(`Version ${u.version} is available. Open the GitHub releases page to download it?`, { title: 'Update Available', kind: 'info' });
        if (yes) window.open('https://github.com/antnjhn/vault-launcher/releases', '_blank');
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

/* ── Backup manager ─────────────────────────────────────────────────────────*/
document.getElementById('btn-backup-manager').addEventListener('click', () => {
  document.getElementById('settings-overlay').classList.add('hidden');
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
      document.getElementById('settings-overlay').classList.add('hidden');
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
      showToast('success', 'Import Complete', `Library restored. Reloading...`);
      setTimeout(() => window.location.reload(), 1500);
    }
  } catch (e) {
    if (e !== 'Restore cancelled') showToast('error', 'Import Failed', e);
  }
});

/* ── Go ────────────────────────────────────────────────────────────────────*/
init();
