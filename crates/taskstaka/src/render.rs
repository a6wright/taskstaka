//! Text rendering for the CLI. Everything here is pure: state in, string out.

use std::fmt::Write;

use jiff::Zoned;
use taskstaka_core::action::{StoryPlan, Summary, derive, is_agent_step};
use taskstaka_core::humanize;
use taskstaka_core::{NextItem, Status, Task, TaskId, Tier, Tree};

/// The same glyphs as the web and the TUI: ✉ a person, ▽ prerequisites
/// below, ↑ ready, ▶ doing, 🤖 Claude's step, 💤 parked.
pub fn tier_badge(t: Tier) -> &'static str {
    match t {
        Tier::Overdue => "OVERDUE",
        Tier::Resurfaced => "↑ ready",
        Tier::Doing => "▶ doing",
        Tier::FollowUp => "✉ follow up",
        Tier::DueSoon => "due soon",
        Tier::High => "high",
        Tier::Normal => "",
        Tier::Low => "low",
    }
}

pub fn summary_line(s: &Summary) -> String {
    if s.open == 0 {
        return "Nothing open. Add something with `taskstaka add`.".into();
    }
    if s.actionable == 0 {
        return format!(
            "Nothing you can do right now. {} blocked by prerequisites, {} waiting on people{} — chase someone or unblock something.",
            s.blocked,
            s.waiting,
            if s.someday > 0 {
                format!(", {} someday", s.someday)
            } else {
                String::new()
            }
        );
    }
    let mut parts = vec![format!("{} you can do now", s.actionable)];
    if s.doing > 0 {
        parts.push(format!("{} doing", s.doing));
    }
    if s.blocked > 0 {
        parts.push(format!("{} blocked", s.blocked));
    }
    if s.waiting > 0 {
        parts.push(format!("{} waiting on people", s.waiting));
    }
    if s.someday > 0 {
        parts.push(format!("{} someday", s.someday));
    }
    parts.join(" · ")
}

/// The *now* rows of one story: `id  ☐ title ‹ step above   badge   age`.
/// Rows start with the id at column 0 so scripts (and the tests) can find them.
pub fn now_rows(
    tree: &Tree,
    items: &[NextItem],
    in_hand: Option<&TaskId>,
    explain: bool,
    show_context: bool,
) -> String {
    let mut out = String::new();
    // The story is the heading; a row names only the step it unlocks.
    let crumb = |it: &NextItem| match it.ancestry.last().filter(|_| it.ancestry.len() > 1) {
        Some(c) => format!("{} ‹ {}", it.title, c.title),
        None => it.title.clone(),
    };
    let title_w = items
        .iter()
        .map(|i| crumb(i).chars().count())
        .max()
        .unwrap_or(0)
        .clamp(10, 64);
    for it in items {
        let mut badge = if in_hand == Some(&it.id) {
            "▶ in hand".to_string()
        } else {
            tier_badge(it.tier).to_string()
        };
        if tree.get(&it.id).is_some_and(is_agent_step) {
            badge = if badge.is_empty() {
                "🤖 Claude".into()
            } else {
                format!("{badge} · 🤖")
            };
        }
        if it.reason.contains("parent is already finished") {
            badge = if badge.is_empty() {
                "⚠ orphaned".into()
            } else {
                format!("{badge} · ⚠")
            };
        }
        let ctx = if show_context {
            format!(" {:<8}", it.context.as_str())
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "{}  ☐{} {:<title_w$}  {:<14} {:>4}",
            it.id,
            ctx,
            truncate(&crumb(it), title_w),
            badge,
            humanize::duration(it.queued_secs),
        );
        if explain {
            let _ = writeln!(out, "{:8}└ {}", "", it.reason);
        }
    }
    out
}

pub fn tree_text(tree: &Tree, now: &Zoned, roots: &[&Task], include_closed: bool) -> String {
    let mut out = String::new();
    for r in roots {
        node(&mut out, tree, r, now, 0, include_closed);
    }
    out
}

