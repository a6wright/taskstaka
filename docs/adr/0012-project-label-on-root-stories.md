# 0012: A project label on root stories

## Context

With several repos feeding one list, Next could not say which project a row belonged to.
The user asked for a label that spans several root stories, shows as a coloured pill beside
*now* and *in hand*, appears on the Outline root, and that the agent skill keeps tidy.

## Decision

- `project` (free text) and `swatch` (a palette index) are frontmatter fields on a **root**
  task. Everything below inherits the root's project; it is derived, never copied
  (`project::tag_of`), as `context` is. Setting it on a subtask is refused with a message
  naming the root. Moving a root under a story drops its own project.
- Names match case-insensitively: "DevOps" reuses "devops" and its colour.
- The colour is **assigned when a name first appears**: the first of ten palette colours no
  other project uses (least used once all are taken), then stored on the root. It does not
  shift when other projects come and go, and every root of one project shares it. Hashing the
  name was rejected: with a handful of projects it collides too often, and unique colours are
  the point. A hand-written note with a project and no swatch falls back to a hash.
- `TaskView.project_tag {name, color}` carries it to clients; the TUI computes it from its
  local tree with the same core function, so the two cannot disagree.
- Surfaces: CLI `add --project`, `edit --project/--no-project`; MCP `add_task`/`update_task`
  `project` and a `projects` tool (name, colour, root stories) so the skill can reuse a
  match; web pill and a pane field with the known names; TUI pill in the Next box border,
  the in-hand plate and on Outline roots.
- The skill's rule: call `projects` before a new root, reuse a match, otherwise name a new
  one for the repo.

## Consequences

Two roots with the same project need no extra object. There is no rename-everywhere or
colour picker yet: renaming means editing each root.
