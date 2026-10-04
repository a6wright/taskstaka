//! HTTP surface. Resource-style endpoints for web/MCP/curl, plus a generic
//! `POST /api/v1/commands` that the CLI uses. Every mutation funnels through
//! `apply`, which serializes on the store mutex and bumps the revision.

use std::sync::Arc;
use std::time::Duration;

use axum::Json;
use axum::Router;
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::routing::{get, post};
use jiff::Zoned;
use jiff::civil::Date;
use serde::{Deserialize, Serialize};
use taskstaka_core::action::{RankConfig, needs_action, summary};
use taskstaka_core::command::{AddTask, EditTask, Set};
use taskstaka_core::view::{NextResponse, TreeNode};
use taskstaka_core::{Command, Context, Outcome, Priority, Status, TaskId, TaskView};

use super::AppState;
use super::error::ApiError;
use crate::VERSION;

type S = State<Arc<AppState>>;
type R<T> = Result<Json<T>, ApiError>;

pub fn router(state: Arc<AppState>) -> Router {
    Router::new()
        .route("/health", get(health))
        .route("/api/v1/next", get(next))
        .route("/api/v1/plan", get(plan))
        .route("/api/v1/tasks", get(list_tasks).post(create_task))
        .route(
            "/api/v1/tasks/{id}",
            get(get_task).patch(patch_task).delete(delete_task),
        )
        .route("/api/v1/tasks/{id}/status", post(set_status))
        .route("/api/v1/tasks/{id}/move", post(move_task))
        .route("/api/v1/tasks/{id}/touch", post(touch_task))
        .route("/api/v1/tasks/{id}/chase", post(chase_task))
        .route("/api/v1/tasks/{id}/reply", post(reply_task))
        .route("/api/v1/waiting", get(waiting))
        .route("/api/v1/tree", get(tree))
        .route("/api/v1/resolve/{prefix}", get(resolve))
        .route("/api/v1/commands", post(command))
        .route("/api/v1/changes", get(changes))
        .route("/api/v1/nudges", get(nudges))
        .route("/api/v1/reindex", post(reindex))
        .route("/api/v1/reload", post(reload))
        .route("/api/v1/shutdown", post(shutdown))
        .merge(super::web::router())
        .with_state(state)
}

#[derive(Serialize)]
struct Health {
    version: &'static str,
    rev: u64,
    todo_dir: std::path::PathBuf,
    pid: u32,
    started: String,
}

async fn health(State(s): S) -> Json<Health> {
    Json(Health {
        version: VERSION,
        rev: s.current_rev(),
        todo_dir: s.dir.clone(),
        pid: std::process::id(),
        started: s.started.to_string(),
    })
}

/// Run a mutation on the blocking pool and bump the revision.
pub(super) async fn apply(s: &Arc<AppState>, cmd: Command) -> Result<Outcome, ApiError> {
    let st = s.clone();
    let out = tokio::task::spawn_blocking(move || {
        let mut store = st
            .store
            .lock()
            .map_err(|_| ApiError::internal("store mutex poisoned"))?;
        store.apply(cmd, &Zoned::now()).map_err(ApiError::from)
    })
    .await
    .map_err(ApiError::internal)??;
    s.bump();
    if let Some(nudger) = s.nudger() {
        let st = s.clone();
        let snapshot = out.clone();
        tokio::task::spawn_blocking(move || nudger.on_outcome(&st, &snapshot));
    }
    Ok(out)
}

pub(super) fn resolve_id(s: &AppState, raw: &str) -> Result<TaskId, ApiError> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    Ok(store.resolve(raw)?)
}

#[derive(Deserialize)]
struct NextQuery {
    context: Option<Context>,
    limit: Option<usize>,
}

async fn next(State(s): S, Query(q): Query<NextQuery>) -> R<NextResponse> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let now = Zoned::now();
    let mut items = needs_action(store.tree(), &now, q.context, &RankConfig::default());
    // `limit=0` means "no limit", so a client can always pass the parameter.
    if let Some(n) = q.limit.filter(|n| *n > 0) {
        items.truncate(n);
    }
    Ok(Json(NextResponse {
        rev: s.current_rev(),
        summary: summary(store.tree(), &now, q.context),
        items,
    }))
}

#[derive(Deserialize)]
struct ListQuery {
    status: Option<Status>,
    context: Option<Context>,
    parent: Option<String>,
    #[serde(default)]
    include_closed: bool,
}

async fn list_tasks(State(s): S, Query(q): Query<ListQuery>) -> R<Vec<TaskView>> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let now = Zoned::now();
    let parent = q.parent.as_deref().map(|p| store.resolve(p)).transpose()?;
    let tree = store.tree();
    let views = tree
        .iter()
        .filter(|t| q.status.is_none_or(|st| t.status == st))
        .filter(|t| q.context.is_none_or(|c| t.context == c))
        .filter(|t| parent.as_ref().is_none_or(|p| t.parent.as_ref() == Some(p)))
        .filter(|t| q.include_closed || q.status.is_some() || t.status.is_open())
        .map(|t| TaskView::build(tree, t, &now, store.path_of(&t.id).map(|p| p.to_path_buf())))
        .collect();
    Ok(Json(views))
}

