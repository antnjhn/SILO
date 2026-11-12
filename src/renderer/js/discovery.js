/* ── discovery.js — offer games found in the user's registered folders ──────
   The backend scans only the folders the user registered in Settings and
   returns candidates that are not in the library and have not been answered
   yet. This module asks about them one at a time, in scan order, and records
   the answer:

     Add        → the game is created through the normal add path, remembered as
                  `added`, and goes straight into Edit with its artwork.
     Don't Add  → remembered as `ignored`; never offered again.
     Not now    → nothing is recorded, so it stays pending and is offered on a
                  future startup.

   Closing the prompt (Escape, backdrop, the ✕) is "Not now", never a rejection.
   ───────────────────────────────────────────────────────────────────────── */

import { showToast } from './utils.js';
import { currentPage } from './pages.js';
import { setAllGames } from './state.js';
import { applyCategoryFilter } from './library.js';
import { startGameOnboarding } from './modals.js';

const state = {
  queue: [],
  index: 0,
  busy: false,
  started: false,
  retries: 0,
};

const MAX_RETRIES = 40;       // ~2 minutes of waiting for a free screen
const RETRY_MS = 3000;
const SCAN_DELAY_MS = 1600;   // let the library paint before scanning

function el(id) {
  return document.getElementById(id);
}

/* Another modal owns the screen? The prompt must never stack on top of one. */
function promptSlotFree() {
  if (currentPage() !== 'home') return false;
  const busyOverlays = [
    'uninstall-overlay',
    'close-confirm-overlay',
    'online-art-overlay',
    'gif-policy-overlay',
    'metadata-overlay',
    'scan-results-overlay',
    'restore-overlay',
  ];
  return !busyOverlays.some((id) => {
    const node = document.getElementById(id);
    return node && !node.classList.contains('hidden');
  });
}

export function initDiscovery() {
  if (state.started) return;
  state.started = true;
  window.setTimeout(() => { void runDiscoveryScan(); }, SCAN_DELAY_MS);
}

async function runDiscoveryScan() {
  let candidates = [];
  try {
    candidates = await window.vault.discoverGames();
  } catch (error) {
    console.warn('Discovery scan failed', error);
    return;
  }
  state.queue = (Array.isArray(candidates) ? candidates : [])
    .filter((candidate) => candidate && candidate.key && candidate.exePath);
  state.index = 0;
  state.retries = 0;
  scheduleNext();
}

function scheduleNext() {
  if (state.busy) return;
  if (state.index >= state.queue.length) {
    hidePrompt();
    return;
  }
  if (!promptSlotFree()) {
    if (state.retries < MAX_RETRIES) {
      state.retries += 1;
      window.setTimeout(scheduleNext, RETRY_MS);
    }
    return;
  }
  state.retries = 0;
  void showCandidate(state.queue[state.index]);
}

async function showCandidate(candidate) {
  if (!candidate) return;
  state.busy = true;

  el('discovery-name').textContent = candidate.name || 'Unnamed game';
  el('discovery-path').textContent = candidate.exePath;
  el('discovery-path').title = candidate.exePath;
  el('discovery-folder').textContent = candidate.folder ? `Found in ${candidate.folder}` : '';
  el('discovery-folder').hidden = !candidate.folder;

  const progress = el('discovery-progress');
  if (progress) {
    const total = state.queue.length;
    progress.textContent = total > 1 ? `${state.index + 1} of ${total} new games` : '';
    progress.hidden = total <= 1;
  }

  // Artwork is a preview only: remote thumbnails, nothing downloaded here.
  const art = el('discovery-art');
  const fallback = el('discovery-art-fallback');
  if (art && fallback) {
    art.removeAttribute('src');
    art.hidden = true;
    fallback.hidden = false;
    fallback.textContent = String(candidate.name || '?').trim().charAt(0).toUpperCase() || '?';
  }
  try {
    const preview = await window.vault.previewArt(candidate.name);
    const src = preview && (preview.logo || preview.wallpaper);
    if (src && art && state.queue[state.index] === candidate) {
      art.onload = () => {
        art.hidden = false;
        if (fallback) fallback.hidden = true;
      };
      art.onerror = () => {
        art.hidden = true;
        if (fallback) fallback.hidden = false;
      };
      art.src = src;
    }
  } catch {
    // No artwork preview is fine: the prompt still works.
  }

  // The candidate may have been answered while the preview was loading.
  if (state.queue[state.index] !== candidate) return;

  el('discovery-overlay')?.classList.remove('hidden');
  el('btn-discovery-add')?.focus({ preventScroll: true });
}

function hidePrompt() {
  el('discovery-overlay')?.classList.add('hidden');
}

function advance() {
  state.busy = false;
  state.index += 1;
  hidePrompt();
  scheduleNext();
}

async function remember(candidate, decision) {
  try {
    await window.vault.recordDiscoveryDecision(candidate, decision);
  } catch (error) {
    console.warn('Could not record the discovery decision', error);
  }
}

async function refreshLibrary() {
  try {
    setAllGames(await window.vault.getGames());
    applyCategoryFilter(false);
  } catch (error) {
    console.warn('Could not refresh the library after adding a discovered game', error);
  }
}

async function addCandidate(candidate) {
  let added = null;
  try {
    added = await window.vault.addGame({ name: candidate.name, exePath: candidate.exePath });
  } catch (error) {
    showToast('error', 'Add Failed', error);
  }
  if (!added) {
    showToast('error', 'Add Failed', `${candidate.name} could not be added.`);
    advance();
    return;
  }
  await remember(candidate, 'added');
  await refreshLibrary();
  hidePrompt();
  state.busy = false;
  showToast('success', 'Added', `${added.name} added to SILO`);
  // Same onboarding as Add Game: review the new record, artwork arrives in place.
  startGameOnboarding(added);
  state.index += 1;
  scheduleNextSoon();
}

/* After the Edit modal opens, the next prompt waits for the user to come back
   to the library rather than appearing behind an open modal. */
function scheduleNextSoon() {
  window.setTimeout(scheduleNext, 700);
}

async function ignoreCandidate(candidate) {
  await remember(candidate, 'ignored');
  advance();
}

function wire() {
  el('btn-discovery-add')?.addEventListener('click', () => {
    const candidate = state.queue[state.index];
    if (candidate) addCandidate(candidate);
  });
  el('btn-discovery-ignore')?.addEventListener('click', () => {
    const candidate = state.queue[state.index];
    if (candidate) ignoreCandidate(candidate);
  });
  // "Not now": no decision is written, so the candidate stays pending.
  el('btn-discovery-later')?.addEventListener('click', () => advance());
  el('discovery-close')?.addEventListener('click', () => advance());
  el('discovery-overlay')?.addEventListener('click', (event) => {
    if (event.target === el('discovery-overlay')) advance();
  });
  document.addEventListener('keydown', (event) => {
    if (event.key !== 'Escape') return;
    if (el('discovery-overlay')?.classList.contains('hidden')) return;
    event.preventDefault();
    advance();
  });
}

wire();
