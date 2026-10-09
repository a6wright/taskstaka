// taskstaka web app: one shell, sidebar · list · side pane. Everything talks to
// the JSON API every other client uses. No build step; vanilla JS.
(() => {
'use strict';

// ---------- state ----------
const S = {
  view: 'outline', ctx: 'work', open: null,     // route
  stackId: null,                             // the leaf the Stack view is on
  tree: [], next: null, waiting: [], nudges: [], all: [], settings: { notifications: true, dir: null, running_dir: '', restart_needed: false }, dirDraft: null,
  rev: 0, dirty: false, pendingRefresh: false, focus: null,
  showDone: false,                           // outline: show completed tasks
  showNotes: false,                          // outline: show each task's notes under it
  plan: [], expanded: new Set(),             // next: stories, and which 'later' lists are open
  // The always-present capture box at the bottom of the outline keeps its
  // indentation across Enters, like Notion: Tab nests it under the row
  // above, and each created task becomes the `after` for the next.
  draft: { parent: null, after: null, depth: 0 },
};
try { S.showDone = localStorage.getItem('taskstaka.showDone') === '1'; S.showNotes = localStorage.getItem('taskstaka.showNotes') === '1'; } catch (_) {}
const VIEW_HELP = {
  outline: '',
  next: 'What do I have to do now? The task in hand sits on top — the one you most recently started. Below, each story: every stream you could work on right now (they are parallel; pick any), and folded away, everything still required to finish it.',
  waiting: 'People you are waiting on, and for what. Chase when you nudge them; unblock when they get back to you.',
  someday: 'Parked ideas. They do not block anything and stay out of Next until you revive them.',
  done: 'Finished in the last two weeks.',
  nudges: 'Reminders the rules are raising right now: things stuck in doing, follow-ups due, stories rotting untouched, too much in flight.',
};
const VIEWS = {
  outline: { ico: '☰', label: 'Outline' },
  next:    { ico: '▸', label: 'Next' },
  waiting: { ico: '✉', label: 'Waiting on' },
  someday: { ico: '💤', label: 'Someday' },
  done:    { ico: '✓', label: 'Done' },
  nudges:  { ico: '⚑', label: 'Nudges' },
  settings: { ico: '⚙', label: 'Settings' },   // reached from the sidebar footer, not the nav list
};

const $ = (sel, el = document) => el.querySelector(sel);
const esc = (s) => String(s ?? '').replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));
const ago = (iso) => {
  if (!iso) return '';
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (s < 60) return '<1m'; if (s < 3600) return Math.floor(s / 60) + 'm';
  if (s < 86400) return Math.floor(s / 3600) + 'h'; if (s < 14 * 86400) return Math.floor(s / 86400) + 'd';
  return Math.floor(s / (7 * 86400)) + 'w';
};
// Local calendar date (the daemon ranks by its local day too); toISOString would be UTC.
const localIso = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
const today = () => localIso(new Date());

// ---------- api ----------
async function api(method, path, body) {
  const res = await fetch(path, { method, headers: body ? { 'content-type': 'application/json' } : {}, body: body ? JSON.stringify(body) : undefined });
  const text = await res.text();
  let json = null; try { json = JSON.parse(text); } catch (_) {}
  if (!res.ok) throw new Error((json && json.error && json.error.message) || (res.status + ' ' + text));
  return json;
}
const q = (extra = {}) => {
  const p = new URLSearchParams();
  if (S.ctx) p.set('context', S.ctx);
  for (const [k, v] of Object.entries(extra)) p.set(k, v);
  const s = p.toString(); return s ? '?' + s : '';
};

async function load() {
  if (S.dirty) { S.pendingRefresh = true; return; }
  const [tree, next, plan, waiting, nudges, all, settings] = await Promise.all([
    api('GET', '/api/v1/tree' + q({ include_closed: 'true' })),
    api('GET', '/api/v1/next' + q()),
    api('GET', '/api/v1/plan' + q()),
    api('GET', '/api/v1/waiting' + q()),
    api('GET', '/api/v1/nudges'),
    api('GET', '/api/v1/tasks' + q({ include_closed: 'true' })),
    api('GET', '/api/v1/settings'),
  ]);
  Object.assign(S, { tree, next, plan, waiting, nudges, all, settings, rev: next.rev });
  render();
}

// ---------- routing ----------
function routeFromUrl() {
  const path = location.pathname.replace(/\/+$/, '') || '/';
  const sp = new URLSearchParams(location.search);
  // No param means work (the default tab); 'all' is spelled out.
  const ctx = sp.get('context');
  S.ctx = ctx === 'all' ? '' : (ctx === 'personal' ? 'personal' : 'work');
  const m = path.match(/^\/t\/([0-9a-z]+)$/);
  if (m) { S.view = sp.get('view') || 'outline'; S.open = m[1]; return; }
  S.view = path === '/' ? 'outline' : path.slice(1);
  if (S.view === 'stack') S.view = 'next';
  if (!VIEWS[S.view]) S.view = 'outline';
  S.open = sp.get('open');
  if (sp.get('task')) S.stackId = sp.get('task');
}
function urlFor({ view = S.view, ctx = S.ctx, open = S.open, stackId = S.stackId } = {}) {
  const sp = new URLSearchParams();
  if (ctx !== 'work') sp.set('context', ctx || 'all');
  if (open) sp.set('open', open);
  if (view === 'next' && stackId) sp.set('task', stackId);
  const qs = sp.toString();
  return (view === 'outline' ? '/' : '/' + view) + (qs ? '?' + qs : '');
}
// The server filters tree/next/plan/waiting by context, so a context change
// must refetch; a view change alone only needs a re-render.
function go(opts) {
  const ctxChanged = opts.ctx !== undefined && opts.ctx !== S.ctx;
  Object.assign(S, opts);
  history.pushState(null, '', urlFor());
  if (ctxChanged) load().catch((e) => flash(e.message, 'error'));
  else render();
}
window.addEventListener('popstate', () => {
  const prev = S.ctx;
  routeFromUrl();
  if (S.ctx !== prev) load().catch((e) => flash(e.message, 'error'));
  else render();
});

// ---------- flash ----------
function flash(msg, kind) {
  const el = $('#flash');
  el.textContent = msg; el.className = 'flash ' + (kind || ''); el.hidden = !msg;
  clearTimeout(flash.t); if (msg) flash.t = setTimeout(() => { el.hidden = true; }, 6000);
}
function announce(out) {
  const title = (id) => { const t = byId(id); return t ? t.title : id; };
  const back = out.events.find(e => e.kind === 'resurfaced');
  if (back) return flash('↑ ' + title(back.id) + ' is ready: all its prerequisites are done', 'ok');
  const rep = out.events.find(e => e.kind === 'reply_received');
  if (rep) return flash(rep.person + ' replied on ' + title(rep.id), 'ok');
  const ch = out.events.find(e => e.kind === 'chased');
  if (ch) return flash(ch.deduped ? 'already chased ' + ch.person + ' today' : 'chased ' + ch.person + ' (' + ch.count + '×)', 'ok');
  if (out.warnings && out.warnings.length) flash(out.warnings[0], '');
}
async function act(fn, keep) {
  try { const out = await fn(); if (out && out.events) announce(out); if (keep) S.focus = keep; await load(); shiftStack(out); if (S.view === 'next') render(); return out; }
  catch (e) { flash(e.message, 'error'); await load(); return null; }
}

// ---------- lookups ----------
const byId = (id) => S.all.find(t => t.id === id);
const childrenOf = (id) => S.all.filter(t => t.parent === id).sort((a, b) => (a.order ?? Date.parse(a.created) / 1000) - (b.order ?? Date.parse(b.created) / 1000));
// A project pill: the project's own colour, inherited from the root story.
const pill = (tag) => tag ? `<span class="proj" style="--pc:${esc(tag.color)}" title="project: ${esc(tag.name)}">${esc(tag.name)}</span>` : '';
const isClosed = (t) => t.status === 'done' || t.status === 'dropped';
const recent = (iso) => iso && (Date.now() - Date.parse(iso)) < 86400e3;

