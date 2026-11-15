/* ── onboarding.js. First run: where the games are, and the artwork key ─────
   Shown once, on the first start with an empty library. It asks the two things
   SILO cannot guess:

     1. Which folder(s) hold the installed games. This is the same list Settings
        manages (`discovery.json`), so anything added here appears there too.
     2. A SteamGridDB API key. Artwork is fetched from SteamGridDB, and without
        the key the launcher can only add games without covers. The step links
        to the page where the free key is created and can be skipped.

   Finishing the setup scans the chosen folders at once, and the usual
   "new game found" prompts follow. Nothing is added without an answer.
   ───────────────────────────────────────────────────────────────────────── */

import { showToast } from './utils.js';
import { initDiscovery, startDiscoveryWith } from './discovery.js';
import { setAllGames } from './state.js';
import { applyCategoryFilter } from './library.js';

const DONE_KEY = 'silo.onboarding.done';
const SGDB_KEY_URL = 'https://www.steamgriddb.com/profile/preferences/api';

const STEPS = 4;

const state = {
  folders: [],
  candidates: [],
  busy: false,
  // Games brought in from a store, so a second click never re-adds them.
  importedExes: new Set(),
};

function el(id) {
  return document.getElementById(id);
}

function markDone() {
  try { localStorage.setItem(DONE_KEY, '1'); } catch { /* private mode */ }
}

/* One rail for all four screens: completed pills in the darker violet, the
   current one in the accent, the rest neutral. The count is spelled out beside
   it and mirrored into the group's accessible name. */
function setStep(step) {
  for (let n = 1; n <= STEPS; n += 1) {
    el(`ob-step-${n}`)?.classList.toggle('hidden', n !== step);
    const pill = document.querySelector(`.ob-step-pill[data-pill="${n}"]`);
    if (!pill) continue;
    pill.classList.toggle('is-current', n === step);
    pill.classList.toggle('is-done', n < step);
  }
  const count = el('ob-step-count');
  if (count) count.textContent = `${step} of ${STEPS}`;
  const rail = el('ob-steps');
  if (rail) rail.setAttribute('aria-label', `Step ${step} of ${STEPS}`);
}

function hideOverlay() {
  el('onboarding-overlay')?.classList.add('hidden');
  document.body.classList.remove('onboarding-open');
}

function renderFolders() {
  const list = el('ob-folder-list');
  const empty = el('ob-folder-empty');
  if (!list) return;
  list.replaceChildren();
  for (const folder of state.folders) {
    const row = document.createElement('li');
    row.className = 'ob-folder';
    const label = document.createElement('span');
    label.className = 'ob-folder-path';
    label.textContent = folder;
    label.title = folder;
    const remove = document.createElement('button');
    remove.type = 'button';
    remove.className = 'ob-folder-remove';
    remove.textContent = 'Remove';
    remove.addEventListener('click', async () => {
      try {
        const view = await window.vault.discoveryRemoveFolder(folder);
        state.folders = view?.folders || state.folders.filter((f) => f !== folder);
        renderFolders();
      } catch (error) {
        showToast('error', 'Remove Failed', error);
      }
    });
    row.append(label, remove);
    list.append(row);
  }
  if (empty) empty.hidden = state.folders.length > 0;
}

async function loadFolders() {
  try {
    const view = await window.vault.getDiscoveryState();
    state.folders = (view?.folders || []).slice();
  } catch {
    state.folders = [];
  }
  renderFolders();
}

async function addFolder() {
  let folder = null;
  try {
    folder = await window.vault.pickSaveFolder();
  } catch (error) {
    showToast('error', 'Folder Error', error);
    return;
  }
  if (!folder) return;
  try {
    const view = await window.vault.discoveryAddFolder(folder);
    state.folders = view?.folders || state.folders;
    renderFolders();
  } catch (error) {
    showToast('error', 'Folder Not Added', error);
  }
}

async function saveKey() {
  const input = el('ob-sgdb-key');
  const key = (input?.value || '').trim();
  if (key) {
    try {
      const current = await window.vault.getSettings();
      await window.vault.setSettings({
        settings: {
          sgdbApiKey: key,
          checkUpdatesOnLaunch: !!current?.checkUpdatesOnLaunch,
          showGameTitles: !!current?.showGameTitles,
          closeBehaviour: current?.closeBehaviour || 'askEveryTime',
        },
      });
      showToast('success', 'Artwork Ready', 'SteamGridDB artwork will be fetched automatically.');
    } catch (error) {
      showToast('error', 'Key Not Saved', error);
      return false;
    }
  }
  return true;
}

async function scanNow() {
  setStep(4);
  const title = el('ob-scan-title');
  const detail = el('ob-scan-detail');
  // The scan is quick and has no spinner: the title carries the state, and the
  // description stays empty until there is a result to describe.
  if (title) title.textContent = 'Looking for games…';
  if (detail) detail.textContent = '';
  state.busy = true;
  el('ob-finish')?.classList.add('hidden');
  let found = [];
  try {
    found = (await window.vault.discoverGames()) || [];
  } catch (error) {
    console.warn('First-run scan failed', error);
  }
  state.candidates = Array.isArray(found) ? found : [];
  state.busy = false;
  const count = state.candidates.length;
  if (title) title.textContent = count === 0 ? 'All set.' : `${count} game${count === 1 ? '' : 's'} waiting.`;
  if (detail) {
    detail.textContent = count === 0
      ? 'Nothing new was found in those folders. SILO will look again the next time you start it.'
      : 'SILO will ask about each one, and fetch its artwork automatically when you add it.';
  }
  el('ob-finish')?.classList.remove('hidden');
  el('ob-finish')?.focus({ preventScroll: true });
}

