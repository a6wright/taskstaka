// tododo web app: one shell, sidebar · list · side pane. Everything talks to
// the JSON API every other client uses. No build step; vanilla JS.
(() => {
'use strict';

// ---------- state ----------
const S = {
  view: 'outline', ctx: '', open: null,     // route
  tree: [], next: null, waiting: [], nudges: [], all: [],
  rev: 0, dirty: false, pendingRefresh: false, focus: null,
};
const VIEWS = {
  outline: { ico: '☰', label: 'Outline' },
  next:    { ico: '▸', label: 'Next' },
  waiting: { ico: '⌛', label: 'Waiting on' },
  someday: { ico: '💤', label: 'Someday' },
  done:    { ico: '✓', label: 'Done' },
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
const today = () => new Date().toISOString().slice(0, 10);

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
  const [tree, next, waiting, nudges, all] = await Promise.all([
    api('GET', '/api/v1/tree' + q({ include_closed: 'true' })),
    api('GET', '/api/v1/next' + q({ limit: '0' })),
    api('GET', '/api/v1/waiting' + q()),
    api('GET', '/api/v1/nudges'),
    api('GET', '/api/v1/tasks' + q({ include_closed: 'true' })),
  ]);
  Object.assign(S, { tree, next, waiting, nudges, all, rev: next.rev });
  render();
}

// ---------- routing ----------
function routeFromUrl() {
  const path = location.pathname.replace(/\/+$/, '') || '/';
  const sp = new URLSearchParams(location.search);
  S.ctx = sp.get('context') || '';
  const m = path.match(/^\/t\/([0-9a-z]+)$/);
  if (m) { S.view = sp.get('view') || 'outline'; S.open = m[1]; return; }
  S.view = path === '/' ? 'outline' : path.slice(1);
  if (!VIEWS[S.view]) S.view = 'outline';
  S.open = sp.get('open');
}
function urlFor({ view = S.view, ctx = S.ctx, open = S.open } = {}) {
  const sp = new URLSearchParams();
  if (ctx) sp.set('context', ctx);
  if (open) sp.set('open', open);
  const qs = sp.toString();
  return (view === 'outline' ? '/' : '/' + view) + (qs ? '?' + qs : '');
}
function go(opts) {
  Object.assign(S, opts);
  history.pushState(null, '', urlFor());
  render();
}
window.addEventListener('popstate', () => { routeFromUrl(); render(); });

// ---------- flash ----------
function flash(msg, kind) {
  const el = $('#flash');
  el.textContent = msg; el.className = 'flash ' + (kind || ''); el.hidden = !msg;
  clearTimeout(flash.t); if (msg) flash.t = setTimeout(() => { el.hidden = true; }, 6000);
}
function announce(out) {
  const title = (id) => { const t = byId(id); return t ? t.title : id; };
  const back = out.events.find(e => e.kind === 'resurfaced');
  if (back) return flash('↩ ' + title(back.id) + ' is back: all its subtasks are done', 'ok');
  const rep = out.events.find(e => e.kind === 'reply_received');
  if (rep) return flash(rep.person + ' replied on ' + title(rep.id), 'ok');
  const ch = out.events.find(e => e.kind === 'chased');
  if (ch) return flash(ch.deduped ? 'already chased ' + ch.person + ' today' : 'chased ' + ch.person + ' (' + ch.count + '×)', 'ok');
  if (out.warnings && out.warnings.length) flash(out.warnings[0], '');
}
async function act(fn, keep) {
  try { const out = await fn(); if (out && out.events) announce(out); if (keep) S.focus = keep; await load(); return out; }
  catch (e) { flash(e.message, 'error'); await load(); return null; }
}

// ---------- lookups ----------
const byId = (id) => S.all.find(t => t.id === id);
const childrenOf = (id) => S.all.filter(t => t.parent === id).sort((a, b) => (a.order ?? Date.parse(a.created) / 1000) - (b.order ?? Date.parse(b.created) / 1000));
const isClosed = (t) => t.status === 'done' || t.status === 'dropped';
const recent = (iso) => iso && (Date.now() - Date.parse(iso)) < 86400e3;

// ---------- render ----------
function render() {
  const app = $('#app');
  app.classList.toggle('pane-open', !!S.open);
  renderSide(); renderMain(); renderPane();
  if (S.focus) { placeFocus(S.focus); S.focus = null; }
}

function renderSide() {
  const sum = S.next ? S.next.summary : {};
  const someday = S.all.filter(t => t.status === 'someday' && inCtx(t)).length;
  const done = S.all.filter(t => isClosed(t) && inCtx(t) && t.completed && (Date.now() - Date.parse(t.completed)) < 14 * 86400e3).length;
  const counts = { outline: '', next: sum.actionable ?? '', waiting: S.waiting.reduce((n, g) => n + g.items.length, 0) || '', someday: someday || '', done: done || '' };
  $('#side').innerHTML = `
    <div class="brand"><span><span class="dot"></span>tododo</span></div>
    <div class="ctxs">${['', 'work', 'personal'].map(c => `<a href="${urlFor({ ctx: c })}" data-ctx="${c}" class="${S.ctx === c ? 'active' : ''}">${c || 'all'}</a>`).join('')}</div>
    <nav class="nav">${Object.entries(VIEWS).map(([k, v]) => `<a href="${urlFor({ view: k })}" data-view="${k}" class="${S.view === k ? 'active' : ''}"><span class="ico">${v.ico}</span>${v.label}<span class="count">${counts[k]}</span></a>`).join('')}</nav>
    ${S.nudges.length ? `<div class="nav"><a href="#" class="nudges" title="${esc(S.nudges.map(n => n.message).join('\n'))}"><span class="ico">⚑</span>nudges<span class="count">${S.nudges.length}</span></a></div>` : ''}
    <div class="side-foot"><span class="kbd">Enter</span> new · <span class="kbd">Tab</span> subtask · <span class="kbd">⌫</span> delete empty · <span class="kbd">Esc</span> close</div>`;
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
  $('#main').innerHTML = `
    <div class="head"><button class="btn small menu" id="menu">☰</button><h1>${v.label}</h1><span class="sum">${line}</span><span class="sp"></span><span class="sum">${S.ctx || 'all'}</span></div>
    <div id="flash" class="flash" hidden></div>
    ${S.nudges.length && S.view !== 'waiting' ? `<div class="nudge">⚑ ${esc(S.nudges[0].message)}</div>` : ''}
    <div class="content">${body}</div>`;
}

// chips: the one thing that matters, quietly
function chips(t, d, extra = '') {
  const out = [];
  if (t.status === 'doing') out.push(['doing', '▶ doing ' + ago(t.started || t.created)]);
  if (t.status === 'waiting') out.push(['wait', '⌛ ' + esc(t.waiting_on || '') + ' · ' + ago(t.waiting_since || t.updated) + (t.chases && t.chases.length ? ' · ' + t.chases.length + '×' : '') + (t.until && t.until <= today() ? ' · follow up' : '')]);
  else if (d && d.waiting_on_summary) out.push(['wait', '⌛ ' + esc(d.waiting_on_summary.person) + ' · ' + ago(d.waiting_on_summary.since)]);
  else if (d && d.blocked_by > 0) out.push(['', '⏳ blocked by ' + d.blocked_by]);
  if (t.status === 'someday') out.push(['someday', '💤 someday']);
  if (d && d.resurfaced_at) out.push(['back', '↩ back']);
  if (d && d.orphaned) out.push(['orphan', '⚠ parent finished']);
  if (t.due && !isClosed(t)) out.push([t.due < today() ? 'overdue' : '', (t.due < today() ? '⚠ overdue ' : '📅 ') + t.due]);
  return out.map(([c, s]) => `<span class="chip ${c}">${s}</span>`).join('') + extra;
}
function hoverMeta(t) {
  const bits = [];
  if (t.priority) bits.push(t.priority);
  bits.push(ago(t.created));
  return `<span class="hover">${bits.map(b => `<span>${esc(b)}</span>`).join('')}<button class="openbtn" data-open="${t.id}">open</button></span>`;
}
function rowHtml(t, d, depth, { editable = true, crumb = '' } = {}) {
  const closed = isClosed(t);
  const cls = ['row', closed ? 'closed' : '', t.status === 'someday' ? 'someday' : '', d && d.blocked_by > 0 ? 'blocked' : '', d && d.deferred_by && d.deferred_by !== t.id ? 'deferred' : '', S.open === t.id ? 'selected' : ''].join(' ');
  return `<li class="${cls}" data-id="${t.id}" data-parent="${t.parent || ''}" data-depth="${depth}">
    <input type="checkbox" class="cb" ${closed ? 'checked' : ''} title="${closed ? 'reopen' : 'done'}">
    <span class="title" ${editable ? 'contenteditable="true" spellcheck="false"' : ''}>${esc(t.title)}</span>${crumb}
    <span class="meta">${chips(t, d)}${hoverMeta(t)}</span>
  </li>`;
}

function renderOutline() {
  const parts = [];
  const walk = (nodes, depth) => {
    parts.push('<ul class="rows">');
    for (const n of nodes) {
      const t = n.task;
      if (isClosed(t) && !recent(t.completed)) continue;
      if (depth === 0 && t.status === 'someday') continue;   // the icebox has its own view
      parts.push(rowHtml(t, t.derived, depth));
      if (n.children && n.children.length) { parts.push('<li class="kids">'); walk(n.children, depth + 1); parts.push('</li>'); }
    }
    parts.push('</ul>');
  };
  if (!S.tree.length) parts.push('<p class="empty">Nothing here yet. Type below.</p>');
  else walk(S.tree, 0);
  parts.push(`<div class="newline"><span class="plus">＋</span><span class="title draft" contenteditable="true" spellcheck="false" data-placeholder="add a task… (end with @Name to wait on someone)"></span></div>`);
  return parts.join('');
}

function renderNext() {
  const items = S.next ? S.next.items : [];
  if (!items.length) return '<p class="empty">Nothing actionable. Check <a href="/waiting">Waiting on</a> — go unblock something.</p>';
  const tiers = {};
  for (const it of items) (tiers[it.tier] ||= []).push(it);
  const label = { overdue: 'Overdue', resurfaced: 'Back on your plate', doing: 'Doing', follow_up: 'Follow up', due_soon: 'Due soon', high: 'High priority', normal: 'Up next', low: 'Low priority' };
  const parts = [];
  for (const [tier, list] of Object.entries(tiers)) {
    parts.push(`<div class="group"><span>${label[tier] || tier}</span><span class="n">${list.length}</span></div><ul class="rows">`);
    for (const it of list) {
      const t = byId(it.id); if (!t) continue;
      const crumb = it.ancestry.length ? `<span class="crumb">‹ ${esc(it.ancestry[0].title)}</span>` : '';
      parts.push(rowHtml(t, t.derived, 0, { editable: false, crumb }).replace('<span class="meta">', `<span class="meta"><span class="hover" style="display:none"></span><span class="chip tier" title="${esc(it.reason)}">${esc(it.reason)}</span>`));
    }
    parts.push('</ul>');
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
      return rowHtml(t, t.derived, 0, { editable: false, crumb }).replace('</span>\n  </li>', extra + '</span>\n  </li>');
    }).join('')}</ul></div>`).join('');
}

function renderSomeday() {
  const list = S.all.filter(t => t.status === 'someday' && inCtx(t));
  if (!list.length) return '<p class="empty">The icebox is empty. Park a task with <kbd>someday</kbd> in its pane.</p>';
  return `<ul class="rows">${list.map(t => {
    const p = t.parent && byId(t.parent);
    return rowHtml(t, t.derived, 0, { editable: false, crumb: p ? `<span class="crumb">‹ ${esc(p.title)}</span>` : '' })
      .replace('<span class="meta">', `<span class="meta"><button class="btn small" data-status="todo" data-id="${t.id}">revive</button>`);
  }).join('')}</ul>`;
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
  if (st === 'waiting') actions.push(['todo', 'unblock', '']);
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
        <label>parent</label><input data-prop="parent" data-id="${t.id}" value="${esc(t.parent || '')}" placeholder="id (blank = root)">
        <label>queued</label><span class="ro">${ago(t.created)} · touched ${ago(t.updated)}${t.started ? ' · doing ' + ago(t.started) : ''}</span>
        ${st === 'waiting' ? `
        <label>waiting on</label><input data-prop="waiting_on" data-id="${t.id}" value="${esc(t.waiting_on || '')}">
        <label>since</label><span class="ro">${ago(t.waiting_since || t.updated)} · chased ${(t.chases || []).length}× · follow up ${esc(t.until || '')}</span>
        <label>thread</label><input data-prop="waiting_ref" data-id="${t.id}" value="${esc(t.waiting_ref || '')}" placeholder="link to the conversation">` : ''}
      </div>
      ${st === 'waiting' ? `
      <div class="inline-form"><input id="chase-note" placeholder="chased them again — note (optional)"><button class="btn" data-chase="${t.id}">chase</button></div>
      <div class="inline-form"><select id="reply-outcome"><option value="answered">answered</option><option value="partial">partial</option><option value="declined">declined</option><option value="redirected">redirected to…</option></select><input id="reply-to" placeholder="person" style="max-width:110px"><input id="reply-summary" placeholder="what they said"><button class="btn" data-reply="${t.id}">reply</button></div>` : `
      <div class="inline-form"><input id="wait-who" placeholder="waiting on…"><input id="wait-until" placeholder="until (+3d)" style="max-width:110px"><button class="btn" data-wait="${t.id}">wait</button></div>`}
      <h3>subtasks</h3>
      <ul class="rows subs">${kids.map(k => rowHtml(k, k.derived, 0, { editable: false })).join('')}</ul>
      <div class="inline-form"><input id="sub-title" placeholder="add a subtask"><button class="btn" data-addsub="${t.id}">add</button></div>
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
  const el = f.id ? $(`#main li[data-id="${f.id}"] .title`) : $('#main .newline .title');
  if (el) placeCaret(el, f.atEnd, f.offset);
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
  if (m) { title = m[1]; waiting_on = m[2]; until = m[3] || null; }
  await act(() => api('POST', '/api/v1/tasks', { title, parent: parent || null, after: after || null, context: (!parent && S.ctx) ? S.ctx : null, waiting_on, until }), { id: null, atEnd: true });
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
  if (b.hasAttribute('data-close')) return go({ open: null });
  if (b.id === 'menu') return $('#app').classList.toggle('side-open');
  if (b.dataset.status) return act(() => api('POST', `/api/v1/tasks/${id}/status`, { status: b.dataset.status, cascade: true }));
  if (b.dataset.chase) { const note = $('#chase-note') ? $('#chase-note').value : ''; return act(() => api('POST', `/api/v1/tasks/${id}/chase`, { note: note || null })); }
  if (b.dataset.reply) {
    const outcome = $('#reply-outcome').value, to = $('#reply-to').value.trim(), summary = $('#reply-summary').value.trim();
    if (!summary) return flash('say what they said (one line)', 'error');
    const body = { outcome: outcome === 'redirected' ? { kind: 'redirected', to } : { kind: outcome }, summary };
    return act(() => api('POST', `/api/v1/tasks/${id}/reply`, body));
  }
  if (b.dataset.wait) {
    const who = $('#wait-who').value.trim(), until = $('#wait-until').value.trim();
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
  if (s === 'today') return d.toISOString().slice(0, 10);
  if (s === 'tomorrow') { d.setDate(d.getDate() + 1); return d.toISOString().slice(0, 10); }
  const m = s.match(/^\+(\d+)([dwm]?)$/);
  if (m) { const n = +m[1]; if (m[2] === 'w') d.setDate(d.getDate() + 7 * n); else if (m[2] === 'm') d.setMonth(d.getMonth() + n); else d.setDate(d.getDate() + n); return d.toISOString().slice(0, 10); }
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
    else if (prop === 'waiting_on') body = { waiting_on: v ? { to: v } : 'keep' };
    else if (prop === 'waiting_ref') body = { waiting_ref: v ? { to: v } : 'keep' };
    else if (prop === 'parent') return act(() => api('POST', `/api/v1/tasks/${id}/move`, { parent: v || null }));
    if (body && body.due && body.due.to === undefined) return;
    return act(() => api('PATCH', `/api/v1/tasks/${id}`, body));
  }
});

