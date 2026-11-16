/* ── osk.js — on-screen keyboard for controller-only setup ───────────────────
   First-run setup can be walked with a pad, but the one field it asks for is a
   SteamGridDB key, and a pad cannot type one. This is the smallest keyboard that
   makes that step possible: letters, digits, the punctuation a key or a path
   holds, backspace and done. It is driven by the same d-pad the rest of setup
   uses, and works with a mouse because it is just buttons. */

const ROWS = [
  ['a', 'b', 'c', 'd', 'e', 'f', 'g', 'h', 'i', 'j'],
  ['k', 'l', 'm', 'n', 'o', 'p', 'q', 'r', 's', 't'],
  ['u', 'v', 'w', 'x', 'y', 'z', '0', '1', '2', '3'],
  ['4', '5', '6', '7', '8', '9', '-', '_', '.', '/'],
  ['@', ':', 'BACKSPACE', 'SPACE', 'DONE'],
];

const COLUMNS = 10;

let state = null;

export function isKeyboardOpen() {
  return !!state;
}

function keyLabel(key) {
  if (key === 'BACKSPACE') return 'Delete';
  if (key === 'SPACE') return 'Space';
  if (key === 'DONE') return 'Done';
  return key;
}

function buildPanel() {
  const overlay = document.createElement('div');
  overlay.className = 'osk-overlay';
  overlay.setAttribute('role', 'dialog');
  overlay.setAttribute('aria-label', 'On-screen keyboard');

  const panel = document.createElement('div');
  panel.className = 'osk-panel';

  const keys = [];
  ROWS.forEach((row) => {
    const rowEl = document.createElement('div');
    rowEl.className = 'osk-row';
    row.forEach((key) => {
      const button = document.createElement('button');
      button.type = 'button';
      button.className = 'osk-key';
      if (key.length > 1) button.classList.add('osk-key-wide');
      button.textContent = keyLabel(key);
      button.dataset.key = key;
      button.addEventListener('click', () => press(key));
      rowEl.append(button);
      keys.push(button);
    });
    // The last row is shorter; it keeps its own buttons aligned to the grid.
    if (row.length < COLUMNS) rowEl.classList.add('osk-row-last');
    panel.append(rowEl);
  });

  const hint = document.createElement('p');
  hint.className = 'osk-hint';
  hint.textContent = 'D-pad moves, A types, B closes.';
  panel.append(hint);

  overlay.append(panel);
  return { overlay, panel, keys };
}

function paint() {
  if (!state) return;
  const { keys, index } = state;
  keys.forEach((button, position) => {
    const on = position === index;
    button.classList.toggle('osk-focus', on);
    if (on) button.setAttribute('aria-current', 'true');
    else button.removeAttribute('aria-current');
  });
  keys[index]?.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

function move(delta) {
  if (!state) return;
  const count = state.keys.length;
  state.index = ((state.index + delta) % count + count) % count;
  paint();
}

function press(key) {
  if (!state) return;
  const { input } = state;
  if (key === 'DONE') {
    closeKeyboard();
    return;
  }
  const start = input.selectionStart ?? input.value.length;
  const end = input.selectionEnd ?? start;
  if (key === 'BACKSPACE') {
    if (start === end && start === 0) return;
    const from = start === end ? start - 1 : start;
    input.value = input.value.slice(0, from) + input.value.slice(end);
    input.selectionStart = input.selectionEnd = from;
  } else {
    const text = key === 'SPACE' ? ' ' : key;
    input.value = input.value.slice(0, start) + text + input.value.slice(end);
    input.selectionStart = input.selectionEnd = start + text.length;
  }
  // The rest of the app listens for typing, not for value changes made in code.
  input.dispatchEvent(new Event('input', { bubbles: true }));
}

export function openKeyboard(input) {
  if (!input) return;
  closeKeyboard();
  const parts = buildPanel();
  document.body.append(parts.overlay);
  document.body.classList.add('osk-open');
  state = { input, index: 0, ...parts };
  paint();
}

export function closeKeyboard() {
  if (!state) return;
  const { overlay, input } = state;
  overlay.remove();
  document.body.classList.remove('osk-open');
  state = null;
  input?.focus({ preventScroll: true });
}

/// Routes one frame of pad input while the keyboard is up. The caller owns edge
/// detection, so this only has to act on events it is handed.
export function handleKeyboardPad(events) {
  if (!state) return;
  const { keys, index } = state;
  if (events.a) press(keys[index].dataset.key);
  if (events.b) closeKeyboard();
  if (!events.a && !events.b) {
    if (events.left && !events.wasLeft) move(-1);
    if (events.right && !events.wasRight) move(1);
    if (events.up && !events.wasUp) move(-COLUMNS);
    if (events.down && !events.wasDown) move(COLUMNS);
  }
}
