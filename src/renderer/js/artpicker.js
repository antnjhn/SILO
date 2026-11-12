/* ── artpicker.js — SteamGridDB artwork browser ─────────────────────────────
   Two stages, plain DOM only: pick the game on SteamGridDB, then browse its
   whole artwork catalogue and preview an image before applying it. Choosing a
   card only SELECTS it — the preview panel explains how SILO will render it
   and Apply fills the add/edit form. Picker state lives here; modals.js only
   owns the form fields the apply callback writes into. */
import { esc } from './utils.js';
import { describeWallpaper, sortWallpapers, displaySummary, onDisplayChange } from './display.js';

const PAGE_LIMIT = 50;   // backend maximum
const PAGE_CAP = 20;     // hard stop: a wrong `total` can never loop forever
const BATCH_SIZE = 18;   // cards appended per batch / per "Show more"

const KIND = { wallpaper: 'heroes', logo: 'logos' };
const KEY_HINT = 'Add your free key in Settings → SteamGridDB.';
const searchCache = new Map();
const artCache = new Map();

const state = {
  mode: 'wallpaper',
  apply: null,
  game: null,
  arts: [],
  shown: 0,
  selected: -1,
  request: 0,   // bumped on every new load so stale responses are dropped
  flight: null, // { query, request } while a stage-A search is in flight
};

let wired = false;
let pickerTrigger = null;

function el(id) {
  return document.getElementById(id);
}

function overlayEl() {
  return el('online-art-overlay');
}

/* ── Small render helpers ───────────────────────────────────────────────────*/
function setStage(text) {
  const stage = el('online-art-stage');
  if (stage) stage.textContent = text;
}

function galleryStepTitle() {
  const noun = state.mode === 'logo' ? 'Logos' : 'Wallpapers';
  return `2 · ${noun} for ${state.game ? state.game.name : ''}`;
}

function updateMonitorLine() {
  const monitor = el('online-art-monitor');
  if (!monitor) return;
  const summary = displaySummary();
  monitor.textContent = summary === 'Monitor unknown'
    ? 'Monitor unknown'
    : `Monitor ${summary} · sorted by fit`;
}

function errorMessage(error) {
  if (typeof error === 'string' && error) return error;
  if (error && typeof error.message === 'string' && error.message) return error.message;
  return 'The art service could not be reached.';
}

/* Text nodes only: API and user strings are never parsed as HTML. */
function showNotice(message, hint) {
  const notice = el('online-art-notice');
  if (!notice) return;
  if (!message) {
    notice.replaceChildren();
    notice.hidden = true;
    return;
  }
  const line = document.createElement('span');
  line.className = 'online-art-notice-line';
  line.textContent = message;
  notice.replaceChildren(line);
  if (hint) {
    const hintLine = document.createElement('span');
    hintLine.className = 'online-art-notice-hint';
    hintLine.textContent = hint;
    notice.append(hintLine);
  }
  notice.hidden = false;
}

function showEmpty(message, hint) {
  const empty = el('online-art-empty');
  if (!empty) return;
  if (!message) {
    empty.replaceChildren();
    empty.hidden = true;
    return;
  }
  const line = document.createElement('span');
  line.className = 'online-art-empty-line';
  line.textContent = message;
  empty.replaceChildren(line);
  if (hint) {
    const hintLine = document.createElement('span');
    hintLine.className = 'online-art-empty-hint';
    hintLine.textContent = hint;
    empty.append(hintLine);
  }
  empty.hidden = false;
}

function setApplyEnabled(enabled) {
  const apply = el('btn-art-apply');
  if (apply) apply.disabled = !enabled;
}

function clearPreview() {
  const preview = el('art-preview');
  const img = el('art-preview-img');
  const meta = el('art-preview-meta');
  const note = el('art-preview-note');
  if (img) {
    img.removeAttribute('src');
    img.hidden = true;
  }
  if (meta) meta.replaceChildren();
  if (note) {
    note.textContent = '';
    note.hidden = true;
    note.classList.remove('is-caution');
  }
  if (preview) preview.hidden = true;
}

