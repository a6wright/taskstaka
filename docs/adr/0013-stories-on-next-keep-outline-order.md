---
status: accepted
date: 2026-10-07
---

# 0013: Stories on Next keep their outline order

Amends ADR 0009, which said "tiers still order stories".

## Context

The user ticked a row and the story boxes on Next swapped places. Stories were ordered
by their best-ranked `now` row, so finishing the overdue or in-progress row that carried
a story's rank re-ranked the story, and the box the eye was on moved. With two stories
in view the swap happened on almost every completion. ADR 0009 had already made the
rows inside a story unranked parallel streams in outline order, for the same reason:
choosing is the user's, and a list that reorders itself under the hand cannot be
scanned.

## Decision

- `action::plan` lists stories in **outline order** (`Tree::roots`, the order the user
  drags them into). Stories with something to do now come first; stories with open work
  but nothing actionable (stuck) follow, in the same order. A story moves only when it
  gains or loses its last doable row, never because a row's tier changed.
- Tiers still **badge** rows (overdue, ↑ ready, due soon, doing) and still order the flat
  `next` list used by the MCP `next` tool and `GET /api/v1/next`. Only the grouped view
  stops ranking.
- The web, TUI and CLI Next views all render `plan`'s order, so none of them can disagree.

## Consequences

An urgent row in the last story no longer lifts that story to the top; its chip and the
nudges are the signal. Users who want a story higher drag it higher in the Outline, and
that choice sticks.