/* ── Step 3: the store libraries ─────────────────────────────────────────────
   Each store is read on demand and the games it reports are added through the
   normal add path, so they land in the library exactly as a hand-added game
   would. Artwork is fetched per game afterwards when a key is saved. Nothing
   is imported without a click, and folders are untouched. */
/* One store at a time: the chosen card keeps the violet border and the others
   drop it, so the status line below always belongs to a card you can see. */
function selectStore(button) {
  for (const card of document.querySelectorAll('.ob-card')) {
    const chosen = card === button;
    card.classList.toggle('is-selected', chosen);
    card.setAttribute('aria-pressed', chosen ? 'true' : 'false');
  }
}

async function importStore(source, button) {
  if (state.busy || !button) return;
  const status = el('ob-import-status');
  state.busy = true;
  selectStore(button);
  // The card carries only its logo, so progress is spoken and shown below it
  // instead of swapped into the button's text.
  button.disabled = true;
  button.setAttribute('aria-busy', 'true');
  if (status) {
    status.hidden = false;
    status.textContent = `Looking through your ${source} library…`;
  }

  let candidates = [];
  try {
    candidates = (await window.vault[`import${source}Library`]()) || [];
  } catch (error) {
    console.warn('Library import failed', error);
    if (status) status.textContent = `${source} could not be read. You can import it later from the Import page.`;
    button.disabled = false;
    button.removeAttribute('aria-busy');
    state.busy = false;
    return;
  }

  let known = new Set();
  try {
    const library = (await window.vault.getGames()) || [];
    known = new Set(library.map((game) => game.exePath).filter(Boolean));
  } catch {
    known = new Set();
  }

  const fresh = candidates.filter((candidate) => candidate
    && candidate.exePath
    && !known.has(candidate.exePath)
    && !state.importedExes.has(candidate.exePath));

  let added = 0;
  for (const candidate of fresh) {
    try {
      const game = await window.vault.addGame({ name: candidate.name, exePath: candidate.exePath });
      if (!game) continue;
      added += 1;
      state.importedExes.add(candidate.exePath);
      try { await window.vault.autoArtForGame(game.id); } catch { /* artwork is optional */ }
    } catch (error) {
      console.warn(`Could not add ${candidate.name}`, error);
    }
  }

  button.disabled = false;
  button.removeAttribute('aria-busy');
  state.busy = false;
  if (status) {
    status.textContent = added
      ? `${added} game${added === 1 ? '' : 's'} added from ${source}.`
      : `Nothing new found in your ${source} library.`;
  }
  try {
    setAllGames(await window.vault.getGames());
    applyCategoryFilter(false);
  } catch (error) {
    console.warn('Could not refresh the library after the import', error);
  }
  // A burst of adds can outrun its own per-game artwork lookup, which leaves
  // letter tiles behind. The sweep retries everything still missing and reports
  // its progress in a notification, so setup ends with artwork in place.
  if (added) {
    try { window.vault.sweepArt(); } catch { /* artwork is optional */ }
  }
}

function finish() {
  markDone();
  hideOverlay();
  if (state.candidates.length) startDiscoveryWith(state.candidates);
}

/* Skipping still leaves a normal startup scan in place for the folders that
   were already registered. The setup is an offer, not a gate. */
function skipEverything() {
  markDone();
  hideOverlay();
  window.setTimeout(() => initDiscovery(), 400);
}

function wire() {
  // The overlay is wired once; every handler is a no-op when it is closed.
  el('ob-add-folder')?.addEventListener('click', addFolder);
  el('ob-next-1')?.addEventListener('click', () => setStep(2));
  el('ob-skip-1')?.addEventListener('click', () => setStep(2));
  el('ob-get-key')?.addEventListener('click', async (event) => {
    event.preventDefault();
    try {
      await window.vault.openExternalUrl(SGDB_KEY_URL);
    } catch (error) {
      showToast('error', 'Link Failed', error);
    }
  });
  el('ob-save-key')?.addEventListener('click', async () => {
    if (state.busy) return;
    state.busy = true;
    const saved = await saveKey();
    state.busy = false;
    if (saved) setStep(3);
  });
  // Skipping the key still leaves the store libraries, so both answers go
  // there; only the last step runs the scan.
  el('ob-skip-2')?.addEventListener('click', () => {
    if (state.busy) return;
    setStep(3);
  });
  for (const source of ['steam', 'epic', 'gog']) {
    const button = el(`ob-import-${source}`);
    button?.addEventListener('click', () => importStore(button.dataset.source, button));
  }
  el('ob-skip-3')?.addEventListener('click', () => { if (!state.busy) scanNow(); });
  el('ob-next-3')?.addEventListener('click', () => { if (!state.busy) scanNow(); });
  el('ob-finish')?.addEventListener('click', finish);
  el('ob-close')?.addEventListener('click', skipEverything);
}

/* Only on a genuine first run: the flag is written the moment the user answers
   the last step (or skips), and an existing library means this is not a first
   run even if the flag was lost. */
export async function initOnboarding() {
  try {
    if (localStorage.getItem(DONE_KEY) === '1') return false;
  } catch {
    return false;
  }
  let games = [];
  try {
    games = await window.vault.getGames();
  } catch {
    return false;
  }
  if (Array.isArray(games) && games.length > 0) {
    markDone();
    return false;
  }
  await loadFolders();
  wire();
  setStep(1);
  el('onboarding-overlay')?.classList.remove('hidden');
  document.body.classList.add('onboarding-open');
  // The dialog takes focus, not a button: a ring around "+ Add folder" before
  // the user has touched anything reads as a hover state that is already on.
  el('onboarding-card')?.focus({ preventScroll: true });
  return true;
}
