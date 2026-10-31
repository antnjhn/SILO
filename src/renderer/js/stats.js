/* ── stats.js — Statistics hub, activity charts, stat popovers ─────────────── */
import { allGames, games, selectedIndex, findGameById } from './state.js';
import { showPage, onPageEnter, onPageLeave } from './pages.js';
import {
  esc, fmtTime, fmtDate, fmtShort, fmtDayTitle, fmtAxisLabel, fmtSessionWhen,
  startOfLocalDay, addDaysLocal, localDayKey, parseSessionDate,
} from './utils.js';

/* ── Stats & SaveGuard status module ───────────────────────────────────────*/
let popoverTimer = null;
let popoverState = null; // { metric, gameId, days, sessions, source }
/* When the preview was last dismissed by hand, so the focus it hands back
   does not open it again straight away. */
let lastDismissedAt = 0;
/* The number of the open that is current. */
let openSeq = 0;
/* True while the preview is fetching its data. */
let popoverLoading = false;
let statsView = 'overall'; // 'overall' | 'games'
let statsGameId = null;
let statsSessions = [];

/* ── Activity bucketing ── */
function buildBuckets(sessions, gameId, days) {
  const todayStart = startOfLocalDay(new Date());
  const rangeStart = addDaysLocal(todayStart, -(days - 1));
  const map = new Map();
  for (let i = 0; i < days; i++) {
    const d = addDaysLocal(rangeStart, i);
    map.set(localDayKey(d), { date: d, minutes: 0, count: 0 });
  }
  (sessions || []).forEach(s => {
    if (gameId && s.gameId !== gameId) return;
    const d = parseSessionDate(s);
    if (!d) return;
    const d0 = startOfLocalDay(d);
    if (d0 < rangeStart || d0 > todayStart) return;
    const b = map.get(localDayKey(d0));
    if (!b) return;
    b.minutes += s.minutes || 0;
    b.count += 1;
  });
  return [...map.values()];
}

function hasActivityIn(sessions, gameId, days) {
  const todayStart = startOfLocalDay(new Date());
  const start = addDaysLocal(todayStart, -(days - 1));
  return (sessions || []).some(s => {
    if (gameId && s.gameId !== gameId) return false;
    const d = parseSessionDate(s);
    if (!d) return false;
    const d0 = startOfLocalDay(d);
    return d0 >= start && d0 <= todayStart;
  });
}

function activeDayCount(sessions) {
  const set = new Set();
  const todayStart = startOfLocalDay(new Date());
  (sessions || []).forEach(s => {
    const d = parseSessionDate(s);
    if (d && startOfLocalDay(d) <= todayStart) set.add(localDayKey(startOfLocalDay(d)));
  });
  return set.size;
}

/* ── Bar chart renderer (animated left→right sweep) ── */
function renderChartBars(host, sessions, gameId, metric, days) {
  host.innerHTML = '';
  const buckets = buildBuckets(sessions, gameId, days);
  const hasAny = (sessions || []).some(s => !gameId || s.gameId === gameId);

  if (!buckets.some(b => b.count > 0)) {
    const hint = hasAny
      ? 'No sessions in this range yet.'
      : (metric === 'sessions' ? 'No sessions tracked yet.' : 'No playtime tracked yet.');
    host.innerHTML = `<div class="stats-empty">${hint}<br><span style="font-size:10px;color:rgba(255,255,255,0.28);">Launch a game - history appears after your first session.</span></div>`;
    return;
  }

  const values = buckets.map(b => metric === 'sessions' ? b.count : b.minutes);
  const max = Math.max(1, ...values);
  const disp = buckets.map((b, i) => {
    if (metric === 'sessions') {
      return values[i] > 0 ? Math.max(6, Math.round(values[i] / max * 100)) : 0;
    }
    if (b.minutes > 0) return Math.max(6, Math.round(b.minutes / max * 100));
    return b.count > 0 ? 4 : 0; // sub-minute play day still shows a sliver
  });

  const labelEvery = days <= 7 ? 1 : 5;
  const cols = buckets.map((b, i) => {
    const tip = metric === 'sessions'
      ? `${fmtDayTitle(b.date)} - ${b.count} session${b.count === 1 ? '' : 's'}${b.minutes ? ` · ${fmtShort(b.minutes)}` : ''}`
      : `${fmtDayTitle(b.date)} - ${fmtShort(b.minutes)} in ${b.count} session${b.count === 1 ? '' : 's'}`;
    return `<div class="act-col${disp[i] === 0 ? ' zero' : ''}" title="${esc(tip)}"><div class="act-bar" style="transition-delay:${i * 18}ms;"></div></div>`;
  }).join('');
  const axis = buckets.map((b, i) =>
    `<span>${i % labelEvery === 0 ? esc(fmtAxisLabel(b.date, days)) : ''}</span>`
  ).join('');

  host.innerHTML = `<div class="act-wrap"><div class="act-chart">${cols}</div><div class="act-axis">${axis}</div></div>`;

  const chart = host.querySelector('.act-chart');
  chart.querySelectorAll('.act-col').forEach((col, i) => {
    col.addEventListener('mouseenter', () => col.classList.add('hovered'));
    col.addEventListener('mouseleave', () => col.classList.remove('hovered'));
  });
  // Two frames later the bars are in the DOM at height 0; raise them with a stagger.
  requestAnimationFrame(() => {
    requestAnimationFrame(() => {
      chart.querySelectorAll('.act-col').forEach((col, i) => {
        const bar = col.querySelector('.act-bar');
        if (bar) bar.style.height = `${disp[i]}%`;
      });
    });
  });
}

