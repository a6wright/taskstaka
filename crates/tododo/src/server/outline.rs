//! The outliner: the tree as an editable nested checklist. The server sends a
//! shell; a small piece of vanilla JS fetches `/api/v1/tree`, renders rows,
//! and turns keystrokes into the same JSON API calls every client uses.
//!
//!   Enter       new sibling below (draft until you type)
//!   Tab         indent: becomes a subtask of the row above
//!   Shift+Tab   outdent: becomes a sibling of its parent, right after it
//!   ⌫ on empty  delete (leaves only)
//!   checkbox    done / reopen (ticking a parent completes its subtree)
//!   ↑ ↓         move between rows · Esc  stop editing

use std::sync::Arc;

use axum::extract::{Query, State};
use axum::response::Html;
use serde::Deserialize;
use tododo_core::Context;

use super::AppState;
use super::web::{esc, page_shell};

#[derive(Deserialize)]
pub struct OutlineQuery {
    pub context: Option<Context>,
    pub msg: Option<String>,
}

pub async fn outline(
    State(s): State<Arc<AppState>>,
    Query(q): Query<OutlineQuery>,
) -> Html<String> {
    let ctx = q.context.map(|c| c.to_string()).unwrap_or_default();
    let mut body = String::new();
    body.push_str(&super::web::nav_html("outline", q.context));
    if let Ok(store) = s.store.lock() {
        let back = match q.context {
            Some(c) => format!("/?context={c}"),
            None => "/".to_string(),
        };
        body.push_str(&super::web::waiting_strip(
            store.tree(),
            &jiff::Zoned::now(),
            q.context,
            &back,
        ));
    }
    if let Some(m) = &q.msg {
        body.push_str(&format!(r#"<p class="flash">{}</p>"#, esc(m)));
    }
    body.push_str(&format!(
        r#"<p class="summary" id="summary">…</p>
<p class="flash" id="flash" hidden></p>
<div id="outline" data-context="{ctx}" data-rev="{rev}"><p class="empty">loading…</p></div>
<p class="hint">Enter new line · Tab subtask · Shift+Tab outdent · ⌫ on empty line deletes · ↑↓ move · click a bullet for details</p>
<script>{js}</script>"#,
        rev = s.current_rev(),
        js = JS
    ));
    page_shell("tododo", &body)
}

const JS: &str = r#"
(() => {
const root = document.getElementById('outline');
const ctx = root.dataset.context || '';
let rev = Number(root.dataset.rev) || 0;
let roots = [];
let focus = null;          // {id, atEnd:boolean, offset:number}
let dirty = false;         // an edit is in progress; defer live refreshes
let pendingRefresh = false;

const api = async (method, path, body) => {
  const res = await fetch(path, {method, headers: body ? {'content-type': 'application/json'} : {}, body: body ? JSON.stringify(body) : undefined});
  const text = await res.text();
  let json = null; try { json = JSON.parse(text); } catch (_) {}
  if (!res.ok) throw new Error((json && json.error && json.error.message) || (res.status + ' ' + text));
  return json;
};

const flash = (msg, kind) => {
  const el = document.getElementById('flash');
  el.textContent = msg; el.className = 'flash ' + (kind || ''); el.hidden = !msg;
  if (msg) clearTimeout(flash.t), flash.t = setTimeout(() => { el.hidden = true; }, 6000);
};

const ago = (iso) => {
  const s = Math.max(0, (Date.now() - Date.parse(iso)) / 1000);
  if (s < 60) return '<1m'; if (s < 3600) return Math.floor(s/60) + 'm';
  if (s < 86400) return Math.floor(s/3600) + 'h'; if (s < 14*86400) return Math.floor(s/86400) + 'd';
  return Math.floor(s/(7*86400)) + 'w';
};
const recent = (iso) => iso && (Date.now() - Date.parse(iso)) < 86400e3;

async function refresh() {
  if (dirty) { pendingRefresh = true; return; }
  const q = ctx ? '&context=' + ctx : '';
  const [tree, next] = await Promise.all([
    api('GET', '/api/v1/tree?include_closed=true' + q),
    api('GET', '/api/v1/next?limit=0' + q),
  ]);
  roots = tree;
  rev = next.rev;
  const s = next.summary;
  document.getElementById('summary').innerHTML = s.open === 0 ? 'nothing open — type below to add a task'
    : s.actionable === 0 ? '<strong>nothing actionable</strong> · ' + s.blocked + ' blocked · ' + s.waiting + ' waiting — go unblock something'
    : s.actionable + ' actionable · ' + s.doing + ' doing · ' + s.blocked + ' blocked · ' + s.waiting + ' waiting · ' + s.someday + ' someday';
  render();
}

const visible = (node, isRoot) => {
  const t = node.task;
  if (t.status === 'done' || t.status === 'dropped') return recent(t.completed);
  return true;
};

function badges(node) {
  const t = node.task, d = node.task.derived;
  const out = [];
  if (t.status === 'doing') out.push(['doing', '▶ doing ' + ago(t.started || t.created)]);
  if (t.status === 'waiting') out.push(['waiting', '⌛ ' + (t.waiting_on || '') + ' · ' + ago(t.waiting_since || t.updated) + (t.chases && t.chases.length ? ' · ' + t.chases.length + '×' : '') + (t.until ? ' · follow up ' + t.until : '')]);
  if (d.waiting_on_summary) out.push(['waiting', '⌛ ' + d.waiting_on_summary.person + ' · ' + ago(d.waiting_on_summary.since)]);
  if (t.status === 'someday') out.push(['someday', '💤 someday']);
  if (d.blocked_by > 0 && !d.waiting_on_summary) out.push(['blocked', '⏳ blocked by ' + d.blocked_by]);
  if (d.resurfaced_at) out.push(['back', '↩ back']);
  if (d.orphaned) out.push(['orphan', '⚠ parent finished']);
  if (t.due) out.push([t.due < new Date().toISOString().slice(0,10) ? 'overdue' : 'due', '📅 ' + t.due]);
  if (t.priority) out.push(['prio', t.priority]);
  return out.map(([c, s]) => `<span class="b ${c}">${esc(s)}</span>`).join('');
}

const esc = (s) => String(s).replace(/[&<>"']/g, c => ({'&':'&amp;','<':'&lt;','>':'&gt;','"':'&quot;',"'":'&#39;'}[c]));

function rowHtml(node, depth) {
  const t = node.task;
  const closed = t.status === 'done' || t.status === 'dropped';
  const cls = ['row', 'st-' + t.status, closed ? 'closed' : '', t.derived.actionable ? 'actionable' : ''].join(' ');
  return `<li class="${cls}" data-id="${t.id}" data-parent="${t.parent || ''}" data-depth="${depth}">
    <span class="bullet"><input type="checkbox" ${closed ? 'checked' : ''} title="${closed ? 'reopen' : 'done'}"><a class="dot" href="/t/${t.id}" title="details">•</a></span>
    <span class="title" contenteditable="true" spellcheck="false">${esc(t.title)}</span>
    <span class="meta">${badges(node)}<span class="age" title="queued · touched">${ago(t.created)}</span></span>
  </li>`;
}

function render() {
  const parts = [];
  const walk = (nodes, depth) => {
    parts.push('<ul class="outline">');
    for (const n of nodes) {
      if (!visible(n, depth === 0)) continue;
      parts.push(rowHtml(n, depth));
      if (n.children && n.children.length) { parts.push('<li class="kids">'); walk(n.children, depth + 1); parts.push('</li>'); }
    }
    parts.push('</ul>');
  };
  if (!roots.length) parts.push('<p class="empty">Nothing here yet.</p>');
  else walk(roots, 0);
  parts.push(`<div class="newline"><span class="bullet">＋</span><span class="title draft" contenteditable="true" spellcheck="false" data-placeholder="add a task…"></span></div>`);
  root.innerHTML = parts.join('');
  if (focus) {
    const el = focus.id ? root.querySelector(`li[data-id="${focus.id}"] .title`) : root.querySelector('.newline .title');
    if (el) placeCaret(el, focus.atEnd, focus.offset);
    focus = null;
  }
}

function placeCaret(el, atEnd, offset) {
  el.focus();
  const range = document.createRange();
  const node = el.firstChild;
  if (!node) { range.setStart(el, 0); }
  else {
    const len = node.textContent.length;
    range.setStart(node, atEnd ? len : Math.min(offset || 0, len));
  }
  range.collapse(true);
  const sel = window.getSelection(); sel.removeAllRanges(); sel.addRange(range);
}

const rowOf = (el) => el.closest('li.row');
const idOf = (li) => li && li.dataset.id;
const allRows = () => [...root.querySelectorAll('li.row')];
const titleEls = () => [...root.querySelectorAll('.title')];
const caretOffset = (el) => { const s = window.getSelection(); if (!s.rangeCount) return 0; const r = s.getRangeAt(0).cloneRange(); r.selectNodeContents(el); r.setEnd(s.getRangeAt(0).endContainer, s.getRangeAt(0).endOffset); return r.toString().length; };

function prevSibling(li) {
  // previous li.row at the same depth with the same parent
  let p = li.previousElementSibling;
  while (p) { if (p.classList.contains('row')) return p; p = p.previousElementSibling; }
  return null;
}

async function act(fn) {
  try { const out = await fn(); if (out && out.events) announce(out); await refresh(); }
  catch (e) { flash(e.message, 'error'); await refresh(); }
}

function announce(out) {
  const back = out.events.find(e => e.kind === 'resurfaced');
  if (back) { const li = root.querySelector(`li[data-id="${back.id}"] .title`); flash('↩ ' + (li ? li.textContent : back.id) + ' is back: all its subtasks are done', 'ok'); return; }
  if (out.warnings && out.warnings.length) flash(out.warnings[0], '');
}

async function commitTitle(el) {
  const li = rowOf(el);
  if (!li) return;
  const id = idOf(li), text = el.textContent.trim();
  const original = el.dataset.original;
  dirty = false;
  if (text && original !== undefined && text !== original) {
    el.dataset.original = text;
    await act(() => api('PATCH', '/api/v1/tasks/' + id, {title: text}));
  }
  if (pendingRefresh) { pendingRefresh = false; refresh(); }
}

async function createDraft(el, parent, after) {
  const text = el.textContent.trim();
  if (!text) return;
  el.textContent = '';
  dirty = false;
  await act(async () => {
    // "ask about TTL @B" → waiting on B; "@B +3d" also sets the follow-up date.
    let title = text, waiting_on = null, until = null;
    const m = text.match(/^(.*\S)\s+@(\S+)(?:\s+(\+\d+[dwm]|\d{4}-\d{2}-\d{2}|today|tomorrow))?\s*$/);
    if (m) { title = m[1]; waiting_on = m[2]; until = m[3] || null; }
    const out = await api('POST', '/api/v1/tasks', {title, parent: parent || null, after: after || null, context: (!parent && ctx) ? ctx : null, waiting_on, until});
    focus = {id: null, atEnd: true};       // stay on the new-line box for rapid capture
    lastCreated = out.primary;
    return out;
  });
}
let lastCreated = null;

root.addEventListener('focusin', (e) => {
  const el = e.target.closest('.title'); if (!el) return;
  if (!el.classList.contains('draft')) el.dataset.original = el.textContent;
});
root.addEventListener('input', (e) => { if (e.target.closest('.title')) dirty = true; });
root.addEventListener('focusout', (e) => {
  const el = e.target.closest('.title'); if (!el) return;
  if (el.classList.contains('draft')) { if (el.textContent.trim()) createDraft(el, el.dataset.parent, el.dataset.after); else dirty = false; }
  else commitTitle(el);
});

root.addEventListener('change', (e) => {
  const cb = e.target; if (cb.type !== 'checkbox') return;
  const li = rowOf(cb); const id = idOf(li);
  focus = {id, atEnd: true};
  act(() => api('POST', `/api/v1/tasks/${id}/status`, {status: cb.checked ? 'done' : 'todo', cascade: true}));
});

root.addEventListener('keydown', async (e) => {
  const el = e.target.closest('.title'); if (!el) return;
  const li = rowOf(el); const id = idOf(li);
  const isDraft = el.classList.contains('draft');

  if (e.key === 'Enter') {
    e.preventDefault();
    if (isDraft) { await createDraft(el, el.dataset.parent, el.dataset.after); return; }
    await commitTitle(el);
    // Insert a draft line right after this row, at the same depth.
    const draft = document.createElement('div');
    draft.className = 'row draftrow'; draft.style.setProperty('--depth', li.dataset.depth);
    draft.innerHTML = `<span class="bullet">＋</span><span class="title draft" contenteditable="true" spellcheck="false" data-placeholder="new task…"></span>`;
    const t = draft.querySelector('.title'); t.dataset.parent = li.dataset.parent; t.dataset.after = id;
    // if the row has visible children, the new line goes first under it instead
    const kids = li.nextElementSibling && li.nextElementSibling.classList.contains('kids') ? li.nextElementSibling : null;
    if (kids) { t.dataset.parent = id; t.dataset.after = ''; kids.querySelector('ul').prepend(draft); draft.style.setProperty('--depth', Number(li.dataset.depth) + 1); }
    else li.after(draft);
    t.focus();
    return;
  }
  if (e.key === 'Tab') {
    e.preventDefault();
    if (isDraft) {
      // reparent the draft locally
      const prev = el.closest('.draftrow') ? prevSibling(el.closest('.draftrow')) : null;
      if (!e.shiftKey && prev) { el.dataset.parent = idOf(prev); el.dataset.after = ''; el.closest('.draftrow').style.setProperty('--depth', Number(prev.dataset.depth) + 1); }
      return;
    }
    await commitTitle(el);
    const offset = caretOffset(el);
    if (e.shiftKey) {
      const parentId = li.dataset.parent; if (!parentId) return;
      const parentLi = root.querySelector(`li[data-id="${parentId}"]`);
      focus = {id, offset};
      await act(() => api('POST', `/api/v1/tasks/${id}/move`, {parent: parentLi.dataset.parent || null, after: parentId}));
    } else {
      const prev = prevSibling(li); if (!prev) return;
      focus = {id, offset};
      await act(() => api('POST', `/api/v1/tasks/${id}/move`, {parent: idOf(prev), after: null}));
    }
    return;
  }
  if (e.key === 'Backspace' && el.textContent.length === 0) {
    e.preventDefault();
    const rows = titleEls(); const i = rows.indexOf(el); const prev = rows[i - 1];
    if (isDraft) { const d = el.closest('.draftrow'); if (d) d.remove(); dirty = false; if (prev) placeCaret(prev, true); return; }
    const hasKids = li.nextElementSibling && li.nextElementSibling.classList.contains('kids');
    if (hasKids) { flash('has subtasks; finish or move them first', ''); return; }
    focus = prev ? {id: idOf(rowOf(prev)), atEnd: true} : null;
    dirty = false;
    await act(() => api('DELETE', `/api/v1/tasks/${id}`));
    return;
  }
  if (e.key === 'ArrowDown' || e.key === 'ArrowUp') {
    const rows = titleEls(); const i = rows.indexOf(el);
    const next = rows[i + (e.key === 'ArrowDown' ? 1 : -1)];
    if (next) { e.preventDefault(); placeCaret(next, true); }
    return;
  }
  if (e.key === 'Escape') { el.blur(); return; }
});

// Live updates from other clients.
(async function watch() {
  for (;;) {
    try {
      const r = await api('GET', '/api/v1/changes?after=' + rev);
      if (r.rev > rev) { rev = r.rev; await refresh(); }
    } catch (_) { await new Promise(r => setTimeout(r, 3000)); }
  }
})();

refresh();
})();
"#;