// ---------- render ----------
function render() {
  // Re-renders happen for many reasons (an action, a live update from
  // another client); none of them may steal the caret.
  if (!S.focus) S.focus = captureFocus();
  const app = $('#app');
  app.classList.toggle('pane-open', !!S.open);
  renderSide(); renderMain(); renderPane();
  if (S.focus) { placeFocus(S.focus); S.focus = null; }
}
function captureFocus() {
  const ae = document.activeElement;
  if (!ae || !ae.closest) return null;
  if (ae.classList.contains('pane-title')) return { pane: true, offset: caretOffset(ae) };
  const tl = ae.closest('.title'); if (!tl) return null;
  const li = rowOf(tl);
  if (li) return { id: li.dataset.id, offset: caretOffset(tl) };
  if (tl.closest('.newline')) return { id: null, atEnd: true, text: tl.textContent };
  return null;
}

function renderSide() {
  const sum = S.next ? S.next.summary : {};
  const someday = S.all.filter(t => t.status === 'someday' && inCtx(t)).length;
  const done = S.all.filter(t => isClosed(t) && inCtx(t) && t.completed && (Date.now() - Date.parse(t.completed)) < 14 * 86400e3).length;
  const counts = { nudges: S.nudges.length || '', outline: '', next: sum.actionable ?? '', waiting: S.waiting.reduce((n, g) => n + g.items.length, 0) || '', someday: someday || '', done: done || '' };
  $('#side').innerHTML = `
    <div class="brand"><span><img class="mark" src="/mark.png" alt="">taskstaka</span></div>
    <div class="ctxs">${['', 'work', 'personal'].map(c => `<a href="${urlFor({ ctx: c })}" data-ctx="${c}" class="${S.ctx === c ? 'active' : ''}">${c || 'all'}</a>`).join('')}</div>
    <nav class="nav">${Object.entries(VIEWS).filter(([k]) => k !== 'settings').map(([k, v]) => `<a href="${urlFor({ view: k })}" data-view="${k}" class="${S.view === k ? 'active' : ''}"><span class="ico">${v.ico}</span>${v.label}<span class="count">${counts[k]}</span></a>`).join('')}</nav>
    <div class="sidefoot"><a href="${urlFor({ view: 'settings' })}" data-view="settings" class="${S.view === 'settings' ? 'active' : ''}"><span class="ico">⚙</span>Settings</a></div>
`;
}

function inCtx(t) {
  if (!S.ctx) return true;
  let r = t; const seen = new Set();
  while (r && r.parent && !seen.has(r.id)) { seen.add(r.id); const p = byId(r.parent); if (!p) break; r = p; }
  return r.context === S.ctx;
}

function renderMain() {
  const sum = S.next ? S.next.summary : null;
  const line = !sum ? '' : sum.open === 0 ? 'nothing open'
    : sum.actionable === 0 ? `<strong>nothing actionable</strong> · ${sum.blocked} blocked · ${sum.waiting} waiting — go unblock something`
    : `${sum.actionable} actionable · ${sum.doing} doing · ${sum.blocked} blocked · ${sum.waiting} waiting · ${sum.someday} someday`;
  const v = VIEWS[S.view];
  let body = '';
  if (S.view === 'outline') body = renderOutline();
  else if (S.view === 'next') body = renderNext();
  else if (S.view === 'waiting') body = renderWaiting();
  else if (S.view === 'someday') body = renderSomeday();
  else if (S.view === 'done') body = renderDone();
  else if (S.view === 'nudges') body = renderNudges();
  else if (S.view === 'settings') body = renderSettings();
  const doneToggle = S.view === 'outline'
    ? `<button class="toggle ${S.showNotes ? 'on' : ''}" id="toggle-notes" title="show each task's notes beneath it">≡ notes</button><button class="toggle ${S.showDone ? 'on' : ''}" id="toggle-done" title="show completed tasks">✓ completed</button>` : '';
  $('#main').innerHTML = `
    <div class="head"><h1>${v.label}</h1><span class="sum">${S.view === 'settings' ? '' : line}</span><span class="sp"></span>${doneToggle}<span class="sum">${S.view === 'settings' ? '' : (S.ctx || 'all')}</span></div>
    ${VIEW_HELP[S.view] ? `<p class="viewhelp">${esc(VIEW_HELP[S.view])}</p>` : ''}
    <div id="flash" class="flash" hidden></div>
    ${S.nudges.length && S.view !== 'waiting' ? `<div class="nudge">⚑ ${esc(S.nudges[0].message)}</div>` : ''}
    <div class="content">${body}</div>`;
}

// chips: the one thing that matters, quietly
function chips(t, d, extra = '') {
  const out = [];
  if (t.status === 'doing') out.push(['doing', '▶ doing ' + ago(t.started || t.created)]);
  if (t.status === 'waiting') out.push(['wait', '✉ ' + esc(t.waiting_on || '') + ' · ' + ago(t.waiting_since || t.updated) + (t.chases && t.chases.length ? ' · ' + t.chases.length + '×' : '') + (t.until && t.until <= today() ? ' · follow up' : '')]);
  else if (d && d.waiting_on_summary) out.push(['wait', '✉ ' + esc(d.waiting_on_summary.person) + ' · ' + ago(d.waiting_on_summary.since)]);
  else if (d && d.blocked_by > 0) out.push(['', '▽ blocked by ' + d.blocked_by]);
  if (t.status === 'someday') out.push(['someday', '💤 someday']);
  if (d && d.resurfaced_at) out.push(['back', '↑ ready']);
  if (d && d.orphaned) out.push(['orphan', '⚠ parent finished']);
  if (t.due && !isClosed(t)) out.push([t.due < today() ? 'overdue' : '', (t.due < today() ? '⚠ overdue ' : '📅 ') + t.due]);
  for (const tag of (t.tags || [])) out.push([tag === 'agent' ? 'agent' : 'tag', (tag === 'agent' ? '🤖 ' : '#') + esc(tag)]);
  return out.map(([c, s]) => `<span class="chip ${c}">${s}</span>`).join('') + extra;
}
function hoverMeta(t) {
  const bits = [];
  if (t.priority) bits.push(t.priority);
  bits.push(ago(t.created));
  // start/stop is the one status change that is not a checkbox, so it
  // lives on the row too, not only in the pane.
  const startStop = t.status === 'doing'
    ? `<button class="rowbtn" data-status="todo" data-id="${t.id}" title="stop working on this">stop</button>`
    : t.status === 'waiting' ? `<button class="rowbtn start" data-status="doing" data-id="${t.id}" title="they got back to you — start on it">unblock</button>`
    : (t.status === 'todo' || t.status === 'someday') ? `<button class="rowbtn start" data-status="doing" data-id="${t.id}" title="start working on this (▶)">▶ start</button>` : '';
  const first = isClosed(t) ? '' : `<button class="rowbtn" data-first="draft" data-id="${t.id}" title="add something that must happen before this">↳ first</button>`;
  return `<span class="hover">${bits.map(b => `<span>${esc(b)}</span>`).join('')}${first}${startStop}<button class="openbtn" data-open="${t.id}">open</button></span>`;
}
function rowHtml(t, d, depth, { editable = true, crumb = '', extra = '' } = {}) {
  const closed = isClosed(t);
  // A root story's project leads its row, left of the title.
  const lead = depth === 0 && editable && t.project_tag ? pill(t.project_tag) : '';
  const cls = ['row', lead ? 'has-proj' : '', editable ? '' : 'compact', closed ? 'closed' : '', t.status === 'someday' ? 'someday' : '', d && d.blocked_by > 0 ? 'blocked' : '', d && d.deferred_by && d.deferred_by !== t.id ? 'deferred' : '', S.open === t.id ? 'selected' : ''].join(' ');
  return `<li class="${cls}" data-id="${t.id}" data-parent="${t.parent || ''}" data-depth="${depth}">
    ${editable ? `<span class="grip" draggable="true" title="drag to reorder or nest">⋮⋮</span>` : ''}
    <input type="checkbox" class="cb" ${closed ? 'checked' : ''} title="${closed ? 'reopen' : 'done'}">
    ${lead}<span class="title" ${editable ? 'contenteditable="true" spellcheck="false"' : ''}>${esc(t.title)}${crumb}</span>
    <span class="meta">${chips(t, d)}${hoverMeta(t)}${extra}</span>
    ${editable && S.showNotes ? noteHtml(t) : ''}
  </li>`;
}
const isJournalLine = (l) => /^- \d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(l);
function noteHtml(t) {
  const notes = (t.body || '').split('\n').filter(l => !isJournalLine(l)).join('\n').trim();
  return notes ? `<div class="note">${esc(notes)}</div>` : '';
}