/* ── Playtime trend-line (SVG area chart) ───────────────────────────────────*/
let trendGradSeq = 0;

/* One plot height for every chart, matching the CSS, so the week and month
   graphs are laid out identically. */
const PLOT_H = 140;

// Right-most run of consecutive active days (minutes > 0).
function trendRun(buckets) {
  let end = -1;
  for (let i = buckets.length - 1; i >= 0; i--) {
    if (buckets[i].minutes > 0) { end = i; break; }
  }
  if (end === -1) return null;
  let len = 0;
  for (let i = end; i >= 0 && buckets[i].minutes > 0; i--) len++;
  return { end, len };
}

// A streak counts when 3+ consecutive play days are still "live" (they reach
// today or yesterday, so a run already broken by idle days stays normal).
function isActiveStreak(buckets) {
  const run = trendRun(buckets);
  return !!run && run.len >= 3 && run.end >= buckets.length - 2;
}

function setPopoverFlame(on, delay) {
  const flame = document.getElementById('sp-flame');
  if (!flame) return;
  if (flame._timer) { clearTimeout(flame._timer); flame._timer = null; }
  flame.classList.remove('show');
  if (!on) return;
  const reduced = window.matchMedia?.('(prefers-reduced-motion: reduce)').matches;
  if (reduced) { flame.classList.add('show'); return; }
  flame._timer = setTimeout(() => flame.classList.add('show'), delay || 760);
}