async fn get_task(State(s): S, Path(id): Path<String>) -> R<TaskView> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let id = store.resolve(&id)?;
    let t = store
        .tree()
        .get(&id)
        .ok_or_else(|| ApiError::new(StatusCode::NOT_FOUND, "not_found", id.to_string()))?;
    Ok(Json(TaskView::build(
        store.tree(),
        t,
        &Zoned::now(),
        store.path_of(&id).map(|p| p.to_path_buf()),
    )))
}

#[derive(Serialize)]
struct Resolved {
    id: TaskId,
}

async fn resolve(State(s): S, Path(prefix): Path<String>) -> R<Resolved> {
    Ok(Json(Resolved {
        id: resolve_id(&s, &prefix)?,
    }))
}

#[derive(Deserialize)]
struct TreeQuery {
    root: Option<String>,
    #[serde(default)]
    include_closed: bool,
    context: Option<Context>,
}

async fn tree(State(s): S, Query(q): Query<TreeQuery>) -> R<Vec<TreeNode>> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let now = Zoned::now();
    let tree = store.tree();
    let roots: Vec<&taskstaka_core::Task> = match &q.root {
        Some(r) => {
            let id = store.resolve(r)?;
            tree.get(&id).into_iter().collect()
        }
        None => tree
            .roots()
            .into_iter()
            .filter(|t| q.context.is_none_or(|c| t.context == c))
            .filter(|t| q.include_closed || t.status.is_open())
            .collect(),
    };
    Ok(Json(
        roots
            .into_iter()
            .map(|r| TreeNode::build(tree, r, &now, q.include_closed))
            .collect(),
    ))
}

async fn create_task(State(s): S, Json(mut add): Json<AddTask>) -> R<Outcome> {
    // Allow a prefix in `parent`; resolve it server-side like everything else.
    if let Some(p) = &add.parent {
        add.parent = Some(resolve_id(&s, p.as_str())?);
    }
    Ok(Json(apply(&s, Command::Add(add)).await?))
}

/// `EditTask` without the id, which comes from the path.
#[derive(Deserialize)]
struct EditFields {
    title: Option<String>,
    context: Option<Context>,
    #[serde(default)]
    priority: Set<Priority>,
    #[serde(default)]
    due: Set<Date>,
    tags: Option<Vec<String>>,
    body: Option<String>,
    #[serde(default)]
    waiting_on: Set<String>,
    #[serde(default)]
    waiting_ref: Set<String>,
}

async fn patch_task(State(s): S, Path(id): Path<String>, Json(f): Json<EditFields>) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    let edit = EditTask {
        id,
        title: f.title,
        context: f.context,
        priority: f.priority,
        due: f.due,
        tags: f.tags,
        body: f.body,
        waiting_on: f.waiting_on,
        waiting_ref: f.waiting_ref,
    };
    Ok(Json(apply(&s, Command::Edit(edit)).await?))
}

#[derive(Deserialize)]
struct StatusBody {
    status: Status,
    waiting_on: Option<String>,
    until: Option<Date>,
    #[serde(default)]
    cascade: bool,
}

async fn set_status(State(s): S, Path(id): Path<String>, Json(b): Json<StatusBody>) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    Ok(Json(
        apply(
            &s,
            Command::SetStatus {
                id,
                status: b.status,
                waiting_on: b.waiting_on,
                until: b.until,
                cascade: b.cascade,
            },
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct MoveBody {
    parent: Option<String>,
    after: Option<String>,
    before: Option<String>,
}

async fn move_task(State(s): S, Path(id): Path<String>, Json(b): Json<MoveBody>) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    let parent = b.parent.as_deref().map(|p| resolve_id(&s, p)).transpose()?;
    let after = b.after.as_deref().map(|p| resolve_id(&s, p)).transpose()?;
    let before = b.before.as_deref().map(|p| resolve_id(&s, p)).transpose()?;
    Ok(Json(
        apply(
            &s,
            Command::Move {
                id,
                parent,
                after,
                before,
            },
        )
        .await?,
    ))
}

#[derive(Deserialize, Default)]
struct TouchBody {
    note: Option<String>,
}

async fn touch_task(
    State(s): S,
    Path(id): Path<String>,
    body: Option<Json<TouchBody>>,
) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    let note = body.and_then(|Json(b)| b.note);
    Ok(Json(apply(&s, Command::Touch { id, note }).await?))
}

#[derive(Deserialize)]
struct DeleteQuery {
    #[serde(default)]
    force: bool,
}

async fn delete_task(
    State(s): S,
    Path(id): Path<String>,
    Query(q): Query<DeleteQuery>,
) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    Ok(Json(
        apply(&s, Command::Delete { id, force: q.force }).await?,
    ))
}

async fn command(State(s): S, Json(cmd): Json<Command>) -> R<Outcome> {
    Ok(Json(apply(&s, cmd).await?))
}

