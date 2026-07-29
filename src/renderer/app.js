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
  addGame: (data) => invoke('add_game', { name: data.name, exePath: data.exePath || null, wallpaper: data.wallpaper || null, logoPath: data.logoPath || null, fontFamily: data.fontFamily || null, fontColor: data.fontColor || null }),
  updateGame: (id, updates) => invoke('update_game', { id, updates }),
  deleteGame: (id) => invoke('delete_game', { id }),
  pickExe: () => invoke('pick_exe'),
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
function applyCategoryFilter(resetIndex = true) {
  if (currentCategory === 'RECENTLY PLAYED') {
    games = [...allGames].sort((a, b) => {
      const aTime = a.lastPlayed ? new Date(a.lastPlayed).getTime() : 0;
      const bTime = b.lastPlayed ? new Date(b.lastPlayed).getTime() : 0;
      return bTime - aTime;
    }).slice(0, 4);
  } else {
    games = [...allGames];
  }
  
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
           <span class="fallback-letter" style="display:none; color:${esc(g.fontColor||'')}; font-family:'${esc(g.fontFamily||'')}';">${g.name.charAt(0)}</span>`
        : `<span class="fallback-letter" style="color:${esc(g.fontColor||'')}; font-family:'${esc(g.fontFamily||'')}';">${g.name.charAt(0)}</span>`}
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
    const cats = ['ALL GAMES', 'RECENTLY PLAYED'];
    if (currentCategory !== cats[categorySelectedIndex]) {
      currentCategory = cats[categorySelectedIndex];
      applyCategoryFilter();
    }
  }
  vibrate(false); // Right side
}

function changeCategorySelection(delta) {
  const numCats = 2;
  categorySelectedIndex = (categorySelectedIndex + delta + numCats) % numCats;
  document.querySelectorAll('.category-dot').forEach((el, i) => {
    el.classList.toggle('active', i === categorySelectedIndex);
  });
}

document.querySelectorAll('.category-dot').forEach((el, i) => {
  el.addEventListener('click', () => {
    categorySelectedIndex = i;
    document.querySelectorAll('.category-dot').forEach((e, idx) => e.classList.toggle('active', idx === i));
    closeCategories(true);
  });
});

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
  if (!games.length) return;
  const g = games[selectedIndex];
  
  const sidebarItem = document.querySelectorAll('.game-item')[selectedIndex];
  const sidebarImg = sidebarItem?.querySelector('img');
  if (!g?.logoPath || !sidebarImg || sidebarImg.style.display === 'none') {
    applyOpenDetailsState();
    return;
  }

  // FIRST: capture sidebar logo position (Start Rect)
  const startRect = sidebarImg.getBoundingClientRect();

  // Measure details-content start rect
  const contentEl = document.getElementById('details-content');
  const contentStartRect = contentEl.getBoundingClientRect();

  // Create clone
  const clone = document.createElement('img');
  clone.id = 'logo-transition-clone';
  clone.src = sidebarImg.src;
  clone.style.left = startRect.left + 'px';
  clone.style.top = startRect.top + 'px';
  clone.style.width = startRect.width + 'px';
  clone.style.height = startRect.height + 'px';
  clone.style.objectFit = 'contain';
  clone.style.transition = 'none';
  document.body.appendChild(clone);

  const sidebar = document.getElementById('game-list-panel');
  const details = document.getElementById('details-panel');

  // TEMPORARILY disable transitions to measure final target state
  sidebar.style.transition = 'none';
  details.style.transition = 'none';
  
  sidebar.classList.add('collapsed');
  applyOpenDetailsState();

  sidebar.offsetHeight; details.offsetHeight; // force layout

  // Measure targets
  const detailsImg = document.getElementById('details-logo-img');
  const targetRect = detailsImg.getBoundingClientRect();
  const contentTargetRect = contentEl.getBoundingClientRect();

  // REVERT to start state so transitions will trigger
  sidebar.classList.remove('collapsed');
  details.classList.add('hidden');
  details.classList.add('is-preview');
  
  sidebar.offsetHeight; details.offsetHeight; // force layout
  
  // RE-ENABLE transitions
  sidebar.style.transition = '';
  details.style.transition = '';

  // APPLY END STATE for real
  sidebar.classList.add('collapsed');
  sidebarImg.style.opacity = '0';
  applyOpenDetailsState();
  
  detailsImg.classList.add('transitioning');

  requestAnimationFrame(() => {
    const dx = contentStartRect.left - contentTargetRect.left;
    const dy = contentStartRect.top - contentTargetRect.top;

    contentEl.style.transition = 'none';
    contentEl.style.transform = `translate(${dx}px, ${dy}px)`;
    contentEl.offsetHeight;

    contentEl.style.transition = 'transform 0.45s cubic-bezier(0.4, 0, 0.2, 1)';
    contentEl.style.transform = 'translate(0, 0)';

    const dxClone = targetRect.left - startRect.left;
    const dyClone = targetRect.top - startRect.top;

    clone.style.transition = 'none';
    clone.style.transform = 'translate(0, 0)';
    clone.offsetHeight;

    clone.style.transition = 'transform 0.45s cubic-bezier(0.4, 0, 0.2, 1), width 0.45s cubic-bezier(0.4, 0, 0.2, 1), height 0.45s cubic-bezier(0.4, 0, 0.2, 1)';
    clone.style.transform = `translate(${dxClone}px, ${dyClone}px)`;
    clone.style.width = targetRect.width + 'px';
    clone.style.height = targetRect.height + 'px';

    const cleanup = () => {
      detailsImg.classList.remove('transitioning');
      sidebarImg.style.opacity = '1';
      contentEl.style.transition = '';
      contentEl.style.transform = '';
      if (clone.parentNode) clone.remove();
    };

    clone.addEventListener('transitionend', cleanup, { once: true });
    setTimeout(cleanup, 550); // fallback
  });
}

function applyOpenDetailsState() {
  vibrate(false); // Right navigation
  detailsOpen = true;
  document.getElementById('details-panel').classList.remove('is-preview');
  document.getElementById('details-panel').classList.remove('hidden');
  document.getElementById('empty-state').style.display = 'none';
  renderDetails(games[selectedIndex]);
}

function closeDetails() {
  const g = games[selectedIndex];
  const sidebarItem = document.querySelectorAll('.game-item')[selectedIndex];
  const sidebarImg = sidebarItem?.querySelector('img');
  const detailsImg = document.getElementById('details-logo-img');
  
  if (!g?.logoPath || !sidebarImg || !detailsImg || detailsImg.style.display === 'none') {
    applyCloseDetailsState();
    return;
  }

  // START: capture details logo position
  const startRect = detailsImg.getBoundingClientRect();
  const contentEl = document.getElementById('details-content');
  const contentStartRect = contentEl.getBoundingClientRect();

  // Create clone
  const clone = document.createElement('img');
  clone.id = 'logo-transition-clone';
  clone.src = detailsImg.src;
  clone.style.left = startRect.left + 'px';
  clone.style.top = startRect.top + 'px';
  clone.style.width = startRect.width + 'px';
  clone.style.height = startRect.height + 'px';
  clone.style.objectFit = 'contain';
  clone.style.transition = 'none';
  document.body.appendChild(clone);

  const sidebar = document.getElementById('game-list-panel');
  const details = document.getElementById('details-panel');

  // TEMPORARILY disable transitions to measure final target state
  sidebar.style.transition = 'none';
  details.style.transition = 'none';
  
  applyCloseDetailsState();
  
  sidebar.offsetHeight; details.offsetHeight; // force layout
  
  // Measure targets
  const targetRect = sidebarImg.getBoundingClientRect();
  const contentTargetRect = contentEl.getBoundingClientRect();
  
  // REVERT to start state so transitions will trigger
  sidebar.classList.add('collapsed');
  details.classList.remove('is-preview');
  if (games.length) details.classList.remove('hidden');
  
  sidebar.offsetHeight; details.offsetHeight; // force layout
  
  // RE-ENABLE transitions
  sidebar.style.transition = '';
  details.style.transition = '';

  // APPLY END STATE for real
  sidebarImg.style.opacity = '0';
  applyCloseDetailsState();

  requestAnimationFrame(() => {
    const dxClone = targetRect.left - startRect.left;
    const dyClone = targetRect.top - startRect.top;

    clone.style.transition = 'none';
    clone.style.transform = 'translate(0, 0)';
    clone.offsetHeight;

    clone.style.transition = 'transform 0.45s cubic-bezier(0.4, 0, 0.2, 1), width 0.45s cubic-bezier(0.4, 0, 0.2, 1), height 0.45s cubic-bezier(0.4, 0, 0.2, 1)';
    clone.style.transform = `translate(${dxClone}px, ${dyClone}px)`;
    clone.style.width = targetRect.width + 'px';
    clone.style.height = targetRect.height + 'px';
    
    // Content transition
    const dx = contentStartRect.left - contentTargetRect.left;
    const dy = contentStartRect.top - contentTargetRect.top;

    contentEl.style.transition = 'none';
    contentEl.style.transform = `translate(${dx}px, ${dy}px)`;
    contentEl.offsetHeight;

    contentEl.style.transition = 'transform 0.4s cubic-bezier(0.4, 0, 0.2, 1)';
    contentEl.style.transform = 'translate(0, 0)';

    const cleanup = () => {
      sidebarImg.style.opacity = '1';
      contentEl.style.transition = '';
      contentEl.style.transform = '';
      if (clone.parentNode) clone.remove();
    };

    clone.addEventListener('transitionend', cleanup, { once: true });
    setTimeout(cleanup, 550);
  });
}

function applyCloseDetailsState() {
  if (detailsOpen) vibrate(true); // Left navigation
  detailsOpen = false;
  document.getElementById('game-list-panel').classList.remove('collapsed');
  document.getElementById('details-panel').classList.add('is-preview');
  if (!games.length) {
    document.getElementById('details-panel').classList.add('hidden');
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
        const typeTag = b.is_auto ? '<span style="color: #a0a0a0; font-size: 10px; border: 1px solid #555; padding: 2px 4px; border-radius: 4px; margin-right: 6px;">AUTO</span>' 
                                  : '<span style="color: #66ccff; font-size: 10px; border: 1px solid #3388aa; padding: 2px 4px; border-radius: 4px; margin-right: 6px;">MANUAL</span>';
        const displayName = b.custom_name ? `<div style="font-size: 14px; font-weight: bold; color: #fff;">${esc(b.custom_name)}</div>` : '';
        
        return `
        <div class="restore-item" style="display: flex; justify-content: space-between; align-items: center; padding: 10px; background: rgba(0,0,0,0.2); border-radius: 8px;">
          <div style="display: flex; flex-direction: column; gap: 4px;">
            ${displayName}
            <div style="display: flex; align-items: center;">
              ${typeTag}
              <span style="font-weight: bold; font-size: 12px;">${esc(b.timestamp)}</span>
            </div>
            <div style="font-size: 11px; color: rgba(255,255,255,0.5);">${(b.size_bytes / 1024).toFixed(1)} KB</div>
          </div>
          <div style="display: flex; gap: 6px; align-items: center;">
            <button class="glass-btn small" onclick="doRestore('${g.id}', '${esc(b.name)}')">Restore</button>
            <button class="glass-btn small" style="border-color: #ff4444; color: #ff4444; padding: 4px 8px;" onclick="doDeleteBackup('${g.id}', '${esc(b.name)}')">🗑</button>
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

window.doRestore = async (gameId, backupName) => {
  showToast('info', 'Restoring', 'Restoring save file from backup...');
  try {
    await window.__TAURI__.core.invoke('restore_backup', { gameId, backupName });
    showToast('success', 'Restored', 'Save files successfully restored!');
    document.getElementById('restore-overlay').classList.add('hidden');
  } catch (err) {
    showToast('error', 'Restore Failed', err);
  }
};

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
  document.getElementById('input-name').value = '';
  document.getElementById('input-font').value = '';
  document.getElementById('input-color').value = '#ffffff';
  document.getElementById('color-preview-text').textContent = '#ffffff';
  document.getElementById('input-exe').value = '';
  
  document.getElementById('edit-wallpaper-preview-img').style.display = 'none';
  document.getElementById('edit-logo-preview-img').style.display = 'none';
  
  document.getElementById('modal-overlay').classList.remove('hidden');
  setTimeout(() => document.getElementById('input-name').focus(), 50);
}

