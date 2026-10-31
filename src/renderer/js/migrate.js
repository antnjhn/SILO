/* ── migrate.js — "this library came from another PC" ───────────────────────
   One job, two steps. A library made on another machine keeps save folders that
   point at paths that cannot exist here, so this flow points each game at where
   its saves live on this PC (relink), then copies saves in for the games that
   have nowhere here to point (bring across).

   It used to be three separate buttons in Settings. It is now offered once,
   right after a library restore puts another PC's paths in the library, and can
   be reopened from Advanced. The per-game version of the same idea lives in the
   Edit dialog's "Find on this PC".

   Nothing in either step deletes or moves anything. */
import { showToast } from './utils.js';
import { showPage } from './pages.js';

const OVERLAY = 'migrate-overlay';
// Set before a library restore reloads the window, read once on the next boot.
const PENDING_KEY = 'silo.setupFromAnotherPc';

/* ── The modal ──────────────────────────────────────────────────────────────*/

export function openMigrateModal() {
  const overlay = document.getElementById(OVERLAY);
  if (!overlay) return;
  setStatus('');
  overlay.classList.remove('hidden');
}

export function closeMigrateModal() {
  document.getElementById(OVERLAY)?.classList.add('hidden');
}

function setStatus(text) {
  const el = document.getElementById('migrate-status');
  if (el) el.textContent = text || '';
}

// Ask for the setup step on the next boot. Called just before a restore reloads
// the window, so the user is met with the flow rather than a library full of
// paths from the machine they left.
export function markSetupPending() {
  try {
    localStorage.setItem(PENDING_KEY, '1');
  } catch {
    // A blocked localStorage is not worth failing a restore over.
  }
}

// Offer the setup once, the first time the app boots after a restore.
export function offerSetupIfPending() {
  let pending = null;
  try {
    pending = localStorage.getItem(PENDING_KEY);
    if (pending) localStorage.removeItem(PENDING_KEY);
  } catch {
    return;
  }
  if (pending) openMigrateModal();
}

/* ── What could not be resolved, named rather than counted ──────────────────*/

// A relink and a bring-across both end with games the user has to sort out by
// hand; the heading differs only in what was attempted.
const RELINK_LEAD = 'Their save folders could not be matched to anything on this machine,\nso they need setting by hand in Edit:';
const BRING_ACROSS_LEAD = 'Their saves could not be brought across, so their save folders still need\nsetting by hand in Edit:';

// The games the survey could not place, with the reason it found for each one —
// a count alone leaves the user with no idea where the saves actually are.
function unresolvedReport(issues, lead = RELINK_LEAD) {
  const lines = issues.slice(0, 8).map(i => {
    const where = i.installPresent ? '' : ' (not installed on this machine)';
    // Only a match SILO would not act on by itself carries a marker saying why:
    // a folder matched by name alone, or one found in a prefix that is not this
    // game's own.
    const how = i.confidence === 'guess' ? ' — only the name matched'
      : i.confidence === 'likely' ? ' — prefix not tied to this game' : '';
    return `• ${i.gameName}${where}${how}\n    ${i.reason}`;
  });
  if (issues.length > 8) lines.push(`…and ${issues.length - 8} more.`);
  return `${lead}\n\n${lines.join('\n\n')}`;
}

// Shown after a step, so the games that still need attention are named rather
// than only counted. Skipped when the dialog plugin has no message() to call.
async function showUnresolved(issues, lead = RELINK_LEAD) {
  if (!issues.length) return;
  const { message } = window.__TAURI__.dialog;
  if (!message) return;
  await message(unresolvedReport(issues, lead), {
    title: 'Save Folders Still To Sort Out',
    kind: 'info',
  });
}

/* ── Step 1: relink save folders ────────────────────────────────────────────*/

// Point each game whose save folder was recorded on another machine at the
// matching folder here. The survey runs first and is shown before anything is
// written, and games SILO can only match by name are reported, not applied.
async function runRelink(btn) {
  const original = btn.textContent;
  btn.disabled = true;
  btn.textContent = 'Checking…';
  setStatus('');
  try {
    const survey = await window.vault.surveySaveFolders();
    if (!survey.gamesWithSaveFolder) {
      setStatus('No game has a save folder recorded yet, so there is nothing to relink.');
      return;
    }
    if (!survey.relinkable.length) {
      const folders = `${survey.usable} ${survey.usable === 1 ? 'folder is' : 'folders are'}`;
      const byHand = survey.unresolved.length
        ? ` ${survey.unresolved.length} ${survey.unresolved.length === 1 ? 'still needs' : 'still need'} setting by hand in Edit.`
        : '';
      setStatus(`Nothing to relink: ${folders} already on this machine.${byHand}`);
      await showUnresolved(survey.unresolved);
      return;
    }

    const { ask } = window.__TAURI__.dialog;
    const names = survey.relinkable.map(i => i.gameName);
    const shown = names.slice(0, 5).join(', ') + (names.length > 5 ? ` and ${names.length - 5} more` : '');
    const confirmed = await ask(
      `${survey.relinkable.length} save folder(s) were recorded on another PC:\n\n${shown}\n\nPoint them at the folders on this machine? Nothing is moved or deleted.`,
      { title: 'Relink Save Folders', kind: 'warning' },
    );
    if (!confirmed) {
      setStatus('Relink cancelled — nothing was changed.');
      return;
    }

    const applied = await window.vault.relinkSaveFolders();
    const count = applied.relinkable.length;
    const byHand = applied.unresolved.length
      ? `, ${applied.unresolved.length} ${applied.unresolved.length === 1 ? 'still needs' : 'still need'} setting by hand in Edit`
      : '';
    setStatus(`${count} ${count === 1 ? 'game now saves' : 'games now save'} to this machine${byHand}.`);
    await showUnresolved(applied.unresolved);
  } catch (err) {
    showToast('error', 'Relink Failed', err);
    setStatus('Relink failed — nothing was changed.');
  } finally {
    btn.disabled = false;
    btn.textContent = original;
  }
}

