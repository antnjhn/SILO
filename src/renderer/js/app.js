/* ── app.js — entry point: wires the modules together and boots the launcher ── */
import {
  allGames, games, selectedIndex, setAllGames, setShowGameTitles, setSearchQuery, gameNameById,
} from './state.js';
import { listen } from './api.js';
import { showToast, backupTimestampLabel, esc } from './utils.js';
import {
  layoutRail, syncMotionToggle, applyCategoryFilter, renderDetails,
  retriggerHeroAnimation, runInstallCheck, toggleSearch,
  changeCategorySelection, closeCategories, openCategories,
  selectGame, railEnterGame, railExitGame,
  toggleFilterPanel, isFilterPanelOpen,
  closeDetails, openDetails, detailsOpen, railEntered, categoriesOpen,
  forceMotion, focusDock, activateDockFocus,
} from './library.js';
import { closeMetadataPicker, closeOnlineArtPicker } from './modals.js';
// Side-effect module: binds Backup Manager and the library export / restore
// buttons. It has no named exports, so it is loaded for its handlers alone.
import './backups.js';
import { bindStatPopoverHover, hideStatPopoverNow, isStatPopoverOpen } from './stats.js';
import { applyShowGameTitles } from './settings.js';
import { offerSetupIfPending } from './migrate.js';
import { initGamepadPolling } from './gamepad.js';
import { initDiscovery } from './discovery.js';
import { initDisplayMonitoring } from './display.js';
import { showPage, currentPage } from './pages.js';
import { rememberedGameId, rememberedPage } from './uistate.js';
import {
  initNav, revealNavAndFocus, dismissNav, isNavRevealed,
  isNavKeyboardActive, moveNavFocus, activateNavItem,
} from './nav.js';

