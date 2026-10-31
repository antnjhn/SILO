/* ── library.js — the rail, categories, game details, launch + install state ─ */
import {
  allGames, games, selectedIndex, searchQuery,
  setAllGames, setGames, setSelectedIndex, setSearchQuery,
} from './state.js';
import { showToast, esc, escJs, fmtTime, fmtDate } from './utils.js';
import { getFileSrc } from './api.js';
import { onPageEnter, onPageLeave } from './pages.js';
import { hideStatPopoverNow, refreshStatPopover } from './stats.js';
import { vibrate, vibrateVertical } from './gamepad.js';
import { openEditModal } from './modals.js';

/* Rail / list state owned here but read across modules. Variables read by other
   modules are exported as live bindings; the rest stay file-local. */
export let detailsOpen = false;
export let categoriesOpen = false;
export let categorySelectedIndex = 0;
let currentCategory = 'ALL GAMES';   // Library filter selected by the navigation menu
let installationFilter = 'all';     // 'all' | 'installed' | 'uninstalled'
let selectedTags = new Set();
let filterPanelOpen = false;
let sortMode = 'name';               // 'name' | 'playtime' | 'recent'
let lastHeroRenderedId = null;

// Wallpaper crossfade state
let currentBgLayer = 'bg-wallpaper-1';
let currentWallpaperUrl = '';
let wallpaperGeneration = 0;

// Item height for centering (logo size + gap)
const ITEM_SIZE = 140;
const ITEM_GAP = 14;
const ITEM_STEP = ITEM_SIZE + ITEM_GAP;

/* ── Rail carousel — config ────────────────────────────────────────────────
   The rail is a vertical stack of logos that share one alignment line. Every
   logo's look is derived from its distance to the focused index:

       distance = itemIndex - selectedIndex

   Distance 0 is the focused logo: it detaches from the line and grows.
   Distances ≥ 1 recede — smaller, fainter, softly blurred — the further out
   they sit; anything past the last entry reuses it. Everything is tuneable
   from here: the numbers are published as CSS custom properties and the
   geometry is recomputed from the window size on start-up and on resize. */
const RAIL = {
  MS: 560,                             // duration of one focus transition
  FADE_MS: 260,                        // how fast the dim/blur settles (see below)
  // Ease-out-cubic: a smooth start and a firm settle. The previous curve
  // (ease-out-quint) left the line at full speed and spent most of its time
  // crawling, which read as a snap followed by a drift.
  EASE: 'cubic-bezier(0.215, 0.61, 0.355, 1)',
  LINE_X: 64,                          // px from the sidebar edge to the line
  GAP: 16,                             // gap between the line and a resting logo
  PAD: 18,                             // breathing room right of the focused logo
  MIN_W: 236,                          // the rail never shrinks past this
  MAX_W: 400,                          // …nor grows past this
  WIDTH_RATIO: 0.32,                   // …nor past this share of the window
  FOCUS_SCALE: 2.0,                    // how much larger the focused logo gets
  FOCUS_OFFSET: 44,                    // px it slides off the line
  SCALE:   [1.00, 0.66, 0.48, 0.38],   // by distance 0…3+ (0 uses FOCUS_SCALE)
  OPACITY: [1.00, 0.42, 0.20, 0.07],
  BRIGHT:  [1.00, 0.72, 0.50, 0.34],
  BLUR:    [0.00, 0.60, 1.60, 2.80],
  Z:       [40, 20, 10, 5],
};

/* Rail view, two levels: browsing the rail attaches the focused game's stats to
   its logo; stepping INTO the game adds the action buttons. Those two timings
   live here. */
const RAIL_FX = {
  BUTTONS_MS: 240,      // buttons row reveal duration
  EXIT_MS: 180,         // fade-out duration when stepping back out
};

export let railEntered = false;   // the focused game has been stepped into (buttons out)

/* Room the focused logo needs, derived from the window instead of hard-coded:
   line + gap + offset + ITEM_SIZE × scale + padding. The offset is reserved
   first and the scale takes what is left, so the carousel stays consistent
   across window sizes. */
const railGeo = { scale: RAIL.FOCUS_SCALE, offset: RAIL.FOCUS_OFFSET, width: 378 };
const reduceMotion = window.matchMedia?.('(prefers-reduced-motion: reduce)');

/* The OS reduce-motion setting is respected by default, but it is not the last
   word: 'Always animate' in Settings overrides it for the launcher. Stored
   locally so it survives restarts without a backend round-trip. */
const MOTION_PREF_KEY = 'silo.forceMotion';
export let forceMotion = false;
try { forceMotion = localStorage.getItem(MOTION_PREF_KEY) === '1'; } catch { /* private mode */ }
export const setForceMotion = (v) => { forceMotion = v; };

/* True when the launcher should drop its travel animations: the system asked
   for reduced motion and the override is off. */
function motionReduced() {
  return !!reduceMotion?.matches && !forceMotion;
}

/* Drops instant-mode on the next frame (so the re-attached state lands without
   interpolating), with a timer backstop: if that frame is throttled — minimised
   window, heavy load — the class would otherwise stick and silently kill every
   rail transition from then on. */
function releaseRailInstant() {
  requestAnimationFrame(() => document.body.classList.remove('rail-fx-instant'));
  setTimeout(() => document.body.classList.remove('rail-fx-instant'), 150);
}

export function syncMotionToggle() {
  const row = document.querySelector('label[for="input-force-motion"]');
  const cb = document.getElementById('input-force-motion');
  if (cb) cb.checked = forceMotion;
  row?.querySelector('.toggle-track')?.classList.toggle('on', forceMotion);
  const sub = row?.querySelector('.toggle-sub');
  if (sub) {
    sub.textContent = reduceMotion?.matches
      ? 'Your system asks for reduced motion, so the rail is standing still. Turn this on to animate fully anyway.'
      : 'Your system does not ask for reduced motion. Turn this on to keep animating even if it ever does.';
  }
}

/* Applies the override to the live UI: the body class lifts the CSS-side
   reduced-motion rules, layoutRail republishes the durations, and the setting
   is remembered. No restart, no flicker. */
export function applyMotionPref() {
  document.body.classList.toggle('force-motion', forceMotion);
  try { localStorage.setItem(MOTION_PREF_KEY, forceMotion ? '1' : '0'); } catch { /* ignore */ }
  syncMotionToggle();
  layoutRail();
}

export function layoutRail() {
  const maxW = Math.max(RAIL.MIN_W, Math.min(RAIL.MAX_W, window.innerWidth * RAIL.WIDTH_RATIO));
  const budget = Math.max(0, maxW - RAIL.LINE_X - RAIL.GAP - RAIL.PAD);
  const offset = Math.round(Math.min(RAIL.FOCUS_OFFSET, budget * 0.22));
  const scale = Math.min(RAIL.FOCUS_SCALE, Math.max(1.15, (budget - offset) / ITEM_SIZE));
  railGeo.offset = offset;
  railGeo.scale = scale;
  railGeo.width = RAIL.LINE_X + RAIL.GAP + offset + ITEM_SIZE * scale + RAIL.PAD;

  const s = document.documentElement.style;
  s.setProperty('--rail-w', Math.round(railGeo.width) + 'px');
  s.setProperty('--rail-line-x', RAIL.LINE_X + 'px');
  s.setProperty('--rail-gap', RAIL.GAP + 'px');
  s.setProperty('--rail-item', ITEM_SIZE + 'px');
  s.setProperty('--rail-item-gap', ITEM_GAP + 'px');
  // The rail's head (toolbar + filter bar) is where the focused logo's stack
  // starts; the details block reuses that offset so the stats and buttons sit
  // level with the logo instead of hanging off the bottom of the window.
  const panelEl = document.getElementById('game-list-panel');
  const viewportEl = document.getElementById('game-list-viewport');
  if (panelEl && viewportEl) {
    const head = viewportEl.getBoundingClientRect().top - panelEl.getBoundingClientRect().top;
    s.setProperty('--rail-head-h', Math.max(0, Math.round(head)) + 'px');
  }
  // Reduced motion keeps the hierarchy but lands on it immediately. The value
  // is published here (not in CSS) because the live config is an inline style.
  s.setProperty('--rail-ms', (motionReduced() ? 1 : RAIL.MS) + 'ms');
  s.setProperty('--rail-fade-ms', (motionReduced() ? 1 : RAIL.FADE_MS) + 'ms');
  s.setProperty('--rail-ease', RAIL.EASE);
  s.setProperty('--rail-buttons-ms', RAIL_FX.BUTTONS_MS + 'ms');
  s.setProperty('--rail-exit-ms', RAIL_FX.EXIT_MS + 'ms');
}