/* ── Step 2: bring saves across ─────────────────────────────────────────────*/

// One line per game: where its saves come from and where they are going, so the
// user can see what would land before anything is copied.
function importReport(items) {
  const lines = items.slice(0, 8).map(item => {
    const from = item.source === 'backup' ? `SILO backup ${item.sourcePath}` : item.sourcePath;
    return `• ${item.gameName}\n    from  ${from}\n    into  ${item.targetPath}`;
  });
  if (items.length > 8) lines.push(`…and ${items.length - 8} more.`);
  return lines.join('\n\n');
}

// The folder the old PC's saves are in: a drive pulled out of it, or a copy of
// its folders. Cancelling is not fatal — games with a SILO backup can still be
// restored, and the plan below names whichever source each one would use.
async function pickOldSaveFolder() {
  const { open } = window.__TAURI__.dialog;
  if (!open) return null;
  const chosen = await open({
    directory: true,
    multiple: false,
    title: "Choose the folder holding the old PC's saves (an old drive, or a copy of its Documents)",
  });
  if (!chosen) return null;
  return typeof chosen === 'string' ? chosen : (chosen[0] || null);
}

async function runBringAcross(btn) {
  const original = btn.textContent;
  btn.disabled = true;
  setStatus('');
  try {
    const sourceDir = await pickOldSaveFolder();

    btn.textContent = 'Checking…';
    const plan = await window.vault.planSaveImport(sourceDir);
    if (!plan.ready.length) {
      setStatus(plan.stillMissing.length
        ? `No saves could be brought across for ${plan.stillMissing.length} ${plan.stillMissing.length === 1 ? 'game' : 'games'}.`
        : "Every game's save folder already works on this machine.");
      await showUnresolved(plan.stillMissing, BRING_ACROSS_LEAD);
      return;
    }

    const { ask } = window.__TAURI__.dialog;
    const confirmed = await ask(
      `${plan.ready.length} ${plan.ready.length === 1 ? 'game' : 'games'} can have their saves brought across:\n\n`
      + `${importReport(plan.ready)}\n\n`
      + 'They are copied into the folder they belong in on this machine. Nothing is deleted, and a folder that already holds saves is left alone.',
      { title: 'Bring Saves Across', kind: 'warning' },
    );
    if (!confirmed) {
      setStatus('Bring across cancelled — nothing was copied.');
      return;
    }

    btn.textContent = 'Bringing…';
    const applied = await window.vault.bringSavesAcross(sourceDir);
    const count = applied.ready.length;
    const byHand = applied.stillMissing.length
      ? `, ${applied.stillMissing.length} ${applied.stillMissing.length === 1 ? 'still needs' : 'still need'} setting by hand in Edit`
      : '';
    setStatus(count
      ? `${count} ${count === 1 ? 'game now saves' : 'games now save'} to this machine${byHand}.`
      : 'None of the saves could be brought across.');
    if (!count) showToast('error', 'Nothing Brought Across', 'None of the saves could be brought across.');
    await showUnresolved(applied.stillMissing, BRING_ACROSS_LEAD);
  } catch (err) {
    showToast('error', 'Bring Saves Failed', err);
    setStatus('Bring across failed — nothing was copied.');
  } finally {
    btn.disabled = false;
    btn.textContent = original;
  }
}

/* ── Wiring ─────────────────────────────────────────────────────────────────*/

document.getElementById('btn-migrate-relink')?.addEventListener('click', (e) => runRelink(e.currentTarget));
document.getElementById('btn-migrate-bring')?.addEventListener('click', (e) => runBringAcross(e.currentTarget));
document.getElementById('btn-migrate-close')?.addEventListener('click', closeMigrateModal);
document.getElementById('btn-setup-from-another-pc')?.addEventListener('click', () => {
  showPage('home');
  openMigrateModal();
});