/* ── Init ──────────────────────────────────────────────────────────────────*/
async function init() {
  // Apply the motion override before anything measures or animates.
  document.body.classList.toggle('force-motion', forceMotion);
  syncMotionToggle();
  layoutRail();   // publish the rail geometry before the first logo renders
  setAllGames(await window.vault.getGames());
  try {
    const s = await window.vault.getSettings();
    setShowGameTitles(!!s.showGameTitles);
  } catch (e) {
    setShowGameTitles(false);
  }
  applyShowGameTitles(false);
  applyCategoryFilter();
  initNav();
  initGamepadPolling();
  // Folders the user registered in Settings are scanned in the background; any
  // genuinely new game is offered one prompt at a time, long after first paint.
  initDiscovery();

  // Verify the library only once the UI is on screen, so the app opens
  // instantly; badges appear when the backend answers. requestIdleCallback keeps
  // the check off the first paint, and its timeout guarantees it still runs if
  // the window never goes idle (minimised at startup, backgrounded).
  if (window.requestIdleCallback) {
    window.requestIdleCallback(() => runInstallCheck({ force: true }), { timeout: 1000 });
  } else {
    setTimeout(() => runInstallCheck({ force: true }), 300);
  }
  // Coming back to the window re-checks, debounced to at most once every 30s.
  window.addEventListener('focus', () => runInstallCheck());

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
    hideStatPopoverNow();
  });

  // ── SaveGuard status events (backed by Rust launch_game) ──
  listen('saveguard-path-detected', (event) => {
    const p = event.payload || {};
    const name = gameNameById(p.gameId);
    if (p.savePath) {
      const idx = allGames.findIndex(x => x.id === p.gameId);
      if (idx !== -1) { allGames[idx].savePath = p.savePath; allGames[idx].savePathSource = 'auto'; }
      if (detailsOpen && games[selectedIndex]?.id === p.gameId) renderDetails(games[selectedIndex]);
    }
    showToast('success', 'SaveGuard', `${name}: save folder found and saved. Auto-backups will run from now on.`);
  });
  listen('saveguard-backup-complete', (event) => {
    const p = event.payload || {};
    const ts = p.timestamp ? ` at ${backupTimestampLabel(p.timestamp)}` : '';
    showToast('success', 'Auto-Backup Complete', `${gameNameById(p.gameId)}: save files backed up${ts}.`);
  });
  listen('saveguard-backup-failed', (event) => {
    const p = event.payload || {};
    showToast('error', 'Auto-Backup Failed', `${gameNameById(p.gameId)}: ${p.reason || 'unknown error'}`);
  });
  listen('saveguard-path-missing', (event) => {
    const p = event.payload || {};
    showToast('info', 'Save Folder Missing', `${gameNameById(p.gameId)}: the saved folder is gone. It will be re-detected automatically on the next launch.`);
  });
  listen('saveguard-not-found', (event) => {
    const p = event.payload || {};
    showToast('info', 'SaveGuard', `${gameNameById(p.gameId)}: no save writes were detected this session. If the game saves elsewhere, set the folder manually in Edit.`);
  });
  // Relinking after a PC move rewrites several games at once, so the rail is
  // patched in one pass instead of a toast per game. Each entry carries how the
  // folder was found — a relink here, or saves brought across from another
  // machine — so the tag matches what was written to the library.
  listen('savefolders-relinked', (event) => {
    const p = event.payload || {};
    (p.games || []).forEach((entry) => {
      const idx = allGames.findIndex(x => x.id === entry.gameId);
      if (idx !== -1) {
        allGames[idx].savePath = entry.savePath;
        allGames[idx].savePathSource = entry.savePathSource || 'auto';
      }
    });
    if (detailsOpen && games[selectedIndex]) renderDetails(games[selectedIndex]);
  });

  // ── Gaming Mode lifecycle ──
  // The opt-in launch returns as soon as the monitor accepts it, so an outcome
  // discovered later (untrackable process, refused concurrent launch, failed
  // spawn) reaches the user here. The launcher is visible again by the time
  // these arrive: the monitor restores it before reporting.
  listen('gaming-session-uncertain', (event) => {
    const p = event.payload || {};
    showToast('info', 'Gaming Mode', p.message || 'This session could not be tracked.');
  });
  listen('gaming-session-rejected', (event) => {
    const p = event.payload || {};
    showToast('warning', 'Gaming Mode', p.message || 'Another game is already running under Gaming Mode.');
  });
  listen('gaming-session-launch-failed', (event) => {
    const p = event.payload || {};
    showToast('error', 'Gaming Mode', p.message || 'The game could not be started.');
  });
  listen('gaming-session-launch-confirmed', (event) => {
    const p = event.payload || {};
    showToast('success', 'Launched', p.gameName || 'Game is running');
  });

  // The backend defers the close and asks the user when the close behaviour is
  // "Ask every time". Show the confirmation panel and wait for their choice.
  listen('close-confirm-requested', () => {
    showCloseConfirm();
  });

  // ── Hover mini-charts over PLAYTIME / SESSIONS ──
  bindStatPopoverHover();

  if (games.length === 0) {
    document.getElementById('details-panel').classList.add('hidden');
    document.getElementById('empty-state').style.display = 'flex';
  } else {
    document.getElementById('details-panel').classList.remove('hidden');
    document.getElementById('details-panel').classList.add('is-preview');
    document.getElementById('empty-state').style.display = 'none';
    if (!detailsOpen) {
      renderDetails(games[selectedIndex]);
      retriggerHeroAnimation();
    }
  }

  // A library restored from another PC is offered the setup step once, on the
  // boot after the restore. It can be reopened later from Advanced.
  offerSetupIfPending();

  // Route memory: this boot may be the renderer that replaced the one Stage 6
  // destroyed while a game was running. The library itself already came back
  // from Rust above, so this only puts the launcher back where the user left it
  // (a game that no longer exists, or a stale hint, simply does nothing).
  restoreRouteMemory();
}

/* Puts the launcher back on the page and game it was last showing. Called after
   the library is loaded, so both are applied to a UI that is already usable. */
function restoreRouteMemory() {
  const rememberedId = rememberedGameId();
  if (rememberedId) {
    const index = allGames.findIndex(game => game.id === rememberedId);
    if (index !== -1) selectGame(index);
  }
  const page = rememberedPage();
  if (page && page !== 'home' && page === currentPage()) {
    return;
  }
  if (page && page !== 'home') showPage(page, { instant: true });
}

/* ── Window controls ───────────────────────────────────────────────────────*/
document.getElementById('btn-minimize').addEventListener('click', () => window.vault.windowMinimize());
document.getElementById('btn-maximize').addEventListener('click', () => window.vault.windowMaximize());
document.getElementById('btn-close').addEventListener('click', () => window.vault.windowClose());

document.getElementById('titlebar').addEventListener('mousedown', (e) => {
  if (e.target.closest('#titlebar-controls')) return;
  if (e.button === 0) window.vault.windowStartDragging();
});

/* ── Close confirmation — "When closing SILO" = Ask every time ───────────────
   The backend defers the window close and emits close-confirm-requested. This
   panel lets the user pick minimise-to-tray or a genuine exit, and optionally
   remember the choice so they are not asked again. The window stays open until
   the user answers or cancels. ─────────────────────────────────────────────*/

