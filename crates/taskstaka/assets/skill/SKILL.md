---
name: taskstaka
description: Load this FIRST — before investigating, planning or asking clarifying questions — whenever a request has more than one step, whenever some steps are the user's to do by hand (run something in a console or on their machine, email or message someone, approve, wait for a reply) and some are yours, and whenever they say "what should I work on", "what's next", "I'm waiting on", "add that to my list", "remind me", or mention a story, task, todo or follow-up. The user keeps all their work in taskstaka (a task tree reached through the taskstaka MCP server); a plan that only exists in the chat is lost to them tomorrow. Use it at the start of a session to see what is in flight and at the end to leave the next steps where they will see them, even when nobody says "taskstaka".
allowed-tools: mcp__taskstaka__next, mcp__taskstaka__waiting, mcp__taskstaka__list_tasks, mcp__taskstaka__tree, mcp__taskstaka__get_task, mcp__taskstaka__add_task, mcp__taskstaka__set_status, mcp__taskstaka__update_task, mcp__taskstaka__move_task, mcp__taskstaka__touch, mcp__taskstaka__chase, mcp__taskstaka__record_reply, mcp__taskstaka__nudges
---

# taskstaka

taskstaka is the user's task tree. It exists because work is a tree — a story spawns
subtasks, the subtasks block the story — and every flat list loses the parent once you
descend into a child. The user looks at two computed lists: **Next** (what is theirs to
do right now) and **Waiting on** (who they are waiting on, for what, for how long). Your
job while using this skill is simple: *whatever the plan is, make it exist in that tree
before you do anything else, keep it true as the work moves, and leave the next human
step where they will see it.*

The user has 45 unread threads and a lot of people wanting their time. The thing they
lose is not the work; it is the thread back to it. A plan that lives only in this chat is
gone the moment they switch windows. Every step you write down is one fewer thing they
must hold in their head — which is why recording comes *first*, not after the analysis.

## The model, in seven lines

- A **task** has one stored status: `todo`, `doing`, `waiting` (on a person), `someday`, `done`, `dropped`.
- **Nesting means "first this."** A subtask is a prerequisite: something that must finish before its parent can. *To bake an apple pie from scratch you must first invent the universe* — so "invent the universe" goes under "bake the pie". The tree is a stack of causes, knocked out from the bottom.
- **Siblings mean "and also this."** Everything under one parent is independent and can be in flight at the same time; the parent finishes when all of them do. Siblings never depend on each other — if you want to write "after the sibling above", nest instead.
- A task with open subtasks is **blocked** — not theirs to do; the deepest open leaves are. When its last subtask closes it **resurfaces** at the top of Next. You never manage this; it is computed.
- A subtask tagged **`agent`** is yours. It never appears in the user's Next, but it still blocks the story until you close it. So tag honestly: it is how the user sees what is on you versus on them.
- `waiting` always has a follow-up date (default two days). `chase` records a nudge; `record_reply` records what came back; "unblock" (status `doing`) means it is ready and they are on it.
- Ids are six characters; any unique prefix works. Subtask order is the user's priority order — keep the order they would do things in.
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

1. **Record before you investigate.** Call `next` and `list_tasks`, then create the
   story and the steps you can already see. Refine the tree as you learn more; do not
   wait until you understand everything. If you need to ask the user something before
   the plan can be right, that question *is* a step: create it as their task with the
   questions in the body, then ask.
2. **Reuse, don't twin.** If the story already exists, add to it. If it is parked
   (`someday`) or ordered a certain way, that was the user's decision — ask before
   changing it, and say what you would change.
3. **Confirm the scope you are assuming.** If the request could mean two different
   things ("DNS for taskstaka" might mean a hostname for a demo, not exposing the daemon
   to a network), write the story at the level you are sure of, and make the ambiguity
   the first question-step rather than building a plan on a guess.
