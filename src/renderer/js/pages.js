/* ── pages.js — the application's page switcher ────────────────────────────
   Statistics, Import and Settings are PAGES, not overlays: only one of them
   is on screen at a time, and the page being left stops being visible and
   interactive as soon as the transition is over — nothing is left stacked
   underneath.

   The switch is a short horizontal slide. The outgoing page slides slightly
   out, the incoming page slides in, and the opacity moves with them.

   A module that owns a page registers an "enter" handler, so its content is
   (re)built whenever the page is shown — which is also what fixes a chart
   that was measured while its page was hidden. */
const PAGES = ['home', 'stats', 'import', 'settings'];
const TRANSITION_MS = 220;

const enterHandlers = new Map();
const leaveHandlers = new Map();
let current = 'home';
let transitionTimer = null;
/* The transition that is running right now: the page it came from and the
   work that finishes it. */
let pending = null;

export function currentPage() { return current; }
export function isCurrentPage(id) { return current === id; }
export function onPageEnter(id, fn) { enterHandlers.set(id, fn); }
export function onPageLeave(id, fn) { leaveHandlers.set(id, fn); }

function pageEl(id) {
  return document.querySelector(`#page-host .page[data-page="${id}"]`);
}

/* Puts a page away: no longer on screen, no longer in the tab order and no
   longer announced. */
function park(page) {
  if (!page) return;
  page.classList.remove('leaving');
  page.classList.remove('entering');
  page.classList.add('hidden');
  page.setAttribute('aria-hidden', 'true');
}

/* Finishes a transition at once, without waiting for the animation. */
function settle() {
  if (!pending) return;
  const { from, to, finish } = pending;
  pending = null;
  finish(from, to);
}

export function showPage(id, { instant = false } = {}) {
  if (!PAGES.includes(id)) return;
  const host = document.getElementById('page-host');
  if (!host) return;
  const to = pageEl(id);
  if (!to) return;

  // Any transition still running is completed first. Otherwise its page would
  // be left half way out: invisible, but still there.
  if (pending) {
    clearTimeout(transitionTimer);
    transitionTimer = null;
    settle();
  }

  if (current === id) {
    to.classList.remove('hidden');
    to.removeAttribute('aria-hidden');
    enterHandlers.get(id)?.();
    return;
  }

  const from = pageEl(current);

  // The page that is being left gets its say: a preview or a panel that
  // belongs to it is put away before the new page comes in.
  leaveHandlers.get(current)?.();

  // The incoming page is prepared and measured BEFORE it is shown, so
  // anything that needs its size (the charts) sees the real layout.
  to.classList.remove('hidden');
  to.classList.remove('leaving');
  to.removeAttribute('aria-hidden');
  to.classList.add('entering');
  enterHandlers.get(id)?.();

  if (from && from !== to) {
    from.classList.remove('entering');
    from.classList.add('leaving');
    // The page being left is out of the way for the screen reader too.
    from.setAttribute('aria-hidden', 'true');
  }
  current = id;

  const finish = (outgoing, incoming) => {
    // Focus follows the page: if the focus was inside the page that was left,
    // it would have been lost, so it is moved to the page that is now shown.
    // This happens first, so the focus is never inside a page being hidden.
    const active = document.activeElement;
    const lost = !active || active === document.body
      || (outgoing && outgoing.contains(active));
    if (lost && incoming) incoming.focus({ preventScroll: true });
    if (outgoing && outgoing !== incoming) park(outgoing);
    incoming?.classList.remove('entering');
  };

  pending = { from, to, finish };
  if (instant) {
    const p = pending;
    pending = null;
    p.finish(p.from, p.to);
    return;
  }
  transitionTimer = setTimeout(() => {
    transitionTimer = null;
    settle();
  }, TRANSITION_MS);
}

/* Returns to the library. Used by the Back controls and by Escape. */
export function goHome() { showPage('home'); }