// SVG playtime trend-line with a left→right draw-in, soft area fill, endpoint
// dot and — for active streaks — a dim→bright gradient plus a pulsing glow.
// Returns whether an active streak was detected.
function renderTrendLine(host, sessions, gameId, days, metric = 'playtime') {
  host.innerHTML = '';
  const buckets = buildBuckets(sessions, gameId, days);
  const hasAny = (sessions || []).some(s => !gameId || s.gameId === gameId);
  const isSessions = metric === 'sessions';
  const value = (b) => isSessions ? b.count : b.minutes;
  const emptyMsg = isSessions ? 'No sessions in this range yet.' : 'No playtime in this range yet.';
  const noDataMsg = isSessions ? 'No sessions tracked yet.' : 'No playtime tracked yet.';

  if (!buckets.some(b => value(b) > 0)) {
    const hint = hasAny ? emptyMsg : noDataMsg;
    host.innerHTML = `<div class="stats-empty">${hint}<br><span style="font-size:10px;color:rgba(255,255,255,0.28);">Launch a game - history appears after your first session.</span></div>`;
    return false;
  }

  const streak = isActiveStreak(buckets);
  const n = buckets.length;

  const seq = ++trendGradSeq;
  const lineGrad = `tl-line-${seq}`;
  const areaGrad = `tl-area-${seq}`;
  const glowGrad = `tl-glow-${seq}`;
  const lineStroke = streak ? `url(#${lineGrad})` : 'var(--accent)';

  // The drawing is laid out first, then measured, then drawn into the box it
  // actually occupies. That keeps the line in proportion whatever height the
  // surrounding CSS asks for, and it keeps a chart that was measured while
  // hidden from being drawn at the wrong size.
  host.innerHTML = `
  <svg class="sp-trend${streak ? ' streak' : ''}" viewBox="0 0 100 100" preserveAspectRatio="none" aria-hidden="true">
    <defs>
      <linearGradient id="${lineGrad}" x1="0" y1="0" x2="1" y2="0">
        <stop offset="0%" stop-color="var(--accent-strong)" />
        <stop offset="100%" stop-color="var(--accent)" />
      </linearGradient>
      <linearGradient id="${areaGrad}" x1="0" y1="0" x2="0" y2="1">
        <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.16" />
        <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
      </linearGradient>
      <radialGradient id="${glowGrad}">
        <stop offset="0%" stop-color="var(--accent)" stop-opacity="0.42" />
        <stop offset="100%" stop-color="var(--accent)" stop-opacity="0" />
      </radialGradient>
    </defs>
    <line class="sp-base" x1="0" y1="0" x2="0" y2="0" />
    <path class="sp-area" d="" fill="url(#${areaGrad})" />
    <path class="sp-line" d="" pathLength="1" stroke="${lineStroke}" />
    ${streak ? `<circle class="sp-end-glow" cx="0" cy="0" r="10" fill="url(#${glowGrad})" />` : ''}
    <circle class="sp-end-dot" cx="0" cy="0" r="3" />
  </svg>
  <div class="sp-trend-axis">
    <span>${esc(fmtDayTitle(buckets[0].date))}</span>
    <span>${esc(fmtDayTitle(buckets[n - 1].date))}</span>
  </div>`;

  const svg = host.querySelector('.sp-trend');
  if (!svg) return false;

  const areaPath = svg.querySelector('.sp-area');
  const linePath = svg.querySelector('.sp-line');
  const baseLine = svg.querySelector('.sp-base');
  const endDot = svg.querySelector('.sp-end-dot');
  const endGlow = svg.querySelector('.sp-end-glow');

  const draw = (W, H) => {
    const padX = 8;
    const top = 18;          // headroom, so the peak is never cut off
    const bottom = H - 16;   // the baseline
    const span = Math.max(1, W - padX * 2);
    // The domain is the real maximum with a little headroom above it, so one
    // busy day does not flatten every other day to nothing.
    const peak = Math.max(0, ...buckets.map(b => value(b)));
    const maxVal = peak > 0 ? peak * 1.12 : 1;
    const x = (i) => padX + (n === 1 ? span / 2 : (span * i) / (n - 1));
    const y = (v) => (v > 0 ? top + (1 - Math.min(1, v / maxVal)) * (bottom - top) : bottom);

    const pts = buckets.map((b, i) => `${x(i).toFixed(1)},${y(value(b)).toFixed(1)}`);
    const lineD = `M${pts.join(' L')}`;
    const areaD = `${lineD} L${x(n - 1).toFixed(1)},${bottom} L${x(0).toFixed(1)},${bottom} Z`;
    const lastX = x(n - 1);
    const lastY = y(value(buckets[n - 1]));

    svg.setAttribute('viewBox', `0 0 ${W} ${H}`);
    baseLine.setAttribute('x1', String(padX));
    baseLine.setAttribute('y1', String(bottom));
    baseLine.setAttribute('x2', String(W - padX));
    baseLine.setAttribute('y2', String(bottom));
    areaPath.setAttribute('d', areaD);
    linePath.setAttribute('d', lineD);
    endDot.setAttribute('cx', lastX.toFixed(1));
    endDot.setAttribute('cy', lastY.toFixed(1));
    if (endGlow) {
      endGlow.setAttribute('cx', lastX.toFixed(1));
      endGlow.setAttribute('cy', lastY.toFixed(1));
    }
  };

  // The drawing is measured and placed on the next frame, when the layout of
  // the page is final: a scrollbar that appears with the content would
  // otherwise narrow the box after the chart had already been drawn.
  const paint = () => {
    // The host gives the width and the stylesheet gives the height, so the
    // drawing lands in the pixels it was measured for and is never stretched.
    const measuredW = Math.floor(host.clientWidth);
    const measuredH = parseFloat(window.getComputedStyle(svg).height) || PLOT_H;

    if (!measuredW) {
      // The host had no width, so the page was hidden when this ran. Draw it
      // again on the next frame, once it is on screen. The retry is bounded
      // and only happens while the page is actually on screen, so a page that
      // is left again cannot leave a loop running behind it.
      const tries = parseInt(host.dataset.measureTries || '0', 10);
      if (tries < 2 && host.offsetParent !== null) {
        host.dataset.measureTries = String(tries + 1);
        requestAnimationFrame(paint);
      } else {
        delete host.dataset.measureTries;
      }
      return;
    }
    delete host.dataset.measureTries;
    draw(Math.max(200, measuredW), Math.max(60, Math.round(measuredH)));
    svg.classList.add('sp-anim');
  };

  requestAnimationFrame(paint);
  return streak;
}