function renderOutline() {
  const parts = [];
  // Completed rows stay visible for a few minutes after you tick them (so
  // the tick is not a vanishing act), then hide unless "✓ completed" is on.
  const justDone = (t) => t.completed && (Date.now() - Date.parse(t.completed)) < 5 * 60e3;
  const walk = (nodes, depth) => {
    parts.push('<ul class="rows">');
    for (const n of nodes) {
      const t = n.task;
      if (isClosed(t) && !S.showDone && !justDone(t)) continue;
      if (depth === 0 && t.status === 'someday') continue;   // the icebox has its own view
      parts.push(rowHtml(t, t.derived, depth));
      if (n.children && n.children.length) { parts.push('<li class="kids">'); walk(n.children, depth + 1); parts.push('</li>'); }
    }
    parts.push('</ul>');
  };
  if (!S.tree.length) parts.push('<p class="empty">Nothing here yet. Type below.</p>');
  else walk(S.tree, 0);
  // If the draft's parent vanished (done, deleted), fall back to the root.
  if (S.draft.parent && !byId(S.draft.parent)) S.draft = { parent: null, after: null, depth: 0 };
  const d = S.draft;
  parts.push(`<div class="newline" style="--depth:${d.depth}"><span class="plus">＋</span><span class="title draft" contenteditable="true" spellcheck="false" data-parent="${d.parent || ''}" data-after="${d.after || ''}" data-placeholder="${d.depth ? 'add a subtask… (Shift+Tab to outdent)' : 'add a task… (Tab to nest under the row above, @Name to wait on someone)'}"></span></div>`);
  return parts.join('');
}

function renderNext() {
  const plans = S.plan || [];
  const inHand = pickStackTask();
  const parts = [];
  if (inHand && !isClosed(inHand)) parts.push(renderPlate(inHand));
  if (!plans.length) {
    parts.push(inHand ? '' : '<p class="empty">Nothing to do. Add something in the <a href="/" data-view="outline">Outline</a>, or check <a href="/waiting" data-view="waiting">Waiting on</a>.</p>');
    return parts.join('');
  }
  // Each story is a stack of plates: what you can do now on top, what is
  // still required fading beneath, the story itself at the base.
  for (const p of plans) {
    const open = S.expanded.has(p.id);
    const meta = [];
    if (p.now.length) meta.push(`${p.now.length} now`);
    if (p.later.length) meta.push(p.later_agent ? `${p.later.length} later (${p.later_agent} for Claude)` : `${p.later.length} later`);
    if (p.stuck) meta.push(`<span class="stuck">${esc(p.stuck)}</span>`);
    else if (p.waiting_on.length) meta.push('waiting on ' + esc(p.waiting_on.join(', ')));
    parts.push(`<div class="story stack">`);
    let level = 0;
    if (p.now.length) {
      parts.push(`<div class="plate now" style="--i:${level}"><div class="rel">now${pill((byId(p.id) || {}).project_tag)}</div><ul class="rows">`);
      for (const it of p.now) {
        const t = byId(it.id); if (!t) continue;
        const chain = [...it.ancestry].reverse().slice(0, -1).map(c => esc(c.title)).join(' → ');
        const crumb = chain ? `<span class="crumb" title="finishing this unlocks: ${chain}">→ ${chain}</span>` : '';
        parts.push(rowHtml(t, t.derived, 0, { editable: false, crumb }).replace('<li class="row', `<li title="${esc(it.reason)}" class="row`));
      }
      parts.push('</ul></div>');
      level++;
    }
    if (p.later.length) {
      if (open) {
        parts.push(`<div class="plate later" style="--i:${level}"><div class="rel"><button class="later-toggle" data-expand="${p.id}">▾ ${p.later.length} later</button></div><ul class="rows later">`);
        for (const l of p.later) {
          const t = byId(l.id); if (!t) continue;
          const mark = l.status === 'waiting' ? '✉' : l.status === 'doing' ? '▶' : l.agent ? '🤖' : '▽';
          parts.push(`<li class="row compact later-row" data-id="${l.id}"><span class="mark">${mark}</span><span class="title"><a href="#" data-open="${l.id}">${esc(l.title)}</a><span class="crumb">${esc(l.why)}</span></span><span class="meta">${hoverMeta(t)}</span></li>`);
        }
        parts.push('</ul></div>');
      } else {
        const peek = p.later.slice(0, 2).map(l => esc(l.title)).join(', ') + (p.later.length > 2 ? `, +${p.later.length - 2}` : '');
        parts.push(`<div class="plate later folded" style="--i:${level}"><button class="later-toggle" data-expand="${p.id}">… ${p.later.length} later: ${peek}</button></div>`);
      }
      level++;
    }
    parts.push(`<div class="plate base" style="--i:${level}"><div class="story-head"><span class="rel">so that</span>${pill((byId(p.id) || {}).project_tag)}<a href="#" data-open="${p.id}" class="story-title">${esc(p.title)}</a><span class="story-meta">${meta.join(' · ')}</span></div></div>`);
    parts.push('</div>');
  }
  return parts.join('');
}

function renderWaiting() {
  if (!S.waiting.length) return '<p class="empty">Not waiting on anyone.</p>';
  return S.waiting.map(g => `
    <div class="person"><div class="who">${esc(g.person)} <span class="chip">${g.items.length}</span>${g.any_due ? '<span class="chip overdue">follow-up due</span>' : ''}</div>
    <ul class="rows">${g.items.map(i => {
      const t = byId(i.task_id); if (!t) return '';
      const crumb = i.root_title ? `<span class="crumb">‹ ${esc(i.root_title)}</span>` : '';
      const extra = `<button class="btn small" data-chase="${t.id}" title="I pinged them again">chase</button>${i.waiting_ref ? `<a class="chip" href="${esc(i.waiting_ref)}" target="_blank" rel="noopener">thread ↗</a>` : ''}`;
      return rowHtml(t, t.derived, 0, { editable: false, crumb, extra });
    }).join('')}</ul></div>`).join('');
}

