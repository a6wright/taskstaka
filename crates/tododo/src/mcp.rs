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
use tododo_core::command::{AddTask, EditTask, Set};
use tododo_core::{Command, Context, Priority, Status, TaskId};

use crate::VERSION;
use crate::client::Client;
use crate::dates;

const INSTRUCTIONS: &str = "\
tododo is the user's task tree. Tasks have a stored status (todo, doing, waiting, someday, \
done, dropped) and a parent. What matters is the *computed* needs-action list from `next`: \
a task with open subtasks is blocked and not actionable; when its last subtask closes it \
becomes actionable again and is flagged 'resurfaced'. Prefer `next` to answer \"what should \
I work on?\". Add subtasks under the task they serve (block at the leaf). Mark a task \
`waiting` with a reason when it depends on someone else. Ids are 6 characters; unique \
prefixes are accepted.";

#[derive(Clone)]
pub struct TododoMcp {
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
    /// When closing: also drop every open subtask.
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

impl TododoMcp {
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
impl TododoMcp {
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

    #[tool(description = "Edit a task's title, context, priority, due date, tags or body.")]
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
        edit.body = a.body;
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
        self.apply(Command::Move { id, parent, after }).await
    }

    #[tool(
        description = "Mark a task as touched now, optionally appending a dated note to its body."
    )]
    async fn touch(&self, Parameters(a): Parameters<TouchArgs>) -> Result<String, ErrorData> {
        let id = self.resolve(&a.id).await?;
        self.apply(Command::Touch { id, note: a.note }).await
    }

    #[tool(
        description = "Current nudges: reminders the rules are raising right now (stuck doing, nothing actionable, stale, follow-ups, WIP)."
    )]
    async fn nudges(&self) -> Result<String, ErrorData> {
        let v = self.blocking(|c| c.nudges()).await?;
        pretty(&v)
    }
}

#[tool_handler]
impl ServerHandler for TododoMcp {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(
            ServerCapabilities::builder()
                .enable_tools()
                .enable_resources()
                .build(),
        )
        .with_server_info(Implementation::new("tododo", VERSION))
        .with_instructions(INSTRUCTIONS)
    }

    async fn list_resources(
        &self,
        _request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListResourcesResult, ErrorData> {
        Ok(ListResourcesResult::with_all_items(vec![
            Resource::new("tododo://index", "index")
                .with_description("The generated markdown tree of every open task (index.md)."),
            Resource::new("tododo://next", "next")
                .with_description("What needs action next, as text."),
        ]))
    }

    async fn read_resource(
        &self,
        request: ReadResourceRequestParams,
        _context: RequestContext<RoleServer>,
    ) -> Result<ReadResourceResponse, ErrorData> {
        let text = match request.uri.as_str() {
            "tododo://index" => std::fs::read_to_string(self.dir.join("index.md"))
                .map_err(|e| ErrorData::internal_error(e.to_string(), None))?,
            "tododo://next" => {
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

/// Entry point for `tododo mcp`: connect (auto-starting the daemon), then
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
    let server = TododoMcp {
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