// Builds a full chart section (title + WEEK/MONTH toggle + chart) and appends it.
function appendChartSection(parent, sessions, gameId, metric) {
  const section = document.createElement('div');
  section.className = 'stats-chart-section';

  const head = document.createElement('div');
  head.className = 'stats-chart-head';
  const title = document.createElement('h4');
  title.textContent = metric === 'sessions' ? 'Sessions per Day' : 'Playtime per Day';
  head.appendChild(title);

  const range = document.createElement('div');
  range.className = 'sp-range';
  const b7 = document.createElement('button');
  b7.type = 'button'; b7.className = 'sp-range-btn'; b7.dataset.days = '7'; b7.textContent = 'WEEK';
  const b30 = document.createElement('button');
  b30.type = 'button'; b30.className = 'sp-range-btn'; b30.dataset.days = '30'; b30.textContent = 'MONTH';
  range.append(b7, b30);
  head.appendChild(range);
  section.appendChild(head);

  const host = document.createElement('div');
  section.appendChild(host);

  let days = hasActivityIn(sessions, gameId, 7) ? 7 : 30;
  const paint = () => {
    b7.classList.toggle('active', days === 7);
    b30.classList.toggle('active', days === 30);
    renderTrendLine(host, sessions, gameId, days, metric);
  };
  b7.addEventListener('click', () => { days = 7; paint(); });
  b30.addEventListener('click', () => { days = 30; paint(); });

  // The section goes into the page before it is painted: a chart that is
  // drawn while it is still detached has no width to measure, and would come
  // out at the wrong size.
  parent.appendChild(section);
  paint();
}

/* ── Overall stats ── */
function statCardHtml(value, label) {
  return `<div class="stat-card"><div class="sc-value">${esc(value)}</div><div class="sc-label">${esc(label)}</div></div>`;
}

function renderStatsOverall() {
  const el = document.getElementById('stats-content');
  el.innerHTML = '';

  const totalMin = allGames.reduce((a, g) => a + (g.playtimeMinutes || 0), 0);
  const totalSessions = allGames.reduce((a, g) => a + (g.sessionCount || 0), 0);
  const played = allGames.filter(g => (g.sessionCount || 0) > 0).length;
  const activeDays = activeDayCount(statsSessions);

  const cards = document.createElement('div');
  cards.className = 'stats-cards';
  cards.innerHTML =
    statCardHtml(fmtTime(totalMin), 'TOTAL PLAYTIME') +
    statCardHtml(totalSessions, 'TOTAL SESSIONS') +
    statCardHtml(played, 'GAMES PLAYED') +
    statCardHtml(activeDays, 'DAYS ACTIVE');
  el.appendChild(cards);

  appendChartSection(el, statsSessions, null, 'playtime');
  appendChartSection(el, statsSessions, null, 'sessions');

  const ranked = [...allGames]
    .filter(g => (g.playtimeMinutes || 0) > 0)
    .sort((a, b) => (b.playtimeMinutes || 0) - (a.playtimeMinutes || 0))
    .slice(0, 8);

  const sec = document.createElement('div');
  sec.className = 'stats-chart-section';
  sec.innerHTML = '<div class="stats-chart-head"><h4>Most Played</h4></div>';
  const rankWrap = document.createElement('div');
  rankWrap.className = 'stats-rank';
  if (ranked.length) {
    const maxMin = Math.max(1, ranked[0].playtimeMinutes || 1);
    rankWrap.innerHTML = ranked.map((g, i) => `
      <div class="rank-row">
        <span class="rank-pos">${i + 1}</span>
        <span class="rank-name" title="${esc(g.name)}">${esc(g.name)}</span>
        <span class="rank-track"><span class="rank-fill" style="width:${Math.max(0, Math.min(100, ((g.playtimeMinutes || 0) / maxMin) * 100))}%;"></span></span>
        <span class="rank-time">${fmtTime(g.playtimeMinutes)}</span>
      </div>`).join('');
  } else {
    rankWrap.innerHTML = '<div class="stats-empty">No playtime yet - launch a game to get started.</div>';
  }
  sec.appendChild(rankWrap);
  el.appendChild(sec);
}