function renderSomeday() {
  // Parked stories are a real outline — refine the plan here, then revive it.
  // Parked leaves under live parents are just listed.
  const roots = (S.tree || []).filter(n => n.task.status === 'someday');
  const leaves = S.all.filter(t => t.status === 'someday' && inCtx(t) && t.parent && byId(t.parent) && byId(t.parent).status !== 'someday');
  const parts = [];
  if (!roots.length && !leaves.length) parts.push('<p class="empty">The icebox is empty. Park a task with <kbd>someday</kbd> in its pane, or add one below.</p>');
  const walk = (nodes, depth) => {
    parts.push('<ul class="rows">');
    for (const n of nodes) {
      const t = n.task;
      if (isClosed(t) && !S.showDone) continue;
      let row = rowHtml(t, t.derived, depth);
      if (depth === 0) row = row.replace('<span class="meta">', `<span class="meta"><button class="btn small" data-status="todo" data-id="${t.id}" data-cascade="0" title="back into play">revive</button>`);
      parts.push(row);
      if (n.children && n.children.length) { parts.push('<li class="kids">'); walk(n.children, depth + 1); parts.push('</li>'); }
    }
    parts.push('</ul>');
  };
  if (roots.length) { parts.push('<div class="group"><span>parked stories</span><span class="n">' + roots.length + '</span></div>'); walk(roots, 0); }
  if (leaves.length) {
    parts.push('<div class="group"><span>parked under live stories</span><span class="n">' + leaves.length + '</span></div><ul class="rows">');
    for (const t of leaves) {
      const p = byId(t.parent);
      parts.push(rowHtml(t, t.derived, 0, { editable: false, crumb: `<span class="crumb">‹ ${esc(p.title)}</span>` })
        .replace('<span class="meta">', `<span class="meta"><button class="btn small" data-status="todo" data-id="${t.id}">revive</button>`));
    }
    parts.push('</ul>');
  }
  if (S.draft.parent && !byId(S.draft.parent)) S.draft = { parent: null, after: null, depth: 0 };
  const d = S.draft;
  parts.push(`<div class="newline" style="--depth:${d.depth}"><span class="plus">＋</span><span class="title draft" contenteditable="true" spellcheck="false" data-parent="${d.parent || ''}" data-after="${d.after || ''}" data-placeholder="park an idea… (it stays out of Next until you revive it)"></span></div>`);
  return parts.join('');
}

function renderDone() {
  const list = S.all.filter(t => isClosed(t) && inCtx(t)).sort((a, b) => Date.parse(b.completed || 0) - Date.parse(a.completed || 0)).slice(0, 200);
  if (!list.length) return '<p class="empty">Nothing finished yet.</p>';
  return `<ul class="rows">${list.map(t => rowHtml(t, t.derived, 0, { editable: false, crumb: `<span class="crumb">${t.status} ${ago(t.completed)} ago</span>` })).join('')}</ul>`;
}

// ---------- pane ----------
function renderPane() {
  const pane = $('#pane');
  if (!S.open) { pane.innerHTML = ''; return; }
  const t = byId(S.open);
  if (!t) { pane.innerHTML = `<div class="pane-head"><span class="sp"></span><button class="btn small" data-close>✕</button></div><div class="pane-body"><p class="empty">No such task.</p></div>`; return; }
  const d = t.derived || {};
  const anc = []; let p = t.parent && byId(t.parent); const seen = new Set();
  while (p && !seen.has(p.id)) { seen.add(p.id); anc.unshift(p); p = p.parent && byId(p.parent); }
  const kids = childrenOf(t.id);
  const facts = [];
  if (d.blocked_by) facts.push('blocked by ' + d.blocked_by);
  if (d.actionable) facts.push('actionable');
  if (d.resurfaced_at) facts.push('resurfaced ' + ago(d.resurfaced_at) + ' ago');
  if (d.orphaned) facts.push('parent finished');
  const journal = (t.body || '').split('\n').filter(l => /^- \d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(l));
  const notes = (t.body || '').split('\n').filter(l => !/^- \d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(l)).join('\n').trim();
  const sel = (v, cur) => v === cur ? ' selected' : '';
  const st = t.status;
  const actions = [];
  if (st === 'todo' || st === 'someday') actions.push(['doing', 'start', '']);
  if (!isClosed(t)) actions.push(['done', 'done', 'primary']);
  if (st === 'doing') actions.push(['todo', 'stop', '']);
  if (st === 'waiting') actions.push(['doing', 'unblock', 'primary']);
  if (st !== 'someday' && !isClosed(t)) actions.push(['someday', 'someday', '']);
  if (!isClosed(t)) actions.push(['dropped', 'drop', 'danger']);
  if (isClosed(t)) actions.push(['todo', 'reopen', '']);
  pane.innerHTML = `
    <div class="pane-head"><span class="crumbs">${anc.map(a => `<a href="#" data-open="${a.id}">${esc(a.title)}</a> › `).join('')}<span class="idtag">${t.id}</span></span><span class="sp"></span><a class="btn small" href="/t/${t.id}" title="link to this task">↗</a><button class="btn small" data-close title="close (Esc)">✕</button></div>
    <div class="pane-body">
      <div class="pane-title" contenteditable="true" spellcheck="false" data-title="${t.id}">${esc(t.title)}</div>
      <div class="meta">${chips(t, d)}</div>
      <div class="actions">${actions.map(([s, l, c]) => `<button class="btn ${c}" data-status="${s}" data-id="${t.id}" data-cascade="1">${l}</button>`).join('')}</div>
      <div class="props">
        <label>status</label><span class="ro">${st}${facts.length ? ' · ' + facts.join(' · ') : ''}</span>
        <label>context</label><select data-prop="context" data-id="${t.id}"><option value="work"${sel('work', t.context)}>work</option><option value="personal"${sel('personal', t.context)}>personal</option></select>
        <label>priority</label><select data-prop="priority" data-id="${t.id}"><option value=""${sel(null, t.priority)}>normal</option><option value="high"${sel('high', t.priority)}>high</option><option value="low"${sel('low', t.priority)}>low</option></select>
        <label>due</label><input data-prop="due" data-id="${t.id}" value="${esc(t.due || '')}" placeholder="YYYY-MM-DD or +3d">
        <label>tags</label><input data-prop="tags" data-id="${t.id}" value="${esc((t.tags || []).join(', '))}" placeholder="comma, separated">
        ${t.parent ? `<label>project</label><span class="ro">${t.project_tag ? pill(t.project_tag) + ' (set on the root story)' : 'none (set on the root story)'}</span>`
          : `<label>project</label><input data-prop="project" data-id="${t.id}" list="project-names" value="${esc(t.project || '')}" placeholder="e.g. taskstaka, devops">
        <datalist id="project-names">${[...new Set(S.all.map(x => x.project).filter(Boolean))].map(n => `<option value="${esc(n)}">`).join('')}</datalist>`}
        <label>parent</label><input data-prop="parent" data-id="${t.id}" value="${esc(t.parent || '')}" placeholder="id (blank = root)">
        <label>queued</label><span class="ro">${ago(t.created)} · touched ${ago(t.updated)}${t.started ? ' · doing ' + ago(t.started) : ''}</span>
        ${st === 'waiting' ? `
        <label>waiting on</label><input data-prop="waiting_on" data-id="${t.id}" value="${esc(t.waiting_on || '')}">
        <label>since</label><span class="ro">${ago(t.waiting_since || t.updated)} · chased ${(t.chases || []).length}× · follow up ${esc(t.until || '')}</span>
        <label>thread</label><input data-prop="waiting_ref" data-id="${t.id}" value="${esc(t.waiting_ref || '')}" placeholder="link to the conversation">` : ''}
      </div>
      ${st === 'waiting' ? `
      <div class="inline-form"><input id="chase-note" placeholder="chased them again — note (optional)"><button class="btn" data-chase="${t.id}">chase</button></div>` : `
      <div class="inline-form"><input class="wait-who" placeholder="waiting on…"><input class="wait-until" placeholder="until (+3d)" style="max-width:110px"><button class="btn" data-wait="${t.id}">wait</button></div>`}
      <h3>first I need to…</h3>
      <ul class="rows subs">${kids.map(k => rowHtml(k, k.derived, 0, { editable: false })).join('')}</ul>
      <div class="inline-form"><input id="sub-title" placeholder="something that must happen before this"><button class="btn" data-addsub="${t.id}">add</button></div>
      <h3>notes</h3>
      <textarea class="notes" data-notes="${t.id}" placeholder="markdown notes">${esc(notes)}</textarea>
      ${journal.length ? `<h3>activity</h3><ul class="journal">${journal.map(l => {
        const m = l.match(/^- (\d{4}-\d{2}-\d{2} \d{2}:\d{2})(?: \[(\w+)\])? (.*)$/);
        return m ? `<li><span class="when">${m[1]}</span>${m[2] ? `<span class="src">${esc(m[2])}</span>` : ''}${esc(m[3])}</li>` : `<li>${esc(l)}</li>`;
      }).join('')}</ul>` : ''}
      <div class="actions" style="margin-top:20px"><button class="btn danger small" data-delete="${t.id}">delete</button></div>
    </div>`;
}

