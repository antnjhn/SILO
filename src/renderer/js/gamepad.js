/* ── gamepad.js — poll loop, input mapping, rumble ─────────────────────────── */
import { games, selectedIndex } from './state.js';
import {
  getCategories, changeCategorySelection, openCategories, closeCategories,
  selectGame, railEnterGame, railExitGame, closeDetails, openDetails,
  categorySelectedIndex, categoriesOpen, detailsOpen, railEntered, actionHoldTarget,
  focusDock, hasDockFocus, activateDockFocus, focusDetailsStat,
} from './library.js';
import { getActiveModalScope, getPageNavigationScope, moveFocusInScope, dismissActiveModal, focusInView } from './navigation.js';
import { openAddModal } from './modals.js';

import {
  revealNavFromController, revealNavAndFocus, isNavRevealed, hideNavNow, moveNavFocus, activateNavItem, toggleNav,
} from './nav.js';
import { showPage, currentPage } from './pages.js';
import { isKeyboardOpen, openKeyboard, handleKeyboardPad } from './osk.js';

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
   another hold owns the target. The poll may only cancel a hold it started:
   otherwise every frame with a connected controller would reset a hold the
   mouse began on the same button. */
let gamepadHoldTarget = null;
function hold(id, isPressed) {
  const btn = document.getElementById(id);
  if (!btn) return;
  if (isPressed) {
    if (!actionHoldTarget && btn._startGamepadHold) {
      btn._startGamepadHold();
      gamepadHoldTarget = id;
    }
  } else if (gamepadHoldTarget === id && btn._resetGamepadHold) {
    btn._resetGamepadHold();
    gamepadHoldTarget = null;
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
  prevAxes = { right: false, left: false, up: false, down: false, rUp: false, rDown: false };
  bDownAt = 0;
  hold('btn-launch', false);
  gamepadHoldTarget = null;
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

/* ── First run setup on a pad ──────────────────────────────────────────────
   The setup card was mouse and keyboard only, which is a strange first
   impression for a controller-first launcher. The d-pad walks the controls of
   the step on screen, A presses the focused one (or brings up the on-screen
   keyboard for a text field), B takes the step's own skip (or closes setup on
   the last step). */
let onboardingFocusIndex = 0;

function onboardingControls() {
  const step = document.querySelector('#onboarding-card .ob-step:not(.hidden)');
  if (!step) return [];
  return [...step.querySelectorAll('button:not([disabled]), a[href], input:not([type="checkbox"]):not([type="radio"]):not([type="color"])')]
    .filter((control) => control.offsetParent !== null && !control.hidden && !control.readOnly);
}

/* Text fields in whatever panel is open. The same keyboard the setup step uses
   is offered wherever one of these takes focus, so a key or a game name can be
   typed with a pad after setup as well. */
let panelInputIndex = 0;

function panelTextInputs() {
  const panels = ['#modal', '#settings-modal', '#backup-name-modal', '#metadata-modal']
    .map((selector) => document.querySelector(selector))
    .filter((panel) => panel && panel.offsetParent !== null);
  return panels.flatMap((panel) => [...panel.querySelectorAll(
    'input[type="text"], input[type="password"], input[type="number"]',
  )]).filter((input) => input.offsetParent !== null && !input.readOnly && !input.disabled);
}

function paintOnboardingFocus() {
  const controls = onboardingControls();
  if (!controls.length) return;
  onboardingFocusIndex = ((onboardingFocusIndex % controls.length) + controls.length) % controls.length;
  controls.forEach((control, index) => {
    control.classList.toggle('ob-pad-focus', index === onboardingFocusIndex);
  });
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
  // The right stick is the second vertical rail: it scrolls the details stats
  // and the controls of settings, import and the edit dialog, while the left
  // stick keeps its existing job. Both describe the same UP/DOWN intent.
  const ry = pad.axes[3];
  const dUp    = pad.buttons[12]?.pressed;
  const dDown  = pad.buttons[13]?.pressed;
  const dLeft  = pad.buttons[14]?.pressed;
  const dRight = pad.buttons[15]?.pressed;

  const up    = !!(dUp    || ly < -DEADZONE);
  const down  = !!(dDown  || ly >  DEADZONE);
  const right = !!(dRight || lx >  DEADZONE);
  const left  = !!(dLeft  || lx < -DEADZONE);
  const rUp   = !!(ry < -DEADZONE);
  const rDown = !!(ry >  DEADZONE);

  const bHeld = !!pad.buttons[1]?.pressed;
  const xHeld = !!pad.buttons[2]?.pressed;
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
    rb: btnPressed(pad, 5, 'RB'),
    start: btnPressed(pad, 9, 'START'),
  };

  /* All four axes are committed once per frame on every path, so edge detection
     can never drift (this is what used to leave prevAxes.up unassigned). */
  const commitAxes = () => {
    prevAxes.left = left;
    prevAxes.right = right;
    prevAxes.up = up;
    prevAxes.down = down;
    prevAxes.rUp = rUp;
    prevAxes.rDown = rDown;
  };

  // The topmost visible dialog owns the pad. A dismisses nothing and confirms
  // the focused control, B cancels it (and any in-flight hold), the d-pad walks
  // its controls, and the interface beneath it never hears any input. The
  // keyboard runs the exact same scope through navigation.js.
  const modalScope = getActiveModalScope();
  if (modalScope) {
    if (pressed.b) {
      if (actionHoldTarget) document.getElementById(actionHoldTarget)?._resetGamepadHold?.();
      dismissActiveModal(modalScope);
    }
    if ((right && !prevAxes.right) || (down && !prevAxes.down)) {
      moveFocusInScope(modalScope, right ? 'ArrowRight' : 'ArrowDown');
    }
    if ((left && !prevAxes.left) || (up && !prevAxes.up)) {
      moveFocusInScope(modalScope, left ? 'ArrowLeft' : 'ArrowUp');
    }
    if (rDown && !prevAxes.rDown) moveFocusInScope(modalScope, 'ArrowDown');
    if (rUp && !prevAxes.rUp) moveFocusInScope(modalScope, 'ArrowUp');
    if (pressed.a) {
      const active = document.activeElement;
      if (active?.matches('input[type="text"], input[type="password"], input[type="number"], textarea')) {
        openKeyboard(active);
      } else {
        active?.click?.();
      }
    }
    commitAxes();
    return;
  }

  /* The keyboard owns the pad while it is up, wherever it was opened from. */
  if (isKeyboardOpen()) {
    handleKeyboardPad({
      a: pressed.a, b: pressed.b, up, down, left, right,
      wasUp: prevAxes.up, wasDown: prevAxes.down,
      wasLeft: prevAxes.left, wasRight: prevAxes.right,
    });
    commitAxes();
    return;
  }

  /* ── 0. First run setup ──────────────────────────────────────────────────── */
  if (document.body.classList.contains('onboarding-open')) {
    const step = document.querySelector('#onboarding-card .ob-step:not(.hidden)');
    if (step) {
      const controls = onboardingControls();
      if (down && !prevAxes.down) { onboardingFocusIndex += 1; paintOnboardingFocus(); }
      if (up && !prevAxes.up) { onboardingFocusIndex -= 1; paintOnboardingFocus(); }
      if (pressed.a && controls.length) {
        const control = controls[onboardingFocusIndex % controls.length];
        if (control?.tagName === 'INPUT') openKeyboard(control);
        else control?.click();
      }
      if (pressed.b) {
        const stepNumber = step.id.replace('ob-step-', '');
        const skip = document.getElementById(`ob-skip-${stepNumber}`);
        if (skip) skip.click();
        else document.getElementById('ob-close')?.click();
      }
    }
    commitAxes();
    return;
  }

  /* ── The bar's shoulders ───────────────────────────────────────────────────
     LB/RB walk the navigation row wherever the bar is on screen, including
     over the Settings, Import and Statistics pages. They only move its cursor,
     so the d-pad, A and B keep driving the page underneath. */
  if (isNavRevealed() && (pressed.lb || pressed.rb)) {
    moveNavFocus(pressed.rb ? 1 : -1);
    commitAxes();
    return;
  }

  /* ── 1. Statistics page ──────────────────────────────────────────────────── */
  if (statsOpen) {
    // The whole page is the scope, not just the chart buttons: the tab row and
    // Back are controls the player expects to reach, and every step scrolls
    // the focused control into view. UP/DOWN walk the page in reading order so
    // each control is certain to be met; LEFT/RIGHT keep switching the tab.
    const scope = getPageNavigationScope('stats');
    const stepFocus = (dir) => {
      const controls = scope?.controls || [];
      if (!controls.length) return;
      const at = controls.indexOf(document.activeElement);
      const next = at < 0 ? (dir > 0 ? 0 : controls.length - 1) : (at + dir + controls.length) % controls.length;
      focusInView(controls[next]);
    };
    if (pressed.b) document.getElementById('btn-stats-close').click();
    if (right && !prevAxes.right) document.getElementById('btn-stats-games').click();
    if (left && !prevAxes.left) document.getElementById('btn-stats-overall').click();
    if ((up || rUp) && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; stepFocus(-1); }
    if ((down || rDown) && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; stepFocus(1); }
    if (pressed.a) {
      const active = document.activeElement;
      if (active?.matches('input[type="text"], input[type="password"], input[type="number"], textarea')) {
        openKeyboard(active);
      } else {
        (scope?.root.contains(active) ? active : scope?.controls?.[0])?.click?.();
      }
    }
    commitAxes();
    return;
  }

  /* ── 2. Settings and Import: routed pages walked like a dialog ────────────
     The d-pad moves between the page's controls, A activates the focused one
     (or opens the on-screen keyboard for a text field), and B returns to the
     library. Settings used to reach only its text fields with a pad, so its
     toggles and Back button were unreachable. */
  const pageId = currentPage();
  if (pageId === 'settings' || pageId === 'import') {
    const scope = getPageNavigationScope(pageId);
    if ((right && !prevAxes.right) || (down && !prevAxes.down)) {
      moveFocusInScope(scope, right ? 'ArrowRight' : 'ArrowDown');
    }
    if ((left && !prevAxes.left) || (up && !prevAxes.up)) {
      moveFocusInScope(scope, left ? 'ArrowLeft' : 'ArrowUp');
    }
    if (rDown && !prevAxes.rDown) moveFocusInScope(scope, 'ArrowDown');
    if (rUp && !prevAxes.rUp) moveFocusInScope(scope, 'ArrowUp');
    if (pressed.a) {
      const active = document.activeElement;
      if (active?.matches('input[type="text"], input[type="password"], input[type="number"], textarea')) {
        openKeyboard(active);
      } else {
        (scope?.root.contains(active) ? active : scope?.controls?.[0])?.click?.();
      }
    }
    if (pressed.b) showPage('home');
    commitAxes();
    return;
  }

  /* ── 3. Overlays: metadata, online art, backup manager ───────────────────── */
  if (modalOpen || metadataOpen || onlineArtOpen || backupManagerOpen || settingsOpen) {
    // A text field in the open panel can be reached with the d-pad and typed
    // into with the on-screen keyboard. Buttons stay mouse and keyboard work, as
    // before; this only closes the typing gap for a controller.
    const inputs = panelTextInputs();
    if (inputs.length) {
      const active = inputs.indexOf(document.activeElement);
      if (down && !prevAxes.down) {
        panelInputIndex = active < 0 ? 0 : Math.min(active + 1, inputs.length - 1);
        inputs[panelInputIndex].focus({ preventScroll: true });
      }
      if (up && !prevAxes.up) {
        panelInputIndex = active <= 0 ? 0 : active - 1;
        inputs[panelInputIndex].focus({ preventScroll: true });
      }
      if (pressed.a && inputs.includes(document.activeElement)) {
        openKeyboard(document.activeElement);
      }
    }
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
    if (left && !prevAxes.left) {
      if (detailsOpen && !hasDockFocus()) closeDetails();
      else focusDock(-1);
    }
    if (right && !prevAxes.right) focusDock(1);
    // Right stick: the stats are the vertical rail of the open details.
    if (rDown && !prevAxes.rDown) focusDetailsStat(1);
    if (rUp && !prevAxes.rUp) focusDetailsStat(-1);
    // The first A reveals the dock's first action instead of firing it.
    if (pressed.a) {
      if (hasDockFocus()) activateDockFocus();
      else focusDock(1);
    }
    if (games[selectedIndex]) {
      hold('btn-delete-game', xHeld);
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
  // RIGHT opens the selected game's details. Browsing remains a clean artwork
  // rail; the dock is only introduced after this explicit second-stage action.
  if (right && !prevAxes.right) openDetails();
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