#[derive(Deserialize)]
struct ChangesQuery {
    #[serde(default)]
    after: u64,
}

#[derive(Serialize)]
struct Changes {
    rev: u64,
}

/// Long-poll: return as soon as the revision passes `after`, or after ~25s.
async fn changes(State(s): S, Query(q): Query<ChangesQuery>) -> Json<Changes> {
    let mut rx = s.rev.subscribe();
    let mut shut = s.shutting_down.subscribe();
    let deadline = tokio::time::sleep(Duration::from_secs(25));
    tokio::pin!(deadline);
    loop {
        let cur = *rx.borrow_and_update();
        if cur > q.after {
            return Json(Changes { rev: cur });
        }
        tokio::select! {
            changed = rx.changed() => {
                if changed.is_err() {
                    return Json(Changes { rev: cur });
                }
            }
            _ = &mut deadline => return Json(Changes { rev: cur }),
            _ = shut.wait_for(|v| *v) => return Json(Changes { rev: cur }),
        }
    }
}

/// The current nudge set (cooldowns do not apply here; the panel shows all).
async fn nudges(State(s): S) -> R<Vec<taskstaka_core::nudge::Nudge>> {
    let list = s
        .nudges
        .lock()
        .map_err(|_| ApiError::internal("nudges mutex poisoned"))?
        .clone();
    Ok(Json(list))
}

async fn reindex(State(s): S) -> R<serde_json::Value> {
    let st = s.clone();
    tokio::task::spawn_blocking(move || {
        let store = st
            .store
            .lock()
            .map_err(|_| ApiError::internal("store mutex poisoned"))?;
        store.reindex(&Zoned::now()).map_err(ApiError::from)
    })
    .await
    .map_err(ApiError::internal)??;
    Ok(Json(serde_json::json!({ "ok": true })))
}

async fn reload(State(s): S) -> R<serde_json::Value> {
    let st = s.clone();
    let warnings = tokio::task::spawn_blocking(move || {
        let mut store = st
            .store
            .lock()
            .map_err(|_| ApiError::internal("store mutex poisoned"))?;
        store.reload().map_err(ApiError::from)?;
        store.reindex(&Zoned::now()).map_err(ApiError::from)?;
        Ok::<_, ApiError>(store.load_warnings.clone())
    })
    .await
    .map_err(ApiError::internal)??;
    s.bump();
    Ok(Json(
        serde_json::json!({ "ok": true, "warnings": warnings }),
    ))
}

async fn shutdown(State(s): S) -> impl IntoResponse {
    s.request_shutdown();
    Json(serde_json::json!({ "ok": true }))
}

// ---------- waiting on people ----------

#[derive(Deserialize)]
struct WaitingQuery {
    context: Option<Context>,
    /// Only items whose follow-up date has arrived.
    #[serde(default)]
    due: bool,
}

async fn waiting(
    State(s): S,
    Query(q): Query<WaitingQuery>,
) -> R<Vec<taskstaka_core::action::PersonGroup>> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    let mut groups = taskstaka_core::action::waiting_on(store.tree(), &Zoned::now(), q.context);
    if q.due {
        for g in &mut groups {
            g.items.retain(|i| i.due_now);
        }
        groups.retain(|g| !g.items.is_empty());
    }
    Ok(Json(groups))
}

#[derive(Deserialize, Default)]
struct ChaseBody {
    until: Option<Date>,
    note: Option<String>,
    source: Option<String>,
    dedupe: Option<String>,
}

async fn chase_task(
    State(s): S,
    Path(id): Path<String>,
    body: Option<Json<ChaseBody>>,
) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    let b = body.map(|Json(b)| b).unwrap_or_default();
    Ok(Json(
        apply(
            &s,
            Command::Chase {
                id,
                until: b.until,
                note: b.note,
                source: b.source,
                dedupe: b.dedupe,
            },
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct ReplyBody {
    outcome: taskstaka_core::command::ReplyOutcome,
    summary: String,
    waiting_ref: Option<String>,
    source: Option<String>,
}

async fn reply_task(State(s): S, Path(id): Path<String>, Json(b): Json<ReplyBody>) -> R<Outcome> {
    let id = resolve_id(&s, &id)?;
    Ok(Json(
        apply(
            &s,
            Command::RecordReply {
                id,
                outcome: b.outcome,
                summary: b.summary,
                waiting_ref: b.waiting_ref,
                source: b.source,
            },
        )
        .await?,
    ))
}

#[derive(Deserialize)]
struct PlanQuery {
    context: Option<Context>,
}

/// Next, grouped by story: what you can do now and what is still required.
async fn plan(
    State(s): S,
    Query(q): Query<PlanQuery>,
) -> R<Vec<taskstaka_core::action::StoryPlan>> {
    let store = s
        .store
        .lock()
        .map_err(|_| ApiError::internal("store mutex poisoned"))?;
    Ok(Json(taskstaka_core::action::plan(
        store.tree(),
        &Zoned::now(),
        q.context,
        &RankConfig::default(),
    )))
}