/* distance → visual state: the single place scale, opacity, offset, blur and
   stacking order come from. */
function railState(distance) {
  const d = Math.min(Math.abs(distance), RAIL.SCALE.length - 1);
  const focused = distance === 0;
  return {
    scale: focused ? railGeo.scale : RAIL.SCALE[d],
    x: focused ? railGeo.offset : 0,
    opacity: RAIL.OPACITY[d],
    brightness: RAIL.BRIGHT[d],
    blur: RAIL.BLUR[d],
    z: RAIL.Z[d],
  };
}

/* Push that state onto every logo. Each item only carries custom properties,
   so the browser interpolates them: a focus change sends the old logo back to
   the line while the new one moves out, in the same frame, and rapid input
   simply re-targets the transitions in flight — never a queued or stuck logo. */
function applyRailCarousel() {
  document.querySelectorAll('#game-list .game-item').forEach((el, i) => {
    const st = railState(i - selectedIndex);
    const s = el.style;
    s.setProperty('--rail-scale', String(st.scale));
    s.setProperty('--rail-x', st.x + 'px');
    s.setProperty('--rail-op', String(st.opacity));
    s.setProperty('--rail-bright', String(st.brightness));
    s.setProperty('--rail-blur', st.blur + 'px');
    s.zIndex = String(st.z);
  });
}

reduceMotion?.addEventListener?.('change', () => layoutRail());

/* Resizing changes the room the focused logo has: recompute the geometry and
   re-apply the rail in one frame, with transitions suppressed, then re-center
   the stack (the viewport height moved too). */
let railResizeRaf = 0;
window.addEventListener('resize', () => {
  if (railResizeRaf) return;
  railResizeRaf = requestAnimationFrame(() => {
    railResizeRaf = 0;
    layoutRail();
    layoutSearch();
    document.body.classList.add('rail-fx-instant');
    applyRailCarousel();
    centerActiveItem(false);
    void document.body.offsetHeight;
    releaseRailInstant();
  });
});

/* ── Render game list ─────────────────────────────────────────────────────*/
export function applyCategoryFilter(resetIndex = true) {
  const q = searchQuery;
  let filtered;
  if (currentCategory === 'ALL GAMES') {
    filtered = [...allGames];
  } else if (currentCategory === 'RECENTLY PLAYED') {
    filtered = [...allGames].sort((a, b) => {
      const aTime = a.lastPlayed ? new Date(a.lastPlayed).getTime() : 0;
      const bTime = b.lastPlayed ? new Date(b.lastPlayed).getTime() : 0;
      return bTime - aTime;
    }).slice(0, 4);
  } else if (currentCategory === 'FAVORITES') {
    filtered = allGames.filter(g => g.favorite);
  } else if (currentCategory !== 'ALL GAMES') {
    // Any other category is one of the library's own tags.
    filtered = allGames.filter(g => (g.tags || []).includes(currentCategory));
  } else {
    filtered = [...allGames];
  }

  // Installation status and tag selection refine All Games, and they combine
  // with each other. They stay out of the other views so a filtered library
  // never hides Favourites, Recently Played or a genre.
  if (currentCategory === 'ALL GAMES') {
    if (installationFilter !== 'all') {
      filtered = filtered.filter(g => {
        const status = installResults.get(g.id)?.status;
        if (!status) return false;
        return installationFilter === 'installed' ? status === 'installed' : status === 'missing';
      });
    }
    if (selectedTags.size) filtered = filtered.filter(g => (g.tags || []).some(t => selectedTags.has(t)));
  }
  if (q) filtered = filtered.filter(g => String(g.name || '').toLowerCase().includes(q));
  renderFilterPanel();
  const count = document.getElementById('library-result-count');
  if (count) count.textContent = `${filtered.length} ${filtered.length === 1 ? 'GAME' : 'GAMES'}`;
  const title = document.getElementById('library-view-title');
  if (title) title.textContent = currentCategory;
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

  setGames(filtered);

  if (resetIndex) {
    setSelectedIndex(0);
  } else {
    if (selectedIndex >= games.length) setSelectedIndex(Math.max(0, games.length - 1));
  }

  renderGameList();
  centerActiveItem(false);
  crossfadeWallpaper();

  // Keep the preview hero in sync with the newly focused game (category/search changes).
  if (!detailsOpen && games.length) {
    const cur = games[selectedIndex];
    if (cur && lastHeroRenderedId !== cur.id) {
      renderDetails(cur);
      retriggerHeroAnimation();
    }
  }
}

export function renderGameList() {
  const list = document.getElementById('game-list');
  const empty = document.getElementById('empty-state');

  empty.style.display = games.length === 0 ? 'flex' : 'none';

  list.innerHTML = games.map((g, i) => {
    const dist = Math.abs(i - selectedIndex);
    let cls = 'game-item';
    if (i === selectedIndex) cls += ' active';
    else if (dist === 1) cls += ' near';
    if (g.isInstalled === false) cls += ' uninstalled';

    // Sidebar is logos-only: no text titles, no tooltips. Games without a logo
    // asset fall back to the first letter styled with the game's font/color.
    // Install check: a missing game keeps its artwork, dimmed. No badge pill.
    if (installStatus(g.id) !== 'installed') cls += ' install-missing';

    return `
    <div class="${cls}" data-index="${i}" aria-label="${esc(g.name)}">
      ${g.logoPath
        ? `<img data-logo-path="${esc(g.logoPath)}" alt=""
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
    el.addEventListener('click', () => {
      const i = parseInt(el.dataset.index);
      if (i === selectedIndex) railEnterGame();   // clicking the focused logo steps in
      else selectGame(i);
    });
    el.addEventListener('dblclick', () => { selectGame(parseInt(el.dataset.index)); openDetails(); });
  });

  // The list was rebuilt: reconcile the carousel with no animation so the
  // focused logo takes its place in one frame.
  syncRailInstant();
}

/* The category view belongs to the library: it is put away with it, so it
   can never be left on screen over another page. */
onPageLeave('home', () => closeCategories(false));

/* Coming back to the library re-measures the rail. While another page was on
   screen the rail had no height, so neither the carousel nor the stack's
   position could be worked out; both are put back the moment the library
   returns, so the focused logo is on the centreline again. */
onPageEnter('home', () => {
  syncRailInstant();
  centerActiveItem(false);
});

