//! Server-rendered web UI: the same panel and actions as the TUI, as plain
//! HTML forms. No JavaScript build, no template engine; pages are small
//! enough that a few functions over an escaping helper read fine.
//!
//! Form POSTs are the one place a browser can reach a mutation, so they are
//! gated on `Sec-Fetch-Site` / `Origin` being same-origin (see ADR 0003).

use std::fmt::Write as _;
use std::sync::Arc;

use axum::Form;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::{Html, IntoResponse, Redirect, Response};
use axum::routing::{get, post};
use jiff::Zoned;
use serde::Deserialize;
use tododo_core::action::{RankConfig, derive, needs_action, summary};
use tododo_core::command::{AddTask, EditTask, Set};
use tododo_core::{Command, Context, Priority, Status, Task, Tier, Tree, humanize};

use super::AppState;
use super::error::ApiError;
use super::routes::{apply, resolve_id};
use crate::dates;

type S = State<Arc<AppState>>;

pub fn router() -> Router<Arc<AppState>> {
    Router::new()
        .route("/", get(super::outline::outline))
        .route("/next", get(index))
        .route("/t/{id}", get(task_page))
        .route("/w/add", post(form_add))
        .route("/w/{id}/status", post(form_status))
        .route("/w/{id}/edit", post(form_edit))
        .route("/w/{id}/touch", post(form_touch))
        .route("/w/{id}/move", post(form_move))
        .route("/style.css", get(css))
}

// ---------- helpers ----------

pub fn esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(c),
        }
    }
    out
}

fn url_encode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            b' ' => out.push('+'),
            _ => {
                let _ = write!(out, "%{b:02X}");
            }
        }
    }
    out
}

/// Refuse form posts that did not originate from our own page. Returns the
/// 403 to send, or None when the request may proceed.
fn cross_site_refusal(headers: &HeaderMap) -> Option<Response> {
    if let Some(site) = headers.get("sec-fetch-site").and_then(|v| v.to_str().ok())
        && site != "same-origin"
        && site != "none"
    {
        return Some((StatusCode::FORBIDDEN, "cross-site form post refused").into_response());
    }
    if let Some(origin) = headers.get(header::ORIGIN).and_then(|v| v.to_str().ok()) {
        let host = headers
            .get(header::HOST)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        let ok = origin.strip_prefix("http://").is_some_and(|o| o == host);
        if !ok {
            return Some((StatusCode::FORBIDDEN, "cross-origin form post refused").into_response());
        }
    }
    None
}

fn back_to(back: Option<&str>, msg: &str) -> Redirect {
    let base = match back {
        Some(b) if b.starts_with('/') && !b.starts_with("//") => b.to_string(),
        _ => "/".to_string(),
    };
    let sep = if base.contains('?') { '&' } else { '?' };
    Redirect::to(&format!("{base}{sep}msg={}", url_encode(msg)))
}

pub fn page_shell(title: &str, body: &str) -> Html<String> {
    Html(format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>{}</title><link rel="stylesheet" href="/style.css"></head>
<body><main>{}</main></body></html>"#,
        esc(title),
        body
    ))
}

fn tier_class(t: Tier) -> &'static str {
    match t {
        Tier::Overdue => "overdue",
        Tier::Resurfaced => "back",
        Tier::Doing => "doing",
        Tier::FollowUp => "followup",
        Tier::DueSoon => "duesoon",
        Tier::High => "high",
        Tier::Normal => "normal",
        Tier::Low => "low",
    }
}

fn tier_badge(t: Tier) -> &'static str {
    match t {
        Tier::Overdue => "overdue",
        Tier::Resurfaced => "↩ back",
        Tier::Doing => "▶ doing",
        Tier::FollowUp => "follow-up",
        Tier::DueSoon => "due soon",
        Tier::High => "high",
        Tier::Normal => "",
        Tier::Low => "low",
    }
}