function openEditModal(g) {
  editingGameId = g.id;
  modalWallpaperPath = g.wallpaper || null;
  modalLogoPath = g.logoPath || null;
  document.getElementById('modal-title').textContent = 'EDIT GAME';
  document.getElementById('scan-folder-container').style.display = 'none';
  document.getElementById('input-name').value = g.name;
  document.getElementById('input-font').value = g.fontFamily || '';
  document.getElementById('input-color').value = g.fontColor || '#ffffff';
  document.getElementById('color-preview-text').textContent = g.fontColor || '#ffffff';
  document.getElementById('input-exe').value = g.exePath || '';
  
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

  document.getElementById('modal-overlay').classList.remove('hidden');
  setTimeout(() => document.getElementById('input-name').focus(), 50);
}

// Color picker hex preview
document.getElementById('input-color').addEventListener('input', (e) => {
  document.getElementById('color-preview-text').textContent = e.target.value;
});

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
    const data = await window.__TAURI__.core.invoke('fetch_steam_metadata', { name });
    if (data) {
      document.getElementById('input-name').value = data.name;
      
      const lImg = document.getElementById('edit-logo-preview-img');
      if (data.logo) {
        modalLogoPath = data.logo;
        lImg.src = modalLogoPath;
        lImg.style.display = 'block';
        showToast('success', 'Logo Found', `Found Steam logo for ${data.name}`);
      } else {
        modalLogoPath = null;
        lImg.style.display = 'none';
        showToast('info', 'Metadata Found', `Found Steam entry for ${data.name}`);
      }
    } else {
      showToast('error', 'Not Found', `Could not find Steam assets for "${name}"`);
    }
  } catch (err) {
    showToast('error', 'API Error', 'Failed to contact Steam API');
  }
  
  btn.textContent = '✨ Auto-Fill Metadata';
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
});