export function getCategories() {
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

/* ── Category option wheel (LEFT from the rail / C / LB) ─────────────────

/* ── Category option wheel (LEFT from the rail / C / LB) ─────────────────
/* ═══════════════════════════════════════════════════════════════════════════
   CATEGORY SELECTION RAIL — Curved Navigation Rail

   The categories are arranged along a smooth, continuous mathematical arc
   with the selected category sitting at the apex / focal point.
   Surrounding categories follow that same curvature, progressively receding
   toward the LEFT as distance increases.

   Positions derive continuously from relative distance = i - pos:
     x = focusX - curveStrength * (|distance|^curveExponent)
     y = centerY + distance * verticalSpacing
   ═══════════════════════════════════════════════════════════════════════════ */
const CATEGORY_CURVE = {
  // Positioning geometry
  focusX: 88,              // horizontal X position of the focal apex (px, restrained & connected)
  focusY: 0,               // vertical anchor offset (px, relative to center)
  verticalSpacing: 74,     // px vertical distance per category row
  curveStrength: 46,       // horizontal leftward curvature displacement factor (px)
  curveExponent: 1.62,     // nonlinear exponent for the curved arc (smooth, continuous arc)
  downwardLean: 2,         // subtle directional bias to preserve natural left-leaning flow (px)
  leanDeg: 0,              // rail rotation (degrees, text remains horizontal)

  // Scale hierarchy (distance-based)
  focusedScale: 1.0,       // scale at distance 0 (selected/focal item)
  unfocusedScale: 0.78,   // scale at distance 1
  distanceScaleRate: 0.82, // scale decay per unit distance beyond 1
  minScale: 0.50,          // minimum scale floor

  // Opacity hierarchy (distance-based)
  focusedOpacity: 1.0,     // opacity at distance 0
  unfocusedOpacity: 0.74, // opacity at distance 1 — still clearly readable
  distanceOpacityRate: 0.68,// opacity decay per unit distance beyond 1
  minOpacity: 0.52,        // the furthest words still read against the art

  // Softening: the labels stay sharp. A blurred word cannot be read, so the
  // depth is carried by the scale and the opacity instead.
  adjacentBlur: 0,
  distanceBlurRate: 0,
  maxBlur: 0,

  // Visibility cutoff
  maxVisibleDistance: 4.2, // elements beyond this are hidden

  // Motion & Animation
  animationDuration: 480,  // transition duration in ms (400-600ms range)
  dragSnapDuration: 280,   // duration of drag release snap (ms)
  ease: (t) => (t < 0.5 ? 4 * t * t * t : 1 - Math.pow(-2 * t + 2, 3) / 2), // easeInOutCubic
};

// Aliases for compatibility
CATEGORY_CURVE.focusScale = CATEGORY_CURVE.focusedScale;
CATEGORY_CURVE.adjacentScale = CATEGORY_CURVE.unfocusedScale;
CATEGORY_CURVE.focusOpacity = CATEGORY_CURVE.focusedOpacity;
CATEGORY_CURVE.adjacentOpacity = CATEGORY_CURVE.unfocusedOpacity;

let wheelItems = [];
let wheelDrag = null;   // { startY, startOffset, moved }
let wheelAnim = null;   // rAF handle for focus-travel animation
let wheelPos = 0;       // continuous focus position (index units)

function wheelCountFor(cat) {
  if (cat === 'ALL GAMES') return allGames.length;
  if (cat === 'RECENTLY PLAYED') return Math.min(4, allGames.length);
  if (cat === 'FAVORITES') return allGames.filter(g => g.favorite).length;
  return allGames.filter(g => (g.tags || []).includes(cat)).length;
}

function buildWheel() {
  const list = document.getElementById('wheel-list');
  if (!list) return;
  const cats = getCategories();
  list.innerHTML = cats.map((c, i) =>
    `<div class="wheel-item" data-index="${i}"><span class="wheel-label">${esc(c)}</span><span class="wheel-count">${wheelCountFor(c)}</span></div>`
  ).join('');
  wheelItems = [...list.querySelectorAll('.wheel-item')];

  wheelItems.forEach((el, i) => {
    el.addEventListener('click', (e) => {
      e.stopPropagation();
      if (categorySelectedIndex === i) {
        closeCategories(true);
      } else {
        categorySelectedIndex = i;
        wheelTravelTo(i, CATEGORY_CURVE.animationDuration);
        document.querySelectorAll('.category-dot').forEach((d, idx) => {
          d.classList.toggle('active', idx === categorySelectedIndex);
        });
      }
    });
  });
}

/* Calculate horizontal position x along the curved, left-leaning rail.
   The selected category is the focal/rightmost point (apex at focusX).
   Categories farther from the selected category progressively curve toward the LEFT. */
function calculateCurveX(rel, boundsWidth = 480) {
  const ard = Math.abs(rel);
  // Nonlinear arc displacement: apex is at focusX (rel = 0).
  // All other items curve away to the LEFT (xOffset < 0).
  const nonLinearDist = Math.pow(ard, CATEGORY_CURVE.curveExponent);
  let xOffset = -CATEGORY_CURVE.curveStrength * nonLinearDist;

  // Subtle downward lean for natural left-leaning flow
  if (rel > 0) {
    xOffset -= CATEGORY_CURVE.downwardLean * rel;
  }

  // Responsive container scaling
  const widthFactor = Math.min(1.2, Math.max(0.8, boundsWidth / 480));
  return (CATEGORY_CURVE.focusX + xOffset) * widthFactor;
}

/* Single source of truth for visual positioning.
   pos = continuous focus position (index units); every visual property
   derives mathematically from rel = i - pos. */
function renderWheel(pos) {
  const list = document.getElementById('wheel-list');
  if (!list) return;
  const H = list.clientHeight || 380;
  const W = list.clientWidth || 480;

  // Responsive spacing: scales slightly on constrained viewport heights
  const spacingFactor = Math.min(1, Math.max(0.85, H / 380));
  const spacing = CATEGORY_CURVE.verticalSpacing * spacingFactor;
  const centerY = H / 2 - (CATEGORY_CURVE.verticalSpacing * spacingFactor) / 2;

  wheelItems.forEach((el, i) => {
    const rel = i - pos;                     // signed continuous distance from focus
    const ard = Math.abs(rel);               // absolute distance

    if (ard > CATEGORY_CURVE.maxVisibleDistance) {
      el.style.visibility = 'hidden';
      return;
    }
    el.style.visibility = 'visible';

    // Position along the curved, left-leaning rail: selected is apex / rightmost
    const x = calculateCurveX(rel, W);
    const y = centerY + rel * spacing;

    // Distance-based progressive scale: apex is largest (1.0)
    let s;
    if (ard <= 1) {
      s = CATEGORY_CURVE.focusScale - ard * (CATEGORY_CURVE.focusScale - CATEGORY_CURVE.adjacentScale);
    } else {
      s = Math.max(CATEGORY_CURVE.minScale, CATEGORY_CURVE.adjacentScale * Math.pow(CATEGORY_CURVE.distanceScaleRate, ard - 1));
    }

    // Distance-based progressive opacity: apex is brightest (1.0)
    let op;
    if (ard <= 1) {
      op = CATEGORY_CURVE.focusOpacity - ard * (CATEGORY_CURVE.focusOpacity - CATEGORY_CURVE.adjacentOpacity);
    } else {
      op = Math.max(CATEGORY_CURVE.minOpacity, CATEGORY_CURVE.adjacentOpacity * Math.pow(CATEGORY_CURVE.distanceOpacityRate, ard - 1));
    }

    // Distance-based progressive softening (blur): apex is sharpest (0px)
    let blur = 0;
    if (ard <= 1) {
      blur = ard * CATEGORY_CURVE.adjacentBlur;
    } else {
      blur = Math.min(CATEGORY_CURVE.maxBlur, CATEGORY_CURVE.adjacentBlur + (ard - 1) * CATEGORY_CURVE.distanceBlurRate);
    }

    el.style.transform = `translate3d(${x.toFixed(1)}px, ${y.toFixed(1)}px, 0) scale(${s.toFixed(3)})`;
    el.style.transformOrigin = 'left center';
    el.style.opacity = op.toFixed(3);
    el.style.filter = blur > 0.05 ? `blur(${blur.toFixed(2)}px)` : 'none';
    el.style.zIndex = String(100 - Math.round(ard * 10));

    // Crisp high-contrast styling for the focused category
    const isFocused = ard < 0.45;
    el.classList.toggle('focused', isFocused);
  });
}


/* Smoothly travel focus along the curved rail to a target index.
   Interruptible: rapid inputs smoothly retarget from current continuous wheelPos. */
function wheelTravelTo(target, duration = CATEGORY_CURVE.animationDuration) {
  cancelAnimationFrame(wheelAnim);
  const numCats = getCategories().length;
  target = Math.max(0, Math.min(numCats - 1, target));
  const startPos = wheelPos;
  const delta = target - startPos;
  if (Math.abs(delta) < 0.001) {
    wheelPos = target;
    renderWheel(wheelPos);
    return;
  }
  const dur = Math.min(
    CATEGORY_CURVE.animationDuration * 1.25,
    Math.max(160, duration * Math.min(1.2, Math.abs(delta)))
  );
  const t0 = performance.now();
  const step = (now) => {
    const t = Math.min(1, (now - t0) / dur);
    wheelPos = startPos + delta * CATEGORY_CURVE.ease(t);
    renderWheel(wheelPos);
    if (t < 1) {
      wheelAnim = requestAnimationFrame(step);
    } else {
      wheelPos = target;
      renderWheel(wheelPos);
    }
  };
  wheelAnim = requestAnimationFrame(step);
}

export function openCategories() {
  if (detailsOpen || (!games.length && allGames.length === 0)) return;
  categoriesOpen = true;
  // The library steps out of the foreground while the categories are up, so
  // the labels are not read against the previous screen's logo and metadata.
  document.body.classList.add('categories-open');
  // The statistics preview belongs to the library, not to the categories: it
  // steps aside instead of floating over the rail.
  hideStatPopoverNow();
  buildWheel();
  const overlay = document.getElementById('category-wheel-overlay');
  const cats = getCategories();
  categorySelectedIndex = Math.max(0, cats.indexOf(currentCategory));
  wheelPos = categorySelectedIndex;
  renderWheel(wheelPos);
  overlay.classList.remove('hidden');
  requestAnimationFrame(() => overlay.classList.add('open'));
  vibrate(true); // Left side
}

export function closeCategories(apply = false) {
  if (!categoriesOpen) return;
  categoriesOpen = false;
  document.body.classList.remove('categories-open');
  cancelAnimationFrame(wheelAnim);
  const overlay = document.getElementById('category-wheel-overlay');
  overlay.classList.remove('open');
  const overlay2 = overlay;
  setTimeout(() => { if (!categoriesOpen) overlay2.classList.add('hidden'); }, 240);
  if (apply) {
    // Clamped, never wrapped — the rail has hard ends.
    const idx = Math.max(0, Math.min(getCategories().length - 1, Math.round(wheelPos)));
    const chosen = getCategories()[idx];
    categorySelectedIndex = idx;
    if (currentCategory !== chosen) {
      currentCategory = chosen;
      applyCategoryFilter();
    }
  }
  vibrate(false); // Right side
}

/* One source of truth for the selected index — keyboard, controller and
   wheel input all land here.
   IMPORTANT BOUNDARY BEHAVIOR:
   If at index 0 and moving UP: DO NOTHING (no animation, no state change, no wrap).
   If at last index and moving DOWN: DO NOTHING (no animation, no state change, no wrap). */
export function changeCategorySelection(delta) {
  const numCats = getCategories().length;
  if (numCats <= 1) return;
  if (delta < 0 && categorySelectedIndex <= 0) return;             // boundary — ignore
  if (delta > 0 && categorySelectedIndex >= numCats - 1) return;    // boundary — ignore

  const next = categorySelectedIndex + delta;
  if (next < 0 || next > numCats - 1) return;

  categorySelectedIndex = next;
  wheelTravelTo(next);
  document.querySelectorAll('.category-dot').forEach((el, i) => {
    el.classList.toggle('active', i === categorySelectedIndex);
  });
}

// Drag / scroll interaction on the wheel list.
(() => {
  const list = document.getElementById('wheel-list');
  if (!list) return;
  list.addEventListener('pointerdown', (e) => {
    if (!categoriesOpen) return;
    if (e.target.closest('.wheel-item')) return; // allow item click handlers
    e.preventDefault();
    cancelAnimationFrame(wheelAnim);
    wheelDrag = { startY: e.clientY, startOffset: wheelPos, moved: false };
    list.setPointerCapture(e.pointerId);
  });
  list.addEventListener('pointermove', (e) => {
    if (!wheelDrag) return;
    const dy = e.clientY - wheelDrag.startY;
    if (Math.abs(dy) > 4) wheelDrag.moved = true;
    // Drag clamped to rail ends — resistance, no wrap, no overscroll.
    const numCats = getCategories().length;
    const raw = wheelDrag.startOffset - dy / CATEGORY_CURVE.verticalSpacing;
    wheelPos = Math.max(0, Math.min(numCats - 1, raw));
    renderWheel(wheelPos);
  });
  const end = () => {
    if (!wheelDrag) return;
    wheelDrag = null;
    const numCats = getCategories().length;
    const snapped = Math.max(0, Math.min(numCats - 1, Math.round(wheelPos)));
    categorySelectedIndex = snapped;
    wheelTravelTo(snapped, CATEGORY_CURVE.dragSnapDuration);
    document.querySelectorAll('.category-dot').forEach((el, i) => {
      el.classList.toggle('active', i === categorySelectedIndex);
    });
  };
  list.addEventListener('pointerup', end);
  list.addEventListener('pointercancel', end);
  list.addEventListener('wheel', (e) => {
    if (!categoriesOpen) return;
    e.preventDefault();
    if (Math.abs(e.deltaY) < 10) return; // ignore sub-pixel trackpad jitter
    changeCategorySelection(e.deltaY > 0 ? 1 : -1);   // boundary-aware, no wrap
  }, { passive: false });
  // Click outside the wheel cancels.
  document.getElementById('category-wheel-overlay')?.addEventListener('pointerdown', (e) => {
    if (e.target.classList.contains('wheel-backdrop')) closeCategories(false);
  });
})();

/* ── Library filters ───────────────────────────────────────────────────────
   The navigation menu drives these directly: ALL GAMES, RECENTLY PLAYED,
   FAVORITES and a single GENRE. The installation-status and tag filters
   refine All Games and combine with each other. */
function getGenres() {
  const seen = new Set();
  allGames.forEach((g) => (Array.isArray(g.tags) ? g.tags : []).forEach((t) => {
    const name = String(t || '').trim();
    if (name) seen.add(name);
  }));
  return [...seen].sort((a, b) => a.localeCompare(b));
}

/* The filter panel is contextual: it opens on demand and holds the
   installation-status choice and the library's real tags together. */
function setFilterPanelOpen(open) {
  filterPanelOpen = Boolean(open);
  renderFilterPanel();
  if (filterPanelOpen) {
    const first = document.querySelector('#filter-panel .persona-segments button.active')
      || document.querySelector('#filter-panel .tag-filter')
      || document.getElementById('btn-clear-filters');
    first?.focus();
  } else {
    document.getElementById('btn-filter-toggle')?.focus();
  }
}

export function toggleFilterPanel(forceOpen) {
  setFilterPanelOpen(typeof forceOpen === 'boolean' ? forceOpen : !filterPanelOpen);
}

export function isFilterPanelOpen() {
  return filterPanelOpen;
}

/* Clears every active filter: the installation status and every tag. */
export function clearAllFilters() {
  installationFilter = 'all';
  selectedTags.clear();
  applyCategoryFilter();
}

/* Renders the contextual panel: the real tags, the installation choice and
   the state of both. */
function renderFilterPanel() {
  const options = document.getElementById('tag-filter-options');
  const panel = document.getElementById('filter-panel');
  const count = document.getElementById('tag-filter-count');
  if (!options || !panel) return;
  const tags = getGenres();
  options.innerHTML = tags.length
    ? tags.map((tag) =>
      `<button type="button" class="tag-filter${selectedTags.has(tag) ? ' active' : ''}" aria-pressed="${selectedTags.has(tag)}" data-tag="${esc(tag)}">${esc(tag)}</button>`
    ).join('')
    : '<span class="filter-empty">NO TAGS IN YOUR LIBRARY</span>';
  options.querySelectorAll('.tag-filter').forEach((el) => el.addEventListener('click', () => {
    const tag = el.dataset.tag;
    if (selectedTags.has(tag)) selectedTags.delete(tag); else selectedTags.add(tag);
    // The list is rebuilt, so focus is put back on the same tag afterwards —
    // otherwise the keyboard focus is lost on every toggle.
    const keepFocus = document.activeElement === el;
    applyCategoryFilter();
    if (keepFocus) {
      const again = [...document.querySelectorAll('#tag-filter-options .tag-filter')]
        .find((t) => t.dataset.tag === tag);
      again?.focus();
    }
  }));
  panel.onpointerdown = (event) => event.stopPropagation();
  if (count) count.textContent = String(selectedTags.size);
  const installControls = document.getElementById('install-filter');
  if (installControls) {
    installControls.hidden = currentCategory !== 'ALL GAMES';
    installControls.querySelectorAll('[data-install-filter]').forEach((el) => {
      const active = el.dataset.installFilter === installationFilter;
      el.classList.toggle('active', active);
      el.setAttribute('aria-pressed', String(active));
    });
  }
  const hint = document.getElementById('filter-hint');
  if (hint) {
    hint.textContent = currentCategory === 'ALL GAMES'
      ? 'Multiple tags match ANY selected tag.'
      : 'Filters apply to ALL GAMES.';
  }
  panel.classList.toggle('open', filterPanelOpen);
  panel.classList.toggle('hidden', !filterPanelOpen);
  const toggle = document.getElementById('btn-filter-toggle');
  if (toggle) toggle.setAttribute('aria-expanded', String(filterPanelOpen));
  renderActiveFilters();
}

/* The active filters are shown as small chips in the bar, so the state is
   visible while the panel is closed and each one can be removed on its own. */
function renderActiveFilters() {
  const holder = document.getElementById('active-filters');
  if (!holder) return;
  const chips = [];
  if (installationFilter !== 'all') {
    chips.push(`<button type="button" class="filter-chip" data-clear-install="1">${esc(installationFilter.toUpperCase())}<span aria-hidden="true">&times;</span></button>`);
  }
  selectedTags.forEach((tag) => {
    chips.push(`<button type="button" class="filter-chip" data-clear-tag="${esc(tag)}">${esc(tag)}<span aria-hidden="true">&times;</span></button>`);
  });
  holder.innerHTML = chips.join('');
  holder.hidden = chips.length === 0;
  holder.querySelectorAll('[data-clear-install]').forEach((el) => el.addEventListener('click', () => {
    installationFilter = 'all';
    applyCategoryFilter();
  }));
  holder.querySelectorAll('[data-clear-tag]').forEach((el) => el.addEventListener('click', () => {
    selectedTags.delete(el.dataset.clearTag);
    applyCategoryFilter();
  }));

  // The toolbar badge counts every active filter, so the state is legible
  // without opening the panel.
  const badge = document.getElementById('filter-badge');
  if (badge) {
    const active = chips.length;
    badge.textContent = String(active);
    badge.hidden = active === 0;
  }
}

/* Returns the library to its main view: All Games, with the preview panel
   showing the selected game. The navigation's HOME item uses it. */
export function showAllGames() {
  if (detailsOpen) closeDetails();
  currentCategory = 'ALL GAMES';
  applyCategoryFilter();
}

export function getSelectedTags() {
  return [...selectedTags];
}

export function getInstallationFilter() {
  return installationFilter;
}

/* The category dots are replaced by the navigation menu, so the old dot
   strip is no longer rebuilt. */

// Installation-status filter: ALL / INSTALLED / UNINSTALLED, combinable with
// the tag filter. Arrow keys move between the segments.
document.getElementById('install-filter')?.addEventListener('keydown', (event) => {
  if (!['ArrowLeft', 'ArrowRight', 'Home', 'End'].includes(event.key)) return;
  const buttons = [...document.querySelectorAll('#install-filter [data-install-filter]')];
  const index = buttons.indexOf(document.activeElement);
  if (index < 0) return;
  event.preventDefault();
  const next = event.key === 'Home' ? 0 : event.key === 'End' ? buttons.length - 1
    : Math.max(0, Math.min(buttons.length - 1, index + (event.key === 'ArrowRight' ? 1 : -1)));
  buttons[next].focus();
  // The segments behave like a radio group: moving to one selects it, so the
  // list updates straight away.
  installationFilter = buttons[next].dataset.installFilter;
  applyCategoryFilter();
});
document.getElementById('install-filter')?.addEventListener('click', (event) => {
  const button = event.target.closest('[data-install-filter]');
  if (!button) return;
  installationFilter = button.dataset.installFilter;
  applyCategoryFilter();
});
document.getElementById('btn-filter-toggle')?.addEventListener('click', () => toggleFilterPanel());
document.getElementById('btn-clear-filters')?.addEventListener('click', () => {
  clearAllFilters();
});
// Clicking outside the panel closes it, the way a popover should behave.
document.addEventListener('pointerdown', (event) => {
  if (!filterPanelOpen) return;
  const panel = document.getElementById('filter-panel');
  const toggle = document.getElementById('btn-filter-toggle');
  if (panel?.contains(event.target) || toggle?.contains(event.target)) return;
  setFilterPanelOpen(false);
});

// Search & sort controls in the sidebar
document.getElementById('search-input')?.addEventListener('input', (e) => {
  setSearchQuery(e.target.value.trim().toLowerCase());
  applyCategoryFilter();
});
document.getElementById('sort-select')?.addEventListener('change', (e) => {
  sortMode = e.target.value;
  applyCategoryFilter();
});

// The search field lives in the top navigation, so opening a search brings
// the navigation down with it and reveals the field.
let isSearchOpen = false;

export function toggleSearch(forceOpen) {
  const panel = document.getElementById('game-list-panel');
  const nav = document.getElementById('topnav');
  // The search field is not part of the visible interface in this pass, so
  // there is nothing to open. The filtering machinery stays in place for
  // when the control comes back.
  const field = document.getElementById('search-input');
  if (!field || field.hidden) return;
  const next = typeof forceOpen === 'boolean' ? forceOpen : !isSearchOpen;
  isSearchOpen = next;
  panel.classList.toggle('search-mode', next);
  nav?.classList.toggle('searching', next);
  if (next) {
    // The bar has to be on screen for the field to be usable.
    document.dispatchEvent(new Event('silo:reveal-nav'));
    requestAnimationFrame(() => document.getElementById('search-input')?.focus());
  } else {
    const inp = document.getElementById('search-input');
    if (inp?.value) { inp.value = ''; setSearchQuery(''); applyCategoryFilter(); }
    inp?.blur();
  }
}

/* Align the sort dropdown with the search field: measure the real input and
   publish its left edge and width so #filter-bar sits exactly under it at every
   window size. CSS alone cannot know where the flex row puts the input, and a
   guessed width is what used to clip the control at the panel edge. */
function layoutSearch() {
  // The search field and the sort control now share the navigation's
  // right-hand group, so there is no separate row to align. This stays as the
  // one place that re-measures the rail when the navigation changes height.
  layoutRail();
}

document.getElementById('btn-search-toggle')?.addEventListener('click', () => toggleSearch());

/* ── Center active item in viewport ───────────────────────────────────────*/
export function centerActiveItem(animate = true) {
  const viewport = document.getElementById('game-list-viewport');
  const list = document.getElementById('game-list');
  if (!viewport || !list || !games.length) return;

  const viewportH = viewport.clientHeight;
  // The rail cannot be measured while its page is hidden: the viewport has no
  // height then, and the stack would be moved off the top of the screen. The
  // last good position is kept, and the rail is re-centred when the page
  // comes back (see the home page entry below).
  if (!viewportH) return;
  const centerY = viewportH / 2;
  // Position of active item center relative to list top
  const itemCenterY = selectedIndex * ITEM_STEP + ITEM_SIZE / 2;
  const translateY = centerY - itemCenterY;
  // In focused mode the focused logo parks to the right edge of the sidebar:
  // handled here because the inline transform overrides any CSS-only rule.
  const focused = document.getElementById('game-list-panel').classList.contains('focused');
  const translateX = focused ? 60 : 0;

  if (!animate) list.style.transition = 'none';
  list.style.transform = `translateY(${translateY}px) translateX(${translateX}px)`;
  if (!animate) {
    // Force reflow then re-enable transitions
    list.offsetHeight;
    list.style.transition = '';
  }
  // Per-logo scale/offset/fade is not set here: the rail carousel derives all
  // of it from selectedIndex (applyRailCarousel), so scroll position and
  // visual focus stay independent.
}

/* ── Rail view: browsing vs stepped in ────────────────────────────────────
   Browsing the rail attaches the focused game's stats to its logo. Stepping
   into the game (RIGHT / click on the focused logo) brings the action buttons
   in beside them; moving to another game steps back out, so while you browse
   it is always stats only. */
function railSetEntered(on) {
  railEntered = on;
  document.getElementById('details-panel')?.classList.toggle('fx-buttons-in', on);
  // Stepping in also clears the rail: the logos you are not on fade back so the
  // one you are on is the only one in play. Stepping out (LEFT) brings them back.
  document.getElementById('game-list-panel')?.classList.toggle('rail-entered', on);
}

export function railEnterGame() {
  if (detailsOpen || railEntered || !games.length) return;
  railSetEntered(true);
  vibrate(false);   // right side
}

export function railExitGame() {
  if (!railEntered) return;
  railSetEntered(false);
  vibrate(true);    // left side
}

/* Reconcile the rail without animation after a list rebuild (search, sort,
   category switch, add/delete) or when returning from full details: the
   carousel re-attaches to whatever is focused, with transitions suppressed so
   nothing ghosts in from the previous logo. */
export function syncRailInstant() {
  document.body.classList.add('rail-fx-instant');
  applyRailCarousel();
  void document.body.offsetHeight;
  releaseRailInstant();
}

/* ── Select game — with transitions ───────────────────────────────────────*/
export function selectGame(i) {
  if (i < 0 || i >= games.length || i === selectedIndex) return;
  setSelectedIndex(i);
  updateGameListSelection();   // recalculates the visual state of every logo
  centerActiveItem(true);
  crossfadeWallpaper();
  if (games[i]) renderDetails(games[i]);
  // The statistics preview charts the game that is selected, so it follows.
  refreshStatPopover();
  if (!detailsOpen) {
    railExitGame();            // moving on drops back to browsing: stats only
    retriggerHeroAnimation();
  }
}

/* Re-run the hero entrance (logo scales up, stats glide in) whenever the focused
   game changes. Remove + re-add the class inside one frame so rapid scrolling
   cleanly interrupts the previous animation instead of getting stuck. */
export function retriggerHeroAnimation() {
  if (detailsOpen) return;
  const panel = document.getElementById('details-panel');
  if (!panel || panel.classList.contains('hidden')) return;
  panel.classList.remove('hero-anim');
  void panel.offsetWidth; // force reflow so the animation restarts
  panel.classList.add('hero-anim');
}

/* Selection changed: re-tag the items (active / near) and hand the whole rail
   to the carousel so every logo's scale, opacity and offset follow the new
   focus. Nothing animates here — CSS transitions carry the change. */
export function updateGameListSelection() {
  const items = document.querySelectorAll('#game-list .game-item');
  items.forEach((el, i) => {
    const dist = Math.abs(i - selectedIndex);
    el.className = 'game-item'
      + (i === selectedIndex ? ' active' : dist === 1 ? ' near' : '')
      + (el.classList.contains('uninstalled') ? ' uninstalled' : '')
      + (el.classList.contains('install-missing') ? ' install-missing' : '');
  });
  applyRailCarousel();
}

/* ── Dynamic accent color (item 4) ────────────────────────────────────────
   Derives an accent from the focused game's wallpaper: draw the image small,
   bucket the pixels, and pick the most saturated dominant color. Falls back to
   a neutral light gray when there is no wallpaper or nothing saturated enough.
   Published as CSS custom properties; every accent consumer reads var(--accent)
   (the legacy --green alias included), so one update re-themes the whole UI. */
const ACCENT_FALLBACK = { r: 232, g: 232, b: 238 };   // #e8e8ee neutral
let accentApplyTimer = 0;
let accentLastKey = null;

/* WCAG relative luminance (sRGB). */
function relLuminance(r, g, b) {
  const f = (c) => { c /= 255; return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4); };
  return 0.2126 * f(r) + 0.7152 * f(g) + 0.0722 * f(b);
}