fn node(out: &mut String, tree: &Tree, t: &Task, now: &Zoned, depth: usize, include_closed: bool) {
    if !include_closed && t.status.is_closed() {
        return;
    }
    let d = derive(tree, t, now.date());
    let mark = match t.status {
        Status::Todo if d.blocked_by > 0 => "▽",
        Status::Todo => "·",
        Status::Doing => "▶",
        Status::Waiting => "✉",
        Status::Someday => "💤",
        Status::Done => "✓",
        Status::Dropped => "✗",
    };
    let mut extra = Vec::new();
    if d.blocked_by > 0 {
        extra.push(format!("blocked by {}", d.blocked_by));
    }
    if d.resurfaced_at.is_some() {
        extra.push("↑ ready".to_string());
    }
    if let Some(w) = &t.waiting_on {
        extra.push(format!(
            "waiting on {w}{}",
            t.until.map(|u| format!(" until {u}")).unwrap_or_default()
        ));
    }
    if let Some(due) = t.due {
        extra.push(if due < now.date() {
            format!("OVERDUE {due}")
        } else {
            format!("due {due}")
        });
    }
    if let Some(p) = t.priority {
        extra.push(p.to_string());
    }
    if t.tags.iter().any(|x| x.eq_ignore_ascii_case("agent")) {
        extra.push("🤖 Claude".to_string());
    }
    let extra = if extra.is_empty() {
        String::new()
    } else {
        format!("  ({})", extra.join(", "))
    };
    let _ = writeln!(
        out,
        "{}{mark} {}  {}{extra}  {}",
        "  ".repeat(depth),
        t.id,
        t.title,
        humanize::since(t.created, now.timestamp())
    );
    for c in tree.children(&t.id) {
        node(out, tree, c, now, depth + 1, include_closed);
    }
}

pub fn show(tree: &Tree, t: &Task, now: &Zoned) -> String {
    let mut out = String::new();
    let d = derive(tree, t, now.date());
    let anc = tree.ancestors(&t.id);
    if !anc.is_empty() {
        let path: Vec<String> = anc
            .iter()
            .rev()
            .map(|a| format!("{} ({})", a.title, a.id))
            .collect();
        let _ = writeln!(out, "{}", path.join(" › "));
    }
    let _ = writeln!(out, "{}  {}", t.id, t.title);
    let mut state = vec![t.status.to_string()];
    if d.blocked_by > 0 {
        state.push(format!("blocked by {}", d.blocked_by));
    }
    if d.actionable {
        state.push("actionable".into());
    }
    if let Some(at) = d.resurfaced_at {
        state.push(format!(
            "resurfaced {} ago",
            humanize::since(at, now.timestamp())
        ));
    }
    if let Some(by) = &d.deferred_by
        && *by != t.id
    {
        state.push(format!("deferred by {by}"));
    }
    let _ = writeln!(out, "  status    {}", state.join(", "));
    let _ = writeln!(out, "  context   {}", t.context);
    if let Some(p) = t.priority {
        let _ = writeln!(out, "  priority  {p}");
    }
    if let Some(w) = &t.waiting_on {
        let _ = writeln!(
            out,
            "  waiting   {w}{}",
            t.until.map(|u| format!(" (until {u})")).unwrap_or_default()
        );
    }
    if let Some(due) = t.due {
        let _ = writeln!(out, "  due       {due}");
    }
    if !t.tags.is_empty() {
        let _ = writeln!(out, "  tags      {}", t.tags.join(", "));
    }
    let ts = now.timestamp();
    let _ = writeln!(out, "  created   {} ago", humanize::since(t.created, ts));
    let _ = writeln!(out, "  touched   {} ago", humanize::since(t.updated, ts));
    if let Some(s) = t.started {
        let _ = writeln!(out, "  started   {} ago", humanize::since(s, ts));
    }
    if let Some(c) = t.completed {
        let _ = writeln!(out, "  completed {} ago", humanize::since(c, ts));
    }
    let kids: Vec<&Task> = tree.children(&t.id).collect();
    if !kids.is_empty() {
        out.push_str("  subtasks\n");
        for k in kids {
            let mark = match k.status {
                Status::Done => "✓",
                Status::Dropped => "✗",
                Status::Doing => "▶",
                Status::Waiting => "✉",
                Status::Someday => "💤",
                Status::Todo => "·",
            };
            let _ = writeln!(out, "    {mark} {}  {}", k.id, k.title);
        }
    }
    if !t.body.trim().is_empty() {
        out.push('\n');
        out.push_str(t.body.trim_end());
        out.push('\n');
    }
    out
}

fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut t: String = s.chars().take(max.saturating_sub(1)).collect();
        t.push('…');
        t
    }
}

/// `taskstaka waiting`: grouped by person, due follow-ups first.
pub fn waiting_text(groups: &[taskstaka_core::action::PersonGroup], now: &Zoned) -> String {
    let mut out = String::new();
    let today = now.date();
    for g in groups {
        let due = if g.any_due { " · follow-up due" } else { "" };
        let _ = writeln!(
            out,
            "{}  ({} item{} · oldest {}{due})",
            g.person,
            g.items.len(),
            if g.items.len() == 1 { "" } else { "s" },
            humanize::duration(g.oldest_secs),
        );
        for i in &g.items {
            let root = i
                .root_title
                .as_deref()
                .map(|r| format!("  ‹ {r}"))
                .unwrap_or_default();
            let chased = match i.wait.chases {
                0 => String::new(),
                n => format!(" · chased {n}×"),
            };
            let until = match i.wait.until {
                Some(u) if u < today => format!(" · follow-up was {u}"),
                Some(u) if u == today => " · follow up today".to_string(),
                Some(u) => format!(" · follow up {u}"),
                None => String::new(),
            };
            let link = i
                .wait
                .waiting_ref
                .as_deref()
                .map(|r| format!("\n           {r}"))
                .unwrap_or_default();
            let _ = writeln!(
                out,
                "   {}  {}{root}  {}{chased}{until}{link}",
                i.wait.task_id,
                i.wait.title,
                humanize::duration(i.since_secs),
            );
        }
    }
    out
}

/// Notes without the journal stamps, newest last, at most `n`.
fn note_lines(body: &str, n: usize) -> Vec<&str> {
    let lines: Vec<&str> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l.trim_start_matches("- ");
            match l.find("] ") {
                Some(i) if l.starts_with(char::is_numeric) => &l[i + 2..],
                _ => l,
            }
        })
        .collect();
    let skip = lines.len().saturating_sub(n);
    lines.into_iter().skip(skip).collect()
}

/// The in-hand plate: the one thing to go do, with its notes and the four
/// verbs that move it. Shared by `next` and `stack`.
pub fn plate(tree: &Tree, t: &Task, now: &Zoned) -> String {
    let mut out = String::new();
    let ts = now.timestamp();
    let d = derive(tree, t, now.date());
    let state = if t.status.is_closed() {
        "done".to_string()
    } else if t.status == Status::Waiting {
        format!("✉ waiting on {}", t.waiting_on.as_deref().unwrap_or("?"))
    } else if d.blocked_by > 0 {
        format!("▽ blocked by {} — finish what is under it", d.blocked_by)
    } else if t.status == Status::Doing {
        format!(
            "started {}",
            humanize::since(t.started.unwrap_or(t.created), ts)
        )
    } else {
        "not started".to_string()
    };
    let agent = if is_agent_step(t) {
        "  🤖 Claude's step — prompt it, then check"
    } else {
        ""
    };
    let _ = writeln!(out, "▶ in hand  {}  {}   {state}{agent}", t.id, t.title);
    let chain = tree.ancestors(&t.id);
    if chain.is_empty() {
        let _ = writeln!(out, "           a story of its own");
    } else {
        let path: Vec<&str> = chain.iter().rev().map(|c| c.title.as_str()).collect();
        let _ = writeln!(out, "           so that › {}", path.join(" › "));
    }
    let notes = note_lines(&t.body, 6);
    if notes.is_empty() {
        let _ = writeln!(
            out,
            "           no notes yet · taskstaka touch {} NOTE",
            t.id
        );
    }
    for l in notes {
        let _ = writeln!(out, "           ≡ {l}");
    }
    let _ = writeln!(
        out,
        "           done {id} · reopen {id} (put back) · add -p {id} --start \"first I need to…\" · wait {id} WHO",
        id = t.id
    );
    out
}

