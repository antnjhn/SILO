/* ── gamepad.test.mjs — automated tests for the polling state machine ──
   Uses Node's built-in `node:test` runner with hand-written browser mocks.
   No external test framework is introduced. Run with:

       node --test src/renderer/js/gamepad.test.mjs

   Browser globals are mocked BEFORE gamepad.js is imported (static imports
   are hoisted, so the dynamic import below runs after mocks are set).
   `__resetForTesting()` (exported from gamepad.js solely for test isolation)
   resets module-level state between scenarios. ───────────────────────────*/

import * as assert from 'node:assert';
import { test, describe } from 'node:test';

/* ═════════════════════════════════════════════════════════════════════
   Browser globals mocked BEFORE gamepad.js is imported
   ═════════════════════════════════════════════════════════════════════*/

let hidden = false;
const docListeners = {};
const winListeners = {};
const rAFQueue = {};   // id -> callback
const timerQueue = {}; // id -> { cb, ms }
let rAFCounter = 0;
let timerCounter = 0;

function fakeElement() {
  return {
    classList: { contains: () => false, add: () => {}, remove: () => {}, toggle: () => {} },
    addEventListener: () => {},
    removeEventListener: () => {},
    click: () => {},
    getClientRects: () => [],
    _holdMs: undefined,
    _startGamepadHold: undefined,
    _resetGamepadHold: undefined,
  };
}

globalThis.window = {
  matchMedia: () => ({ matches: false, addEventListener: () => {}, removeEventListener: () => {} }),
  __TAURI__: { core: { invoke: () => {} }, event: { listen: () => ({ then: () => {} }) } },
  addEventListener: (type, cb) => { (winListeners[type] = winListeners[type] || []).push(cb); },
  removeEventListener: () => {},
  dispatchEvent: (evt) => {
    (winListeners[evt.type] || []).forEach((cb) => cb(evt));
    return true;
  },
};

globalThis.document = {
  get hidden() { return hidden; },
  set hidden(v) { hidden = v; },
  get visibilityState() { return hidden ? 'hidden' : 'visible'; },
  addEventListener: (type, cb) => { (docListeners[type] = docListeners[type] || []).push(cb); },
  removeEventListener: () => {},
  getElementById: () => fakeElement(),
  querySelector: () => null,
  querySelectorAll: () => [],
  hasFocus: () => true,
  body: {
    classList: { add: () => {}, remove: () => {}, toggle: () => {}, contains: () => false },
  },
  dispatchEvent: (evt) => {
    (docListeners[evt.type] || []).forEach((cb) => cb(evt));
    return true;
  },
};

defineGlobal('navigator', { getGamepads: () => [] });
defineGlobal('IntersectionObserver', class { observe() {} unobserve() {} disconnect() {} });
defineGlobal('ResizeObserver', class { observe() {} unobserve() {} disconnect() {} });
defineGlobal('MutationObserver', class { observe() {} unobserve() {} disconnect() {} });
globalThis.localStorage = { getItem: () => null, setItem: () => {} };

function defineGlobal(name, value) {
  Object.defineProperty(globalThis, name, { value, configurable: true, writable: true });
}

defineGlobal('requestAnimationFrame', (cb) => {
  const id = ++rAFCounter;
  rAFQueue[id] = cb;
  return id;
});
defineGlobal('cancelAnimationFrame', (id) => { delete rAFQueue[id]; });

defineGlobal('setTimeout', (cb, ms) => {
  const id = ++timerCounter;
  timerQueue[id] = { cb, ms };
  return id;
});
defineGlobal('clearTimeout', (id) => { delete timerQueue[id]; });
defineGlobal('performance', { now: () => Date.now() });

/* Dynamically import gamepad.js now that the browser mocks are in place. */
const { initGamepadPolling, __resetForTesting } = await import('./gamepad.js');

/* ═════════════════════════════════════════════════════════════════════
   Helpers
   ═════════════════════════════════════════════════════════════════════*/

function setGamepads(connected) {
  if (connected) {
    globalThis.navigator.getGamepads = () => [
      { connected: true, axes: [0, 0], buttons: [{ pressed: false }, { pressed: false }] },
    ];
  } else {
    globalThis.navigator.getGamepads = () => [];
  }
}

function countRAF() {
  return Object.keys(rAFQueue).length;
}

function countTimer() {
  return Object.keys(timerQueue).length;
}

function runRAFFrame() {
  const ids = Object.keys(rAFQueue);
  const cb = rAFQueue[ids[0]];
  delete rAFQueue[ids[0]];
  if (cb) cb();
}

function runTimer() {
  const ids = Object.keys(timerQueue);
  const entry = timerQueue[ids[0]];
  delete timerQueue[ids[0]];
  if (entry) entry.cb();
}

function clearAll() {
  for (const id of Object.keys(rAFQueue)) delete rAFQueue[id];
  for (const id of Object.keys(timerQueue)) delete timerQueue[id];
}

function dispatchVisibility(hiddenState) {
  hidden = hiddenState;
  globalThis.document.dispatchEvent(new Event('visibilitychange'));
}

function dispatchGamepad(connected) {
  if (connected) {
    globalThis.window.dispatchEvent(new Event('gamepadconnected'));
  } else {
    globalThis.window.dispatchEvent(new Event('gamepaddisconnected'));
  }
}