/* Text color to sit ON the accent. Whichever of black/white scores the higher
   contrast wins, so labels on an accent fill always clear 4.5:1. */
function inkForAccent(r, g, b) {
  const L = relLuminance(r, g, b);
  const onBlack = (L + 0.05) / 0.05;
  const onWhite = 1.05 / (L + 0.05);
  return onBlack >= onWhite ? '#000000' : '#ffffff';
}

function applyAccentColor(r, g, b) {
  const root = document.documentElement.style;
  root.setProperty('--accent', `rgb(${r}, ${g}, ${b})`);
  root.setProperty('--accent-strong', `rgb(${Math.round(r * 0.78)}, ${Math.round(g * 0.78)}, ${Math.round(b * 0.78)})`);
  root.setProperty('--accent-soft', `rgba(${r}, ${g}, ${b}, 0.16)`);
  root.setProperty('--accent-ink', inkForAccent(r, g, b));
}

function resetAccentColor() {
  applyAccentColor(ACCENT_FALLBACK.r, ACCENT_FALLBACK.g, ACCENT_FALLBACK.b);
}

function rgbToHsl(r, g, b) {
  r /= 255; g /= 255; b /= 255;
  const max = Math.max(r, g, b), min = Math.min(r, g, b);
  const l = (max + min) / 2;
  if (max === min) return [0, 0, l];
  const d = max - min;
  const s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
  let h;
  if (max === r) h = ((g - b) / d + (g < b ? 6 : 0));
  else if (max === g) h = (b - r) / d + 2;
  else h = (r - g) / d + 4;
  return [h * 60, s, l];
}