// ---------- editing helpers ----------
function placeCaret(el, atEnd, offset) {
  el.focus();
  const range = document.createRange(); const node = el.firstChild;
  if (!node) range.setStart(el, 0);
  else { const len = node.textContent.length; range.setStart(node, atEnd ? len : Math.min(offset || 0, len)); }
  range.collapse(true); const s = window.getSelection(); s.removeAllRanges(); s.addRange(range);
}
function placeFocus(f) {
  const el = f.pane ? $('#pane .pane-title') : f.id ? $(`#main li[data-id="${f.id}"] .title`) : $('#main .newline .title');
  if (!el) return;
  if (f.text && !el.textContent) { el.textContent = f.text; S.dirty = true; }
  placeCaret(el, f.atEnd || f.text !== undefined, f.offset);
}
const rowOf = (el) => el.closest('li.row');
const titleEls = () => [...document.querySelectorAll('#main .title')];
const caretOffset = (el) => { const s = window.getSelection(); if (!s.rangeCount) return 0; const r = s.getRangeAt(0).cloneRange(); r.selectNodeContents(el); r.setEnd(s.getRangeAt(0).endContainer, s.getRangeAt(0).endOffset); return r.toString().length; };
function prevSibling(li) { let p = li.previousElementSibling; while (p) { if (p.classList.contains('row')) return p; p = p.previousElementSibling; } return null; }

async function commitTitle(el) {
  const li = rowOf(el); const id = li ? li.dataset.id : el.dataset.title;
  if (!id) return;
  const text = el.textContent.trim(); const original = el.dataset.original;
  S.dirty = false;
  if (text && original !== undefined && text !== original) { el.dataset.original = text; await act(() => api('PATCH', '/api/v1/tasks/' + id, { title: text })); }
  if (S.pendingRefresh) { S.pendingRefresh = false; load(); }
}
async function createDraft(el, parent, after) {
  const text = el.textContent.trim(); if (!text) return;
  el.textContent = ''; S.dirty = false;
  let title = text, waiting_on = null, until = null;
  const m = text.match(/^(.*\S)\s+@(\S+)(?:\s+(\+\d+[dwm]|\d{4}-\d{2}-\d{2}|today|tomorrow))?\s*$/);
  if (m) {
    title = m[1]; waiting_on = m[2];
    try { until = m[3] ? await resolveDate(m[3]) : null; } catch (err) { el.textContent = text; return flash(err.message, 'error'); }
  }
  const fromBottom = !!el.closest('.newline');
  const parked = S.view === 'someday' && !parent ? 'someday' : null;
  // "Goal > intermediate > first thing": the planning direction, left to right.
  // Each segment nests under the one before; @Name applies to the last.
  const chain = title.split(/\s+>\s+/).map(s => s.trim()).filter(Boolean);
  if (chain.length > 1) {
    S.dirty = false;
    try {
      let p = parent || null, created = null;
      for (let i = 0; i < chain.length; i++) {
        const last = i === chain.length - 1;
        const out = await api('POST', '/api/v1/tasks', { title: chain[i], parent: p, after: (i === 0 ? (after || null) : null), context: (!p && S.ctx) ? S.ctx : null, waiting_on: last ? waiting_on : null, until: last ? until : null, status: (i === 0 ? parked : null) });
        p = out.primary; created = created || out.primary;
      }
      if (fromBottom) { S.draft.after = created; }
      S.focus = { id: null, atEnd: true };
      flash(`stacked ${chain.length} tasks: ${chain[chain.length - 1]} first`, 'ok');
      await load();
    } catch (e) { flash(e.message, 'error'); await load(); }
    return;
  }
  const out = await act(() => api('POST', '/api/v1/tasks', { title, parent: parent || null, after: after || null, context: (!parent && S.ctx) ? S.ctx : null, waiting_on, until, status: parked }), { id: null, atEnd: true });
  // Rapid capture: the next line from the bottom box goes right after this one.
  if (fromBottom && out && out.primary) { S.draft.after = out.primary; const box = $('#main .newline .title'); if (box) box.dataset.after = out.primary; }
}

// Tab / Shift+Tab in the bottom box: nest under the row above (at the same
// depth), or step back out. Purely local until the line is submitted.
function indentBottomDraft(el, outdent) {
  const rows = [...document.querySelectorAll('#main li.row')];
  if (outdent) {
    if (!S.draft.depth) return;
    const p = byId(S.draft.parent);
    S.draft = { parent: p && p.parent ? p.parent : null, after: p ? p.id : null, depth: S.draft.depth - 1 };
  } else {
    const anchor = [...rows].reverse().find(li => +li.dataset.depth === S.draft.depth && (li.dataset.parent || '') === (S.draft.parent || ''));
    if (!anchor) return;
    S.draft = { parent: anchor.dataset.id, after: null, depth: S.draft.depth + 1 };
  }
  el.dataset.parent = S.draft.parent || ''; el.dataset.after = S.draft.after || '';
  const box = el.closest('.newline'); if (box) box.style.setProperty('--depth', S.draft.depth);
  el.dataset.placeholder = S.draft.depth ? 'add a subtask… (Shift+Tab to outdent)' : 'add a task… (Tab to nest under the row above, @Name to wait on someone)';
}

