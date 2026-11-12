/* ── gamepad.js — poll loop, input mapping, rumble ─────────────────────────── */
import { games, selectedIndex } from './state.js';
import {
  getCategories, changeCategorySelection, openCategories, closeCategories,
  selectGame, railEnterGame, railExitGame, closeDetails, openDetails,
  categorySelectedIndex, categoriesOpen, detailsOpen, railEntered, actionHoldTarget,
  focusDock, hasDockFocus, activateDockFocus,
} from './library.js';
import { openAddModal } from './modals.js';
import { navigateStatsFocus } from './stats.js';
import {
  revealNavFromController, revealNavAndFocus, isNavRevealed, hideNavNow, moveNavFocus, activateNavItem, toggleNav,
} from './nav.js';
import { showPage } from './pages.js';

// Gamepad
const DEADZONE = 0.4;
const NAV_REPEAT = 175;

/* The launch hold. The button owns the real duration (`btn._holdMs`, published
   by setupHoldButton); this is only the fallback for when the details panel has
   not rendered yet. */
const B_HOLD_MS = 3000;
/* A release this much before the hold completes still counts as a tap, so a
   press that lands a frame or two early is never lost. */
const TAP_SLACK_MS = 250;

let lastNavTime = 0;
let prevAxes = { right: false, left: false, up: false, down: false };
let buttonWas = {};
/* When the current B press started, or 0 when no press is being tracked. */
let bDownAt = 0;