document.getElementById('btn-scan-add').addEventListener('click', async () => {
  const checkboxes = document.querySelectorAll('#scan-results-list input[type="checkbox"]:checked');
  const indices = Array.from(checkboxes).map(cb => parseInt(cb.dataset.scanIndex));
  
  let addedCount = 0;
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
  
  document.getElementById('scan-results-overlay').classList.add('hidden');
  pendingScanResults = [];
  
  if (addedCount > 0) {
    applyCategoryFilter(false);
    renderGameList();
    showToast('success', 'Added', `${addedCount} game${addedCount > 1 ? 's' : ''} added to SILO!`);
  }
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
  const inp = document.getElementById('input-name');
  if (!name) { inp.style.borderColor = 'rgba(255,100,100,0.6)'; setTimeout(() => inp.style.borderColor = '', 1500); inp.focus(); return; }

  try {
    if (editingGameId) {
      const u = await window.vault.updateGame(editingGameId, {
        name, exePath: exePath || undefined,
        wallpaper: modalWallpaperPath || undefined,
        logoPath: modalLogoPath || undefined,
        fontFamily: fontFamily || undefined,
        fontColor: fontColor || undefined,
      });
      const idx = allGames.findIndex(g => g.id === editingGameId);
      if (idx !== -1 && u) allGames[idx] = u;
      applyCategoryFilter(false);
      if (detailsOpen && games[selectedIndex]?.id === editingGameId) renderDetails(u);
      showToast('success', 'Updated', `${name} saved`);
    } else {
      const g = await window.vault.addGame({ name, exePath, wallpaper: modalWallpaperPath, logoPath: modalLogoPath, fontFamily: fontFamily || undefined, fontColor: fontColor || undefined });
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
  if (!document.getElementById('modal-overlay').classList.contains('hidden')) return;
  if (!document.getElementById('launch-overlay').classList.contains('hidden')) {
    if (e.key === 'Escape') document.getElementById('btn-launch-cancel').click();
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
  if (modalOpen) { savePad(pad); return; }

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

/* ── Settings & Backup ─────────────────────────────────────────────────────*/
document.getElementById('btn-settings').addEventListener('click', () => {
  document.getElementById('settings-overlay').classList.remove('hidden');
});
document.getElementById('btn-settings-close').addEventListener('click', () => {
  document.getElementById('settings-overlay').classList.add('hidden');
});

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
