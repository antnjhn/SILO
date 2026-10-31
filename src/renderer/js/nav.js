/* ── nav.js — the auto-hiding top navigation ────────────────────────────────
   The navigation is hidden by default so the game artwork owns the screen.
   It is revealed by the pointer approaching the top edge, by an upward
   controller input, or by an explicit shortcut, and it hides itself again
   after a short idle.

   The navigation never takes layout space: it is an overlay, so revealing it
   cannot shift or resize the game content underneath.

   The buttons keep the ids the rest of the app already binds (btn-stats,
   btn-search-toggle, btn-add-game, btn-settings), so this module only owns
   the reveal/hide behaviour and the active-state underline — it does not
   duplicate any of the existing action handlers. */
import { showAllGames } from './library.js';
import { showPage, currentPage } from './pages.js';

/* How close to the top edge the pointer has to come to reveal the bar. */
const TOP_ZONE = 130;
/* How long the bar lingers after the last interaction. */
const IDLE_MS = 2600;

const NAV_ITEMS = ['home', 'stats', 'import', 'settings'];

let idleTimer = null;
let pointerInside = false;
let focusInside = false;
let revealed = false;

function navEl() { return document.getElementById('topnav'); }

/* ── Reveal / hide ─────────────────────────────────────────────────────────*/
export function revealNav() {
  const nav = navEl();
  if (!nav) return;
  // Idempotent: the class and the flag are always set together, so they can
  // never drift apart.
  revealed = true;
  nav.classList.add('revealed');
  scheduleHide();
}

export function hideNav() {
  const nav = navEl();
  if (!nav) return;
  // Never hide while the pointer or the keyboard focus is on the bar, while
  // a search is open, or while a full-screen overlay is up.
  if (pointerInside || focusInside) return;
  if (isBusy()) return;
  revealed = false;
  nav.classList.remove('revealed');
  document.body.classList.remove('nav-open');
  releaseNavKeyboard();
}

/* Hides the bar immediately, with no idle wait. Hiding always hands the
   keyboard back to the library: otherwise the arrow keys would stay captured
   by a bar that is no longer on screen. */
export function hideNavNow() {
  const nav = navEl();
  revealed = false;
  if (nav) nav.classList.remove('revealed');
  releaseNavKeyboard();
  clearTimeout(idleTimer);
}

/* Escape closes the bar even though the focus is inside it: the focus is
   handed back to the page that is on screen, so the bar is not "in use" any
   more and the page keeps the keyboard. */
export function dismissNav() {
  const nav = navEl();
  const page = document.querySelector('#page-host .page:not(.hidden)');
  if (nav && nav.contains(document.activeElement)) {
    if (page) page.focus({ preventScroll: true });
    else document.activeElement.blur();
  }
  focusInside = false;
  pointerInside = false;
  hideNavNow();
}

/* The bar stays up while something that lives inside it is in use. */
function isBusy() {
  if (document.getElementById('game-list-panel')?.classList.contains('search-mode')) return true;
  if (document.getElementById('filter-panel')?.classList.contains('open')) return true;
  return false;
}

function scheduleHide() {
  clearTimeout(idleTimer);
  idleTimer = setTimeout(() => {
    // Re-check inside the timer: the pointer may have come back.
    hideNav();
  }, IDLE_MS);
}

/* ── Active state ──────────────────────────────────────────────────────────
   A restrained white underline under the active item. */
export function setActiveNav(id) {
  document.querySelectorAll('#topnav .topnav-item').forEach((el) => {
    const active = el.dataset.navId === id;
    el.classList.toggle('active', active);
    // The active item is the page that is on screen.
    if (active) el.setAttribute('aria-current', 'page');
    else el.removeAttribute('aria-current');
  });
  // The cursor follows the destination, unless the keyboard has moved it.
  if (!isNavKeyboardActive()) {
    const items = navItems();
    const active = items.findIndex((el) => el.dataset.navId === id);
    if (active >= 0) {
      navCursor = active;
      items.forEach((el, n) => el.classList.toggle('cursor', n === active));
    }
  }
}

/* ── Keyboard navigation ───────────────────────────────────────────────────
   The bar is a single row, so the arrow keys walk along it.

   The keyboard cursor is deliberately NOT DOM focus. Moving real focus into
   the bar would (a) keep the bar on screen for ever, because a focused
   control counts as "in use", and (b) steal the left and right arrows from
   the library, which already use them to open the category wheel and to
   step into a game. The cursor instead drives the selection, and the DOM
   focus stays where the user left it. */
function navItems() {
  return [...document.querySelectorAll('#topnav .topnav-item')];
}

let navCursor = 0;
let navKeyboardMode = false;