document.addEventListener('focusin', (e) => {
  const el = e.target.closest('.title, .pane-title'); if (!el) return;
  if (!el.classList.contains('draft')) el.dataset.original = el.textContent;
});
document.addEventListener('input', (e) => { if (e.target.closest('.title, .pane-title, .notes')) S.dirty = true; });
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
  const tl = el.closest && el.closest('.title, .pane-title'); if (!tl) return;
  if (tl.classList.contains('draft')) { if (tl.textContent.trim()) createDraft(tl, tl.dataset.parent, tl.dataset.after); else S.dirty = false; }
  else commitTitle(tl);
});

document.addEventListener('keydown', async (e) => {
  if (e.key === 'Escape' && S.open && !e.target.closest('[contenteditable], input, textarea, select')) return go({ open: null });
  const el = e.target.closest('.title, .pane-title'); if (!el) return;
  if (el.classList.contains('pane-title')) { if (e.key === 'Enter') { e.preventDefault(); el.blur(); } return; }
  const li = rowOf(el); const id = li && li.dataset.id; const isDraft = el.classList.contains('draft');
  if (S.view !== 'outline' && !isDraft) { if (e.key === 'Enter') { e.preventDefault(); el.blur(); } return; }

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

// ---------- live ----------
(async function watch() {
  for (;;) {
    try { const r = await api('GET', '/api/v1/changes?after=' + S.rev); if (r.rev > S.rev) { S.rev = r.rev; await load(); } }
    catch (_) { await new Promise(r => setTimeout(r, 3000)); }
  }
})();

routeFromUrl();
load();
})();
