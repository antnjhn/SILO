/* Shared focus and back navigation for modal overlays and routed pages. */

const EXCLUDED_OVERLAYS = new Set(['bg-overlay', 'category-wheel-overlay', 'onboarding-overlay']);
const BACK_CONTROLS = {
  'close-confirm-overlay': '#close-confirm-cancel',
  'discovery-overlay': '#btn-discovery-later',
  'scan-results-overlay': '#btn-scan-cancel',
  'metadata-overlay': '#btn-metadata-cancel',
  'online-art-overlay': '#btn-online-art-cancel',
  'restore-overlay': '#btn-restore-cancel',
  'gif-policy-overlay': '[data-policy="cancel"]',
  'uninstall-overlay': '#btn-uninstall-cancel',
  'backup-name-overlay': '#btn-backup-name-cancel',
  'migrate-overlay': '#btn-migrate-close',
  'backup-manager-overlay': '#btn-backup-manager-close',
};

const CONTROL_SELECTOR = [
  'button:not(:disabled)', 'a[href]', 'input:not(:disabled):not([type="hidden"])',
  'select:not(:disabled)', 'textarea:not(:disabled)',
  '[tabindex]:not([tabindex="-1"])',
].join(',');

function isVisible(element) {
  if (!element || element.hidden || element.getAttribute('aria-hidden') === 'true') return false;
  if (element.classList?.contains('hidden')) return false;
  const style = getComputedStyle(element);
  return style.display !== 'none' && style.visibility !== 'hidden' && element.getClientRects().length > 0;
}

export function controlsInScope(root) {
  if (!root) return [];
  return [...root.querySelectorAll(CONTROL_SELECTOR)].filter(isVisible);
}

export function getActiveModalScope() {
  const overlays = [...document.querySelectorAll('[id$="-overlay"]')]
    .filter((overlay) => !EXCLUDED_OVERLAYS.has(overlay.id)
      && !overlay.closest('#page-host') && isVisible(overlay));
  if (!overlays.length) return null;

  // The visible overlay with the highest stacking order owns input. DOM order
  // breaks ties so a confirmation layered later in the document is topmost.
  overlays.sort((a, b) => {
    const za = Number.parseInt(getComputedStyle(a).zIndex, 10) || 0;
    const zb = Number.parseInt(getComputedStyle(b).zIndex, 10) || 0;
    return za - zb || [...a.parentElement.children].indexOf(a) - [...b.parentElement.children].indexOf(b);
  });
  const overlay = overlays.at(-1);
  const root = overlay.querySelector('[role="dialog"]')
    || overlay.querySelector('.glass-panel') || overlay;
  return { kind: 'modal', overlay, root, controls: controlsInScope(root) };
}

export function getPageNavigationScope(pageId) {
  if (!pageId || pageId === 'home') return null;
  const root = document.querySelector(`#page-host .page[data-page="${pageId}"]`);
  if (!isVisible(root)) return null;
  return { kind: 'page', root, controls: controlsInScope(root) };
}

/* Focus without letting the browser scroll on its own, then bring the control
   into view ourselves. Settings and Import are taller than the window, so a
   focus step that only moved focus would land on a control the player cannot
   see. 'nearest' leaves an already visible control where it is instead of
   re-centring the list on every step. */
export function focusInView(element) {
  element.focus({ preventScroll: true });
  element.scrollIntoView({ block: 'nearest', inline: 'nearest' });
}

export function focusFirstInScope(scope) {
  if (!scope) return false;
  const first = scope.controls?.[0];
  if (first) focusInView(first);
  else scope.root?.focus?.({ preventScroll: true });
  return Boolean(first || scope.root);
}

