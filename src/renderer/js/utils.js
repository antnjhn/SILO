/* ── utils.js — toast + shared formatting helpers ────────────────────────────
   Pure helpers with no dependency on app state, so any module can import them
   without creating an import cycle. */

/* ── Toast ─────────────────────────────────────────────────────────────────*
   Variant notifications, top right. At most 3 are visible, each auto-dismisses
   (errors get longer, warnings/info 4s), hovering or focusing pauses the
   countdown, and an optional text action ("View", "Undo") sits inline.
   Variants: info · success · warning · error — colour, icon and duration each. */
const TOAST_MAX = 3;
const TOAST_VARIANT = {
  info:    { ms: 4000,  icon: 'ℹ',  label: 'Info' },
  success: { ms: 4000,  icon: '✓',  label: 'Success' },
  warning: { ms: 5000,  icon: '⚠',  label: 'Warning' },
  error:   { ms: 8000,  icon: '⚠',  label: 'Error' },
};

function dismissToast(toast) {
  if (!toast || !toast.isConnected || toast.classList.contains('leaving')) return;
  toast.classList.add('leaving');
  const kill = () => toast.remove();
  toast.addEventListener('animationend', kill, { once: true });
  setTimeout(kill, 400);   // fallback when animation is disabled
}

export function showToast(type, title, msg, opts = {}) {
  const container = document.getElementById('toast-container');
  if (!container) return null;
  const variant = TOAST_VARIANT[type] || TOAST_VARIANT.info;
  const kind = variant === TOAST_VARIANT.info && !TOAST_VARIANT[type] ? 'info' : type;
  const persistent = opts.persistent === true;
  const toast = document.createElement('div');
  toast.className = `toast ${kind}`;
  toast.setAttribute('role', kind === 'error' ? 'alert' : 'status');
  toast.setAttribute('aria-live', kind === 'error' ? 'assertive' : 'polite');
  toast.setAttribute('aria-label', `${variant.label}: ${esc(title)}${msg ? '. ' + esc(msg) : ''}`);
  toast.innerHTML = `
    <div class="toast-header">
      <span class="toast-icon" aria-hidden="true">${variant.icon}</span>
      <div class="toast-body">
        <div class="toast-title">${esc(title)}</div>
        ${msg ? `<div class="toast-msg">${esc(msg)}</div>` : ''}
      </div>
    </div>
    ${!persistent ? `<div class="toast-progress"><span></span></div>` : ''}
    <div class="toast-actions">
      ${opts.actionLabel ? `<button type="button" class="toast-action">${esc(opts.actionLabel)}</button>` : ''}
      <button type="button" class="toast-close" aria-label="Dismiss"><svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round"><line x1="18" y1="6" x2="6" y2="18"/><line x1="6" y1="6" x2="18" y2="18"/></svg></button>
    </div>
  `;
  container.appendChild(toast);

  // Cap the stack: the oldest toast leaves to make room.
  const live = [...container.querySelectorAll('.toast:not(.leaving)')];
  while (live.length > TOAST_MAX) dismissToast(live.shift());

  // Countdown, paused while the toast is hovered or focused inside.
  // Persistent toasts never auto-dismiss.
  if (!persistent) {
    let remaining = opts.duration ?? variant.ms;
    let startedAt = performance.now();
    let timer = 0;
    const start = () => {
      startedAt = performance.now();
      timer = setTimeout(() => dismissToast(toast), remaining);
    };
    const hold = () => {
      clearTimeout(timer);
      remaining -= performance.now() - startedAt;
    };
    start();
    toast.addEventListener('mouseenter', hold);
    toast.addEventListener('mouseleave', () => { if (remaining > 0) start(); });
    toast.addEventListener('focusin', hold);
    toast.addEventListener('focusout', () => { if (remaining > 0) start(); });
  }

  const action = toast.querySelector('.toast-action');
  if (action) action.addEventListener('click', () => { dismissToast(toast); opts.onAction?.(); });
  toast.querySelector('.toast-close')?.addEventListener('click', (e) => { e.stopPropagation(); dismissToast(toast); });
  toast.addEventListener('click', (e) => {
    if (e.target.closest('.toast-action, .toast-close')) return;
    dismissToast(toast);
  });
  return toast;
}