/* ── Game-wise stats ── */
function renderStatsGames() {
  const el = document.getElementById('stats-content');
  el.innerHTML = '';

  const list = [...allGames].sort((a, b) =>
    ((b.playtimeMinutes || 0) - (a.playtimeMinutes || 0)) ||
    ((b.sessionCount || 0) - (a.sessionCount || 0)) ||
    String(a.name || '').localeCompare(String(b.name || ''))
  );

  if (!list.length) {
    el.innerHTML = '<div class="stats-empty">No games in your library yet.</div>';
    return;
  }
  if (!statsGameId || !list.some(g => g.id === statsGameId)) {
    statsGameId = (list.find(g => (g.sessionCount || 0) > 0) || list[0]).id;
  }

  const layout = document.createElement('div');
  layout.className = 'stats-games-layout';

  const listCol = document.createElement('div');
  listCol.className = 'stats-game-list';

  const detailCol = document.createElement('div');
  detailCol.className = 'stats-game-detail';
  layout.append(listCol, detailCol);
  el.appendChild(layout);

  const paintList = () => {
    listCol.innerHTML = list.map(g => `
      <button type="button" class="stats-game-btn${g.id === statsGameId ? ' active' : ''}" data-game="${esc(g.id)}">
        <span class="gb-name" title="${esc(g.name)}">${esc(g.name)}</span>
        <span class="gb-sub">${fmtTime(g.playtimeMinutes)} · ${g.sessionCount || 0} sessions</span>
      </button>`).join('');
    listCol.querySelectorAll('.stats-game-btn').forEach(btn => {
      btn.addEventListener('click', () => {
        statsGameId = btn.dataset.game;
        paintList();
        paintDetail();
      });
    });
  };

  const paintDetail = () => {
    const game = findGameById(statsGameId);
    detailCol.innerHTML = '';
    if (!game) return;

    const name = document.createElement('div');
    name.className = 'gd-name';
    name.textContent = game.name;
    detailCol.appendChild(name);

    const cards = document.createElement('div');
    cards.className = 'stats-cards';
    cards.innerHTML =
      statCardHtml(fmtTime(game.playtimeMinutes), 'PLAYTIME') +
      statCardHtml(game.sessionCount || 0, 'SESSIONS') +
      statCardHtml(fmtDate(game.lastPlayed), 'LAST PLAYED');
    detailCol.appendChild(cards);

    appendChartSection(detailCol, statsSessions, game.id, 'playtime');
    appendChartSection(detailCol, statsSessions, game.id, 'sessions');

    const recent = statsSessions.filter(s => s.gameId === game.id).slice(0, 8);
    if (recent.length) {
      const sec = document.createElement('div');
      sec.className = 'stats-chart-section';
      sec.innerHTML = '<div class="stats-chart-head"><h4>Recent Sessions</h4></div>';
      const listEl = document.createElement('div');
      listEl.className = 'stats-recent';
      listEl.innerHTML = recent.map(s => `
        <div class="recent-row">
          <span class="rr-time">${esc(fmtSessionWhen(s.startedAt))}</span>
          <span class="rr-dur">${fmtShort(s.minutes)}</span>
        </div>`).join('');
      sec.appendChild(listEl);
      detailCol.appendChild(sec);
    }
  };

  paintList();
  paintDetail();
}

function renderStatsContent() {
  if (statsView === 'games') renderStatsGames();
  else renderStatsOverall();
}