/* Cards are buttons; the caption reads as one line under the image. */
function pill(text, caution) {
  const span = document.createElement('span');
  span.className = caution ? 'art-pill is-caution' : 'art-pill';
  span.textContent = text;
  return span;
}

function buildCard(art, index) {
  const button = document.createElement('button');
  button.type = 'button';
  button.className = state.mode === 'logo' ? 'art-card art-card-logo' : 'art-card art-card-wallpaper';
  button.dataset.index = String(index);
  button.setAttribute('aria-pressed', index === state.selected ? 'true' : 'false');
  if (index === state.selected) button.classList.add('selected');

  /* Accessible name: size + style/tier, per better-accessibility name precedence. */
  const parts = [];
  if (art.width && art.height) parts.push(`${art.width} × ${art.height}`);
  else parts.push('Size unknown');
  if (state.mode === 'logo') {
    if (art.style) parts.push(art.style);
    if (art.animated) parts.push('Animated logo');
  } else {
    const described = describeWallpaper(art.width, art.height);
    if (described.label) parts.push(described.label);
  }
  button.setAttribute('aria-label', `Select artwork: ${parts.join(', ')}`);

  const img = document.createElement('img');
  img.src = art.thumb || art.url;
  img.loading = 'lazy';
  img.alt = '';
  img.addEventListener('error', () => {
    // Some SGDB thumbnail hosts are stale while the canonical asset still works.
    if (img.src !== art.url) img.src = art.url;
    else img.classList.add('art-image-failed');
  }, { once: true });
  button.append(img);

  const caption = document.createElement('span');
  caption.className = 'art-caption';

  const size = document.createElement('span');
  size.className = 'art-caption-size';
  size.textContent = art.width && art.height ? `${art.width} × ${art.height}` : 'Size unknown';
  caption.append(size);

  if (state.mode === 'logo') {
    if (art.style) {
      const style = document.createElement('span');
      style.className = 'art-caption-style';
      style.textContent = art.style;
      caption.append(style);
    }
    if (art.animated) caption.append(pill('Animated', false));
  } else {
    const described = describeWallpaper(art.width, art.height);
    caption.append(pill(described.label, described.tier >= 2));
  }

  button.append(caption);
  return button;
}

/* ── Stage A — choose the game ──────────────────────────────────────────────*/
function resetGallery() {
  state.game = null;
  state.arts = [];
  state.shown = 0;
  state.selected = -1;

  const gallery = el('online-art-gallery');
  if (gallery) {
    gallery.replaceChildren();
    gallery.hidden = true;
  }
  const pagination = el('online-art-pagination');
  if (pagination) pagination.hidden = true;
  const showMore = el('art-show-more');
  if (showMore) showMore.hidden = true;
  const count = el('online-art-count');
  if (count) count.textContent = '';

  clearPreview();
  setApplyEnabled(false);
}

function showStageA() {
  state.request += 1;   // any in-flight gallery load is now stale
  state.flight = null;  // the search surface re-arms (Back = explicit reset)
  resetGallery();

  const list = el('online-art-list');
  if (list) list.hidden = false;
  const back = el('btn-online-art-back');
  if (back) back.hidden = true;
  const go = el('btn-online-art-go');
  if (go) go.disabled = false;

  showNotice(null);
  showEmpty(null);
  setStage('1 · Choose the game');
  el('online-art-search')?.focus();
}

function renderGames(games, query) {
  const list = el('online-art-list');
  if (!list) return;
  const needle = query.trim().toLowerCase();
  list.innerHTML = games.map((game, index) => {
    const name = String(game.name || '');
    const exact = name.trim().toLowerCase() === needle;
    const year = game.year
      ? `<span class="online-art-game-year">${esc(game.year)}</span>`
      : '';
    const badge = exact ? '<span class="online-art-badge">Exact</span>' : '';
    return `<button type="button" class="online-art-game" data-game-index="${index}">
      <span class="online-art-game-name">${esc(name)}</span>
      ${year}${badge}
    </button>`;
  }).join('');

  list.querySelectorAll('[data-game-index]').forEach((button) => {
    button.addEventListener('click', () => {
      const game = games[Number(button.dataset.gameIndex)];
      if (game) openGallery(game);
    });
  });
}