/* Contrast check against near-black so a mud-dark dominant never becomes the
   accent: lift lightness into the readable band instead of rejecting outright. */
function normalizeAccent(r, g, b) {
  let [h, s, l] = rgbToHsl(r, g, b);
  l = Math.min(0.78, Math.max(0.6, l));
  s = Math.min(0.85, Math.max(0.35, s));
  const c = (1 - Math.abs(2 * l - 1)) * s;
  const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
  const m = l - c / 2;
  const seg = Math.floor(h / 60) % 6;
  const table = [[c, x, 0], [x, c, 0], [0, c, x], [0, x, c], [x, 0, c], [c, 0, x]];
  const [rr, gg, bb] = table[seg];
  return [Math.round((rr + m) * 255), Math.round((gg + m) * 255), Math.round((bb + m) * 255)];
}

function extractDominantColor(img) {
  try {
    const SIZE = 48;
    const canvas = document.createElement('canvas');
    canvas.width = SIZE; canvas.height = SIZE;
    const ctx = canvas.getContext('2d', { willReadFrequently: true });
    if (!ctx) return null;
    ctx.drawImage(img, 0, 0, SIZE, SIZE);
    let data;
    try {
      data = ctx.getImageData(0, 0, SIZE, SIZE).data;
    } catch {
      return null; // tainted canvas (no CORS on this source)
    }
    // 4-bit-per-channel buckets; score = saturation-weighted frequency so vivid
    // colors beat big dull areas (dark vignettes, gray skies).
    const buckets = new Map();
    for (let i = 0; i < data.length; i += 4) {
      const a = data[i + 3];
      if (a < 125) continue;
      const r = data[i], g = data[i + 1], b = data[i + 2];
      const [, s, l] = rgbToHsl(r, g, b);
      if (l < 0.08 || l > 0.95) continue;      // crush blacks / blown whites
      const key = ((r >> 4) << 8) | ((g >> 4) << 4) | (b >> 4);
      const e = buckets.get(key) || { r: 0, g: 0, b: 0, n: 0, w: 0 };
      e.r += r; e.g += g; e.b += b; e.n += 1; e.w += 0.25 + s;
      buckets.set(key, e);
    }
    let best = null;
    for (const e of buckets.values()) {
      const score = e.w * e.n;
      if (!best || score > best.score) best = { r: e.r / e.n, g: e.g / e.n, b: e.b / e.n, score };
    }
    if (!best) return null;
    const [h, s] = rgbToHsl(best.r, best.g, best.b);
    if (s < 0.18) return null;                 // effectively grayscale art
    return [best.r, best.g, best.b];
  } catch { return null; }                     // tainted canvas, OOM, anything
}

