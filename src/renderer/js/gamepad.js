/* ── gamepad.js — poll loop, input mapping, rumble ─────────────────────────── */
import { games, selectedIndex } from './state.js';
import {
  getCategories, changeCategorySelection, openCategories, closeCategories,
  selectGame, railEnterGame, railExitGame, closeDetails, openDetails, doLaunch,
  categorySelectedIndex, categoriesOpen, detailsOpen, railEntered, actionHoldTarget,
} from './library.js';
import { openAddModal } from './modals.js';
import { navigateStatsFocus } from './stats.js';
import { revealNavFromController, isNavRevealed } from './nav.js';
import { showPage } from './pages.js';

// Gamepad
const DEADZONE = 0.4;
const NAV_REPEAT = 175;
let lastNavTime = 0;
let prevAxes = { right: false, left: false, up: false, down: false };
let buttonWas = {};

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
/* ── Gamepad ───────────────────────────────────────────────────────────────*/
export function pollGamepad() {
  const pads = navigator.getGamepads?.() || [];
  const pad = Array.from(pads).find(p => p?.connected);
  if (pad) {
    document.body.classList.add('gamepad-active');
    handlePad(pad);
  } else {
    document.body.classList.remove('gamepad-active');
  }
  requestAnimationFrame(pollGamepad);
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

  const up    = dUp    || ly < -DEADZONE;
  const down  = dDown  || ly >  DEADZONE;
  const right = dRight || lx >  DEADZONE;
  const left  = dLeft  || lx < -DEADZONE;

  if (statsOpen) {
    if (btnPressed(pad, 1, 'B')) document.getElementById('btn-stats-close').click();
    if (right && !prevAxes.right) document.getElementById('btn-stats-games').click();
    if (left && !prevAxes.left) document.getElementById('btn-stats-overall').click();
    if (up && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; navigateStatsFocus(-1); }
    if (down && now - lastNavTime > NAV_REPEAT) { lastNavTime = now; navigateStatsFocus(1); }
    prevAxes.right = right; prevAxes.left = left;
    savePad(pad); return;
  }

  if (modalOpen || metadataOpen || onlineArtOpen || backupManagerOpen || settingsOpen) {
    // The import and settings panels are pages now, so the controller needs a
    // way out of them: B goes back to the library, like it does on the
    // statistics page.
    if ((modalOpen || settingsOpen) && btnPressed(pad, 1, 'B')) {
      showPage('home');
    }
    savePad(pad); return;
  }

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
    if ((right && !prevAxes.right) || btnPressed(pad, 0, 'A')) { closeCategories(true); }
    if (btnPressed(pad, 1, 'B')) { closeCategories(false); }
    prevAxes.right = right; prevAxes.left = left;
    savePad(pad); return;
  }

  // Two levels: d-pad / stick UP+DOWN browse the rail (hold to scrub), RIGHT
  // steps into the focused game so its buttons come out, LEFT steps back out.
  if (!detailsOpen) {
    // An upward push reveals the navigation when it is hidden. The rail is
    // only browsed while the navigation is already up, so one press never
    // does two things at once.
    if (up && now - lastNavTime > NAV_REPEAT) {
      if (!isNavRevealed()) {
        revealNavFromController();
        lastNavTime = now;
      } else if (selectedIndex > 0) {
        lastNavTime = now;
        selectGame(selectedIndex - 1);
        vibrateVertical();
      }
    }
    if (down  && now - lastNavTime > NAV_REPEAT && selectedIndex < games.length - 1) { lastNavTime = now; selectGame(selectedIndex + 1); vibrateVertical(); }
    if (right && !prevAxes.right) railEnterGame();
    // LEFT steps back out of a game first; otherwise it opens the category
    // wheel, the way the keyboard's LEFT does.
    if (left && !prevAxes.left) {
      if (railEntered) railExitGame();
      else openCategories();
    }
    if (btnPressed(pad, 4, 'LB')) openCategories();
  } else if (left && !prevAxes.left) {
    closeDetails();
  }

  prevAxes.right = right; prevAxes.left = left;

  const bPressed = !!pad.buttons[1]?.pressed;
  const xPressed = !!pad.buttons[2]?.pressed;
  const yPressed = !!pad.buttons[3]?.pressed;

  // The direct action buttons work inside the game — full details or stepped in.
  if ((detailsOpen || railEntered) && games[selectedIndex]) {
    const handleGamepadHold = (isPressed, btnId) => {
      const btn = document.getElementById(btnId);
      if (!btn) return;
      if (isPressed) {
        if (!actionHoldTarget && btn._startGamepadHold) btn._startGamepadHold();
      } else {
        if (actionHoldTarget === btnId && btn._resetGamepadHold) btn._resetGamepadHold();
      }
    };
    
    handleGamepadHold(bPressed, 'btn-launch');
    if (btnPressed(pad, 2, 'X')) document.getElementById('btn-delete-game').click();
    handleGamepadHold(yPressed, 'btn-edit-game');
  } else {
    // If not in details, Y opens add
    if (btnPressed(pad, 3, 'Y') && !detailsOpen) openAddModal();
    if (btnPressed(pad, 0, 'A') && !detailsOpen && games.length) openDetails();
  }

  savePad(pad);
}

function btnPressed(pad, idx, key) {
  const cur = !!pad.buttons[idx]?.pressed;
  const prev = !!buttonWas[key];
  buttonWas[key] = cur;
  return cur && !prev;
}

function savePad(pad) {
  pad.buttons.forEach((b, i) => { buttonWas['b' + i] = b.pressed; });
}