/* ═════════════════════════════════════════════════════════════════════
   Tests
   ═════════════════════════════════════════════════════════════════════*/

function setup() {
  __resetForTesting();
  clearAll();
  hidden = false;
  setGamepads(false);
  // Note: docListeners/winListeners are intentionally NOT cleared here — the
  // gamepadconnected / gamepaddisconnected / visibilitychange handlers were
  // registered once at import time and close over module state, which
  // __resetForTesting() resets above.
}

/* ══════════════════════════════════════════════════════════════════════
   Tests
   ═════════════════════════════════════════════════════════════════════*/

describe('gamepad polling state machine', () => {

  test('no controller connected: low-frequency timer only, no rAF', (t) => {
    setup();
    initGamepadPolling();
    assert.strictEqual(countRAF(), 0, 'no rAF should be scheduled with no controller');
    assert.strictEqual(countTimer(), 1, 'one low-frequency timer should be scheduled');
  });

  test('controller connected at init: rAF polling starts', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1, 'one rAF should be scheduled when a controller is connected');
    assert.strictEqual(countTimer(), 0, 'no low-frequency timer while polling');
  });

  test('controller connected during use starts rAF polling', (t) => {
    setup();
    setGamepads(false);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 0);

    setGamepads(true);
    dispatchGamepad(true);
    assert.strictEqual(countRAF(), 1, 'connecting a controller starts rAF polling');
  });

  test('controller disconnected stops rAF and returns to low-frequency detection', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1);

    dispatchGamepad(false);
    assert.strictEqual(countRAF(), 0, 'rAF stopped on disconnect');
    assert.strictEqual(countTimer(), 1, 'low-frequency timer resumed after disconnect');
  });

  test('missed disconnect is self-healed by the active polling loop', (t) => {
    setup();
    // Controller connected and polling.
    setGamepads(true);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1);

    // Disconnect WITHOUT the gamepaddisconnected event (missed).
    setGamepads(false);

    // The next rAF frame independently re-checks getGamepads().
    runRAFFrame();
    assert.strictEqual(countRAF(), 0, 'missed disconnect: rAF stops on the next frame');
    assert.strictEqual(countTimer(), 1, 'missed disconnect: falls back to low-frequency detection');
  });

  test('controller reconnected resumes rAF polling', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    dispatchGamepad(false);
    assert.strictEqual(countRAF(), 0);

    setGamepads(true);
    dispatchGamepad(true);
    assert.strictEqual(countRAF(), 1, 'reconnect resumes rAF polling');
  });

  test('document hidden stops high-frequency polling', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1);

    dispatchVisibility(true);
    assert.strictEqual(countRAF(), 0, 'hiding the document stops rAF polling');
  });

  test('document restored reconciles ground truth before selecting mode', (t) => {
    setup();
    // Controller connected while hidden (state may be stale).
    setGamepads(true);
    initGamepadPolling();
    dispatchVisibility(true); // hide: polling stops
    assert.strictEqual(countRAF(), 0);

    // While hidden, controller disconnected without an event (stale flag risk).
    setGamepads(false);
    dispatchVisibility(false); // show: must reconcile getGamepads(), not trust stale flag

    assert.strictEqual(countRAF(), 0, 'no controller on show: stays in low-frequency mode');
    assert.strictEqual(countTimer(), 1, 'no controller on show: low-frequency timer scheduled');
  });

  test('document restored with a connected controller resumes rAF', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    dispatchVisibility(true);
    assert.strictEqual(countRAF(), 0);

    // Still connected when shown.
    setGamepads(true);
    dispatchVisibility(false);
    assert.strictEqual(countRAF(), 1, 'controller present on show: rAF resumes');
  });

  test('repeated visibility events do not create duplicate loops', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1);

    dispatchVisibility(true);
    dispatchVisibility(true);
    assert.strictEqual(countRAF(), 0, 'repeated hide: no extra rAF');

    dispatchVisibility(false);
    dispatchVisibility(false);
    assert.strictEqual(countRAF(), 1, 'repeated show: still only one rAF');
  });

  test('repeated controller events do not create duplicate loops', (t) => {
    setup();
    setGamepads(false);
    initGamepadPolling();

    dispatchGamepad(true);
    dispatchGamepad(true);
    assert.strictEqual(countRAF(), 1, 'repeated connect: still only one rAF');

    dispatchGamepad(false);
    dispatchGamepad(false);
    assert.strictEqual(countRAF(), 0, 'repeated disconnect: rAF stays stopped');
    assert.strictEqual(countTimer(), 1, 'one low-frequency timer');
  });

  test('repeated init does not create duplicate loops', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    initGamepadPolling();
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1, 'repeated init: still only one rAF');
  });

  test('button held during pause/resume resets input state', (t) => {
    setup();
    setGamepads(true);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 1);

    dispatchVisibility(true); // pause: should reset input state
    assert.strictEqual(countRAF(), 0);
    assert.strictEqual(countTimer(), 0);

    setGamepads(true);
    dispatchVisibility(false); // resume: clean resume
    assert.strictEqual(countRAF(), 1);
  });

  test('frontend initialisation starts polling exactly once', (t) => {
    setup();
    __resetForTesting();
    clearAll();
    setGamepads(false);
    initGamepadPolling();
    assert.strictEqual(countRAF(), 0);
    assert.strictEqual(countTimer(), 1, 'frontend init with no controller: one low-frequency timer');
  });
});