function showCloseConfirm() {
  const overlay = document.getElementById('close-confirm-overlay');
  const remember = document.getElementById('close-confirm-remember');
  if (!overlay) return;
  if (remember) remember.checked = false;
  overlay.classList.remove('hidden');
  requestAnimationFrame(() => overlay.classList.add('open'));
}

function hideCloseConfirm() {
  const overlay = document.getElementById('close-confirm-overlay');
  if (!overlay) return;
  overlay.classList.remove('open');
  setTimeout(() => overlay.classList.add('hidden'), 240);
}

function handleCloseConfirm(choice) {
  const remember = document.getElementById('close-confirm-remember');
  hideCloseConfirm();
  window.vault
    .confirmClose(choice, Boolean(remember && remember.checked))
    .catch((err) => {
      console.error('Close SILO failed:', err);
      showToast('error', 'Close Failed', String(err));
    });
}

document.getElementById('close-choice-tray')?.addEventListener('click', () => handleCloseConfirm('tray'));
document.getElementById('close-choice-exit')?.addEventListener('click', () => handleCloseConfirm('exit'));
document.getElementById('close-confirm-cancel')?.addEventListener('click', hideCloseConfirm);
document.getElementById('close-confirm-overlay')?.addEventListener('click', (e) => {
  if (e.target.id === 'close-confirm-overlay') hideCloseConfirm();
});

/* ── Keyboard ──────────────────────────────────────────────────────────────*/

/* True for a control that is being typed into or chosen from: Escape belongs
   to that control, not to the page around it. */
function isTextField(el) {
  if (!el) return false;
  const tag = el.tagName;
  return tag === 'INPUT' || tag === 'SELECT' || tag === 'TEXTAREA' || !!el.isContentEditable;
}

