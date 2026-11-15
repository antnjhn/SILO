/* ── background.js — keeps the animated backdrop out of the way ──────────────
   The backdrop itself is one fixed layer drawn by body::before (see style.css).
   Its breathing halo is a transform on a painted gradient, so a frame costs a
   composited transform rather than a repaint of the whole window. A launcher
   should still never animate a window nobody can see.

   `bg-motion-paused` on <body> chooses the static frame. It is set when the
   document is hidden (minimised, or the window released while a Gaming Mode
   session owns the machine) and when the window is in the background. Losing
   focus pauses as well, because SILO sits behind whatever the user is doing.
   ───────────────────────────────────────────────────────────────────────── */

const PAUSED_CLASS = 'bg-motion-paused';

function apply() {
  // document.hidden covers minimised/occluded windows; hasFocus covers another
  // app being in front while the window is still visible.
  const paused = document.hidden || !document.hasFocus();
  document.body.classList.toggle(PAUSED_CLASS, paused);
}

export function initBackgroundMotion() {
  apply();
  document.addEventListener('visibilitychange', apply);
  window.addEventListener('focus', apply);
  window.addEventListener('blur', apply);
}