/// `taskstaka stack`: the plate, then the chain it unlocks, bottom up.
pub fn stack(tree: &Tree, t: &Task, now: &Zoned) -> String {
    let mut out = plate(tree, t, now);
    let under: Vec<&Task> = tree
        .children(&t.id)
        .filter(|k| k.status.is_open())
        .collect();
    if !under.is_empty() {
        let _ = writeln!(out, "  first:");
        for k in under {
            let _ = writeln!(out, "    ▽ {}  {}", k.id, k.title);
        }
    }
    let chain = tree.ancestors(&t.id);
    let n = chain.len();
    let mut prev_id = &t.id;
    for (i, c) in chain.iter().enumerate() {
        let rel = if i + 1 == n { "so that" } else { "then" };
        let also: Vec<String> = tree
            .children(&c.id)
            .filter(|k| k.status.is_open() && k.id != *prev_id)
            .map(|k| k.title.clone())
            .collect();
        let also = if also.is_empty() {
            String::new()
        } else {
            format!("   (also needs: {})", also.join(", "))
        };
        let agent = if is_agent_step(c) { " 🤖" } else { "" };
        let _ = writeln!(out, "  {rel:>7}: {}  {}{agent}{also}", c.id, c.title);
        prev_id = &c.id;
    }
    out
}

/// How `taskstaka next` prints.
#[derive(Clone, Copy, Debug, Default)]
pub struct PlanOpts {
    /// The reason under each now row.
    pub explain: bool,
    /// Expand `… N later` into one line per task.
    pub later: bool,
    /// Stories to show; 0 means all.
    pub limit: usize,
    pub show_context: bool,
}

/// `taskstaka next`: the plate in hand, then each story as a stack — what
/// you can do now, and `… N later` (what is still required, and why not yet).
pub fn plan_text(
    tree: &Tree,
    plans: &[StoryPlan],
    in_hand: Option<&Task>,
    now: &Zoned,
    opts: PlanOpts,
) -> String {
    let PlanOpts {
        explain,
        later: expand_later,
        limit,
        show_context,
    } = opts;
    let limit = if limit == 0 { usize::MAX } else { limit };
    let mut out = String::new();
    if let Some(t) = in_hand {
        out.push_str(&plate(tree, t, now));
        out.push('\n');
    }
    let hand_id = in_hand.map(|t| &t.id);
    for (shown, p) in plans.iter().enumerate() {
        if shown >= limit {
            let _ = writeln!(out, "… and {} more stories (use -n)", plans.len() - shown);
            break;
        }
        let mut head = Vec::new();
        if !p.now.is_empty() {
            head.push(format!("{} now", p.now.len()));
        }
        if !p.later.is_empty() {
            head.push(match p.later_agent {
                0 => format!("{} later", p.later.len()),
                n => format!("{} later ({n} 🤖)", p.later.len()),
            });
        }
        if let Some(stuck) = &p.stuck {
            head.push(stuck.clone());
        } else if !p.waiting_on.is_empty() {
            head.push(format!("✉ {}", p.waiting_on.join(", ")));
        }
        let _ = writeln!(out, "{}   {}", p.title, head.join(" · "));
        if !p.now.is_empty() {
            out.push_str(&now_rows(tree, &p.now, hand_id, explain, show_context));
        }
        if !p.later.is_empty() {
            if expand_later {
                for l in &p.later {
                    let mark = match l.status {
                        Status::Waiting => "✉",
                        Status::Doing => "▶",
                        Status::Someday => "💤",
                        _ if l.open_under > 0 => "▽",
                        _ => "·",
                    };
                    let robot = if l.agent { " 🤖" } else { "" };
                    let _ = writeln!(
                        out,
                        "        {mark} {}  {}{robot}  — {}",
                        l.id, l.title, l.why
                    );
                }
            } else {
                let peek: Vec<String> = p
                    .later
                    .iter()
                    .take(2)
                    .map(|l| format!("{} ({})", l.title, l.why))
                    .collect();
                let more = if p.later.len() > 2 {
                    format!(", +{}", p.later.len() - 2)
                } else {
                    String::new()
                };
                let _ = writeln!(
                    out,
                    "        … {} later: {}{more}   (--later)",
                    p.later.len(),
                    peek.join(", ")
                );
            }
        }
        out.push('\n');
    }
    out
}