export function setStatsView(view) {
  statsView = view === 'games' ? 'games' : 'overall';
  document.getElementById('btn-stats-overall').classList.toggle('active', statsView === 'overall');
  document.getElementById('btn-stats-games').classList.toggle('active', statsView === 'games');
  renderStatsContent();
}

export function openStats(opts) {
  hideStatPopoverNow();
  if (opts && opts.gameId) statsGameId = opts.gameId;
  showPage('stats');
}

// The statistics page is rebuilt every time it is shown, so the charts are
// measured against a visible layout and always reflect the real data.
onPageEnter('stats', () => {
  (async () => {
    try {
      statsSessions = await window.vault.listSessions();
    } catch (e) {
      statsSessions = statsSessions || [];
    }
    setStatsView(statsView === 'games' ? 'games' : 'overall');
  })();
});

export function closeStats() {
  showPage('home');
}

// Gamepad focus cycling inside the stats modal (left/right already switch the view).
export function navigateStatsFocus(delta) {
  const focusables = [...document.querySelectorAll('#stats-content button')].filter(el => el.offsetParent !== null);
  if (!focusables.length) return;
  let i = focusables.indexOf(document.activeElement);
  if (i < 0) i = delta > 0 ? -1 : 0;
  i = (i + delta + focusables.length) % focusables.length;
  focusables[i].focus();
}

/* ── Hover popover (PLAYTIME / SESSIONS stat blocks) ──────────────────────
   The preview appears over the library when the pointer rests on one of the
   two statistics. It is also reachable with the keyboard (the statistic is a
   button) and with a tap, where there is no hover at all.

   How the preview was opened decides what closes it: a hover closes when the
   pointer leaves, an explicit activation closes on the next activation or on
   Escape. */
export function bindStatPopoverHover() {
  ['playtime', 'sessions'].forEach(metric => {
    const block = document.getElementById('stat-block-' + metric);
    if (!block) return;

    const canPreview = () => {
      // Available wherever the stats are on screen: the rail view (browsing or
      // stepped into the game) as well as the full details pane. Requiring
      // full details here is why the chart never appeared in the rail.
      const panel = document.getElementById('details-panel');
      return !!(panel && !panel.classList.contains('hidden') && games[selectedIndex]);
    };

    const isOpenFor = () => !!(popoverState && popoverState.metric === metric && pvIsOpen());

    // The pointer: the preview opens under the pointer.
    block.addEventListener('mouseenter', () => {
      if (!canPreview() || isOpenFor()) return;
      openStatPopover(metric, games[selectedIndex].id, 'hover');
    });
    block.addEventListener('mouseleave', () => {
      // Only the hover preview follows the pointer away. One that was opened
      // with the keyboard or with a tap stays until it is dismissed, so a tap
      // is not undone by the pointer leaving the statistic.
      if (popoverState && popoverState.source !== 'hover') return;
      // A preview that is still loading is not dismissed: the pointer may
      // simply be on its way to the panel it is about to open.
      if (popoverLoading) return;
      scheduleStatPopoverHide();
    });

    // The keyboard: focusing the statistic reveals the same information.
    block.addEventListener('focus', () => {
      if (!canPreview() || isOpenFor()) return;
      // A preview that was just dismissed hands the focus back to the
      // statistic; it must not open again straight away.
      if (Date.now() - lastDismissedAt < 400) return;
      openStatPopover(metric, games[selectedIndex].id, 'focus');
    });
    block.addEventListener('blur', (e) => {
      // The focus may be on its way somewhere else: the preview's own
      // controls, or another control on the page. Only a focus that leaves
      // the statistic AND the preview closes it.
      const to = e.relatedTarget;
      if (to && (to === block || (to.closest && to.closest('#stat-popover')))) return;
      scheduleStatPopoverHide();
    });

    // A tap, or a click, opens and closes the preview where there is no
    // pointer to hover with.
    block.addEventListener('click', () => {
      if (!canPreview()) return;
      if (isOpenFor()) {
        if (popoverState.source === 'activate') {
          hideStatPopoverNow();
          return;
        }
        // It is already open: only the way it was opened changes. Nothing is
        // fetched and nothing is drawn a second time.
        popoverState.source = 'activate';
        setStatExpanded(metric, true);
        return;
      }
      openStatPopover(metric, games[selectedIndex].id, 'activate');
    });

    block.addEventListener('keydown', (e) => {
      if (e.key === 'Enter' || e.key === ' ' || e.key === 'Spacebar') {
        e.preventDefault();
        // The preview is the only thing this key does here: the library's own
        // Enter (open the game) must not fire as well.
        e.stopPropagation();
        if (isOpenFor()) {
          if (popoverState.source === 'activate') {
            hideStatPopoverNow();
            return;
          }
          popoverState.source = 'activate';
          setStatExpanded(metric, true);
          return;
        }
        if (canPreview()) openStatPopover(metric, games[selectedIndex].id, 'activate');
      } else if (e.key === 'Escape') {
        e.preventDefault();
        e.stopPropagation();
        hideStatPopoverNow();
      }
    });
  });
  const pv = document.getElementById('stat-popover');
  if (pv) {
    pv.addEventListener('mouseenter', cancelStatPopoverHide);
    pv.addEventListener('mouseleave', scheduleStatPopoverHide);
    pv.addEventListener('focusin', cancelStatPopoverHide);
    pv.addEventListener('focusout', (e) => {
      // The focus is only really gone when it leaves the preview as well.
      const to = e.relatedTarget;
      if (to && to.closest && to.closest('#stat-popover')) return;
      scheduleStatPopoverHide();
    });
  }
}

