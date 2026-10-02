---
name: taskstaka
description: Keep the user's work tracked in taskstaka (their task tree, via the taskstaka MCP server) while you help them. Use this whenever a request turns into a plan with several steps, whenever some steps are for the user to do by hand (run something in a console, email or message someone, approve, wait for a reply) and some are for you, whenever they say "what should I work on", "what's next", "I'm waiting on", "add that to my list", "remind me", or mention a story/task/todo/follow-up — even if they never say "taskstaka". Also use it at the start of a session to see what is already in flight, and at the end to leave the next steps where they will see them.
allowed-tools: mcp__taskstaka__next, mcp__taskstaka__waiting, mcp__taskstaka__list_tasks, mcp__taskstaka__tree, mcp__taskstaka__get_task, mcp__taskstaka__add_task, mcp__taskstaka__set_status, mcp__taskstaka__update_task, mcp__taskstaka__move_task, mcp__taskstaka__touch, mcp__taskstaka__chase, mcp__taskstaka__record_reply, mcp__taskstaka__nudges
---

# taskstaka

taskstaka is the user's task tree. It exists because work is a tree — a story spawns
subtasks, the subtasks block the story — and every flat list loses the parent once you
descend into a child. The user looks at two computed lists: **Next** (what is theirs to
do right now) and **Waiting on** (who they are waiting on, for what, for how long). Your
job while using this skill is simple: *whatever the plan is, make it exist in that tree,
keep it true as the work moves, and leave the next human step where they will see it.*

The user has 45 unread threads and a lot of people wanting their time. The thing they
lose is not the work; it is the thread back to it. Every step you write down is one
fewer thing they must hold in their head.

## The model, in five lines

- A **task** has one stored status: `todo`, `doing`, `waiting` (on a person), `someday`, `done`, `dropped`.
- A task with open subtasks is **blocked** — not theirs to do; the subtasks are. When its last subtask closes it **resurfaces** at the top of Next. You never manage this; it is computed.
- `waiting` always has a follow-up date (default two days). `chase` records a nudge; `record_reply` records what came back; "unblock" (status `doing`) means it is ready and they are on it.
- Ids are six characters; any unique prefix works. Subtask order is the user's priority order.
- Block at the leaf: if a story is stuck on a person, the *subtask* that depends on them is `waiting`, not the story.

## Tools (taskstaka MCP server)

| Tool | Use it to |
|---|---|
| `next` | See what is theirs to do, ranked, with reasons. Start here. |
| `waiting` | See who they are waiting on. `due=true` for follow-ups that are due. |
| `list_tasks`, `tree`, `get_task` | Find existing work before creating duplicates; read notes and history. |
| `add_task` | Create a task or subtask (`parent`), optionally already `waiting_on` someone, with `tags`, `due`, `body`. |
| `set_status` | `doing` / `done` / `todo` / `someday` / `dropped` / `waiting` (needs `waiting_on`). `cascade=true` closes subtasks with a parent. |
| `update_task` | Title, notes (`body`), due, priority, tags, handoff (`waiting_on`), conversation link (`waiting_ref`). |
| `chase`, `record_reply` | The user pinged someone again / someone got back to them. |
| `move_task`, `touch` | Reorder or reparent; bump "last touched" with a note. |

## When a request becomes a plan

1. **Look first.** Call `next` and `list_tasks`. If the story already exists, add to it; do not create a twin.
2. **Create the story** as one task with a short imperative title ("Get DNS address for taskstaka").
3. **One subtask per step, in the order they will happen**, under the story. Say who does each:
   - Steps **you** will do in this session: tag `agent`. Mark `doing` when you start, `done` when finished. If you could not finish, leave it `todo` and say why in the notes.
   - Steps **the user** must do by hand (console clicks, commands on a machine you cannot reach, approvals, sending an email): no tag. Put *exactly what to do* in the `body` — the command, the URL, the draft message — so the task is self-contained when they come back to it tomorrow.
   - Steps that **depend on another person**: create them `waiting_on` that person, with a `waiting_ref` if there is a thread link. The follow-up date defaults sensibly; set `until` only when you know better.
4. **Keep it true as you go.** When you finish an agent step, close it. When you hand something to the user, tell them in one line and leave it `todo` — it is now in their Next. When they tell you "I asked B" / "B replied" / "I pinged B again", record exactly that (`set_status waiting`, `record_reply`, `chase`). Never guess at what a person said; ask, then record the summary.
5. **Close the loop.** At the end, say what is in taskstaka: the next human step, and who they are waiting on. That is the handoff.

Do not mark a user's manual step `done` unless they say they did it. Do not send
messages on their behalf — draft them into the task's notes. Do not create subtasks for
your own trivial sub-steps; a step is something that could be picked up tomorrow by
someone else.

## Example: the DNS story

User: "I need a DNS name for taskstaka on our GCP project; figure out what's possible and
get it set up." You find the docs say an internal zone needs a platform engineer to
approve, and the Cloud DNS change itself must be run by someone with the right role.

```
add_task  "Get DNS address for taskstaka"                           → story
add_task  "Confirm what's possible for an internal zone"   parent, tags=[agent]
add_task  "Ask Priya about wildcard certs on the zone"     parent, waiting_on="Priya"
add_task  "Create the Cloud DNS record"                    parent  (user; body: the exact gcloud command)
add_task  "Email platform-eng to approve the zone"         parent  (user; body: the draft email)
set_status <confirm…> doing  → …work…  → set_status done
```

You report: "Recorded under *Get DNS address*. Your next step is to run the gcloud
command (it's in the task). We're waiting on Priya about wildcard certs — I'll show up in
your follow-ups in two days if she hasn't answered." Later the user says "Priya says talk
to Bo" → `record_reply outcome=redirected to=Bo summary="Bo owns the zone"`. "I ran the
command" → `set_status done` on that step; the story resurfaces when the last step closes.

## Session habits

- **Start:** `next` once. Mention anything resurfaced or overdue in a sentence; don't lecture.
- **During:** record as you go, not in a batch at the end — the point is that the tree is true while they are away from the conversation.
- **End:** one line per open human step and one per person waited on. That is the whole summary; they will read it tomorrow.