/* True when the keyboard is driving the bar: either the user has asked for
   it with the up arrow, or the focus is genuinely inside the bar. */
export function isNavKeyboardActive() {
  const nav = navEl();
  return navKeyboardMode || !!(nav && nav.contains(document.activeElement));
}

function setNavCursor(index) {
  const items = navItems();
  if (!items.length) return;
  const i = ((index % items.length) + items.length) % items.length;
  navCursor = i;
  items.forEach((el, n) => el.classList.toggle('cursor', n === i));
  revealNav();
}

/* Moves the cursor along the row. */
export function moveNavFocus(delta) {
  const items = navItems();
  if (!items.length) return;
  const current = items.findIndex((el) => el.classList.contains('cursor'));
  const from = current < 0 ? items.findIndex((el) => el.classList.contains('active')) : current;
  setNavCursor(from + delta);
}

/* Activates the item the cursor is on. */
export function activateNavItem() {
  const items = navItems();
  const el = items.find((item, i) => i === navCursor && item.classList.contains('cursor'))
    || items.find((item) => item.classList.contains('active'));
  el?.click();
}

/* Reveals the bar and puts the keyboard cursor on its active item, so the
   arrow keys can go straight on to drive it. */
export function revealNavAndFocus() {
  revealNav();
  const items = navItems();
  const active = items.findIndex((el) => el.classList.contains('active'));
  navKeyboardMode = true;
  setNavCursor(active < 0 ? 0 : active);
}

/* Leaves keyboard mode: the pointer or the library takes over again. */
export function releaseNavKeyboard() {
  navKeyboardMode = false;
  navItems().forEach((el) => el.classList.remove('cursor'));
}

function syncActiveNav() {
  const page = currentPage();
  if (page === 'stats') return setActiveNav('stats');
  if (page === 'settings') return setActiveNav('settings');
  if (page === 'import') return setActiveNav('import');
  return setActiveNav('home');
}

/* ── Wiring ────────────────────────────────────────────────────────────────*/
export function initNav() {
  const nav = navEl();
  if (!nav) return;

  // Give each item a stable id for the active-state logic.
  const ids = {
    'nav-home': 'home',
    'btn-stats': 'stats',
    'btn-add-game': 'import',
    'btn-settings': 'settings',
  };
  Object.entries(ids).forEach(([elId, navId]) => {
    const el = document.getElementById(elId);
    if (el) el.dataset.navId = navId;
  });

  // The navigation switches pages: each item activates its own page, and
  // the page that was left is gone once the transition is over.
  document.getElementById('nav-home')?.addEventListener('click', () => {
    showAllGames();
    showPage('home');
    setActiveNav('home');
  });

  // The pointer near the top edge reveals the bar; leaving it starts the
  // idle countdown.
  window.addEventListener('pointermove', (e) => {
    if (e.clientY <= TOP_ZONE) {
      revealNav();
    } else if (revealed && !pointerInside) {
      scheduleHide();
    }
  }, { passive: true });

  nav.addEventListener('pointerenter', () => { pointerInside = true; releaseNavKeyboard(); revealNav(); });
  nav.addEventListener('pointerleave', () => { pointerInside = false; scheduleHide(); });
  nav.addEventListener('focusin', () => { focusInside = true; revealNav(); });
  nav.addEventListener('focusout', (e) => {
    // Focus may have moved to another control inside the bar.
    if (nav.contains(e.relatedTarget)) return;
    focusInside = false;
    scheduleHide();
  });

  // Any click on a navigation item keeps it up for a moment and refreshes
  // the active state.
  nav.addEventListener('click', () => {
    releaseNavKeyboard();
    revealNav();
    // The overlay that opened may need a frame to appear.
    requestAnimationFrame(() => requestAnimationFrame(syncActiveNav));
  });

  // Other parts of the app can ask for the bar (the search shortcut uses it).
  document.addEventListener('silo:reveal-nav', () => revealNav());

  // The bar can be hidden through the same code path the UI uses, so its
  // internal state can never drift out of sync with the class list.
  window.__hideNavNow = hideNavNow;

  // The active item follows the page that is on screen.
  const host = document.getElementById('page-host');
  if (host) {
    new MutationObserver(syncActiveNav).observe(host, { attributes: true, attributeFilter: ['class', 'hidden'], subtree: true });
  }

  syncActiveNav();
}

/* The controller reveals the bar with an upward push. It lives here so the
   gamepad handler stays a thin input mapping. */
export function revealNavFromController() {
  revealNav();
}

/* Whether the bar is currently on screen. The controller uses it so an
   upward push reveals the bar first and only browses the rail afterwards. */
export function isNavRevealed() {
  return revealed;
}