export function moveFocusInScope(scope, direction) {
  const controls = scope?.controls || [];
  if (!controls.length) return false;
  const active = document.activeElement;
  const currentIndex = controls.indexOf(active);
  if (currentIndex < 0) {
    focusFirstInScope(scope);
    return true;
  }

  const current = controls[currentIndex].getBoundingClientRect();
  const cx = current.left + current.width / 2;
  const cy = current.top + current.height / 2;
  const vector = {
    ArrowUp: [0, -1], ArrowDown: [0, 1], ArrowLeft: [-1, 0], ArrowRight: [1, 0],
  }[direction];
  if (!vector) return false;

  const candidates = controls.map((element, index) => {
    if (index === currentIndex) return null;
    const rect = element.getBoundingClientRect();
    const dx = rect.left + rect.width / 2 - cx;
    const dy = rect.top + rect.height / 2 - cy;
    const primary = dx * vector[0] + dy * vector[1];
    if (primary <= 1) return null;
    const cross = Math.abs(dx * vector[1] - dy * vector[0]);
    return { element, index, score: primary + cross * 1.75 };
  }).filter(Boolean).sort((a, b) => a.score - b.score);

  if (candidates.length) focusInView(candidates[0].element);
  else {
    // Wrap to the opposite edge when there is no control in that direction.
    const ordered = controls.map((element, index) => {
      const rect = element.getBoundingClientRect();
      const x = rect.left + rect.width / 2;
      const y = rect.top + rect.height / 2;
      return { element, index, edge: x * vector[0] + y * vector[1] };
    }).sort((a, b) => a.edge - b.edge);
    const next = vector[0] + vector[1] > 0 ? ordered[0] : ordered.at(-1);
    focusInView(next.element);
  }
  return true;
}

export function trapTabInScope(event, scope) {
  const controls = scope?.controls || [];
  if (!controls.length) {
    event.preventDefault();
    scope?.root?.focus?.({ preventScroll: true });
    return true;
  }
  const first = controls[0], last = controls.at(-1);
  if (event.shiftKey && (document.activeElement === first || !scope.root.contains(document.activeElement))) {
    event.preventDefault(); last.focus({ preventScroll: true });
  } else if (!event.shiftKey && (document.activeElement === last || !scope.root.contains(document.activeElement))) {
    event.preventDefault(); first.focus({ preventScroll: true });
  }
  return true;
}

/* A dialog whose text field confirms on Enter names its primary action here,
   so the keyboard, the pad and the dialog's own handler agree on one button. */
const PRIMARY_CONTROLS = {
  'backup-name-overlay': '#btn-backup-name-ok',
};

export function submitActiveModal(scope = getActiveModalScope()) {
  if (!scope) return false;
  const selector = PRIMARY_CONTROLS[scope.overlay.id];
  const control = (selector && scope.overlay.querySelector(selector))
    || scope.root.querySelector('[type="submit"], .highlight');
  if (!control || control.disabled) return false;
  control.click();
  return true;
}

export function dismissActiveModal(scope = getActiveModalScope()) {
  if (!scope) return false;
  const selector = BACK_CONTROLS[scope.overlay.id];
  const control = selector && scope.overlay.querySelector(selector);
  if (control && !control.disabled) control.click();
  else scope.overlay.classList.add('hidden');
  return true;
}

// Keep focus inside visible overlays and return it to the opener after any
// dismissal path, including a mouse Cancel button.
const modalOpeners = new Map();
let focusObserverInstalled = false;

function installModalFocusManagement() {
  if (focusObserverInstalled || typeof document === 'undefined') return;
  focusObserverInstalled = true;
  const observer = new MutationObserver((records) => {
    for (const { target } of records) {
      if (!(target instanceof HTMLElement) || !target.id.endsWith('-overlay')
          || EXCLUDED_OVERLAYS.has(target.id) || target.closest('#page-host')) continue;
      if (isVisible(target) && !modalOpeners.has(target)) {
        modalOpeners.set(target, document.activeElement);
        requestAnimationFrame(() => {
          const active = getActiveModalScope();
          if (active?.overlay === target && !active.root.contains(document.activeElement)) {
            focusFirstInScope(active);
          }
        });
      } else if (!isVisible(target) && modalOpeners.has(target)) {
        // The dialog itself knows which control opened it (a controller hold
        // has no active element, so the DOM's activeElement is not enough).
        const opener = target._siloOpener || modalOpeners.get(target);
        modalOpeners.delete(target);
        requestAnimationFrame(() => {
          const active = getActiveModalScope();
          if (active) focusFirstInScope(active);
          else if (opener && isVisible(opener)) opener.focus({ preventScroll: true });
        });
      }
    }
  });
  observer.observe(document.documentElement, { subtree: true, attributes: true, attributeFilter: ['class', 'hidden', 'aria-hidden'] });

  document.addEventListener('focusin', (event) => {
    const scope = getActiveModalScope();
    if (scope && !scope.root.contains(event.target)) focusFirstInScope(scope);
  });
}

installModalFocusManagement();