async function runSearch() {
  const input = el('online-art-search');
  const list = el('online-art-list');
  if (!input || !list) return;
  const query = input.value.trim();
  if (!query) return;

  /* Same-query dedupe: while an identical search is in flight, Enter and click
     re-triggers are ignored entirely — jumping the generation counter would
     only cancel a request whose answer we already expect. */
  if (state.flight && state.flight.query === query) return;

  state.request += 1;
  const request = state.request;
  state.flight = { query, request };
  const isCurrent = () => request === state.request;

  const go = el('btn-online-art-go');
  if (go && isCurrent()) go.disabled = true;
  /* Only the current generation owns the button and the flight flag: a stale
     request releasing them would re-enable mid-flight of a newer search. */
  const finishFlight = () => {
    if (state.flight && state.flight.request === request) {
      state.flight = null;
      if (go) go.disabled = false;
    }
  };

  resetGallery();
  showNotice(null);
  showEmpty(null);
  setStage('1 · Choose the game');
  list.hidden = false;
  list.replaceChildren();
  list.setAttribute('aria-busy', 'true');
  const back = el('btn-online-art-back');
  if (back) back.hidden = true;

  const loading = document.createElement('div');
  loading.className = 'online-art-loading';
  loading.setAttribute('role', 'status');
  loading.textContent = 'Searching…';
  list.append(loading);

  let games;
  try {
    games = searchCache.get(query);
    if (!games) {
      games = await window.vault.sgdbSearchGames(query);
      if (Array.isArray(games)) searchCache.set(query, games);
    }
  } catch (error) {
    if (!isCurrent()) return;
    list.removeAttribute('aria-busy');
    finishFlight();
    list.replaceChildren();
    showNotice(errorMessage(error), KEY_HINT);
    return;
  }
  if (!isCurrent()) return;
  list.removeAttribute('aria-busy');
  finishFlight();

  if (!Array.isArray(games) || games.length === 0) {
    list.replaceChildren();
    list.hidden = true;
    showEmpty('No matching games found.', 'Edit your query and search again.');
    return;
  }

  renderGames(games, query);
}

/* ── Stage B — gallery ──────────────────────────────────────────────────────*/
/* Pages the whole catalogue: no cap, no resolution filtering, no dropping. */
async function loadAllArts(gameId, kind, request) {
  const cacheKey = `${gameId}:${kind}`;
  if (artCache.has(cacheKey)) return artCache.get(cacheKey);
  const arts = [];
  for (let page = 0; page < PAGE_CAP; page += 1) {
    const result = await window.vault.sgdbArt(gameId, kind, page, PAGE_LIMIT);
    if (request !== state.request) return arts;

    const batch = Array.isArray(result && result.arts) ? result.arts : [];
    arts.push(...batch);
    setStage(`Loading ${kind}… ${arts.length}`);

    const total = Number(result && result.total) || 0;
    if (batch.length < PAGE_LIMIT) break;
    if (total && page + 1 >= Math.ceil(total / PAGE_LIMIT)) break;
  }
  if (request === state.request) artCache.set(cacheKey, arts);
  return arts;
}

function renderPreview(art) {
  const preview = el('art-preview');
  const img = el('art-preview-img');
  const meta = el('art-preview-meta');
  const note = el('art-preview-note');
  if (!preview || !img || !meta || !note) return;

  if (state.mode === 'logo' && art.animated) img.removeAttribute('src');
  else img.src = art.url || art.thumb;
  img.onerror = () => {
    if (art.thumb && img.src !== art.thumb) img.src = art.thumb;
    else {
      img.hidden = true;
      showNotice('This image could not be loaded.', 'Try another result or check the connection.');
    }
  };
  img.hidden = false;

  const parts = [];
  parts.push(art.width && art.height ? `${art.width} × ${art.height}` : 'Size unknown');
  if (art.author) parts.push(String(art.author));
  if (art.style) parts.push(String(art.style));
  meta.replaceChildren(...parts.map((part) => {
    const span = document.createElement('span');
    span.className = 'art-preview-part';
    span.textContent = part;
    return span;
  }));

  if (state.mode === 'wallpaper') {
    const described = describeWallpaper(art.width, art.height);
    note.textContent = described.note;
    note.classList.toggle('is-caution', described.tier >= 2);
  } else {
    note.textContent = art.animated ? 'Animated WebP logo.' : '';
    note.classList.remove('is-caution');
  }
  note.hidden = !note.textContent;
  preview.hidden = false;
}

