---
status: accepted
date: 2026-10-27
---

# 0018: `/today` discovers its sources at run time and remembers them per machine

The user's words: "we want this to be generic enough so that it can use anything that has
an MCP that we could get info from, meeting transcripts, DMs in applications, anything
really", and, on a first draft that inventoried this machine: "the connectors available on
this machine shouldn't be what we design for. we should make it a config the skill writes
and remembers based on what is available to whatever Claude it's used on."

## Context

The day-start skill reads other systems — calendar, mail, chat, trackers, document stores,
folders of transcripts — to bring action items and replies into the tree before asking
the user anything. Which systems exist differs per machine and per Claude (Claude Code on
a laptop, Cowork, a work account with Linear and Gmail, a personal one with neither), and
they change: a connector is signed in, another is removed. A skill written against a fixed
list is wrong everywhere but where it was written.

## Decision

- **The session's tool names are the inventory.** Every `mcp__<server>__<tool>` present is
  a server the skill may read; a server exposing only `authenticate` tools is unconnected.
  The skill never names a product in its steps; the playbooks are per **role** (calendar,
  mail, chat, tracker, docs, folder, generic) and a role is chosen per server.
- **One file per machine, written by the skill:** `today.md` in taskstaka's machine
  settings folder (ADR 0011: `$TASKSTAKA_CONFIG_DIR`, `$XDG_CONFIG_HOME/taskstaka`,
  `~/.config/taskstaka`). It records the servers seen, each server's and folder's `read` /
  `skip` / `unconnected` and role, optional `look for:`, `project:` and `tags:` lines, and
  the last run. The setup round runs when the file is missing, when the server set changed,
  or on `/today sources`; otherwise nothing is asked about sources.
- **Projects and tags are inferred, then remembered.** A source's `project:` line, a
  project name found in the candidate, the repo the session runs in, or the matched story
  pre-fill every card; a pick the user makes is written back as the source's `project:`.
- **Reconcile writes silently only what a message proves**: a reply from the person a task
  waits on, the user's own ping, a thread link. Creating, closing, moving, parking or
  unparking a task goes through the interview. The config holds sources, roles, projects
  and dates, nothing about the user.
- The skill ships in the binary (`assets/skill-today/`) and `taskstaka setup claude`
  installs it beside the `taskstaka` skill, so every install gets both and a rerun after an
  upgrade refreshes both.

## Consequences

- A new kind of server costs a playbook file, not a rewrite; a server with no fitting role
  works through `generic.md` and one question.
- Two machines can read different sources and share the same tree; the tree never learns
  where a task came from except through the citation in its notes.
- An unattended run is out of scope: the interview is the product. An evening wrap
  (`/today wrap`) is the obvious next step and is on the backlog.
