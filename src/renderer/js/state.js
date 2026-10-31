/* ── state.js — shared application state ────────────────────────────────────
   The single source of truth for state that more than one module touches.

   Each variable is exported as a live binding: an importer always reads the
   current value. Because ES module code cannot assign to an imported binding,
   every write that happens outside this file goes through the matching setter
   (e.g. setGames(...) instead of `games = ...`). Modules that own state read by
   nothing else keep it local to themselves. */

let allGames = [];
let games = [];
let selectedIndex = 0;
let showGameTitles = false;   // 'Show game titles' Display setting
let searchQuery = '';

export { allGames, games, selectedIndex, showGameTitles, searchQuery };

export const setAllGames = (v) => { allGames = v; };
export const setGames = (v) => { games = v; };
export const setSelectedIndex = (v) => { selectedIndex = v; };
export const setShowGameTitles = (v) => { showGameTitles = v; };
export const setSearchQuery = (v) => { searchQuery = v; };

// Lookups that read the live library lists.
export function findGameById(id) {
  if (!id) return null;
  return allGames.find(x => x.id === id) || games.find(x => x.id === id) || null;
}

export function gameNameById(id) {
  const g = findGameById(id);
  return g ? g.name : 'Game';
}