/* The wallpaper is served through Tauri's asset protocol (convertFileSrc),
   which taints a plain <img> canvas. Load with CORS first; if that is
   refused, refetch the bytes into a blob URL — blob images are same-origin
   and never taint. Results are memoized per wallpaper. */
function loadImageCors(url) {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.crossOrigin = 'anonymous';
    img.onload = () => resolve(img);
    img.onerror = reject;
    img.src = url;
  });
}

async function loadImageViaFetch(url) {
  const res = await fetch(url, { mode: 'cors' });
  if (!res.ok) throw new Error('accent fetch failed');
  const blob = await res.blob();
  const objUrl = URL.createObjectURL(blob);
  try {
    return await new Promise((resolve, reject) => {
      const img = new Image();
      img.onload = () => resolve(img);
      img.onerror = reject;
      img.src = objUrl;
    });
  } finally {
    setTimeout(() => URL.revokeObjectURL(objUrl), 5000);
  }
}

const accentCache = new Map();   // wallpaper key -> [r,g,b] | null

async function accentForWallpaper(url) {
  if (accentCache.has(url)) return accentCache.get(url);
  let result = null;
  try {
    let img;
    try { img = await loadImageCors(url); }
    catch { img = await loadImageViaFetch(url); }
    result = extractDominantColor(img);
  } catch { result = null; }
  if (accentCache.size > 24) accentCache.clear();
  accentCache.set(url, result);
  return result;
}

