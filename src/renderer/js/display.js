/* Monitor-aware wallpaper description and ordering. Nothing is filtered any
   more: every wallpaper is listed and selectable, and the label explains how
   SILO will render it on this screen. Dimensions come from the artwork's
   native width/height reported by SteamGridDB, never from a thumbnail. */
const ASPECT_TOLERANCE = 0.03;
let display = null;
const listeners = new Set();

export function currentDisplay() {
  return display;
}

export function displaySummary() {
  if (!display || !display.width || !display.height) return 'Monitor unknown';
  return `${display.width} × ${display.height}`;
}

/* One of four tiers, best fit first:
   0 native match, 1 bigger but a different shape, 2 smaller than the screen,
   3 dimensions missing (or the monitor is unknown). */
export function describeWallpaper(width, height) {
  const unknown = {
    tier: 3,
    label: 'Size unknown',
    note: 'Dimensions unavailable — SILO crops it to fill.',
  };
  if (!width || !height) return unknown;
  if (!display || !display.width || !display.height) return unknown;

  const imageRatio = width / height;
  const displayRatio = display.width / display.height;
  const sameShape = Math.abs(imageRatio - displayRatio) / displayRatio <= ASPECT_TOLERANCE;
  const largeEnough = width >= display.width && height >= display.height;

  if (largeEnough && sameShape) {
    return { tier: 0, label: 'Fits this monitor', note: 'Matches your screen. No cropping.' };
  }
  if (largeEnough) {
    return {
      tier: 1,
      label: 'Larger, different shape',
      note: 'Higher resolution than your screen but a different shape — SILO crops it to fill.',
    };
  }
  return {
    tier: 2,
    label: 'Below display size',
    note: 'Smaller than your monitor — SILO upscales it, so it will look softer.',
  };
}

/* Stable, non-mutating: tier, then pixel area, then the order the API sent. */
export function sortWallpapers(arts) {
  if (!Array.isArray(arts)) return [];
  return arts
    .map((art, index) => ({
      art,
      index,
      tier: describeWallpaper(art && art.width, art && art.height).tier,
      area: ((art && art.width) || 0) * ((art && art.height) || 0),
    }))
    .sort((a, b) => a.tier - b.tier || b.area - a.area || a.index - b.index)
    .map((entry) => entry.art);
}

export async function refreshDisplay() {
  try {
    const next = await window.vault.getDisplayInfo();
    const changed = !display || display.width !== next.width || display.height !== next.height || display.scaleFactor !== next.scaleFactor;
    display = next;
    if (changed) listeners.forEach((listener) => listener(display));
    return display;
  } catch (error) {
    console.warn('Display detection unavailable:', error);
    return display;
  }
}

/* Subscribe to resolution / scale changes. Returns an unsubscribe function. */
export function onDisplayChange(listener) {
  if (typeof listener !== 'function') return () => {};
  listeners.add(listener);
  return () => listeners.delete(listener);
}

export function imageDimensions(url) {
  return new Promise((resolve) => {
    const image = new Image();
    image.onload = () => resolve({ width: image.naturalWidth, height: image.naturalHeight });
    image.onerror = () => resolve(null);
    image.src = url;
  });
}

export function initDisplayMonitoring() {
  refreshDisplay();
  window.setInterval(refreshDisplay, 2500);
}