export function vibrate(left) {
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

export function vibrateVertical() {
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

/* ── Hold buttons ────────────────────────────────────────────────────────────
   The press starts the ring, the release cancels it. Starting is skipped while
   another hold owns the target, exactly like the previous per-frame helper. */
function hold(id, isPressed) {
  const btn = document.getElementById(id);
  if (!btn) return;
  if (isPressed) {
    if (!actionHoldTarget && btn._startGamepadHold) btn._startGamepadHold();
  } else if (actionHoldTarget === id && btn._resetGamepadHold) {
    btn._resetGamepadHold();
  }
}

/* The real hold duration, so the tap window follows the button. */
function launchHoldMs() {
  const ms = Number(document.getElementById('btn-launch')?._holdMs);
  return ms > 0 ? ms : B_HOLD_MS;
}

/* ── Gamepad polling state ───────────────────────────────────────────────

   High-frequency (`requestAnimationFrame`) polling runs only while a
   controller is connected *and* the document is visible. When no controller
   is connected, a low-frequency timer (`setTimeout`) polls for connection
   instead of burning a frame every ~7 ms. The document `visibilitychange`
   event pauses high-frequency polling when the page is hidden (tray, minimise,
   background tab) and resumes it when shown.

   Only one loop may be active at a time: `startPolling`/`stopPolling` guard
   against duplicate rAF timers, and the low-frequency timer is likewise
   single-instance.
──────────────────────────────────────────────────────────────────────────*/

let rafId = 0;            // outstanding requestAnimationFrame id (0 = none)
let timerId = 0;          // outstanding low-frequency timer id (0 = none)
let controllerConnected = false;  // true while a gamepad is connected
let polling = false;      // true while high-frequency rAF polling is active

const LOW_FREQUENCY_MS = 1000;  // connection-detection cadence with no controller

/// Test-only hook: resets all polling state so each scenario starts clean.
/// Clears any outstanding rAF / low-frequency timer and drops connection state.
export function __resetForTesting() {
  cancelRaf();
  cancelTimer();
  polling = false;
  controllerConnected = false;
  rafId = 0;
  timerId = 0;
  resetInputState();
}

function cancelRaf() {
  if (rafId) {
    cancelAnimationFrame(rafId);
    rafId = 0;
  }
}

function cancelTimer() {
  if (timerId) {
    clearTimeout(timerId);
    timerId = 0;
  }
}

/// Reset edge/hold state so a pause/resume cycle cannot produce a stuck
/// button, phantom press, or accidental launch.
function resetInputState() {
  buttonWas = {};
  prevAxes = { right: false, left: false, up: false, down: false };
  bDownAt = 0;
  hold('btn-launch', false);
}

/// Stops high-frequency polling. Callers own `controllerConnected`; this only
/// ends the rAF chain.
function stopPolling() {
  if (!polling) return;
  polling = false;
  cancelRaf();
}

/// Starts high-frequency polling. No-op if already polling or the document is
/// hidden (visibility handles the hidden case separately).
function startPolling() {
  if (polling || document.hidden) return;
  polling = true;
  cancelRaf();
  cancelTimer();
  rafId = requestAnimationFrame(pollGamepad);
}

/// Schedules the low-frequency connection-detection pass. Single-instance.
function scheduleLowFrequency() {
  if (timerId) return;
  timerId = setTimeout(() => {
    timerId = 0;
    const pads = navigator.getGamepads?.() || [];
    const connected = pads.some((p) => p?.connected);
    if (connected && !controllerConnected) {
      controllerConnected = true;
      startPolling();
    } else if (!connected && controllerConnected) {
      controllerConnected = false;
      stopPolling();
      resetInputState();
      scheduleLowFrequency();
    } else if (!connected) {
      scheduleLowFrequency();
    }
    // connected && controllerConnected: high-frequency loop already running
  }, LOW_FREQUENCY_MS);
}

/// Initialises polling once at frontend start-up.
export function initGamepadPolling() {
  cancelRaf();
  cancelTimer();
  polling = false;
  const pads = navigator.getGamepads?.() || [];
  controllerConnected = pads.some((p) => p?.connected);
  if (controllerConnected) startPolling();
  else scheduleLowFrequency();
}

export function pollGamepad() {
  rafId = 0;  // this frame is executing; clear the outstanding id
  const pads = navigator.getGamepads?.() || [];
  const pad = Array.from(pads).find((p) => p?.connected);

  if (pad) {
    document.body.classList.add('gamepad-active');
    handlePad(pad);
    // Re-arm high-frequency polling only while still polling.
    if (polling) rafId = requestAnimationFrame(pollGamepad);
  } else {
    document.body.classList.remove('gamepad-active');
    controllerConnected = false;
    stopPolling();
    resetInputState();
    scheduleLowFrequency();
  }
}

function handlePad(pad) {
  const now = Date.now();
  const modalOpen = !document.getElementById('modal-overlay').classList.contains('hidden');
  const metadataOpen = !document.getElementById('metadata-overlay').classList.contains('hidden');
  const onlineArtOpen = !document.getElementById('online-art-overlay').classList.contains('hidden');
  const backupManagerOpen = !document.getElementById('backup-manager-overlay').classList.contains('hidden');
  const settingsOpen = !document.getElementById('settings-overlay').classList.contains('hidden');
  const statsOpen = !document.getElementById('stats-overlay').classList.contains('hidden');

  const ly = pad.axes[1];
  const lx = pad.axes[0];
  const dUp    = pad.buttons[12]?.pressed;
  const dDown  = pad.buttons[13]?.pressed;
  const dLeft  = pad.buttons[14]?.pressed;
  const dRight = pad.buttons[15]?.pressed;

  const up    = !!(dUp    || ly < -DEADZONE);
  const down  = !!(dDown  || ly >  DEADZONE);
  const right = !!(dRight || lx >  DEADZONE);
  const left  = !!(dLeft  || lx < -DEADZONE);

  const bHeld = !!pad.buttons[1]?.pressed;
  const yHeld = !!pad.buttons[3]?.pressed;

  /* Every edge is sampled exactly once per frame, here, before any branch can
     return. A button pressed inside an overlay therefore cannot fire late when
     the overlay closes, and no button is read twice in one frame. */
  const pressed = {
    a: btnPressed(pad, 0, 'A'),
    b: btnPressed(pad, 1, 'B'),
    x: btnPressed(pad, 2, 'X'),
    y: btnPressed(pad, 3, 'Y'),
    lb: btnPressed(pad, 4, 'LB'),
    start: btnPressed(pad, 9, 'START'),
  };

  /* All four axes are committed once per frame on every path, so edge detection
     can never drift (this is what used to leave prevAxes.up unassigned). */
  const commitAxes = () => {
    prevAxes.left = left;
    prevAxes.right = right;
    prevAxes.up = up;
    prevAxes.down = down;
  };

  /* ── 1. Statistics page ──────────────────────────────────────────────────── */
  if (statsOpen) {
    if (pressed.b) document.getElementById('btn-stats-close').click();
    if (right && !prevAxes.right) document.getElementById('btn-stats-games').click();
    if (left && !prevAxes.left) document.getElementById('btn-stats-overall').click();
    if (up && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; navigateStatsFocus(-1); }
    if (down && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; navigateStatsFocus(1); }
    commitAxes();
    return;
  }

  /* ── 2. Overlays: import, settings, metadata, online art, backup manager ──── */
  if (modalOpen || metadataOpen || onlineArtOpen || backupManagerOpen || settingsOpen) {
    // The import and settings panels are pages now, so the controller needs a
    // way out of them: B goes back to the library, like it does on the
    // statistics page.
    if ((modalOpen || settingsOpen) && pressed.b) showPage('home');
    commitAxes();
    return;
  }

  /* ── 3. Categories wheel ─────────────────────────────────────────────────── */
  if (categoriesOpen) {
    const numCats = getCategories().length;
    if (up && now - lastNavTime > NAV_REPEAT) {
      if (categorySelectedIndex > 0) {
        lastNavTime = now;
        changeCategorySelection(-1);
        vibrateVertical();
      }
    }
    if (down && now - lastNavTime > NAV_REPEAT) {
      if (categorySelectedIndex < numCats - 1) {
        lastNavTime = now;
        changeCategorySelection(1);
        vibrateVertical();
      }
    }
    if ((right && !prevAxes.right) || pressed.a) closeCategories(true);
    if (pressed.b) closeCategories(false);
    commitAxes();
    return;
  }

  /* ── 4. START: an explicit two-state toggle for the bar ──────────────────
     It never touches the selected game or the page, so the bar can come and go
     over exactly the focus the user left behind. */
  if (pressed.start) {
    toggleNav();
    commitAxes();
    return;
  }

  /* ── 5. B is context-sensitive: tap = back, hold = launch ─────────────────
     The press starts the ring; the release decides. A release before the hold
     completes is a tap, and what "back" means depends on the state below. */
  let bTap = false;
  if (bDownAt && !bHeld) {
    const heldMs = performance.now() - bDownAt;
    bDownAt = 0;
    hold('btn-launch', false);
    bTap = heldMs < launchHoldMs() - TAP_SLACK_MS;
  }
  if (pressed.b) {
    bDownAt = performance.now();
    hold('btn-launch', true);
  }

  /* ── 6. The revealed bar owns the d-pad ───────────────────────────────────
     Nothing else runs while it is up, so the rail can never scroll behind it.
     UP is deliberately inert (it must not re-reveal or scroll), DOWN dismisses,
     A activates, B taps back or holds to launch. */
  if (isNavRevealed()) {
    if (left && !prevAxes.left) moveNavFocus(-1);
    if (right && !prevAxes.right) moveNavFocus(1);
    if (down && !prevAxes.down) hideNavNow();
    if (pressed.a) activateNavItem();
    if (bTap) hideNavNow();
    commitAxes();
    return;
  }

  /* ── 7. Game view: full details, or stepped into the focused game ─────────
     The dock owns LEFT/RIGHT, A picks an action, B taps back out. */
  if (detailsOpen || railEntered) {
    if (left && !prevAxes.left) focusDock(-1);
    if (right && !prevAxes.right) focusDock(1);
    // The first A reveals the dock's first action instead of firing it.
    if (pressed.a) {
      if (hasDockFocus()) activateDockFocus();
      else focusDock(1);
    }
    if (games[selectedIndex]) {
      if (pressed.x) document.getElementById('btn-delete-game').click();
      hold('btn-edit-game', yHeld);
    }
    if (bTap) {
      if (detailsOpen) closeDetails();
      else railExitGame();
    }
    commitAxes();
    return;
  }

  /* ── 8. Browsing the rail (bar hidden) ────────────────────────────────────
     UP asks for the bar and lands with its cursor visible; while it is up the
     d-pad belongs to it (see 6), so one push never does two things at once.
     DOWN scrubs the rail. */
  if (up && now - lastNavTime > NAV_REPEAT) {
    lastNavTime = now;
    if (selectedIndex === 0) {
      revealNavAndFocus();
    } else {
      selectGame(selectedIndex - 1);
    }
  }
  if (down && now - lastNavTime > NAV_REPEAT && selectedIndex < games.length - 1) {
    lastNavTime = now;
    selectGame(selectedIndex + 1);
    vibrateVertical();
  }
  if (right && !prevAxes.right) railEnterGame();
  // LEFT opens the category wheel, the way the keyboard's LEFT does.
  if (left && !prevAxes.left) openCategories();
  if (pressed.lb) openCategories();
  if (pressed.y) openAddModal();
  if (pressed.a && games.length) openDetails();

  commitAxes();
}

function btnPressed(pad, idx, key) {
  const cur = !!pad.buttons[idx]?.pressed;
  const prev = !!buttonWas[key];
  buttonWas[key] = cur;
  return cur && !prev;
}

/* ── Controller connection / visibility control ───────────────────────────

   `gamepadconnected` / `gamepaddisconnected` drive the high-frequency poll
   loop: connect starts it, disconnect returns to low-frequency detection.
   `visibilitychange` pauses polling when the document is hidden and resumes
   the correct mode when shown. Neither creates duplicate loops because
   `startPolling`/`stopPolling`/`scheduleLowFrequency` are single-instance.
──────────────────────────────────────────────────────────────────────────*/

window.addEventListener('gamepadconnected', () => {
  controllerConnected = true;
  startPolling();
});

window.addEventListener('gamepaddisconnected', () => {
  controllerConnected = false;
  stopPolling();
  resetInputState();
  scheduleLowFrequency();
});

document.addEventListener('visibilitychange', () => {
  if (document.hidden) {
    stopPolling();
    resetInputState();
  } else {
    // Reconcile ground truth: a controller may have connected or disconnected
    // while the document was hidden (missed gamepad events). Do not trust the
    // stale `controllerConnected` flag here — this distinguishes document
    // visibility from native HWND visibility; minimising/hiding the native
    // window does not necessarily set `document.hidden`.
    cancelTimer();
    const pads = navigator.getGamepads?.() || [];
    controllerConnected = pads.some((p) => p?.connected);
    if (controllerConnected) startPolling();
    else scheduleLowFrequency();
  }
});