// ---------- events ----------
document.addEventListener('click', async (e) => {
  const a = e.target.closest('a[data-view], a[data-ctx], a[data-open]');
  if (a) {
    e.preventDefault();
    if (a.dataset.view) return go({ view: a.dataset.view });
    if (a.dataset.ctx !== undefined) return go({ ctx: a.dataset.ctx });
    if (a.dataset.open) return go({ open: a.dataset.open });
  }
  const b = e.target.closest('button');
  if (!b) return;
  const id = b.dataset.id || b.dataset.open || b.dataset.chase || b.dataset.reply || b.dataset.wait || b.dataset.addsub || b.dataset.delete;
  if (b.dataset.open) return go({ open: b.dataset.open });
  if (b.hasAttribute('data-close') || b.id === 'pane-toggle') return go({ open: null });
  if (b.id === 'save-dir') {
    const dir = ($('#set-dir') || {}).value || '', msg = $('#setmsg'), was = S.settings.running_dir;
    b.disabled = true; msg.className = 'setnote'; msg.textContent = 'Saving…';
    try {
      const r = await api('PUT', '/api/v1/settings', { dir });
      S.dirty = false;
      if (r.restarting) {
        msg.textContent = 'Restarting on the new folder…';
        const now = await waitForNewDaemon(was);
        S.dirDraft = null; S.settings = now; await load();
        flash('Now using ' + now.running_dir, 'ok');
      } else { S.dirDraft = null; S.settings = r; render(); flash('That is already the folder in use', ''); }
    } catch (err) { msg.className = 'setnote error'; msg.textContent = err.message; b.disabled = false; }
    return;
  }
  if (b.id === 'toggle-notify') {
    try { S.settings = await api('PUT', '/api/v1/settings', { notifications: !S.settings.notifications }); } catch (err) { flash(err.message, 'error'); }
    return render();
  }
  if (b.id === 'toggle-done') { S.showDone = !S.showDone; try { localStorage.setItem('taskstaka.showDone', S.showDone ? '1' : '0'); } catch (_) {} return render(); }
  if (b.dataset.expand) { const id = b.dataset.expand; if (S.expanded.has(id)) S.expanded.delete(id); else S.expanded.add(id); return render(); }
  if (b.id === 'toggle-notes') { S.showNotes = !S.showNotes; try { localStorage.setItem('taskstaka.showNotes', S.showNotes ? '1' : '0'); } catch (_) {} return render(); }
  if (b.dataset.status) {
    const toStack = b.dataset.status === 'doing';
    return act(() => api('POST', `/api/v1/tasks/${id}/status`, { status: b.dataset.status, cascade: true }))
      .then(() => { if (toStack) go({ view: 'next', stackId: id, open: null }); });
  }
  if (b.dataset.first) {
    // "first I need to…": a prerequisite under this row (or stack plate).
    const form = b.closest('.inline-form');
    const title = (b.dataset.first === 'prompt') ? ((form && form.querySelector('.first-title')) ? form.querySelector('.first-title').value.trim() : '') : '';
    if (b.dataset.first === 'prompt') {
      if (!title) return;
      // From the plate: the new prerequisite is what you do now, so it starts
      // in hand and the daemon pauses the task above it.
      const fromPlate = !!b.closest('.plate.top');
      return act(() => api('POST', '/api/v1/tasks', { title, parent: id, status: fromPlate ? 'doing' : null }))
        .then(out => { if (fromPlate && out && out.primary) { S.stackId = out.primary; render(); } });
    }
    return insertDraftUnder(id);
  }
  if (b.dataset.chase) { const note = $('#chase-note') ? $('#chase-note').value : ''; return act(() => api('POST', `/api/v1/tasks/${id}/chase`, { note: note || null })); }
  if (b.dataset.reply) {
    const outcome = $('#reply-outcome').value, to = $('#reply-to').value.trim(), summary = $('#reply-summary').value.trim();
    if (!summary) return flash('say what they said (one line)', 'error');
    const body = { outcome: outcome === 'redirected' ? { kind: 'redirected', to } : { kind: outcome }, summary };
    return act(() => api('POST', `/api/v1/tasks/${id}/reply`, body));
  }
  if (b.dataset.wait) {
    const form = b.closest('.inline-form') || document;
    const who = form.querySelector('.wait-who').value.trim(), until = form.querySelector('.wait-until').value.trim();
    if (!who) return flash('waiting on whom?', 'error');
    let untilIso = null;
    try { untilIso = until ? await resolveDate(until) : null; } catch (err) { return flash(err.message, 'error'); }
    return act(() => api('POST', `/api/v1/tasks/${id}/status`, { status: 'waiting', waiting_on: who, until: untilIso }));
  }
  if (b.dataset.addsub) { const title = $('#sub-title').value.trim(); if (!title) return; return act(() => api('POST', '/api/v1/tasks', { title, parent: id })); }
  if (b.dataset.delete) { if (!confirm('Delete this task? (drop keeps the record)')) return; return act(() => api('DELETE', `/api/v1/tasks/${id}`), null).then(() => go({ open: null })); }
});

// Dates typed as +3d / today are resolved by the server through `until` on the
// CLI path; the JSON API expects ISO. Resolve a few forms here.
async function resolveDate(s) {
  s = s.trim(); if (!s) return null;
  if (/^\d{4}-\d{2}-\d{2}$/.test(s)) return s;
  const d = new Date();
  if (s === 'today') return localIso(d);
  if (s === 'tomorrow') { d.setDate(d.getDate() + 1); return localIso(d); }
  const m = s.match(/^\+(\d+)([dwm]?)$/);
  if (m) { const n = +m[1]; if (m[2] === 'w') d.setDate(d.getDate() + 7 * n); else if (m[2] === 'm') d.setMonth(d.getMonth() + n); else d.setDate(d.getDate() + n); return localIso(d); }
  throw new Error(`${s} is not a date; use YYYY-MM-DD, today, tomorrow or +3d`);
}

document.addEventListener('change', async (e) => {
  const el = e.target;
  if (el.classList.contains('cb')) {
    const li = rowOf(el); const id = li.dataset.id;
    return act(() => api('POST', `/api/v1/tasks/${id}/status`, { status: el.checked ? 'done' : 'todo', cascade: true }), { id, atEnd: true });
  }
  if (el.dataset.prop) {
    const id = el.dataset.id, prop = el.dataset.prop, v = el.value.trim();
    let body;
    if (prop === 'priority') body = { priority: v ? { to: v } : 'clear' };
    else if (prop === 'due') body = { due: v ? { to: await resolveDate(v).catch(err => { flash(err.message, 'error'); return undefined; }) } : 'clear' };
    else if (prop === 'tags') body = { tags: v ? v.split(',').map(x => x.trim()).filter(Boolean) : [] };
    else if (prop === 'context') body = { context: v };
    else if (prop === 'project') body = { project: v ? { to: v } : 'clear' };
    else if (prop === 'waiting_on') body = { waiting_on: v ? { to: v } : 'keep' };
    else if (prop === 'waiting_ref') body = { waiting_ref: v ? { to: v } : 'keep' };
    else if (prop === 'parent') return act(() => api('POST', `/api/v1/tasks/${id}/move`, { parent: v || null }));
    if (body && body.due && body.due.to === undefined) return;
    return act(() => api('PATCH', `/api/v1/tasks/${id}`, body));
  }
});

document.addEventListener('focusin', (e) => {
  const el = e.target.closest('.title, .pane-title, .plate-title'); if (!el) return;
  if (!el.classList.contains('draft')) el.dataset.original = el.textContent;
});
document.addEventListener('input', (e) => { if (e.target.closest('.title, .pane-title, .plate-title, .notes')) S.dirty = true; });
document.addEventListener('focusout', async (e) => {
  const el = e.target;
  if (el.classList && el.classList.contains('notes')) {
    S.dirty = false;
    const t = byId(el.dataset.notes); if (!t) return;
    const journal = (t.body || '').split('\n').filter(l => /^- \d{4}-\d{2}-\d{2} \d{2}:\d{2}/.test(l));
    const body = (el.value.trim() + (journal.length ? '\n\n' + journal.join('\n') : '')).trim() + '\n';
    if (body.trim() !== (t.body || '').trim()) await act(() => api('PATCH', '/api/v1/tasks/' + t.id, { body }));
    return;
  }
  const tl = el.closest && el.closest('.title, .pane-title, .plate-title'); if (!tl) return;
  if (tl.classList.contains('draft')) { if (tl.textContent.trim()) createDraft(tl, tl.dataset.parent, tl.dataset.after); else S.dirty = false; }
  else commitTitle(tl);
});