/* Debounced: fires on every rail scroll, but only the focused game's wallpaper
   matters and loads race through wallpaperGeneration, so cheap re-entries just
   reschedule. */
function updateAccentFromGame() {
  const g = games[selectedIndex];
  const key = g?.wallpaper || '';
  if (key === accentLastKey) return;
  accentLastKey = key;
  clearTimeout(accentApplyTimer);
  if (!key) { resetAccentColor(); return; }
  accentApplyTimer = setTimeout(async () => {
    const dom = await accentForWallpaper(key);
    if (dom) { const [r, gg, b] = normalizeAccent(dom[0], dom[1], dom[2]); applyAccentColor(r, gg, b); }
    else resetAccentColor();
  }, 120);
}

/* ── Wallpaper crossfade between two layers ───────────────────────────────*/
export function crossfadeWallpaper() {
  updateAccentFromGame();
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
export function openDetails() {
  if (!games.length || detailsOpen) return;
  vibrate(false);
  detailsOpen = true;
  document.getElementById('empty-state').style.display = 'none';
  renderDetails(games[selectedIndex]);
  // Full details takes over the layout: the rail settles back onto the line
  // (see #game-list-panel.focused in style.css) while the stats and buttons
  // step forward together.
  const sidebar = document.getElementById('game-list-panel');
  const details = document.getElementById('details-panel');
  // Focus flow (navigate right): the OTHER logos fade away while the focused
  // logo slides to the right edge of the sidebar and STAYS. The details panel
  // (stats + buttons) appears in the space to the LEFT of the logo.
  details.classList.remove('hidden');
  details.classList.remove('is-preview');
  sidebar.classList.add('focused');
  sidebar.classList.remove('logos-mode');
}

export function closeDetails() {
  if (!detailsOpen) return;
  vibrate(true);
  detailsOpen = false;
  // The statistics preview belongs to the details, so it goes with them.
  hideStatPopoverNow();
  const sidebar = document.getElementById('game-list-panel');
  const details = document.getElementById('details-panel');
  // Navigate left: other game logos reappear on the line and the focused logo
  // returns to its centered spot among them.
  sidebar.classList.remove('focused');
  details.classList.add('is-preview');
  if (!games.length) {
    details.classList.add('hidden');
    document.getElementById('empty-state').style.display = 'flex';
  }
  // Back to browsing: re-attach the carousel to the focused logo, stats only.
  railSetEntered(false);
  syncRailInstant();
}

export function renderDetails(g) {
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

  // Secondary title caption under the hero logo (shown only when enabled).
  const caption = document.getElementById('details-logo-caption');
  if (caption) {
    caption.textContent = g.name;
    caption.style.fontFamily = g.fontFamily ? `'${g.fontFamily}', 'Inter', sans-serif` : 'inherit';
    caption.style.color = g.fontColor || '#fff';
  }
  lastHeroRenderedId = g.id;

  document.getElementById('stat-playtime').textContent = fmtTime(g.playtimeMinutes);
  document.getElementById('stat-sessions').textContent = g.sessionCount || 0;
  document.getElementById('stat-last-played').textContent = fmtDate(g.lastPlayed);
  renderSavePathStatus(g);
  applyLaunchAvailability(g);

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
        showToast('error', 'Not Ready', 'Launch the game - SaveGuard will find the save folder and backups will start automatically.');
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
        showToast('error', 'Not Ready', 'Launch the game - SaveGuard will find the save folder and backups will start automatically.');
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

  setupHoldButton('btn-launch', 5000, () => doLaunch(g.id));
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
      setGames(games.filter(x => x.id !== game.id));
      setAllGames(allGames.filter(x => x.id !== game.id));
      renderGameList();
    }
  }
}

let actionHoldStart = 0;
export let actionHoldTarget = null;
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

/* ── Startup install check ────────────────────────────────────────────────
   The Rust side verifies every library exe (and the resolved game binary when
   the exe is a launcher) in parallel with a per-path timeout, so a disconnected
   network share or unplugged external drive can never freeze startup. Results
   land here keyed by game id. Nothing is persisted, and no game, save, backup
   or stat is ever modified because of a check. */
