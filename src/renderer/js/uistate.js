/* ── uistate.js — the launcher's route memory across a WebView rebuild ───────
   Stage 6 destroys the launcher's WebView while a game runs and builds a brand
   new one afterwards. The new renderer starts from a fresh document, so the
   durable library state comes back from Rust, while this light route memory
   (which page is on screen, which game is focused) is kept here so the launcher
   opens where the user left it instead of back at the default landing state.

   It lives in localStorage, which the rebuilt WebView shares because it uses
   the same WebView2 data directory as the window it replaced. Everything here
   is best-effort: a missing or unreadable hint simply means the launcher opens
   at the default state, which is exactly how it behaved before. */

const KEY = 'silo.ui';
const WRITE_DEBOUNCE_MS = 250;

let writeTimer = null;

/* The stored hint, or null when there is nothing usable. */
export function readUiState() {
  try {
    const raw = window.localStorage.getItem(KEY);
    if (!raw) return null;
    const value = JSON.parse(raw);
    return value && typeof value === 'object' ? value : null;
  } catch (e) {
    return null;
  }
}

/* Records one field of the route memory, keeping the others (debounced, so
   arrow-key browsing does not write on every frame). */
export function rememberUiState(patch) {
  const merged = { ...(readUiState() || {}), ...patch };
  if (writeTimer !== null) clearTimeout(writeTimer);
  writeTimer = setTimeout(() => {
    writeTimer = null;
    try {
      window.localStorage.setItem(KEY, JSON.stringify(merged));
    } catch (e) {
      /* storage may be full or blocked; the launcher still works without it */
    }
  }, WRITE_DEBOUNCE_MS);
}

/* Flushes the pending write immediately: used before the window is released,
   so a route change in the last moment is not lost. */
export function flushUiState() {
  if (writeTimer === null) return;
  clearTimeout(writeTimer);
  writeTimer = null;
}

/* The game id that should be focused when the rebuilt renderer boots. */
export function rememberedGameId() {
  const id = readUiState()?.selectedGameId;
  return typeof id === 'string' && id ? id : null;
}

/* The page that should be shown when the rebuilt renderer boots. */
export function rememberedPage() {
  const page = readUiState()?.page;
  return typeof page === 'string' && page ? page : null;
}