document.addEventListener('keydown', async (e) => {
  // Enter in any inline box (wait on…, first I need to…, add subtask, chase)
  // presses its button; nobody should have to reach for the mouse mid-flow.
  if (e.key === 'Enter' && e.target.matches && e.target.matches('.inline-form input')) {
    const btn = e.target.closest('.inline-form').querySelector('button');
    if (btn) { e.preventDefault(); btn.click(); return; }
  }
  const typing = e.target.closest('[contenteditable], input, textarea, select');
  if (e.key === 'Escape' && S.open && !typing) return go({ open: null });
  if (e.key === 'n' && !typing && !e.ctrlKey && !e.metaKey && !e.altKey) { e.preventDefault(); return quickAdd(); }
  // ▶ with a row focused: start (or stop) it without opening the pane.
  if ((e.ctrlKey || e.metaKey) && e.key === '.' ) {
    const li = e.target.closest && e.target.closest('li.row');
    if (li) { e.preventDefault(); const t = byId(li.dataset.id); if (t) return act(() => api('POST', `/api/v1/tasks/${t.id}/status`, { status: t.status === 'doing' ? 'todo' : 'doing' }), { id: t.id, atEnd: true }); }
  }
  const el = e.target.closest('.title, .pane-title, .plate-title'); if (!el) return;
  if (el.classList.contains('pane-title') || el.classList.contains('plate-title')) { if (e.key === 'Enter') { e.preventDefault(); el.blur(); } return; }
  const li = rowOf(el); const id = li && li.dataset.id; const isDraft = el.classList.contains('draft');
  if (S.view !== 'outline' && S.view !== 'someday' && !isDraft) { if (e.key === 'Enter') { e.preventDefault(); el.blur(); } return; }

  if (e.key === 'Enter') {
    e.preventDefault();
    if (isDraft) return createDraft(el, el.dataset.parent, el.dataset.after);
    await commitTitle(el);
    const draft = document.createElement('div');
    draft.className = 'row draftrow'; draft.style.setProperty('--depth', li.dataset.depth);
    draft.innerHTML = `<span></span><span class="title draft" contenteditable="true" spellcheck="false" data-placeholder="new task…"></span>`;
    const t = draft.querySelector('.title'); t.dataset.parent = li.dataset.parent; t.dataset.after = id;
    const kids = li.nextElementSibling && li.nextElementSibling.classList.contains('kids') ? li.nextElementSibling : null;
    if (kids) { t.dataset.parent = id; t.dataset.after = ''; kids.querySelector('ul').prepend(draft); draft.style.setProperty('--depth', +li.dataset.depth + 1); }
    else li.after(draft);
    t.focus(); return;
  }
  if (e.key === 'Tab') {
    e.preventDefault();
    if (isDraft) {
      if (el.closest('.newline')) return indentBottomDraft(el, e.shiftKey);
      const d = el.closest('.draftrow'); const prev = d ? prevSibling(d) : null;
      if (!e.shiftKey && prev) { el.dataset.parent = prev.dataset.id; el.dataset.after = ''; d.style.setProperty('--depth', +prev.dataset.depth + 1); }
      return;
    }
    await commitTitle(el);
    const offset = caretOffset(el);
    if (e.shiftKey) {
      const parentId = li.dataset.parent; if (!parentId) return;
      const parentLi = $(`#main li[data-id="${parentId}"]`);
      return act(() => api('POST', `/api/v1/tasks/${id}/move`, { parent: parentLi ? parentLi.dataset.parent || null : null, after: parentId }), { id, offset });
    }
    const prev = prevSibling(li); if (!prev) return;
    return act(() => api('POST', `/api/v1/tasks/${id}/move`, { parent: prev.dataset.id, after: null }), { id, offset });
  }
  if (e.key === 'Backspace' && el.textContent.length === 0) {
    e.preventDefault();
    const rows = titleEls(); const i = rows.indexOf(el); const prev = rows[i - 1];
    if (isDraft && el.closest('.newline')) { if (S.draft.depth) indentBottomDraft(el, true); return; }
    if (isDraft) { const d = el.closest('.draftrow'); if (d) d.remove(); S.dirty = false; if (prev) placeCaret(prev, true); return; }
    if (li.nextElementSibling && li.nextElementSibling.classList.contains('kids')) return flash('has subtasks; finish or move them first', '');
    S.dirty = false;
    return act(() => api('DELETE', `/api/v1/tasks/${id}`), prev ? { id: rowOf(prev) ? rowOf(prev).dataset.id : null, atEnd: true } : null);
  }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    const rows = titleEls(); const i = rows.indexOf(el); const next = rows[i + (e.key === 'ArrowDown' ? 1 : -1)];
    if (next) { e.preventDefault(); placeCaret(next, true); } return;
  }
  if (e.key === 'Escape') { el.blur(); return; }
  if ((e.metaKey || e.ctrlKey) && e.key === 'Enter' && id) { e.preventDefault(); return go({ open: id }); }
});

// ---------- "first I need to…": a draft line nested under a row ----------
function insertDraftUnder(id) {
  if (S.view !== 'outline' && S.view !== 'someday') { go({ view: 'outline', open: null }); }
  const li = $(`#main li[data-id="${id}"]`); if (!li) return;
  const draft = document.createElement('div');
  draft.className = 'row draftrow'; draft.style.setProperty('--depth', +li.dataset.depth + 1);
  draft.innerHTML = `<span></span><span class="title draft" contenteditable="true" spellcheck="false" data-placeholder="first I need to…"></span>`;
  const t = draft.querySelector('.title'); t.dataset.parent = id; t.dataset.after = '';
  const kids = li.nextElementSibling && li.nextElementSibling.classList.contains('kids') ? li.nextElementSibling : null;
  if (kids) kids.querySelector('ul').prepend(draft); else li.after(draft);
  t.focus();
}

// ---------- Stack: one chain, inverted, the leaf on top ----------
// In hand = the task you most recently started that is still open and not
// blocked. Same rule as core::action::in_hand: doing, actionable (unblocked
// and not under a waiting/parked ancestor), most recently started. Never
// pinned, so every client agrees on what is in hand.
// The plate is never empty while something is ready (core::action::plate,
// ADR 0014): with nothing in hand it shows what is *up next* — the first
// ready row in Next order, preferring rows never started, so putting a
// task back brings up another one rather than the same one again.
function pickStackTask() {
  const eligible = (t) => t && t.status === 'doing' && !!(t.derived && t.derived.actionable) && inCtx(t);
  const doing = S.all.filter(eligible).sort((a, b) => Date.parse(b.started || b.created) - Date.parse(a.started || a.created));
  let pick = doing[0] || null;
  if (!pick) {
    const ready = (S.plan || []).flatMap(p => p.now).map(i => byId(i.id)).filter(Boolean);
    pick = ready.find(t => !t.started) || ready[0] || null;
  }
  S.stackId = pick ? pick.id : null;
  return pick;
}
const inHandOf = (t) => t.status === 'doing';
function renderPlate(t) {
  const d = t.derived || {};
  const notes = (t.body || '').split('\n').filter(l => !isJournalLine(l)).join('\n').trim();
  const closed = isClosed(t);
  const acts = [];
  if (!closed) {
    acts.push(`<button class="btn primary" data-status="done" data-id="${t.id}" data-cascade="1">✓ done</button>`);
    if (inHandOf(t)) acts.push(`<button class="btn" data-status="todo" data-id="${t.id}" title="not now — the next ready task comes up">put back</button>`);
    else acts.push(`<button class="btn" data-status="doing" data-id="${t.id}" title="take it in hand">▶ start</button>`);
  }
  const chain = []; let p = t.parent && byId(t.parent); const seen = new Set();
  while (p && !seen.has(p.id)) { seen.add(p.id); chain.push(p); p = p.parent && byId(p.parent); }
  const unlocks = chain.length ? `<div class="unlocks">→ ${chain.map(c => `<a href="#" data-open="${c.id}">${esc(c.title)}</a>${isAgent(c) ? ' 🤖' : ''}`).join(' → ')}</div>` : '';
  return `
    <div class="plate top ${closed ? 'closed' : ''}">
      <div class="rel">${closed ? 'done' : t.status === 'waiting' ? '✉ waiting on ' + esc(t.waiting_on || '') : d.blocked_by ? '▽ blocked — finish what is under it' : inHandOf(t) ? 'in hand' : 'up next'}${pill(t.project_tag)}</div>
      <h2 class="plate-title" contenteditable="true" spellcheck="false" data-title="${t.id}">${esc(t.title)}</h2>
      ${unlocks}
      <div class="meta">${chips(t, d)}</div>
      <textarea class="notes plate-notes" data-notes="${t.id}" rows="${Math.min(12, Math.max(3, notes.split('\n').length + 1))}" placeholder="what exactly to do — the command, the link, the draft message">${esc(notes)}</textarea>
      <div class="actions">${acts.join('')}<button class="btn small" data-open="${t.id}">details</button></div>
      <div class="inline-form"><input class="first-title" placeholder="first I need to… (becomes in hand; this one pauses)"><button class="btn" data-first="prompt" data-id="${t.id}">push</button></div>
      <div class="inline-form"><input class="wait-who" placeholder="blocked on someone — who?"><input class="wait-until" placeholder="until (+2d)" style="max-width:110px"><button class="btn" data-wait="${t.id}">wait</button></div>
    </div>`;
}
const isAgent = (t) => (t.tags || []).some(x => x.toLowerCase() === 'agent');
document.addEventListener('click', (e) => {
  const a = e.target.closest('a[data-stack]'); if (!a) return;
  e.preventDefault(); go({ view: 'next', stackId: a.dataset.stack, open: null });
});
// After a mutation in the Stack view, shift up when the top plate closed.
function shiftStack(out) {
  if (S.view !== 'next' || !S.stackId) return;
  const t = byId(S.stackId); if (!t || !isClosed(t)) return;
  const parent = t.parent && byId(t.parent);
  if (!parent) { flash('✓ ' + t.title + ' — story done', 'ok'); return; }
  const left = childrenOf(parent.id).filter(k => !isClosed(k));
  if (left.length === 0) {
    // The parent is unblocked. It is not in hand until you start it — Next
    // shows it as a now row, resurfaced — unless it is Claude's.
    S.stackId = null;
    flash(isAgent(parent) ? '✓ ' + t.title + ' — ' + parent.title + ' is Claude\'s; it is back on their plate' : '✓ ' + t.title + ' — ' + parent.title + ' is unblocked', 'ok');
    history.replaceState(null, '', urlFor());
  } else {
    flash('✓ ' + t.title + ' — ' + parent.title + ' still needs: ' + left.map(k => k.title).join(', '), 'ok');
  }
}