4. **Shape the plan as a stack, not a list.** For each step ask: *can this run at the
   same time as the others?* Then it is a sibling. *Must something happen before it?*
   Then that something goes **under** it. "Create the DNS record" needs "Log in", which
   needs "Install the gcloud CLI" — so install nests under log in nests under create,
   and the user's Next shows only *Install the CLI*; finishing it hands them *Log in*,
   and finishing that hands the agent *Create the record*. A prerequisite that two
   siblings both need becomes a sibling placed first (order carries "this one first");
   never duplicate it and never nest one sibling under another. Say who does each:
   - Steps **you** will do: tag `agent`. Mark `doing` when you start, `done` when
     finished. If a step of yours cannot start until the user does theirs, leave it
     `todo` with the tag — it is hidden from their Next automatically and still blocks
     the story. If you could not finish, leave it open and say why in the notes.
   - Steps **the user** must do by hand (console clicks, commands on a machine you
     cannot reach, approvals, sending an email): no tag. Put *exactly how to do that
     one step* in the `body` — the command, the URL, the draft message — so the task
     stands alone when they come back to it tomorrow.
   - **Granularity: one checkbox per thing they would tick.** The body is *how* to do
     one step, never a list of further steps. If you catch yourself writing "install X,
     then run Y, then Z" in a body, those are three subtasks (nest them under the step
     if they only make sense together). A step that cannot be ticked in one sitting is
     two steps. The user works from Next one row at a time; a paragraph of
     instructions is a plan they have to re-read, a row of subtasks is progress they
     can see.
   - Steps that **depend on another person**: create them `waiting_on` that person,
     with a `waiting_ref` if there is a thread link. The follow-up date defaults
     sensibly; set `until` only when you know better.
5. **Keep it true as you go.** When you finish an agent step, close it. When you hand
   something to the user, tell them in one line and leave it `todo` — it is now in
   their Next. When they tell you "I asked B" / "B replied" / "I pinged B again",
   record exactly that (`set_status waiting`, `record_reply`, `chase`). Never guess at
   what a person said; ask, then record the summary.
6. **Close the loop.** At the end, say what is in taskstaka: the next human step, and
   who they are waiting on. That is the handoff.

Do not mark a user's manual step `done` unless they say they did it. Do not send
messages on their behalf — draft them into the task's notes. For your *own* work, a step
is something that could be picked up tomorrow by someone else — not every command you
run — but the user's manual work gets a checkbox per action, because that is how they
will do it.

## Example: the DNS story

User: "I need a DNS name for taskstaka on our GCP project; figure out what's possible and
get it set up." Before reading a single doc:

```
add_task  "Get DNS address for taskstaka"                           → story
add_task  "Confirm what's possible for an internal zone"   parent=story, tags=[agent]
set_status <confirm…> doing
```

Then you investigate. You find the Cloud DNS change must be run with the right role, the
zone needs a platform engineer's approval, and you are not sure whether "for taskstaka"
means the daemon itself must be reachable from the VPC. The plan, as a stack:

```
Get DNS address for taskstaka
├─ Tell Claude: hostname only, or reachable from the VPC?      user — the question in the body
├─ Create the Cloud DNS zone and record                        agent — blocked until its stack is done
│  └─ Log in: gcloud auth login                                user — the two commands in the body
│     └─ Install the gcloud CLI                                user — the install link in the body
├─ Email platform-eng to approve the zone                      user — draft email in the body
├─ Ask Priya about wildcard certs on the zone                  waiting_on="Priya"
└─ Verify the record resolves                                  agent
```

Build it with `add_task(..., parent=<create>)` for "Log in" and `parent=<log in>` for
"Install". The user's Next then shows exactly what they can do now: answer the question,
install the CLI, send the email. Finishing the install surfaces *Log in*; finishing that
resurfaces *Create the zone* for you; the story resurfaces when everything under it is
closed.

You report: "Recorded under *Get DNS address*. Yours right now: one question (it's a
task), install the gcloud CLI, and an email to platform-eng (drafted in the task). Log in
and the zone come after the install, in order. We're waiting on Priya about wildcard
certs — she'll show in your follow-ups in two days." Later: "Priya says talk to Bo" →
`record_reply outcome=redirected to=Bo summary="Bo owns the zone"`. "Installed it" →
`set_status done` on that step, and *Log in* appears in their Next.

## Session habits

- **Start:** `next` once. Mention anything resurfaced or overdue in a sentence; don't lecture.
- **During:** record as you go, not in a batch at the end — the point is that the tree is true while they are away from the conversation.
- **End:** one line per open human step and one per person waited on. That is the whole summary; they will read it tomorrow.