/* Is the focus inside the preview? */
function previewHasFocus() {
  const pv = document.getElementById('stat-popover');
  return !!(pv && pv.contains(document.activeElement));
}

/* Whether the preview is on screen right now. */
function pvIsOpen() {
  const pv = document.getElementById('stat-popover');
  return !!(pv && pv.classList.contains('show'));
}

/* The statistic that owns the preview says so, for the screen reader. */
function setStatExpanded(metric, on) {
  const block = document.getElementById('stat-block-' + metric);
  if (block) block.setAttribute('aria-expanded', on ? 'true' : 'false');
}

function cancelStatPopoverHide() {
  if (popoverTimer) { clearTimeout(popoverTimer); popoverTimer = null; }
}

function scheduleStatPopoverHide() {
  cancelStatPopoverHide();
  popoverTimer = setTimeout(hideStatPopoverNow, 240);
}

export function hideStatPopoverNow() {
  const pv = document.getElementById('stat-popover');
  if (!pv) return;
  // Any open that is still loading is abandoned.
  openSeq++;
  popoverLoading = false;
  const st = popoverState;
  if (st) setStatExpanded(st.metric, false);
  pv.classList.remove('show');
  // The statistic that opened the preview gets the focus back, so the
  // keyboard is not left on a panel that is disappearing.
  if (st && st.source !== 'hover' && pv.contains(document.activeElement)) {
    lastDismissedAt = Date.now();
    document.getElementById('stat-block-' + st.metric)?.focus({ preventScroll: true });
  }
  setTimeout(() => { if (!pv.classList.contains('show')) pv.classList.add('hidden'); }, 170);
}

/* Is the preview on screen? Used by the keyboard to dismiss it. */
export function isStatPopoverOpen() {
  return pvIsOpen();
}

async function openStatPopover(metric, gameId, source = 'hover') {
  cancelStatPopoverHide();
  const pv = document.getElementById('stat-popover');
  if (!pv) return;
  // Each open gets a number. A newer open, or a dismissal, makes the older
  // one stale, so a preview that was dismissed while its data was loading
  // never comes back on screen.
  const seq = ++openSeq;
  popoverLoading = true;
  const requested = { metric, gameId };
  // The statistic that had the preview gives it up, so only one statistic is
  // ever marked as expanded.
  if (popoverState && popoverState.metric && popoverState.metric !== metric) {
    setStatExpanded(popoverState.metric, false);
  }
  popoverState = { metric, gameId, days: 7, sessions: [], source };
  pv.classList.remove('hidden');
  setStatExpanded(metric, true);

  const g = findGameById(gameId);
  document.getElementById('sp-title').textContent =
    `${metric === 'sessions' ? 'SESSIONS' : 'PLAYTIME'} · ${g ? g.name : ''}`;
  document.getElementById('sp-total').textContent =
    metric === 'sessions' ? String(g ? (g.sessionCount || 0) : 0) : fmtTime(g ? g.playtimeMinutes : 0);

  let sessions = [];
  try {
    sessions = await window.vault.listSessions();
  } catch (e) {
    sessions = [];
  }
  // State may have changed while we fetched (e.g. hovered the other stat),
  // and the preview may have been dismissed in the meantime.
  if (seq !== openSeq) { popoverLoading = false; return; }
  if (!popoverState || popoverState.metric !== requested.metric || popoverState.gameId !== requested.gameId) {
    popoverLoading = false;
    return;
  }
  popoverLoading = false;

  popoverState.sessions = sessions;
  popoverState.days = hasActivityIn(sessions, gameId, 7) ? 7 : 30;
  renderPopoverChart();
  positionStatPopover();
  pv.classList.add('show');
  // The pointer may have left while the data was loading. A preview that
  // nobody is looking at does not stay on the screen.
  if (source === 'hover') {
    const block = document.getElementById('stat-block-' + metric);
    if (block && !block.matches(':hover')) scheduleStatPopoverHide();
  }
}