fn status_buttons(out: &mut String, t: &Task, back: &str) {
    let btn = |out: &mut String, status: &str, label: &str, class: &str| {
        let _ = write!(
            out,
            r#"<form method="post" action="/w/{id}/status" class="inline"><input type="hidden" name="status" value="{status}"><input type="hidden" name="back" value="{back}"><button class="{class}">{label}</button></form>"#,
            id = t.id,
            back = esc(back)
        );
    };
    match t.status {
        Status::Todo => {
            btn(out, "doing", "start", "");
            btn(out, "done", "done", "primary");
            btn(out, "someday", "someday", "quiet");
            btn(out, "dropped", "drop", "quiet");
        }
        Status::Doing => {
            btn(out, "done", "done", "primary");
            btn(out, "todo", "stop", "");
            btn(out, "dropped", "drop", "quiet");
        }
        Status::Waiting => {
            btn(out, "todo", "unblock", "primary");
            btn(out, "done", "done", "");
            btn(out, "dropped", "drop", "quiet");
        }
        Status::Someday => {
            btn(out, "todo", "revive", "primary");
            btn(out, "dropped", "drop", "quiet");
        }
        Status::Done | Status::Dropped => {
            btn(out, "todo", "reopen", "");
        }
    }
}

/// Title, the outline/next tabs, and the context pills (which keep the tab).
pub fn nav_html(tab: &str, ctx: Option<Context>) -> String {
    let ctx_label = ctx.map(|c| c.to_string()).unwrap_or_else(|| "all".into());
    let base = if tab == "next" { "/next" } else { "/" };
    let mut out = format!(
        r#"<header><h1><a href="/">tododo</a> <span class="ctx">· {ctx_label}</span></h1><nav>"#
    );
    let q = ctx.map(|c| format!("?context={c}")).unwrap_or_default();
    for (label, href) in [
        ("all", base.to_string()),
        ("work", format!("{base}?context=work")),
        ("personal", format!("{base}?context=personal")),
    ] {
        let active = if label == ctx_label { " active" } else { "" };
        let _ = write!(out, r#"<a class="pill{active}" href="{href}">{label}</a>"#);
    }
    out.push_str("</nav></header>");
    let _ = write!(
        out,
        r#"<nav class="tabs"><a href="/{q}" class="{}">outline</a><a href="/next{q}" class="{}">next</a></nav>"#,
        if tab == "outline" { "active" } else { "" },
        if tab == "next" { "active" } else { "" },
    );
    out
}

fn header(
    out: &mut String,
    tree: &Tree,
    now: &Zoned,
    ctx: Option<Context>,
    msg: Option<&str>,
    nudges: &[String],
) {
    let s = summary(tree, now, ctx);
    out.push_str(&nav_html("next", ctx));
    let line = if s.open == 0 {
        "nothing open".to_string()
    } else if s.actionable == 0 {
        format!(
            "<strong>nothing actionable</strong> · {} blocked · {} waiting — go unblock something",
            s.blocked, s.waiting
        )
    } else {
        format!(
            "{} actionable · {} doing · {} blocked · {} waiting · {} someday",
            s.actionable, s.doing, s.blocked, s.waiting, s.someday
        )
    };
    let _ = write!(out, r#"<p class="summary">{line}</p>"#);
    if let Some(m) = msg {
        let _ = write!(out, r#"<p class="flash">{}</p>"#, esc(m));
    }
    for n in nudges {
        let _ = write!(out, r#"<p class="nudge">⚑ {}</p>"#, esc(n));
    }
}

// ---------- pages ----------

#[derive(Deserialize)]
struct IndexQuery {
    context: Option<Context>,
    msg: Option<String>,
    #[serde(default)]
    stuck: bool,
}

async fn index(State(s): S, Query(q): Query<IndexQuery>) -> Result<Html<String>, ApiError> {
    let now = Zoned::now();
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let tree = store.tree();
    let nudges: Vec<String> = s
        .nudges
        .lock()
        .map(|n| n.iter().map(|x| x.message.clone()).collect())
        .unwrap_or_default();
    let back = match q.context {
        Some(c) => format!("/next?context={c}"),
        None => "/next".to_string(),
    };

    let mut out = String::new();
    header(&mut out, tree, &now, q.context, q.msg.as_deref(), &nudges);

    let _ = write!(
        out,
        r#"<form method="post" action="/w/add" class="add"><input name="title" placeholder="add a task…" required autofocus><select name="context"><option value="">context</option><option value="work"{}>work</option><option value="personal"{}>personal</option></select><input type="hidden" name="back" value="{}"><button class="primary">add</button></form>"#,
        if q.context == Some(Context::Work) {
            " selected"
        } else {
            ""
        },
        if q.context == Some(Context::Personal) {
            " selected"
        } else {
            ""
        },
        esc(&back)
    );

    let items = needs_action(tree, &now, q.context, &RankConfig::default());
    out.push_str(r#"<section><h2>needs action</h2>"#);
    if items.is_empty() {
        out.push_str(r#"<p class="empty">Nothing actionable.</p>"#);
    } else {
        out.push_str("<ol class=\"rows\">");
        for it in &items {
            let crumb = it
                .ancestry
                .first()
                .map(|c| {
                    format!(
                        r#" <span class="crumb">‹ <a href="/t/{}">{}</a></span>"#,
                        c.id,
                        esc(&c.title)
                    )
                })
                .unwrap_or_default();
            let due = it
                .due
                .filter(|_| matches!(it.tier, Tier::Overdue | Tier::DueSoon))
                .map(|d| format!(r#" <span class="due">due {d}</span>"#))
                .unwrap_or_default();
            let _ = write!(
                out,
                r#"<li class="row {cls}"><span class="badge">{badge}</span><span class="title"><a href="/t/{id}">{title}</a>{crumb}{due}<span class="reason">{reason}</span></span><span class="ages">{q} · {u}</span><span class="actions">"#,
                cls = tier_class(it.tier),
                badge = tier_badge(it.tier),
                id = it.id,
                title = esc(&it.title),
                reason = esc(&it.reason),
                q = humanize::duration(it.queued_secs),
                u = humanize::duration(it.untouched_secs),
            );
            if let Some(t) = tree.get(&it.id) {
                status_buttons(&mut out, t, &back);
            }
            out.push_str("</span></li>");
        }
        out.push_str("</ol>");
    }
    out.push_str("</section>");

    // Waiting & blocked.
    let today = now.date();
    let stuck: Vec<(&Task, String)> = tree
        .iter()
        .filter(|t| t.status.is_open() && t.status != Status::Someday)
        .filter(|t| {
            q.context
                .is_none_or(|c| tree.root_of(&t.id).map(|r| r.context).unwrap_or(t.context) == c)
        })
        .filter_map(|t| {
            let d = derive(tree, t, today);
            if d.actionable {
                return None;
            }
            let why = if d.blocked_by > 0 {
                format!("blocked by {}", d.blocked_by)
            } else if t.status == Status::Waiting {
                format!(
                    "waiting on {}{}",
                    t.waiting_on.as_deref().unwrap_or("?"),
                    t.until.map(|u| format!(" until {u}")).unwrap_or_default()
                )
            } else if let Some(by) = d.deferred_by {
                format!(
                    "under {}",
                    tree.get(&by).map(|b| b.title.as_str()).unwrap_or("?")
                )
            } else {
                t.status.to_string()
            };
            Some((t, why))
        })
        .collect();
    if !stuck.is_empty() {
        let toggle = if q.stuck {
            format!(r#"<a href="{back}">hide</a>"#)
        } else {
            let sep = if back.contains('?') { '&' } else { '?' };
            format!(r#"<a href="{back}{sep}stuck=true">show</a>"#)
        };
        let _ = write!(
            out,
            r#"<section class="stuck"><h2>waiting &amp; blocked ({}) {toggle}</h2>"#,
            stuck.len()
        );
        if q.stuck {
            out.push_str("<ul class=\"rows\">");
            for (t, why) in stuck {
                let _ = write!(
                    out,
                    r#"<li class="row quiet"><span class="badge">·</span><span class="title"><a href="/t/{}">{}</a><span class="reason">{}</span></span><span class="ages">{}</span></li>"#,
                    t.id,
                    esc(&t.title),
                    esc(&why),
                    humanize::since(t.created, now.timestamp())
                );
            }
            out.push_str("</ul>");
        }
        out.push_str("</section>");
    }
    Ok(page_shell("tododo", &out))
}

#[derive(Deserialize)]
struct MsgQuery {
    msg: Option<String>,
}

async fn task_page(
    State(s): S,
    Path(id): Path<String>,
    Query(q): Query<MsgQuery>,
) -> Result<Html<String>, ApiError> {
    let now = Zoned::now();
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let id = store.resolve(&id)?;
    let tree = store.tree();
    let t = tree
        .get(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", id.to_string()))?;
    let d = derive(tree, t, now.date());
    let back = format!("/t/{id}");
    let ts = now.timestamp();

    let mut out = String::new();
    header(&mut out, tree, &now, None, q.msg.as_deref(), &[]);

    let anc = tree.ancestors(&id);
    if !anc.is_empty() {
        out.push_str(r#"<p class="crumbs">"#);
        for a in anc.iter().rev() {
            let _ = write!(out, r#"<a href="/t/{}">{}</a> › "#, a.id, esc(&a.title));
        }
        out.push_str("</p>");
    }
    let _ = write!(
        out,
        r#"<h2 class="task-title"><code>{id}</code> {}</h2>"#,
        esc(&t.title)
    );

    let mut facts = vec![t.status.to_string(), t.context.to_string()];
    if d.blocked_by > 0 {
        facts.push(format!("blocked by {}", d.blocked_by));
    }
    if d.actionable {
        facts.push("actionable".into());
    }
    if let Some(at) = d.resurfaced_at {
        facts.push(format!("resurfaced {} ago", humanize::since(at, ts)));
    }
    if let Some(w) = &t.waiting_on {
        facts.push(format!(
            "waiting on {w}{}",
            t.until.map(|u| format!(" until {u}")).unwrap_or_default()
        ));
    }
    if let Some(p) = t.priority {
        facts.push(format!("priority {p}"));
    }
    if let Some(due) = t.due {
        facts.push(format!("due {due}"));
    }
    facts.push(format!("queued {}", humanize::since(t.created, ts)));
    facts.push(format!("touched {}", humanize::since(t.updated, ts)));
    if let Some(st) = t.started {
        facts.push(format!("doing for {}", humanize::since(st, ts)));
    }
    let _ = write!(out, r#"<p class="facts">{}</p>"#, esc(&facts.join(" · ")));

    out.push_str(r#"<div class="actions">"#);
    status_buttons(&mut out, t, &back);
    out.push_str("</div>");

    // Wait + touch forms.
    let _ = write!(
        out,
        r#"<div class="forms">
<form method="post" action="/w/{id}/status" class="inline"><input type="hidden" name="status" value="waiting"><input type="hidden" name="back" value="{back}"><input name="waiting_on" placeholder="waiting on…" required><input name="until" placeholder="until (+3d)" size="10"><button>wait</button></form>
<form method="post" action="/w/{id}/touch" class="inline"><input type="hidden" name="back" value="{back}"><input name="note" placeholder="note (optional)"><button>touch</button></form>
<form method="post" action="/w/add" class="inline"><input type="hidden" name="parent" value="{id}"><input type="hidden" name="back" value="{back}"><input name="title" placeholder="add subtask…" required><button>add subtask</button></form>
</div>"#
    );

    let kids: Vec<&Task> = tree.children(&id).collect();
    if !kids.is_empty() {
        out.push_str(r#"<section><h3>subtasks</h3><ul class="rows">"#);
        for k in kids {
            let mark = match k.status {
                Status::Done => "✓",
                Status::Dropped => "✗",
                Status::Doing => "▶",
                Status::Waiting => "⌛",
                Status::Someday => "💤",
                Status::Todo => "·",
            };
            let _ = write!(
                out,
                r#"<li class="row {closed}"><span class="badge">{mark}</span><span class="title"><a href="/t/{}">{}</a></span><span class="actions">"#,
                k.id,
                esc(&k.title),
                closed = if k.status.is_closed() { "closed" } else { "" }
            );
            status_buttons(&mut out, k, &back);
            out.push_str("</span></li>");
        }
        out.push_str("</ul></section>");
    }

    // Edit form.
    let sel = |v: bool| if v { " selected" } else { "" };
    let _ = write!(
        out,
        r#"<section><h3>edit</h3><form method="post" action="/w/{id}/edit" class="edit"><input type="hidden" name="back" value="{back}">
<label>title <input name="title" value="{title}" required></label>
<label>context <select name="context"><option value="work"{w}>work</option><option value="personal"{p}>personal</option></select></label>
<label>priority <select name="priority"><option value=""{pn}>normal</option><option value="high"{ph}>high</option><option value="low"{pl}>low</option></select></label>
<label>due <input name="due" value="{due}" placeholder="YYYY-MM-DD or +3d"></label>
<label>tags <input name="tags" value="{tags}" placeholder="comma, separated"></label>
<label>parent <input name="parent" value="{parent}" placeholder="id (blank = root)"></label>
<label class="wide">notes <textarea name="body" rows="8">{body}</textarea></label>
<button class="primary">save</button></form></section>"#,
        title = esc(&t.title),
        w = sel(t.context == Context::Work),
        p = sel(t.context == Context::Personal),
        pn = sel(t.priority.is_none()),
        ph = sel(t.priority == Some(Priority::High)),
        pl = sel(t.priority == Some(Priority::Low)),
        due = t.due.map(|d| d.to_string()).unwrap_or_default(),
        tags = esc(&t.tags.join(", ")),
        parent = t.parent.as_ref().map(|p| p.to_string()).unwrap_or_default(),
        body = esc(&t.body),
    );

    Ok(page_shell(&format!("{} · tododo", t.title), &out))
}

// ---------- forms ----------

#[derive(Deserialize)]
struct AddForm {
    title: String,
    parent: Option<String>,
    context: Option<String>,
    back: Option<String>,
}

async fn form_add(State(s): S, headers: HeaderMap, Form(f): Form<AddForm>) -> Response {
    if let Some(r) = cross_site_refusal(&headers) {
        return r;
    }
    let parent = match f.parent.as_deref().filter(|p| !p.trim().is_empty()) {
        Some(p) => match resolve_id(&s, p.trim()) {
            Ok(id) => Some(id),
            Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
        },
        None => None,
    };
    let context = f
        .context
        .as_deref()
        .filter(|c| !c.is_empty())
        .and_then(|c| c.parse().ok());
    let add = AddTask {
        title: f.title,
        parent,
        context,
        ..Default::default()
    };
    match apply(&s, Command::Add(add)).await {
        Ok(out) => {
            let id = out.primary.map(|i| i.to_string()).unwrap_or_default();
            back_to(f.back.as_deref(), &format!("added {id}")).into_response()
        }
        Err(e) => back_to(f.back.as_deref(), &e.message).into_response(),
    }
}

#[derive(Deserialize)]
struct StatusForm {
    status: Status,
    waiting_on: Option<String>,
    until: Option<String>,
    #[serde(default)]
    cascade: bool,
    back: Option<String>,
}

async fn form_status(
    State(s): S,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<StatusForm>,
) -> Response {
    if let Some(r) = cross_site_refusal(&headers) {
        return r;
    }
    let id = match resolve_id(&s, &id) {
        Ok(id) => id,
        Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
    };
    let until = match f.until.as_deref().map(str::trim).filter(|u| !u.is_empty()) {
        Some(u) => match dates::parse_date(u) {
            Ok(d) => Some(d),
            Err(e) => return back_to(f.back.as_deref(), &e).into_response(),
        },
        None => None,
    };
    let cmd = Command::SetStatus {
        id: id.clone(),
        status: f.status,
        waiting_on: f.waiting_on.filter(|w| !w.trim().is_empty()),
        until,
        cascade: f.cascade,
    };
    match apply(&s, cmd).await {
        Ok(out) => {
            let msg = if out
                .events
                .iter()
                .any(|e| matches!(e, tododo_core::Event::Resurfaced { .. }))
            {
                "↩ a parent task is back: all its subtasks are done".to_string()
            } else if let Some(w) = out.warnings.first() {
                w.clone()
            } else {
                format!("{id} → {}", f.status)
            };
            back_to(f.back.as_deref(), &msg).into_response()
        }
        Err(e) => back_to(f.back.as_deref(), &e.message).into_response(),
    }
}

#[derive(Deserialize)]
struct EditForm {
    title: String,
    context: Option<String>,
    priority: Option<String>,
    due: Option<String>,
    tags: Option<String>,
    parent: Option<String>,
    body: Option<String>,
    back: Option<String>,
}

async fn form_edit(
    State(s): S,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<EditForm>,
) -> Response {
    if let Some(r) = cross_site_refusal(&headers) {
        return r;
    }
    let id = match resolve_id(&s, &id) {
        Ok(id) => id,
        Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
    };
    let mut edit = EditTask::new(id.clone());
    edit.title = Some(f.title);
    edit.context = f
        .context
        .as_deref()
        .filter(|c| !c.is_empty())
        .and_then(|c| c.parse().ok());
    edit.priority = match f.priority.as_deref().map(str::trim) {
        Some("") | None => Set::Clear,
        Some(p) => match p.parse::<Priority>() {
            Ok(p) => Set::To(p),
            Err(e) => return back_to(f.back.as_deref(), &e).into_response(),
        },
    };
    edit.due = match f.due.as_deref().map(str::trim) {
        Some("") | None => Set::Clear,
        Some(d) => match dates::parse_date(d) {
            Ok(d) => Set::To(d),
            Err(e) => return back_to(f.back.as_deref(), &e).into_response(),
        },
    };
    edit.tags = f.tags.map(|t| {
        t.split(',')
            .map(str::trim)
            .filter(|x| !x.is_empty())
            .map(str::to_string)
            .collect()
    });
    edit.body = f.body.map(|b| b.replace("\r\n", "\n"));

    if let Err(e) = apply(&s, Command::Edit(edit)).await {
        return back_to(f.back.as_deref(), &e.message).into_response();
    }

    // Reparent if the parent field changed.
    let current_parent = s
        .store
        .lock()
        .ok()
        .and_then(|st| st.tree().get(&id).and_then(|t| t.parent.clone()));
    let wanted = f.parent.as_deref().map(str::trim).filter(|p| !p.is_empty());
    let wanted_id = match wanted {
        Some(p) => match resolve_id(&s, p) {
            Ok(pid) => Some(pid),
            Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
        },
        None => None,
    };
    if wanted_id != current_parent
        && let Err(e) = apply(
            &s,
            Command::Move {
                id,
                parent: wanted_id,
                after: None,
            },
        )
        .await
    {
        return back_to(f.back.as_deref(), &e.message).into_response();
    }
    back_to(f.back.as_deref(), "saved").into_response()
}

#[derive(Deserialize)]
struct TouchForm {
    note: Option<String>,
    back: Option<String>,
}

async fn form_touch(
    State(s): S,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<TouchForm>,
) -> Response {
    if let Some(r) = cross_site_refusal(&headers) {
        return r;
    }
    let id = match resolve_id(&s, &id) {
        Ok(id) => id,
        Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
    };
    let note = f.note.filter(|n| !n.trim().is_empty());
    match apply(&s, Command::Touch { id, note }).await {
        Ok(_) => back_to(f.back.as_deref(), "touched").into_response(),
        Err(e) => back_to(f.back.as_deref(), &e.message).into_response(),
    }
}

#[derive(Deserialize)]
struct MoveForm {
    parent: Option<String>,
    back: Option<String>,
}

async fn form_move(
    State(s): S,
    headers: HeaderMap,
    Path(id): Path<String>,
    Form(f): Form<MoveForm>,
) -> Response {
    if let Some(r) = cross_site_refusal(&headers) {
        return r;
    }
    let id = match resolve_id(&s, &id) {
        Ok(id) => id,
        Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
    };
    let parent = match f.parent.as_deref().map(str::trim).filter(|p| !p.is_empty()) {
        Some(p) => match resolve_id(&s, p) {
            Ok(pid) => Some(pid),
            Err(e) => return back_to(f.back.as_deref(), &e.message).into_response(),
        },
        None => None,
    };
    match apply(
        &s,
        Command::Move {
            id,
            parent,
            after: None,
        },
    )
    .await
    {
        Ok(_) => back_to(f.back.as_deref(), "moved").into_response(),
        Err(e) => back_to(f.back.as_deref(), &e.message).into_response(),
    }
}

async fn css() -> impl IntoResponse {
    ([(header::CONTENT_TYPE, "text/css; charset=utf-8")], CSS)
}

const CSS: &str = r#"
/* outline */
ul.outline{list-style:none;margin:0;padding:0}ul.outline ul.outline{margin-left:26px;border-left:1px solid var(--line);padding-left:6px}
li.kids{list-style:none}
.outline .row,.draftrow,.newline{display:grid;grid-template-columns:28px 1fr auto;gap:8px;align-items:baseline;padding:4px 0;border:0}
.outline .row.closed .title{text-decoration:line-through;color:var(--muted)}.outline .row.st-someday .title{color:var(--muted)}
.bullet{display:flex;gap:4px;align-items:center;justify-content:flex-end}.bullet input{margin:0}.bullet .dot{text-decoration:none;color:var(--muted);font-size:18px;line-height:1}.bullet .dot:hover{color:var(--accent)}
.title[contenteditable]{outline:none;min-height:1.4em;padding:2px 4px;border-radius:4px}.title[contenteditable]:focus{background:var(--pill)}
.title.draft:empty:before{content:attr(data-placeholder);color:var(--muted)}
.meta{display:flex;gap:6px;align-items:baseline;white-space:nowrap}.meta .b{font-size:12px;padding:1px 6px;border-radius:999px;background:var(--pill);color:var(--muted)}
.meta .b.doing{color:var(--yellow)}.meta .b.blocked{color:var(--muted)}.meta .b.back{color:var(--back);font-weight:600}.meta .b.overdue{color:var(--red)}.meta .b.orphan{color:var(--yellow)}.meta .b.waiting{color:var(--accent)}
.meta .age{font-size:12px;color:var(--muted)}.newline{margin-top:6px;color:var(--muted)}.newline .bullet{color:var(--muted)}
.draftrow{margin-left:calc(var(--depth,0) * 32px)}.hint{color:var(--muted);font-size:12px;margin-top:18px}.flash.error{color:var(--red)}.flash.ok{color:var(--green)}
nav.tabs{display:flex;gap:14px;margin:10px 0 4px}nav.tabs a{text-decoration:none;color:var(--muted);padding-bottom:2px}nav.tabs a.active{color:var(--fg);border-bottom:2px solid var(--accent)}
@media (max-width:640px){.meta .b{display:none}.meta .b.back,.meta .b.blocked,.meta .b.overdue{display:inline}}
:root{--bg:#fff;--fg:#1a1a1a;--muted:#6b6b6b;--line:#e5e5e5;--accent:#2b6cb0;--back:#8e44ad;--red:#c0392b;--yellow:#b7791f;--green:#2f855a;--pill:#f1f1f1}
@media (prefers-color-scheme:dark){:root{--bg:#121417;--fg:#e6e6e6;--muted:#9a9a9a;--line:#2a2e33;--accent:#63b3ed;--back:#c39bd3;--red:#f56565;--yellow:#ecc94b;--green:#68d391;--pill:#1f242a}}
*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--fg);font:15px/1.45 system-ui,-apple-system,Segoe UI,sans-serif}
main{max-width:960px;margin:0 auto;padding:16px}a{color:inherit}
header{display:flex;align-items:baseline;justify-content:space-between;gap:12px;flex-wrap:wrap}h1{font-size:20px;margin:0}h1 a{text-decoration:none}.ctx{color:var(--muted);font-weight:normal}
nav{display:flex;gap:6px}.pill{padding:2px 10px;border-radius:999px;background:var(--pill);text-decoration:none;font-size:13px}.pill.active{background:var(--accent);color:#fff}
.summary{color:var(--muted);margin:6px 0}.flash{background:var(--pill);padding:6px 10px;border-radius:6px}.nudge{color:var(--back);margin:4px 0}
form.add{display:flex;gap:6px;margin:12px 0}form.add input[name=title]{flex:1}
input,select,textarea,button{font:inherit;padding:6px 8px;border:1px solid var(--line);border-radius:6px;background:var(--bg);color:var(--fg)}
button{cursor:pointer;background:var(--pill)}button.primary{background:var(--accent);color:#fff;border-color:var(--accent)}button.quiet{color:var(--muted)}
h2{font-size:14px;text-transform:uppercase;letter-spacing:.05em;color:var(--muted);margin:20px 0 8px}h2 a{font-weight:normal;text-transform:none;letter-spacing:0;margin-left:8px}h3{font-size:14px;color:var(--muted);margin:18px 0 6px}
.rows{list-style:none;margin:0;padding:0}.row{display:grid;grid-template-columns:90px 1fr auto auto;gap:10px;align-items:center;padding:8px 0;border-top:1px solid var(--line)}
.row .badge{font-size:12px;font-weight:600;color:var(--muted)}.row.overdue .badge{color:var(--red)}.row.back .badge{color:var(--back)}.row.doing .badge{color:var(--yellow)}.row.high .badge{color:var(--green)}.row.followup .badge{color:var(--accent)}
.row .title a{text-decoration:none;font-weight:500}.row.closed .title a{text-decoration:line-through;color:var(--muted)}.crumb,.reason,.due{display:inline;color:var(--muted);font-size:13px;margin-left:6px}.reason{display:block;margin:0}.due{color:var(--red)}
.ages{color:var(--muted);font-size:13px;white-space:nowrap}.actions{display:flex;gap:4px;flex-wrap:wrap}.inline{display:inline-flex;gap:4px;margin:0}
.crumbs{color:var(--muted);margin:12px 0 0}.task-title{font-size:20px;text-transform:none;letter-spacing:0;color:var(--fg);margin:4px 0}.task-title code{color:var(--muted);font-size:14px}.facts{color:var(--muted)}
.forms{display:flex;flex-direction:column;gap:6px;margin:12px 0}.edit{display:grid;grid-template-columns:1fr 1fr;gap:8px}.edit label{display:flex;flex-direction:column;gap:2px;font-size:13px;color:var(--muted)}.edit .wide{grid-column:1/-1}.edit textarea{font-family:ui-monospace,monospace}
.empty{color:var(--muted)}.stuck h2 a{color:var(--accent)}
@media (max-width:640px){.row{grid-template-columns:70px 1fr}.ages{display:none}.actions{grid-column:1/-1}.edit{grid-template-columns:1fr}}
"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escaping_and_encoding() {
        assert_eq!(
            esc(r#"<a href="x">&'"#),
            "&lt;a href=&quot;x&quot;&gt;&amp;&#39;"
        );
        assert_eq!(
            url_encode("added k7x2mq ↩ ok/x"),
            "added+k7x2mq+%E2%86%A9+ok%2Fx"
        );
    }

    #[test]
    fn back_urls_are_local_only() {
        // An attacker-controlled `back` cannot redirect off-site.
        let r = back_to(Some("https://evil.example"), "m");
        let resp = r.into_response();
        assert_eq!(resp.headers()[header::LOCATION], "/?msg=m");
        let r = back_to(Some("//evil.example"), "m").into_response();
        assert_eq!(r.headers()[header::LOCATION], "/?msg=m");
        let r = back_to(Some("/t/abc?x=1"), "hi there").into_response();
        assert_eq!(r.headers()[header::LOCATION], "/t/abc?x=1&msg=hi+there");
    }
}