function selectCard(index) {
  const art = state.arts[index];
  if (!art) return;
  showNotice(null);
  state.selected = index;

  const gallery = el('online-art-gallery');
  if (gallery) {
    gallery.querySelectorAll('.art-card').forEach((card) => {
      const selected = Number(card.dataset.index) === index;
      card.classList.toggle('selected', selected);
      card.setAttribute('aria-pressed', selected ? 'true' : 'false');
    });
  }

  renderPreview(art);
  setApplyEnabled(true);
}

function moveSelection(delta) {
  if (!state.arts.length) return;
  let next = state.selected < 0 ? 0 : state.selected + delta;
  if (next < 0) next = 0;
  if (next >= state.arts.length) next = state.arts.length - 1;

  // Walking past the rendered batch extends it rather than stopping dead.
  let guard = 0;
  while (next >= state.shown && state.shown < state.arts.length && guard < PAGE_CAP) {
    renderBatch();
    guard += 1;
  }
  if (next >= state.shown) next = state.shown - 1;
  if (next < 0) return;

  selectCard(next);
  const card = el('online-art-gallery')?.querySelector(`[data-index="${next}"]`);
  if (card) {
    card.focus({ preventScroll: true });
    card.scrollIntoView({ block: 'nearest' });
  }
}

function renderBatch() {
  const gallery = el('online-art-gallery');
  if (!gallery) return;
  const end = Math.min(state.shown + BATCH_SIZE, state.arts.length);
  if (end <= state.shown) return;

  const fragment = document.createDocumentFragment();
  for (let index = state.shown; index < end; index += 1) {
    fragment.append(buildCard(state.arts[index], index));
  }
  gallery.append(fragment);
  state.shown = end;

  const count = el('online-art-count');
  if (count) count.textContent = `${state.shown} of ${state.arts.length} shown`;

  const pagination = el('online-art-pagination');
  if (pagination) pagination.hidden = false;
  const showMore = el('art-show-more');
  if (showMore) showMore.hidden = state.shown >= state.arts.length;
}

async function openGallery(game) {
  const list = el('online-art-list');
  const gallery = el('online-art-gallery');
  if (!list || !gallery) return;

  state.request += 1;
  const request = state.request;
  state.game = game;
  state.arts = [];
  state.shown = 0;
  state.selected = -1;

  list.hidden = true;
  gallery.hidden = false;
  gallery.replaceChildren();
  const pagination = el('online-art-pagination');
  if (pagination) pagination.hidden = true;
  const showMore = el('art-show-more');
  if (showMore) showMore.hidden = true;
  const back = el('btn-online-art-back');
  if (back) back.hidden = false;
  clearPreview();
  setApplyEnabled(false);
  showNotice(null);
  showEmpty(null);

  const kind = KIND[state.mode];
  setStage(`Loading ${kind}… 0`);

  let arts;
  try {
    arts = await loadAllArts(game.id, kind, request);
  } catch (error) {
    if (request !== state.request) return;
    gallery.hidden = true;
    setStage(galleryStepTitle());
    showNotice(errorMessage(error), KEY_HINT);
    return;
  }
  if (request !== state.request) return;

  state.arts = kind === 'heroes' ? sortWallpapers(arts) : arts;
  setStage(galleryStepTitle());

  if (!state.arts.length) {
    gallery.hidden = true;
    showEmpty(`No ${state.mode === 'logo' ? 'logos' : 'wallpapers'} found for ${game.name}.`);
    return;
  }

  renderBatch();
}

