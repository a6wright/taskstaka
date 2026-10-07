//! MCP bridge: a stdio server that proxies to the daemon so agents (Claude
//! Code, Claude Desktop, …) can ask "what's next?" and add or close tasks.
//! Runs as its own process; the daemon stays the only writer.
//!
//! Logging goes to stderr because stdout is the protocol channel.

use std::path::{Path, PathBuf};

use anyhow::{Context as _, Result};
use rmcp::handler::server::wrapper::Parameters;
use rmcp::model::{
    Implementation, ListResourcesResult, PaginatedRequestParams, ReadResourceRequestParams,
    ReadResourceResponse, ReadResourceResult, Resource, ResourceContents, ServerCapabilities,
    ServerConfig,
};
use rmcp::service::RequestContext;
use rmcp::{ErrorData, RoleServer, ServerHandler, ServiceExt, tool, tool_handler, tool_router};
use serde::{Deserialize, Serialize};
use taskstaka_core::command::{AddTask, EditTask, Set};
use taskstaka_core::{Command, Context, Priority, Status, TaskId};

use crate::VERSION;
use crate::client::Client;
use crate::dates;

const INSTRUCTIONS: &str = "\
taskstaka is the user's task tree. Tasks have a stored status (todo, doing, waiting, someday, \
done, dropped) and a parent. What matters is the *computed* needs-action list from `next`: \
a task with open subtasks is blocked and not actionable; when its last subtask closes it \
becomes actionable again and is flagged 'resurfaced'. Prefer `next` to answer \"what should \
I work on?\". Add subtasks under the task they serve (block at the leaf). Mark a task \
`waiting` with a reason when it depends on someone else. Ids are 6 characters; unique \
prefixes are accepted.\n\n\
People: when the user says they asked someone or are waiting on someone, mark the task \
waiting on that person (`set_status` waiting, or `add_task` with waiting_on); a follow-up \
date is set automatically. When they say the person replied, call `record_reply` (answered, \
declined, partial, or redirected to someone else). When they say they pinged the person \
again, call `chase`. `waiting` lists everyone the user is waiting on; use due=true to find \
follow-ups that are due. If you can read the user's messages, check whether those people \
replied and record it — but never send messages on the user's behalf; draft them instead.\n\n\
Plans: when a request becomes several steps, create one task per step under a story, in \
order. Tag steps you will perform `agent` and close them as you go; leave the user's manual \
steps (console work, emails, approvals) untagged with exact instructions in the body; create \
steps that depend on someone else already waiting on that person.";

#[derive(Clone)]
pub struct TaskstakaMcp {
    client: Client,
    dir: PathBuf,
}

fn invalid(msg: impl std::fmt::Display) -> ErrorData {
    ErrorData::invalid_params(msg.to_string(), None)
}

fn failed(e: anyhow::Error) -> ErrorData {
    match e.downcast_ref::<crate::client::ApiError>() {
        Some(api) if api.status == 404 => ErrorData::invalid_params(api.message.clone(), None),
        Some(api) if (400..500).contains(&api.status) => {
            ErrorData::invalid_params(api.message.clone(), api.details.clone())
        }
        _ => ErrorData::internal_error(e.to_string(), None),
    }
}

fn pretty<T: Serialize>(v: &T) -> Result<String, ErrorData> {
    serde_json::to_string_pretty(v).map_err(|e| ErrorData::internal_error(e.to_string(), None))
}

fn parse_date(s: Option<String>) -> Result<Option<jiff::civil::Date>, ErrorData> {
    s.map(|d| dates::parse_date(&d).map_err(invalid))
        .transpose()
}

fn parse_context(s: Option<String>) -> Result<Option<Context>, ErrorData> {
    s.map(|c| c.parse::<Context>().map_err(invalid)).transpose()
}