function renderPopoverChart() {
  const st = popoverState;
  if (!st) return;
  document.querySelectorAll('#sp-range .sp-range-btn').forEach(btn => {
    btn.classList.toggle('active', parseInt(btn.dataset.days, 10) === st.days);
  });
  const chartHost = document.getElementById('sp-chart');
  // Both playtime and sessions use the trend-line chart in the popover.
  const streak = renderTrendLine(chartHost, st.sessions, st.gameId, st.days, st.metric);
  setPopoverFlame(!!streak, 800);

  const buckets = buildBuckets(st.sessions, st.gameId, st.days);
  const sumMin = buckets.reduce((a, b) => a + b.minutes, 0);
  const sumCnt = buckets.reduce((a, b) => a + b.count, 0);
  const sum = document.getElementById('sp-summary');
  if (sum) {
    sum.textContent = st.metric === 'sessions'
      ? `Last ${st.days}d · ${sumCnt} session${sumCnt === 1 ? '' : 's'} · ${fmtShort(sumMin)}`
      : `Last ${st.days}d · ${fmtShort(sumMin)} · ${sumCnt} session${sumCnt === 1 ? '' : 's'}`;
  }
}

function positionStatPopover() {
  const pv = document.getElementById('stat-popover');
  const st = popoverState;
  if (!pv || !st) return;
  const anchor = document.getElementById('stat-block-' + st.metric);
  if (!anchor) return;
  const r = anchor.getBoundingClientRect();
  const w = pv.offsetWidth || 302;
  const h = pv.offsetHeight || 240;
  const left = Math.min(Math.max(12, r.left + r.width / 2 - w / 2), window.innerWidth - w - 12);
  // Always position the popover above the stat block.
  const top = Math.max(10, r.top - h - 12);
  pv.style.left = Math.round(left) + 'px';
  pv.style.top = Math.round(top) + 'px';
}

/* The preview belongs to the library, so it is put away when another page
   takes over the screen. */
onPageLeave('home', () => hideStatPopoverNow());

/* The statistics on the home screen show the game that is selected, so the
   preview follows it: the same metric, the new game. */
export function refreshStatPopover() {
  if (!pvIsOpen() || !popoverState) return;
  const game = games[selectedIndex];
  if (!game || game.id === popoverState.gameId) return;
  openStatPopover(popoverState.metric, game.id, popoverState.source);
}

/* ── Stats UI wiring ── */
document.getElementById('btn-stats').addEventListener('click', () => openStats({ view: 'overall' }));
document.getElementById('btn-stats-overall').addEventListener('click', () => setStatsView('overall'));
document.getElementById('btn-stats-games').addEventListener('click', () => setStatsView('games'));
document.getElementById('btn-stats-close').addEventListener('click', closeStats);

document.getElementById('sp-open-stats')?.addEventListener('click', () => {
  const st = popoverState;
  hideStatPopoverNow();
  if (st && st.gameId) openStats({ view: 'games', gameId: st.gameId });
  else openStats({ view: 'overall' });
});

document.querySelectorAll('#sp-range .sp-range-btn').forEach(btn => {
  btn.addEventListener('click', () => {
    const st = popoverState;
    if (!st) return;
    st.days = parseInt(btn.dataset.days, 10);
    renderPopoverChart();
    positionStatPopover();
  });
});