document.addEventListener('keydown', e => {
  const isEscape = e.key === 'Escape';

  // ── The back hierarchy ───────────────────────────────────────────────────
  // Escape closes the topmost transient thing first: a picker, a manager, a
  // filter panel, then a subpage, and only then the top navigation. It never
  // exits SILO and never discards anything.
  if (isEscape) {
    if (!document.getElementById('metadata-overlay').classList.contains('hidden')) {
      e.preventDefault(); closeMetadataPicker(); return;
    }
    if (!document.getElementById('online-art-overlay').classList.contains('hidden')) {
      e.preventDefault(); closeOnlineArtPicker(); return;
    }
    if (!document.getElementById('backup-manager-overlay').classList.contains('hidden')) {
      e.preventDefault(); document.getElementById('btn-backup-manager-close').click(); return;
    }
    // Canceling the close confirmation keeps the window open (the close is
    // deferred) and never exits SILO.
    if (!document.getElementById('close-confirm-overlay').classList.contains('hidden')) {
      e.preventDefault(); hideCloseConfirm(); return;
    }
    // The statistics preview is a preview: it steps aside before anything
    // else happens, and it never leaves the application.
    if (isStatPopoverOpen()) {
      e.preventDefault(); hideStatPopoverNow(); return;
    }
    if (isFilterPanelOpen()) {
      e.preventDefault(); toggleFilterPanel(false); return;
    }
    if (categoriesOpen) {
      e.preventDefault(); closeCategories(false); return;
    }
    // The search field clears itself and steps aside.
    const searchInput = document.getElementById('search-input');
    if (searchInput && document.activeElement === searchInput) {
      e.preventDefault();
      if (searchInput.value) { searchInput.value = ''; setSearchQuery(''); applyCategoryFilter(); }
      toggleSearch(false);
      searchInput.blur();
      return;
    }
    // A field that is being typed into keeps the page: leaving it would throw
    // away what was typed. The field handles Escape itself.
    if (isTextField(document.activeElement)) return;
    // A subpage goes back to the library.
    if (currentPage() !== 'home') {
      e.preventDefault(); showPage('home'); return;
    }
    // Then the top navigation steps aside.
    if (isNavRevealed()) {
      e.preventDefault(); dismissNav(); return;
    }
    // A search in progress is cleared before anything else happens.
    if (searchInput && searchInput.value) {
      e.preventDefault();
      searchInput.value = '';
      setSearchQuery('');
      applyCategoryFilter();
      return;
    }
  }

  // Metadata picker is opened while the add/edit page is still visible, so it
  // must be checked first
  if (!document.getElementById('metadata-overlay').classList.contains('hidden')) return;
  if (!document.getElementById('online-art-overlay').classList.contains('hidden')) return;
  if (!document.getElementById('backup-manager-overlay').classList.contains('hidden')) return;

  // The statistics page keeps its own left/right for switching the view.
  if (currentPage() === 'stats') {
    if (e.key === 'ArrowLeft') { e.preventDefault(); document.getElementById('btn-stats-overall').click(); }
    if (e.key === 'ArrowRight') { e.preventDefault(); document.getElementById('btn-stats-games').click(); }
    return;
  }

  if (categoriesOpen) {
    switch (e.key) {
      case 'ArrowUp': e.preventDefault(); changeCategorySelection(-1); break;
      case 'ArrowDown': e.preventDefault(); changeCategorySelection(1); break;
      case 'ArrowRight':
      case 'Enter': e.preventDefault(); closeCategories(true); break;
    }
    return;
  }

  if (isFilterPanelOpen()) {
    if (e.key === 'ArrowLeft' && !e.target.closest('#filter-panel')) {
      e.preventDefault();
      toggleFilterPanel(false);
      return;
    }
    if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
      const tags = [...document.querySelectorAll('#tag-filter-options .tag-filter')];
      if (tags.length) {
        e.preventDefault();
        const index = tags.indexOf(document.activeElement);
        const next = index < 0 ? 0 : Math.max(0, Math.min(tags.length - 1, index + (e.key === 'ArrowDown' ? 1 : -1)));
        tags[next].focus();
      }
      return;
    }
    // Any other key belongs to the panel, not to the rail.
    if (e.key.startsWith('Arrow')) { e.preventDefault(); return; }
  }

  // If the user is typing in a text field, don't hijack keys
  if (isTextField(document.activeElement)) return;

  if (e.key === '/' || (e.ctrlKey && e.key.toLowerCase() === 'k')) {
    // The search field is not part of the visible interface in this pass, so
    // the shortcut does not open a search that cannot be seen.
    const field = document.getElementById('search-input');
    if (!field || field.hidden) return;
    e.preventDefault();
    revealNavAndFocus();
    toggleSearch(true);
    return;
  }

  // ── The navigation ──────────────────────────────────────────────────────
  // The bar owns the arrow keys only while the keyboard is driving it. When
  // it is merely on screen because the pointer passed over it, the library
  // keeps its own arrow keys.
  if (isNavKeyboardActive()) {
    switch (e.key) {
      // The bar is a horizontal row, so only the horizontal arrows drive it.
      // DOWN dismisses it (it must not fall through and scroll the rail);
      // UP keeps browsing the rail, as it always has.
      case 'ArrowLeft': e.preventDefault(); moveNavFocus(-1); return;
      case 'ArrowRight': e.preventDefault(); moveNavFocus(1); return;
      case 'ArrowDown': e.preventDefault(); dismissNav(); return;
      case 'Enter': e.preventDefault(); activateNavItem(); return;
      case 'Escape': e.preventDefault(); dismissNav(); return;
    }
  }

  switch (e.key) {
    // Two levels: UP/DOWN browse the rail (the focused game's stats follow it),
    // RIGHT steps into the machine so its action buttons come out, LEFT steps
    // back out. A / Enter still opens the full details view.
    case 'ArrowUp':
      e.preventDefault();
      // Up from the topmost game reveals navigation and focuses it; from any
      // other game it moves to the previous game without touching the nav.
      if (!isNavRevealed()) {
        if (selectedIndex === 0) {
          revealNavAndFocus();
        } else {
          selectGame(selectedIndex - 1);
        }
      } else if (!detailsOpen && selectedIndex > 0) {
        selectGame(selectedIndex - 1);
      }
      break;
    case 'ArrowDown':
      e.preventDefault();
      if (!detailsOpen && selectedIndex < games.length - 1) selectGame(selectedIndex + 1);
      break;
    case 'ArrowRight':
      e.preventDefault();
      // Inside the details the dock owns the horizontal arrows; browsing, RIGHT
      // steps into the focused game.
      if (detailsOpen) focusDock(1);
      else railEnterGame();
      break;
    case 'ArrowLeft':
      e.preventDefault();
      if (detailsOpen) focusDock(-1);           // LEFT walks the action dock
      else if (railEntered) railExitGame();     // LEFT steps back out first...
      else if (isFilterPanelOpen()) toggleFilterPanel(false);
      else openCategories();                    // ...then reveals the category wheel
      break;
    case 'Enter':      e.preventDefault();
      if (detailsOpen) activateDockFocus();
      else if (games.length) openDetails();
      break;
    // C opens the category wheel too, matching the controller's LB.
    case 'c': case 'C':
      if (!detailsOpen && !categoriesOpen) { e.preventDefault(); openCategories(); }
      break;
    case 'Escape':
      if (detailsOpen) closeDetails();
      break;
  }
});

/* ── Go ────────────────────────────────────────────────────────────────────*/
init();
initDisplayMonitoring();