function applySelection() {
  const art = state.arts[state.selected];
  if (!art) return;
  const apply = state.apply;
  if (apply) apply(art.url || art.thumb, art);
  closeArtPicker();
}

/* ── Open / close ───────────────────────────────────────────────────────────*/
export function openArtPicker(mode, options = {}) {
  state.mode = mode === 'logo' ? 'logo' : 'wallpaper';
  state.apply = typeof options.apply === 'function' ? options.apply : null;

  /* Remember the trigger so focus can be returned on close (better-accessibility focus restoration). */
  pickerTrigger = document.activeElement;

  showStageA();   // clears flight state and re-arms the Search button
  el('online-art-list')?.replaceChildren();

  const sub = el('online-art-sub');
  if (sub) {
    sub.textContent = state.mode === 'logo'
      ? 'Search a game, then pick its logo.'
      : 'Search a game, then pick its wallpaper.';
  }
  const browse = el('btn-art-browse');
  if (browse) browse.hidden = state.mode !== 'logo';
  const applyButton = el('btn-art-apply');
  if (applyButton) applyButton.textContent = state.mode === 'logo' ? 'Use this logo' : 'Use this wallpaper';

  updateMonitorLine();

  const search = el('online-art-search');
  if (search) {
    search.value = el('input-name')?.value.trim() || '';
  }

  overlayEl()?.classList.remove('hidden');

  if (search) {
    search.focus();
    if (search.value.length >= 2) runSearch();
  }
}

export function closeArtPicker() {
  state.request += 1;   // cancels any in-flight search or gallery load
  state.flight = null;  // the closed picker does not owe a dedupe decision
  resetGallery();
  overlayEl()?.classList.add('hidden');
  pickerTrigger?.focus?.();
  pickerTrigger = null;
}

/* ── Wiring ─────────────────────────────────────────────────────────────────*/
function onOverlayKeydown(event) {
  if (event.key === 'Escape') {
    event.preventDefault();
    // The page-level Escape handler must not also treat this as "leave the page".
    event.stopPropagation();
    closeArtPicker();
    return;
  }

  const gallery = el('online-art-gallery');
  if (!gallery || gallery.hidden) return;
  if (document.activeElement === el('online-art-search')) return;

  if (event.key === 'ArrowRight') { event.preventDefault(); moveSelection(1); return; }
  if (event.key === 'ArrowLeft') { event.preventDefault(); moveSelection(-1); return; }
  if (event.key === 'ArrowDown') { event.preventDefault(); moveSelection(3); return; }
  if (event.key === 'ArrowUp') { event.preventDefault(); moveSelection(-3); return; }

  if (event.key === 'Enter') {
    const active = document.activeElement;
    const onButton = !!active && active.tagName === 'BUTTON';
    // Enter on a card selects it (native click); Enter elsewhere applies.
    if (onButton && !active.classList.contains('art-card')) return;
    const applyButton = el('btn-art-apply');
    if (!applyButton || applyButton.disabled) return;
    event.preventDefault();
    applySelection();
  }
}

export function initArtPicker() {
  if (wired) return;
  const overlay = overlayEl();
  if (!overlay) return;
  wired = true;

  el('btn-online-art-go')?.addEventListener('click', () => runSearch());
  el('online-art-search')?.addEventListener('keydown', (event) => {
    if (event.key === 'Enter') {
      event.preventDefault();
      runSearch();
    }
  });
  el('art-show-more')?.addEventListener('click', () => renderBatch());
  el('btn-online-art-back')?.addEventListener('click', () => showStageA());
  el('btn-art-apply')?.addEventListener('click', () => applySelection());
  el('btn-art-browse')?.addEventListener('click', () => {
    // The add/edit form already owns local .gif/.webp/.png picking.
    el('btn-pick-logo')?.click();
  });

  el('online-art-gallery')?.addEventListener('click', (event) => {
    const card = event.target.closest && event.target.closest('.art-card');
    if (!card) return;
    selectCard(Number(card.dataset.index));
  });

  overlay.addEventListener('keydown', onOverlayKeydown);

  onDisplayChange(() => {
    if (!overlay.classList.contains('hidden')) updateMonitorLine();
  });
}