fn parse_priority(s: Option<String>) -> Result<Option<Priority>, ErrorData> {
    s.map(|p| p.parse::<Priority>().map_err(invalid))
        .transpose()
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct NextArgs {
    /// Filter to one context: "work" or "personal". Omit for both.
    pub context: Option<String>,
    /// Maximum rows (default 10).
    pub limit: Option<usize>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct IdArgs {
    /// Task id or unique prefix.
    pub id: String,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct AddArgs {
    pub title: String,
    /// Parent task id; makes this a subtask.
    pub parent: Option<String>,
    /// "work" or "personal". Defaults to the parent's, else work.
    pub context: Option<String>,
    /// "high" or "low".
    pub priority: Option<String>,
    /// YYYY-MM-DD, today, tomorrow, or +Nd.
    pub due: Option<String>,
    pub tags: Option<Vec<String>>,
    /// Markdown notes for the task body.
    pub body: Option<String>,
    /// Create parked (someday) instead of todo.
    pub someday: Option<bool>,
    /// Create it already waiting on this person.
    pub waiting_on: Option<String>,
    /// Follow-up date for waiting_on (default +2d).
    pub until: Option<String>,
    /// Link to the conversation (thread URL, message id).
    pub waiting_ref: Option<String>,
    /// The project of a new ROOT story (e.g. the repo name). Ignored on
    /// subtasks, which inherit it. Call `projects` first and reuse a match.
    pub project: Option<String>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct SetStatusArgs {
    pub id: String,
    /// todo | doing | waiting | someday | done | dropped
    pub status: String,
    /// Required for waiting: what/who you are waiting on.
    pub waiting_on: Option<String>,
    /// For waiting: when to follow up (YYYY-MM-DD, today, tomorrow, +Nd).
    pub until: Option<String>,
    /// When closing: also close every open subtask with the same status.
    pub cascade: Option<bool>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct UpdateArgs {
    pub id: String,
    pub title: Option<String>,
    pub context: Option<String>,
    /// "high" or "low"; use clear_priority to remove.
    pub priority: Option<String>,
    pub clear_priority: Option<bool>,
    pub due: Option<String>,
    pub clear_due: Option<bool>,
    /// Replaces the tag list.
    pub tags: Option<Vec<String>>,
    /// Replaces the markdown body.
    pub body: Option<String>,
    /// Handoff: wait on this person instead (task must be waiting).
    pub waiting_on: Option<String>,
    /// Link to the conversation.
    pub waiting_ref: Option<String>,
    /// Set the project (root stories only; subtasks inherit it).
    pub project: Option<String>,
    pub clear_project: Option<bool>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct MoveArgs {
    pub id: String,
    /// New parent id, or omit to make it a root task.
    pub parent: Option<String>,
    /// Sibling to place it after; omit to append at the end.
    pub after: Option<String>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TouchArgs {
    pub id: String,
    /// Optional note appended to the body as a dated journal line.
    pub note: Option<String>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ListArgs {
    /// Filter by stored status.
    pub status: Option<String>,
    pub context: Option<String>,
    /// Include done/dropped tasks (default false).
    pub include_closed: Option<bool>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct TreeArgs {
    pub context: Option<String>,
    pub include_closed: Option<bool>,
}

impl TaskstakaMcp {
    async fn blocking<T: Send + 'static>(
        &self,
        f: impl FnOnce(Client) -> anyhow::Result<T> + Send + 'static,
    ) -> Result<T, ErrorData> {
        let c = self.client.clone();
        tokio::task::spawn_blocking(move || f(c))
            .await
            .map_err(|e| ErrorData::internal_error(e.to_string(), None))?
            .map_err(failed)
    }

    async fn resolve(&self, id: &str) -> Result<TaskId, ErrorData> {
        let id = id.to_string();
        self.blocking(move |c| c.resolve(&id)).await
    }

    async fn apply(&self, cmd: Command) -> Result<String, ErrorData> {
        let out = self.blocking(move |c| c.apply(&cmd)).await?;
        pretty(&out)
    }
}

#[tool_router]
impl TaskstakaMcp {
    #[tool(
        description = "What needs action next, ranked best-first with a reason per row, plus a summary. A task with open subtasks is not listed (it is blocked); 'resurfaced' rows are parents whose last subtask just closed."
    )]
    async fn next(&self, Parameters(a): Parameters<NextArgs>) -> Result<String, ErrorData> {
        let ctx = parse_context(a.context)?;
        let limit = a.limit.or(Some(10));
        let res = self.blocking(move |c| c.next(ctx, limit)).await?;
        pretty(&res)
    }

    #[tool(
        description = "Get one task with its derived facts (blocked_by, actionable, resurfaced_at), children, ancestry and markdown body."
    )]
    async fn get_task(&self, Parameters(a): Parameters<IdArgs>) -> Result<String, ErrorData> {
        let id = a.id;
        let v = self.blocking(move |c| c.task(&id)).await?;
        pretty(&v)
    }

    #[tool(description = "List tasks (open by default) as flat records with derived facts.")]
    async fn list_tasks(&self, Parameters(a): Parameters<ListArgs>) -> Result<String, ErrorData> {
        let status = a
            .status
            .map(|s| s.parse::<Status>().map_err(invalid))
            .transpose()?;
        let ctx = parse_context(a.context)?;
        let closed = a.include_closed.unwrap_or(false);
        let v = self
            .blocking(move |c| c.tasks_filtered(status, ctx, closed))
            .await?;
        pretty(&v)
    }

    #[tool(description = "The nested task tree (roots with their subtasks).")]
    async fn tree(&self, Parameters(a): Parameters<TreeArgs>) -> Result<String, ErrorData> {
        let ctx = parse_context(a.context)?;
        let closed = a.include_closed.unwrap_or(false);
        let v = self.blocking(move |c| c.tree(ctx, closed)).await?;
        pretty(&v)
    }

    #[tool(
        description = "Add a task, optionally as a subtask of `parent`. Returns the outcome with the new id."
    )]
    async fn add_task(&self, Parameters(a): Parameters<AddArgs>) -> Result<String, ErrorData> {
        let parent = match a.parent {
            Some(p) => Some(self.resolve(&p).await?),
            None => None,
        };
        let add = AddTask {
            title: a.title,
            parent,
            context: parse_context(a.context)?,
            priority: parse_priority(a.priority)?,
            due: parse_date(a.due)?,
            tags: a.tags.unwrap_or_default(),
            body: a.body.unwrap_or_default(),
            status: if a.someday.unwrap_or(false) {
                Some(Status::Someday)
            } else {
                None
            },
            after: None,
            waiting_on: a.waiting_on,
            until: parse_date(a.until)?,
            waiting_ref: a.waiting_ref,
            project: a.project,
        };
        self.apply(Command::Add(add)).await
    }

    #[tool(
        description = "Change a task's status: todo, doing, waiting (needs waiting_on), someday, done, dropped. Closing a task with open subtasks is refused unless cascade=true. The outcome's events include 'resurfaced' when a parent becomes actionable again."
    )]
    async fn set_status(
        &self,
        Parameters(a): Parameters<SetStatusArgs>,
    ) -> Result<String, ErrorData> {
        let id = self.resolve(&a.id).await?;
        let status = a.status.parse::<Status>().map_err(invalid)?;
        self.apply(Command::SetStatus {
            id,
            status,
            waiting_on: a.waiting_on,
            until: parse_date(a.until)?,
            cascade: a.cascade.unwrap_or(false),
        })
        .await
    }

    #[tool(
        description = "Edit a task's title, context, priority, due date, tags, body or project (project only on a root story)."
    )]
    async fn update_task(
        &self,
        Parameters(a): Parameters<UpdateArgs>,
    ) -> Result<String, ErrorData> {
        let id = self.resolve(&a.id).await?;
        let mut edit = EditTask::new(id);
        edit.title = a.title;
        edit.context = parse_context(a.context)?;
        edit.priority = if a.clear_priority.unwrap_or(false) {
            Set::Clear
        } else {
            parse_priority(a.priority)?
                .map(Set::To)
                .unwrap_or(Set::Keep)
        };
        edit.due = if a.clear_due.unwrap_or(false) {
            Set::Clear
        } else {
            parse_date(a.due)?.map(Set::To).unwrap_or(Set::Keep)
        };
        edit.tags = a.tags;
        edit.project = if a.clear_project.unwrap_or(false) {
            Set::Clear
        } else {
            a.project.map(Set::To).unwrap_or(Set::Keep)
        };
        edit.body = a.body;
        edit.waiting_on = a.waiting_on.map(Set::To).unwrap_or(Set::Keep);
        edit.waiting_ref = a.waiting_ref.map(Set::To).unwrap_or(Set::Keep);
        self.apply(Command::Edit(edit)).await
    }

    #[tool(description = "Reparent a task (omit parent to make it a root).")]
    async fn move_task(&self, Parameters(a): Parameters<MoveArgs>) -> Result<String, ErrorData> {
        let id = self.resolve(&a.id).await?;
        let parent = match a.parent {
            Some(p) => Some(self.resolve(&p).await?),
            None => None,
        };
        let after = match a.after {
            Some(p) => Some(self.resolve(&p).await?),
            None => None,
        };
        self.apply(Command::Move {
            id,
            parent,
            after,
            before: None,
        })
        .await
    }

    #[tool(
        description = "Mark a task as touched now, optionally appending a dated note to its body."
    )]
    async fn touch(&self, Parameters(a): Parameters<TouchArgs>) -> Result<String, ErrorData> {
        let id = self.resolve(&a.id).await?;
        self.apply(Command::Touch { id, note: a.note }).await
    }

    #[tool(
        description = "The projects in use, each with its colour and the root stories under it. Check this before starting work: reuse a project that matches (same repo or area), otherwise add_task the new root with a new `project`."
    )]
    async fn projects(&self) -> Result<String, ErrorData> {
        let views = self
            .blocking(|c| c.tasks_filtered(None, None, false))
            .await?;
        let mut out: Vec<serde_json::Value> = Vec::new();
        for v in views.iter().filter(|v| v.task.parent.is_none()) {
            let Some(tag) = &v.project_tag else { continue };
            let story = serde_json::json!({ "id": v.task.id, "title": v.task.title });
            match out.iter_mut().find(|p| p["name"] == tag.name.as_str()) {
                Some(p) => p["stories"].as_array_mut().unwrap().push(story),
                None => out.push(serde_json::json!({
                    "name": tag.name, "color": tag.color, "stories": [story]
                })),
            }
        }
        pretty(&out)
    }

    #[tool(
        description = "Current nudges: reminders the rules are raising right now (stuck doing, nothing actionable, stale, follow-ups, WIP)."
    )]
    async fn nudges(&self) -> Result<String, ErrorData> {
        let v = self.blocking(|c| c.nudges()).await?;
        pretty(&v)
    }

    #[tool(
        description = "Next, grouped by story: for each story the leaves the user can do now (ranked, with reasons) and everything still required to finish it with why it is not doable yet (after X, waiting on a person, Claude's step). The best single call to understand what is in flight."
    )]
    async fn plan(&self, Parameters(a): Parameters<WaitingArgs>) -> Result<String, ErrorData> {
        let ctx = parse_context(a.context)?;
        let v = self.blocking(move |c| c.plan(ctx)).await?;
        pretty(&v)
    }

    #[tool(
        description = "Who the user is waiting on, grouped by person: for each item the task, the story it serves, how long, chase count, last chase, follow-up date (`until`), whether the follow-up is due now, and `waiting_ref` (a link to the conversation, if known). Use due=true for only the follow-ups that are due."
    )]
    async fn waiting(&self, Parameters(a): Parameters<WaitingArgs>) -> Result<String, ErrorData> {
        let ctx = parse_context(a.context)?;
        let due = a.due.unwrap_or(false);
        let v = self.blocking(move |c| c.waiting(ctx, due)).await?;
        pretty(&v)
    }

    #[tool(
        description = "Record that the person was chased (pinged again) on a waiting task. Shortens the next follow-up. Pass dedupe (e.g. today's date) so repeated runs are no-ops, and source='agent' when acting on the user's behalf. Never sends a message itself."
    )]
    async fn chase(&self, Parameters(a): Parameters<ChaseArgs>) -> Result<String, ErrorData> {
        let id = self.resolve(&a.id).await?;
        self.apply(Command::Chase {
            id,
            until: parse_date(a.until)?,
            note: a.note,
            source: a.source,
            dedupe: a.dedupe,
        })
        .await
    }

    #[tool(
        description = "Record that the person got back to the user on a waiting task. outcome: answered (task becomes todo), declined (todo), partial (keep waiting, follow-up reset), redirected (keep waiting on `to`). summary is required and is journaled. Pass source='agent' when acting on the user's behalf."
    )]
    async fn record_reply(
        &self,
        Parameters(a): Parameters<ReplyArgs>,
    ) -> Result<String, ErrorData> {
        use taskstaka_core::command::ReplyOutcome as R;
        let id = self.resolve(&a.id).await?;
        let outcome = match a.outcome.to_ascii_lowercase().as_str() {
            "answered" => R::Answered,
            "declined" => R::Declined,
            "partial" => R::Partial,
            "redirected" => R::Redirected {
                to: a.to.ok_or_else(|| invalid("redirected needs `to`"))?,
            },
            other => return Err(invalid(format!("unknown outcome {other:?}"))),
        };
        self.apply(Command::RecordReply {
            id,
            outcome,
            summary: a.summary,
            waiting_ref: a.waiting_ref,
            source: a.source,
        })
        .await
    }
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct WaitingArgs {
    pub context: Option<String>,
    /// Only items whose follow-up date has arrived.
    pub due: Option<bool>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ChaseArgs {
    pub id: String,
    /// Next follow-up date (YYYY-MM-DD, today, tomorrow, +Nd). Default shortens automatically.
    pub until: Option<String>,
    pub note: Option<String>,
    /// e.g. "agent"
    pub source: Option<String>,
    /// Idempotency key, e.g. today's date.
    pub dedupe: Option<String>,
}

#[derive(Deserialize, rmcp::schemars::JsonSchema)]
#[schemars(crate = "rmcp::schemars")]
pub struct ReplyArgs {
    pub id: String,
    /// answered | declined | partial | redirected
    pub outcome: String,
    /// For redirected: who to wait on now.
    pub to: Option<String>,
    /// One line on what they said.
    pub summary: String,
    pub waiting_ref: Option<String>,
    pub source: Option<String>,
}

#[tool_handler]
impl ServerHandler for TaskstakaMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("taskstaka", VERSION))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new("taskstaka://index", "index")
                .with_description("The generated markdown tree of every open task (index.md)."),
            Resource::new("taskstaka://next", "next")
                .with_description("What needs action next, as text."),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let text = match request.uri.as_str() {
            "taskstaka://index" => std::fs::read_to_string(self.dir.join("index.md"))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            "taskstaka://next" => {
                let res = self.blocking(|c| c.next(None, Some(20))).await?;
                let mut s = format!(
                    "{} actionable · {} blocked · {} waiting · {} doing · {} someday\n\n",
                    res.summary.actionable,
                    res.summary.blocked,
                    res.summary.waiting,
                    res.summary.doing,
                    res.summary.someday
                );
                for it in res.items {
                    let crumb = it
                        .ancestry
                        .first()
                        .map(|c| format!(" ‹ {}", c.title))
                        .unwrap_or_default();
                    s.push_str(&format!(
                        "{}  [{}] {}{}  — {}\n",
                        it.id,
                        it.tier.label(),
                        it.title,
                        crumb,
                        it.reason
                    ));
                }
                s
            }
            other => {
                return Err(ErrorData::resource_not_found(
                    format!("unknown resource {other}"),
                    None,
                ));
            }
        };
        Ok(ReadResourceResult::new(vec![ResourceContents::text(text, request.uri)]).into())
    }
}

/// Entry point for `taskstaka mcp`: connect (auto-starting the daemon), then
/// speak MCP on stdin/stdout until the client hangs up.
pub fn run(dir: &Path, port_hint: Option<u16>) -> Result<()> {
    use tracing_subscriber::EnvFilter;
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .with_writer(std::io::stderr)
        .with_target(false)
        .try_init();

    let client = Client::connect(dir, port_hint)?;
    let server = TaskstakaMcp {
        client,
        dir: dir.to_path_buf(),
    };
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("starting the async runtime")?;
    rt.block_on(async move {
        let running = server
            .serve(rmcp::transport::stdio())
            .await
            .context("MCP handshake")?;
        running.waiting().await.context("MCP session")?;
        Ok(())
    })
}
