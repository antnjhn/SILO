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
const app = readFileSync(join(root, 'js', 'app.js'), 'utf8');
const gamepad = readFileSync(join(root, 'js', 'gamepad.js'), 'utf8');
const navigation = readFileSync(join(root, 'js', 'navigation.js'), 'utf8');
const library = readFileSync(join(root, 'js', 'library.js'), 'utf8');

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

describe('library input convergence', () => {
  test('keyboard and controller RIGHT both use the details transition', () => {
    assert.match(
      app,
      /case 'ArrowRight':[\s\S]*?if \(detailsOpen\) focusDock\(1\);\s*else openDetails\(\);/,
      'keyboard RIGHT no longer opens the shared details state',
    );
    assert.match(
      gamepad,
      /if \(right && !prevAxes\.right\) openDetails\(\);/,
      'controller RIGHT no longer opens the shared details state',
    );
  });

  test('keyboard arrows cannot switch games while the details dock is active', () => {
    assert.match(app, /case 'ArrowUp':[\s\S]*?if \(detailsOpen\) break;/,
      'ArrowUp can still change the selected game from details');
    assert.match(app, /case 'ArrowDown':[\s\S]*?if \(detailsOpen\) break;/,
      'ArrowDown can still change the selected game from details');
  });

  test('every top-level dialog owns its input through the shared scope', () => {
    assert.match(app, /const modalScope = getActiveModalScope\(\);[\s\S]*?trapTabInScope\(e, modalScope\);[\s\S]*?dismissActiveModal\(modalScope\);/,
      'keyboard dialog guard is not wired to the shared scope');
    assert.match(gamepad, /const modalScope = getActiveModalScope\(\);[\s\S]*?dismissActiveModal\(modalScope\);[\s\S]*?moveFocusInScope\(modalScope/,
      'controller dialog guard is not wired to the shared scope');
    assert.match(app, /import \{[\s\S]*?getActiveModalScope, getPageNavigationScope, moveFocusInScope, trapTabInScope,[\s\S]*?\} from '\.\/navigation\.js';/,
      'the keyboard handler does not import the shared navigation module');
  });

  test('focused details keep the logo prominent and the block on its centre line', () => {
    assert.ok(!/#game-list-panel\.focused \.game-item img,[\s\S]{0,160}?transform: none;/.test(css),
      'focused details still flatten the logo back to its resting scale');
    assert.match(css, /scale\(calc\(var\(--rail-scale, 1\) \* 1\.04\)\)/,
      'the focused logo no longer gets its subtle enlargement');
    assert.match(css, /#details-panel:not\(\.is-preview\) \{[\s\S]{0,300}?padding: var\(--rail-head-h, 70px\) 60px 0;/,
      'the details block is not anchored to the logo head offset');
  });

  test('launch uses the shared dock metrics and focus is an inner fill', () => {
    // Launch is a normal dock button, so these live in the shared rule once and
    // the launch-specific block stays gone rather than drifting out of step.
    const shared = css.match(/#details-actions \.action-bar \.action-dock-btn \{([^}]*)\}/)?.[1] ?? '';
    assert.match(shared, /padding: 0 14px;/, 'the shared dock padding is missing');
    assert.match(shared, /gap: 9px;/, 'the shared dock gap is missing');
    assert.match(shared, /background: transparent;/, 'the shared dock surface is missing');
    assert.ok(!/#details-actions \.action-bar #btn-launch \{\s*padding/.test(css),
      'launch still carries its own dock styling');
    assert.match(css, /#details-actions \.action-bar \.action-dock-btn::after \{\s*content: '';[\s\S]*?background: color-mix/,
      'the dock focus fill layer is missing');
    assert.ok(!/box-shadow: inset 0 0 0 2px var\(--accent\)/.test(css),
      'a ring is still used as the dock focus indicator');
  });

  test('a pointer hover lifts the dock button instead of filling it', () => {
    assert.ok(!/\.action-dock-btn:hover::after/.test(css),
      'hooking hover up to the selection fill paints the whole button');
    assert.ok(!/#details-actions \.action-dock-btn:is\(:hover/.test(css),
      'hover still swaps the label to the selection ink');
    assert.match(css, /#details-actions \.action-bar \.action-dock-btn:hover,\s*\n#details-actions \.action-bar \.dock-fav:hover \{ background: rgba\(255, 255, 255, 0\.09\); \}/,
      'the hover lift off the bar is missing');
  });

  test('browsing state hides details controls and the instruction strip names keyboard controls', () => {
    assert.match(css, /#details-panel\.is-preview #details-actions,[\s\S]*?opacity:\s*0;/,
      'browsing still exposes the action dock');
    const instructions = html.slice(idAt('details-actions'), html.indexOf('id="btn-delete-game"'));
    assert.match(instructions, /<kbd>Enter<\/kbd>/, 'keyboard Select hint is missing');
    assert.match(instructions, /<kbd>Esc<\/kbd>/, 'keyboard Back hint is missing');
    assert.match(css, /#details-actions > \.action-bar \{ grid-column: 1; grid-row: 1; \}[\s\S]*#details-actions > \.dock-delete \{ grid-column: 2; grid-row: 1; \}/,
      'dock controls no longer keep their shared row');
    assert.match(css, /\.controller-instructions \{[^}]*grid-column: 1;[^}]*grid-row: 2;/,
      'control instructions no longer sit beneath the dock');
  });
});

describe('launch arrow', () => {
  test('the bar glyph is a solid arrow, not an outline', () => {
    assert.match(
      css,
      /#details-actions \.action-bar #btn-launch \.dock-launch-arrow svg \{[^}]*fill: currentColor;[^}]*stroke: none;/,
      'the launch arrow is still drawn as an outline',
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

describe('shared modal and page navigation', () => {
  test('keyboard and gamepad use the same active-scope focus helpers', () => {
    assert.match(navigation, /export function getActiveModalScope/);
    assert.match(navigation, /export function moveFocusInScope/);
    assert.match(navigation, /export function dismissActiveModal/);
    assert.match(app, /getActiveModalScope\(\)/);
    assert.match(gamepad, /getActiveModalScope\(\)/);
  });

  test('Escape and controller B dismiss active dialogs before page navigation', () => {
    assert.match(app, /dismissActiveModal\(modalScope\)/);
    assert.match(gamepad, /dismissActiveModal\(modalScope\)/);
    assert.match(navigation, /btn-backup-manager-close/);
    assert.match(navigation, /btn-online-art-cancel/);
    assert.match(navigation, /target\._siloOpener \|\| modalOpeners\.get\(target\)/,
      'a controller-opened dialog loses its opener on dismissal');
  });

  test('settings and import pages walk the same controls on keyboard and pad', () => {
    assert.match(app, /getPageNavigationScope\(pageId\)/);
    assert.match(gamepad, /getPageNavigationScope\(pageId\)/);
    assert.match(app, /moveFocusInScope\(pageScope, e\.key\)/);
    assert.match(gamepad, /moveFocusInScope\(scope, right \? 'ArrowRight' : 'ArrowDown'\)/);
    assert.match(gamepad, /if \(pressed\.b\) showPage\('home'\);/);
  });

  test('Enter in a dialog text field confirms the dialog primary action', () => {
    assert.match(navigation, /btn-backup-name-ok/);
    assert.match(navigation, /export function submitActiveModal/);
    assert.match(app, /submitActiveModal\(modalScope\)/);
  });

  test('holding X paints a progress ring instead of opening the delete dialog', () => {
    assert.match(css, /#btn-delete-game\.holding::after \{[\s\S]*?conic-gradient/);
    assert.match(library, /setupHoldButton\('btn-delete-game', 900/);
    assert.match(library, /--hold-progress/);
    assert.match(gamepad, /hold\('btn-delete-game', xHeld\)/);
  });

  test('a mouse click opens the removal dialog; only pad X needs a hold', () => {
    assert.ok(!/event\.detail !== 0/.test(library), 'mouse clicks are still swallowed by the hold guard');
    assert.match(library, /getElementById\('uninstall-overlay'\)[\s\S]{0,160}?classList\.contains\('hidden'\)/,
      'the dialog can be re-opened while it is already up');
  });

  test('the right stick walks the stats and the settings/edit controls', () => {
    assert.match(gamepad, /const ry = pad\.axes\[3\];/, 'the right stick is not read');
    assert.match(gamepad, /if \(rDown && !prevAxes\.rDown\) focusDetailsStat\(1\);/,
      'the right stick does not reach the details stats');
    assert.match(gamepad, /if \(rDown && !prevAxes\.rDown\) moveFocusInScope\(scope, 'ArrowDown'\);/,
      'the right stick does not scroll the settings/import page');
    assert.match(gamepad, /if \(rDown && !prevAxes\.rDown\) moveFocusInScope\(modalScope, 'ArrowDown'\);/,
      'the right stick does not scroll the edit dialog');
    assert.match(library, /export function focusDetailsStat/, 'the stats focus helper is missing');
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

describe('controller navigation and the on-screen keyboard', () => {
  const osk = readFileSync(join(here, 'osk.js'), 'utf8');

  test('the top bar walks with LB and RB while it is up', () => {
    assert.match(gamepad, /rb: btnPressed\(pad, 5, 'RB'\)/, 'RB is never sampled');
    // The shoulder rule has to run before the page branches, or the shoulders
    // would do nothing while Settings or Import is open.
    const shoulders = gamepad.indexOf('isNavRevealed() && (pressed.lb || pressed.rb)');
    const pages = gamepad.indexOf("pageId === 'settings' || pageId === 'import'");
    assert.ok(shoulders > 0, 'the shoulders are not wired to the navigation cursor');
    assert.ok(shoulders < pages, 'the shoulders are consumed by the page before the bar sees them');
    assert.match(gamepad, /if \(isNavRevealed\(\) && \(pressed\.lb \|\| pressed\.rb\)\) \{\s*moveNavFocus\(pressed\.rb \? 1 : -1\);/,
      'the shoulders do not move the navigation cursor');
  });

  test('page focus steps bring the control into view', () => {
    assert.match(
      navigation,
      /function focusInView\(element\) \{\s*element\.focus\(\{ preventScroll: true \}\);\s*element\.scrollIntoView\(\{ block: 'nearest', inline: 'nearest' \}\);/,
      'a pad step can land on a control that is off screen',
    );
  });

  test('the on-screen keyboard keeps the field it edits above itself', () => {
    assert.match(osk, /setProperty\('--osk-h'/, 'the keyboard height is not published');
    assert.match(osk, /requestAnimationFrame\(keepInputVisible\)/, 'the field is never scrolled clear');
    assert.match(css, /body\.osk-open \.page \{[^}]*padding-bottom: calc\(var\(--osk-h, 280px\) \+ 32px\)/,
      'the page reserves no room for the keyboard');
  });

  test('the statistics page is walked and activated with the pad', () => {
    const stats = gamepad.slice(gamepad.indexOf('if (statsOpen)'), gamepad.indexOf('if (statsOpen)') + 1600);
    assert.match(stats, /getPageNavigationScope\('stats'\)/, 'the statistics page has no focus scope');
    assert.match(stats, /const stepFocus = \(dir\) => \{[\s\S]{0,400}?focusInView\(controls\[next\]\)/, 'the pad cannot walk the statistics page');
    assert.match(stats, /if \(pressed\.a\)[\s\S]{0,300}?\.click\?\.\(\)/, 'A cannot activate a statistics control');
  });

  test('the bar shows the shoulder keys only for a controller', () => {
    assert.match(html, /class="topnav-shoulder topnav-shoulder-left"[^>]*>LB</, 'the LB hint is missing');
    assert.match(html, /class="topnav-shoulder topnav-shoulder-right"[^>]*>RB</, 'the RB hint is missing');
    assert.match(css, /body\.gamepad-active #topnav\.revealed \.topnav-shoulder \{ display: inline-flex; \}/,
      'the shoulder hints are not tied to a connected controller');
    assert.match(css, /body\.gamepad-active \.page :is\(button, input, select, a\[href\]\):focus \{/,
      'pad focus on a page has no visible mark');
  });
});