// ---------- settings ----------
function renderSettings() {
  const st = S.settings;
  const shown = S.dirDraft ?? st.dir ?? st.running_dir;
  const differs = st.restart_needed
    ? `<p class="setnote warn">The saved folder (<code>${esc(st.dir)}</code>) is not the one running. The next start uses it, unless <code>--dir</code> or <code>$TASKSTAKA_DIR</code> overrides it.</p>` : '';
  return `<div class="settings">
    <section class="setcard">
      <h2>Notifications</h2>
      <p>Desktop popups for nudges: follow-ups due, a story ready, tasks stuck in doing, and the daily Waiting on digest. Turning them off does not hide anything: nudges still show in the Nudges view.</p>
      <button id="toggle-notify" class="toggle ${st.notifications ? 'on' : ''}">🔔 ${st.notifications ? 'on' : 'off'}</button>
    </section>
    <section class="setcard">
      <h2>Notes folder</h2>
      <p>Where your tasks are stored, one markdown file each. Running now: <code>${esc(st.running_dir)}</code></p>
      <div class="setrow"><input id="set-dir" type="text" spellcheck="false" value="${esc(shown)}" placeholder="/home/you/notes"><button id="save-dir" class="toggle on">Save and restart</button></div>
      <p class="setnote">Changing it restarts the daemon on the new folder. If the folder is empty or new, your notes are copied into it. The old folder is left exactly as it is, so nothing is deleted. A folder that already holds taskstaka notes is used as it is.</p>
      ${differs}
      <div id="setmsg" class="setnote"></div>
    </section>
  </div>`;
}
// Wait for the daemon to come back serving a different folder, then reload.
async function waitForNewDaemon(oldDir) {
  for (let i = 0; i < 60; i++) {
    await new Promise(r => setTimeout(r, 500));
    try { const r = await api('GET', '/api/v1/settings'); if (r.running_dir !== oldDir) return r; } catch (_) {}
  }
  throw new Error('the daemon did not come back within 30s; run `taskstaka status`');
}
document.addEventListener('input', (e) => { if (e.target.id === 'set-dir') { S.dirDraft = e.target.value; S.dirty = true; } });
document.addEventListener('focusout', (e) => {
  if (e.target.id !== 'set-dir') return;
  S.dirty = false;
  if (S.pendingRefresh) { S.pendingRefresh = false; load().catch(() => {}); }
});

// ---------- nudges ----------
function renderNudges() {
  if (!S.nudges.length) return '<p class="empty">Nothing to nudge you about.</p>';
  return `<ul class="rows">${S.nudges.map(n => {
    const t = n.task && byId(n.task);
    const link = t ? `<a href="#" data-open="${t.id}">${esc(t.title)}</a> ` : '';
    return `<li class="row compact"><span class="mark">⚑</span><span class="title"><strong>${esc(n.title)}</strong> — ${esc(n.message)}${link ? '<span class="crumb">' + link + '</span>' : ''}</span><span class="meta">${t ? hoverMeta(t) : ''}</span></li>`;
  }).join('')}</ul>`;
}

// ---------- drag to reorder / nest (outline) ----------
let dragId = null;
document.addEventListener('dragstart', (e) => {
  const grip = e.target.closest && e.target.closest('.grip'); if (!grip) return;
  const li = rowOf(grip); dragId = li.dataset.id; li.classList.add('dragging');
  e.dataTransfer.effectAllowed = 'move'; e.dataTransfer.setData('text/plain', dragId);
  try { e.dataTransfer.setDragImage(li, 10, 10); } catch (_) {}
});
const clearDrop = () => document.querySelectorAll('.drop-before, .drop-after, .drop-into').forEach(el => el.classList.remove('drop-before', 'drop-after', 'drop-into'));
function dropZone(li, e) {
  const r = li.getBoundingClientRect(); const y = (e.clientY - r.top) / r.height;
  return y < 0.3 ? 'before' : y > 0.7 ? 'after' : 'into';
}
document.addEventListener('dragover', (e) => {
  if (!dragId) return;
  const li = e.target.closest && e.target.closest('li.row'); if (!li || li.dataset.id === dragId) return;
  e.preventDefault(); e.dataTransfer.dropEffect = 'move';
  clearDrop(); li.classList.add('drop-' + dropZone(li, e));
});
document.addEventListener('dragleave', (e) => { if (e.target.closest && e.target.closest('li.row') && !e.relatedTarget) clearDrop(); });
document.addEventListener('drop', async (e) => {
  if (!dragId) return;
  const li = e.target.closest && e.target.closest('li.row'); if (!li || li.dataset.id === dragId) return;
  e.preventDefault();
  const zone = dropZone(li, e); const target = li.dataset.id; const id = dragId;
  clearDrop(); dragId = null;
  const body = zone === 'into' ? { parent: target, after: null }
    : zone === 'before' ? { parent: li.dataset.parent || null, before: target }
    : { parent: li.dataset.parent || null, after: target };
  await act(() => api('POST', `/api/v1/tasks/${id}/move`, body), { id, atEnd: true });
});
document.addEventListener('dragend', () => { dragId = null; clearDrop(); document.querySelectorAll('.dragging').forEach(el => el.classList.remove('dragging')); });

// ---------- live ----------
(async function watch() {
  for (;;) {
    try { const r = await api('GET', '/api/v1/changes?after=' + S.rev); if (r.rev !== S.rev) { S.rev = r.rev; await load(); } }
    catch (_) { await new Promise(r => setTimeout(r, 3000)); }
  }
})();

// ---------- quick add: `n` anywhere opens a one-line box; Enter adds a new root task ----------
function quickAdd() {
  if ($('#quick')) return $('#quick input').focus();
  const box = document.createElement('div');
  box.id = 'quick'; box.className = 'quick';
  box.innerHTML = `<div class="quick-card"><input autocomplete="off" spellcheck="false" placeholder="Add a task…  (@Name waits on them · #project)"><div class="quick-hint">Enter adds it as a new root task · Esc closes</div></div>`;
  document.body.appendChild(box);
  const input = box.querySelector('input'), hint = box.querySelector('.quick-hint');
  const close = () => box.remove();
  box.addEventListener('mousedown', (e) => { if (e.target === box) close(); });
  input.addEventListener('keydown', async (e) => {
    if (e.key === 'Escape') return close();
    if (e.key !== 'Enter' || !input.value.trim()) return;
    e.preventDefault();
    try {
      await api('POST', '/api/v1/quick', { text: input.value, context: S.ctx || null });
      close(); flash('added to the Outline', 'ok'); await load();
    } catch (err) { hint.textContent = err.message; hint.classList.add('err'); }
  });
  input.focus();
}

routeFromUrl();
load().catch(err => { flash('could not load: ' + err.message, 'error'); $('#main').innerHTML = '<p class="empty">Could not reach the daemon. Is it running? <code>taskstaka status</code></p>'; });
})();