const INSTALL = {
  RECHECK_MS: 30000,   // at most one focus-triggered rescan per 30s
  LABEL: { missing: 'Not installed', unavailable: 'Drive offline' },
};
let installResults = new Map();   // gameId -> { status, checkedAt }
let installChecking = false;
let lastInstallCheckAt = 0;

function installStatus(gameId) {
  return installResults.get(gameId)?.status || 'unknown';
}

function installBadge(gameId, compact) {
  const status = installResults.get(gameId)?.status;
  if (!status || status === 'installed') return '';
  const label = INSTALL.LABEL[status];
  return compact
    ? `<span class="rail-badge${status === 'unavailable' ? ' offline' : ''}">${label}</span>`
    : `<span class="install-badge ${status}">${label}</span>`;
}

/* Ask the backend to verify the whole library. Never blocks the UI: the library
   is already on screen, and badges appear when the answer arrives. */
export async function runInstallCheck({ force = false, manual = false } = {}) {
  if (installChecking) return null;
  const now = Date.now();
  if (!force && now - lastInstallCheckAt < INSTALL.RECHECK_MS) return null;
  installChecking = true;
  lastInstallCheckAt = now;
  try {
    const results = await window.__TAURI__.core.invoke('verify_library_installs');
    return applyInstallResults(results, { manual });
  } catch (err) {
    // A failed check must never notify or be reported as a missing game.
    console.warn('Install check failed', err);
    return null;
  } finally {
    installChecking = false;
  }
}

function applyInstallResults(results, { manual = false } = {}) {
  const next = new Map();
  for (const r of results || []) {
    const id = r.game_id ?? r.gameId;
    if (id == null) continue;
    next.set(id, { status: r.status, checkedAt: Date.now() });
  }
  installResults = next;
  updateInstallIndicators();

  applyCategoryFilter(false);
  const affected = [...next.entries()].filter(([, v]) => v.status !== 'installed');
  if (affected.length === 0) {
    if (manual) showToast('success', 'All games installed', 'Every executable is where SILO last saw it.');
    return 0;
  }
  // Silent: check runs but never notifies. Users may have removed games on purpose.
  return affected.length;
}

function updateInstallIndicators() {
  renderGameList();
  const current = games[selectedIndex];
  if (current) renderDetails(current);
}

function focusFirstAffected() {
  const g = games.find(x => installStatus(x.id) !== 'installed');
  if (!g) return;
  selectGame(games.indexOf(g));
  openDetails();
}

/* Before launching, re-verify just this game: a game can vanish between app
   start and now, and the user must get a clear message, never a silent failure. */
async function reverifyBeforeLaunch(gameId) {
  try {
    const r = await window.__TAURI__.core.invoke('verify_game_install', { gameId });
    if (r && r.status && r.status !== 'installed') {
      installResults.set(gameId, { status: r.status, checkedAt: Date.now() });
      updateInstallIndicators();
      return r.status;
    }
  } catch {
    // Optimistic by design: a broken check never blocks a launch.
  }
  return null;
}

/* Locate…: point SILO at the executable again, then re-verify that one game. */
async function locateGameExe(gameId) {
  const path = await window.vault.pickExe().catch(() => null);
  if (!path) return;
  const updated = await window.vault.updateGame(gameId, { exePath: path }).catch(() => null);
  if (!updated) {
    showToast('error', 'Path not updated', 'SILO could not save the new executable path.');
    return;
  }
  const idx = allGames.findIndex(g => g.id === gameId);
  if (idx !== -1) allGames[idx] = updated;
  applyCategoryFilter(false);
  installResults.delete(gameId);
  await runInstallCheck({ force: true });
  showToast('success', 'Executable updated', updated.name || 'Game');
}

/* Main screen: a missing game cannot launch, but Edit / Backup / Restore stay
   available — the saves can still be there.
   Phase 2: Launch button is HIDDEN (not disabled) when not installed.
   A STATUS stat block appears in the stat row with an amber dot + Locate link. */
function applyLaunchAvailability(g) {
  const btn = document.getElementById('btn-launch');
  if (!btn) return;
  const status = installStatus(g.id);
  const bad = status !== 'installed';

  // Hide launch entirely — never show a disabled button.
  btn.style.display = bad ? 'none' : '';
  btn.disabled = false;
  btn.title = 'Launch';
  const label = btn.querySelector('.pill-label');
  if (label) label.textContent = 'LAUNCH';

  // STATUS stat block: visible only when not installed.
  const statusBlock = document.getElementById('stat-block-status');
  if (statusBlock) {
    statusBlock.style.display = bad ? '' : 'none';
    if (bad) {
      const valEl = statusBlock.querySelector('.stat-install-value');
      const locateBtn = statusBlock.querySelector('.stat-locate-btn');
      if (valEl) valEl.textContent = INSTALL.LABEL[status] || 'Not installed';
      if (locateBtn) {
        locateBtn.onclick = null;
        locateBtn.addEventListener('click', () => locateGameExe(g.id), { once: true });
      }
    }
  }

  // Wallpaper: grayscale + dimmer for not-installed games.
  applyInstallWallpaperFilter(bad);
}

/* Apply or remove the not-installed wallpaper filter (grayscale + dim). */
function applyInstallWallpaperFilter(bad) {
  const bg1 = document.getElementById('bg-wallpaper-1');
  const bg2 = document.getElementById('bg-wallpaper-2');
  const filter = bad ? 'grayscale(0.7) brightness(0.7)' : '';
  if (bg1) bg1.style.filter = filter;
  if (bg2) bg2.style.filter = filter;
}

export async function doLaunch(gameId) {
  const g = games.find(g => g.id === gameId);
  if (g && g.isInstalled === false) {
    showToast('error', 'Not Installed', 'The executable for this game could not be found.');
    return;
  }
  const status = await reverifyBeforeLaunch(gameId);
  if (status) {
    const why = status === 'unavailable'
      ? 'its drive is not available (an external drive may be unplugged).'
      : 'the executable is no longer where SILO last saw it.';
    showToast('error', 'Cannot launch', `${g?.name || 'This game'}: ${why}`);
    return;
  }
  showToast('info', 'Launching', `${g?.name || 'Game'}...`);
  try {
    await window.vault.launchGame(gameId);
    showToast('success', 'Launched', `${g?.name || 'Game'} is running`);
  } catch (err) {
    showToast('error', 'Launch Failed', err);
  }
}

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

/* Save-folder status. When the game has no folder yet there is nothing for
   the user to do, so the line stays out of the way; once a folder is known it
   is shown quietly, and the recovery states (missing / offline) stay loud. */
function renderSavePathStatus(g) {
  const el = document.getElementById('save-path-status');
  if (!el) return;
  el.classList.remove('sp-quiet', 'sp-alert');
  el.hidden = false;
  if (g && g.savePath) {
    const src = String(g.savePathSource || '').toLowerCase();
    const tag = src === 'auto' ? '<span class="sp-path-tag">AUTO</span>'
      : src === 'manual' ? '<span class="sp-path-tag manual">MANUAL</span>'
      : src === 'imported' ? '<span class="sp-path-tag">IMPORTED</span>'
      : '';
    el.innerHTML = `${tag}<span title="${esc(g.savePath)}">${esc(g.savePath)}</span>`;
    el.classList.add('sp-quiet');
    return;
  }
  // No folder yet. If the game is also missing or offline, that is a problem
  // the user can act on, so the message stays visible.
  const status = g ? installStatus(g.id) : 'unknown';
  if (status === 'missing' || status === 'unavailable') {
    el.textContent = 'SAVE FOLDER - NONE LOCATED, SET IT IN EDIT';
    el.classList.add('sp-alert');
    return;
  }
  el.textContent = '';
  el.hidden = true;
}
