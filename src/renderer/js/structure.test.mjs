/* ── structure.test.mjs — the dock and settings skeleton, checked ────────────
   Bugs in this area have all looked the same: an element moved out of the
   container its CSS targets, a duplicate id, a rule that could never win, a
   button that quietly lost its class. None of them need a browser to catch, so
   these read index.html and style.css and assert the structure the styles
   depend on. Run with:

       node --test src/renderer/js/structure.test.mjs
   ───────────────────────────────────────────────────────────────────────────*/

import * as assert from 'node:assert';
import { test, describe } from 'node:test';
import { readFileSync } from 'node:fs';
import { fileURLToPath } from 'node:url';
import { dirname, join } from 'node:path';

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, '..');
const html = readFileSync(join(root, 'index.html'), 'utf8');
const css = readFileSync(join(root, 'style.css'), 'utf8');

function idAt(id) {
  const index = html.indexOf(`id="${id}"`);
  assert.ok(index >= 0, `#${id} is missing from index.html`);
  return index;
}

function countId(id) {
  return html.split(`id="${id}"`).length - 1;
}

/* The bar's own span in the document, found by walking div depth from its
   opening tag. Assertions about what is inside the bar need real bounds, not
   the next id that happens to appear. */
function barBounds() {
  const start = html.indexOf('<div class="action-bar"');
  assert.ok(start > 0, 'the compact action bar exists');
  const tags = /<\/?div\b/g;
  tags.lastIndex = start;
  let depth = 0;
  let match;
  while ((match = tags.exec(html))) {
    depth += match[0] === '<div' ? 1 : -1;
    if (depth === 0) return { start, end: match.index };
  }
  throw new Error('the action bar is never closed');
}

const bar = barBounds();
const detailsStart = html.indexOf('id="details-actions"');

describe('the dock', () => {
  test('the action bar holds the dock actions', () => {
    for (const id of [
      'btn-launch',
      'btn-edit-game',
      'btn-restore-save',
      'btn-backup-save',
      'btn-toggle-favorite',
    ]) {
      const index = idAt(id);
      assert.ok(index > bar.start && index < bar.end, `#${id} is outside the action bar`);
    }
  });

  test('delete sits outside the bar, in the same row', () => {
    const index = idAt('btn-delete-game');
    assert.ok(detailsStart > 0 && detailsStart < bar.start, 'the row starts before the bar');
    assert.ok(index > bar.end, 'delete is inside the action bar, which styles it as one of them');
  });

  test('no dock id is declared twice', () => {
    for (const id of [
      'btn-launch',
      'btn-edit-game',
      'btn-restore-save',
      'btn-backup-save',
      'btn-toggle-favorite',
      'btn-delete-game',
    ]) {
      assert.equal(countId(id), 1, `#${id} is declared more than once`);
    }
  });

  test('the launch arrow leaves its weight to CSS', () => {
    const start = idAt('btn-launch');
    const block = html.slice(start, html.indexOf('</button>', start));
    assert.ok(/dock-launch-arrow/.test(block), 'the arrow wrapper is inside the button');
    assert.ok(!/stroke-width=/.test(block), 'the arrow carries an inline stroke width again');
  });
});

describe('launch arrow weight', () => {
  test('one declaration, in the bar rule that overrides the shared weight', () => {
    const declarations = css.match(/stroke-width:\s*2\.6/g) || [];
    assert.equal(declarations.length, 1, 'the launch weight is declared more than once');
    assert.match(
      css,
      /#details-actions \.action-bar #btn-launch \.dock-launch-arrow svg \{[^}]*stroke-width: 2\.6;/,
      'the surviving declaration is not the bar override',
    );
  });

  test('the bar still sets one weight for the icons beside it', () => {
    assert.match(
      css,
      /#details-actions \.action-bar \.action-icon-tile svg \{[^}]*stroke-width: 1\.7;/,
      'the shared icon weight is missing',
    );
  });
});

describe('settings', () => {
  const advanced = html.indexOf('id="settings-advanced"');
  const about = html.indexOf('class="set-about"');

  test('the snapshot picker and its button live in Advanced', () => {
    for (const id of ['snapshot-block', 'snapshot-select', 'btn-restore-snapshot']) {
      const index = idAt(id);
      assert.ok(index > advanced && index < about, `#${id} is outside the Advanced block`);
    }
  });

  test('Restore is the same button as Export and Import', () => {
    const start = idAt('btn-restore-snapshot');
    const tag = html.slice(html.lastIndexOf('<button', start), html.indexOf('>', start));
    assert.match(tag, /class="settings-btn"/, 'Restore lost the shared button class');
    assert.ok(!/settings-ghost/.test(tag), 'Restore is still a ghost control');
  });

  test('each toggle is a direct child of its item', () => {
    assert.match(
      html,
      /<div class="set-item">\s*<label class="settings-toggle-row" for="input-show-game-titles">/,
      'Show game titles is nested too deep for the alignment rule',
    );
    assert.match(
      html,
      /<div class="set-item">\s*<label class="settings-toggle-row" for="input-force-motion">/,
      'Always animate is nested too deep for the alignment rule',
    );
    assert.match(
      html,
      /<div class="set-item set-item-stack">\s*<label class="settings-toggle-row" for="input-discovery-enabled">/,
      'Scan folders at startup is nested too deep for the alignment rule',
    );
  });

  test('the alignment rule and the header hierarchy survive', () => {
    assert.match(
      css,
      /#settings-modal \.set-item > \.settings-toggle-row \{ width: 100%; \}/,
      'the toggle alignment rule is gone',
    );
    assert.match(
      css,
      /#settings-modal \.settings-head \.settings-title \{[^}]*font-size: 27px;[^}]*font-weight: 700;/,
      'the settings title hierarchy is gone',
    );
  });
});

describe('ids the scripts ask for', () => {
  test('every id a script depends on exists exactly once', () => {
    const required = [
      'settings-modal', 'details-panel', 'details-actions', 'onboarding-card',
      'btn-launch', 'btn-edit-game', 'btn-delete-game', 'btn-settings',
      'snapshot-select', 'btn-restore-snapshot', 'btn-check-updates',
      'btn-verify-backups', 'input-sgdb-key', 'input-name',
      'input-show-game-titles', 'input-force-motion',
    ];
    for (const id of required) {
      assert.equal(countId(id), 1, `#${id} is missing or duplicated`);
    }
  });

  test('the removed update toggle is not still in the markup', () => {
    assert.equal(countId('input-check-updates'), 0, 'the auto update toggle is back');
  });
});