/* ── Helpers ───────────────────────────────────────────────────────────────*/
export function fmtTime(min) {
  if (!min || min < 1) return '0h 0m';
  const h = Math.floor(min / 60), m = min % 60;
  return h > 0 ? `${h}h ${m}m` : `${m}m`;
}

/* Tracked playtime. `uncertain` minutes were reconstructed after SILO was closed
   for part of the session, so they are a lower bound: the total is shown as
   ">= x" and never as an exact figure. */
export function fmtTrackedTime(min, uncertain) {
  const exact = Math.max(0, Math.round(min || 0));
  const lowerBound = Math.max(0, Math.round(uncertain || 0));
  if (!lowerBound) return fmtTime(exact);
  return `\u2265 ${fmtTime(exact + lowerBound)}`;
}

/* Explains a lower-bound duration where there is room for a tooltip. */
export function trackedTimeTitle(uncertain) {
  const lowerBound = Math.max(0, Math.round(uncertain || 0));
  if (!lowerBound) return '';
  return `Includes ${fmtShort(lowerBound)} tracked while SILO was not running: the real playtime is at least this much.`;
}

export function fmtDate(iso) {
  if (!iso) return 'Never';
  const d = new Date(iso), now = new Date();
  const days = Math.floor((now - d) / 86400000);
  if (days === 0) return 'Today';
  if (days === 1) return 'Yesterday';
  if (days < 7) return `${days}d ago`;
  if (days < 30) return `${Math.floor(days/7)}w ago`;
  return d.toLocaleDateString('en-GB', { day: 'numeric', month: 'short' });
}

export function esc(s) {
  return String(s).replace(/&/g,'&amp;').replace(/</g,'&lt;').replace(/>/g,'&gt;').replace(/"/g,'&quot;');
}

// Escape for use inside a JS string literal embedded in an inline onclick handler
export function escJs(s) {
  return esc(s).replace(/'/g, "\\'");
}

// Split a comma-separated tag string into a deduped, trimmed array
export function parseTags(val) {
  return [...new Set(String(val || '').split(',').map(t => t.trim()).filter(Boolean))];
}

/* ── Date / formatting helpers ── */
export function startOfLocalDay(d) {
  const c = new Date(d);
  c.setHours(0, 0, 0, 0);
  return c;
}
export function addDaysLocal(d, n) {
  const c = new Date(d);
  c.setDate(c.getDate() + n);
  return c;
}
export function localDayKey(d) {
  return `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
}
export function parseSessionDate(s) {
  if (!s || !s.startedAt) return null;
  const d = new Date(s.startedAt);
  return isNaN(d.getTime()) ? null : d;
}
export function fmtShort(min) {
  const n = Math.max(0, Math.round(min || 0));
  if (!n) return '0m';
  const h = Math.floor(n / 60), m = n % 60;
  if (h) return m ? `${h}h ${m}m` : `${h}h`;
  return `${m}m`;
}
export function fmtDayTitle(d) {
  return d.toLocaleDateString('en-GB', { weekday: 'short', day: 'numeric', month: 'short' });
}
export function fmtAxisLabel(d, days) {
  return days <= 7 ? d.toLocaleDateString('en-GB', { weekday: 'short' }) : String(d.getDate());
}
export function fmtSessionWhen(iso) {
  const d = new Date(iso);
  return isNaN(d.getTime()) ? '' : d.toLocaleString('en-GB', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}
export function backupTimestampLabel(ts) {
  const m = /^(\d{4})(\d{2})(\d{2})_(\d{2})(\d{2})(\d{2})$/.exec(ts || '');
  if (!m) return ts || '';
  const d = new Date(+m[1], +m[2] - 1, +m[3], +m[4], +m[5], +m[6]);
  return d.toLocaleString('en-GB', { day: 'numeric', month: 'short', hour: '2-digit', minute: '2-digit' });
}

