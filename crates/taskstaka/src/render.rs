//! Text rendering for the CLI. Everything here is pure: state in, string out.

use std::fmt::Write;

use jiff::Zoned;
use taskstaka_core::action::{Summary, derive};
use taskstaka_core::humanize;
use taskstaka_core::{NextItem, Status, Task, Tier, Tree};

pub fn tier_badge(t: Tier) -> &'static str {
    match t {
        Tier::Overdue => "OVERDUE",
        Tier::Resurfaced => "back",
        Tier::Doing => "doing",
        Tier::FollowUp => "follow-up",
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
            "Nothing actionable. {} blocked by subtasks, {} waiting on others{} — go unblock something.",
            s.blocked,
            s.waiting,
            if s.someday > 0 {
                format!(", {} someday", s.someday)
            } else {
                String::new()
            }
        );
    }
    let mut parts = vec![format!("{} actionable", s.actionable)];
    if s.doing > 0 {
        parts.push(format!("{} doing", s.doing));
    }
    if s.blocked > 0 {
        parts.push(format!("{} blocked", s.blocked));
    }
    if s.waiting > 0 {
        parts.push(format!("{} waiting", s.waiting));
    }
    if s.someday > 0 {
        parts.push(format!("{} someday", s.someday));
    }
    parts.join(" · ")
}

pub fn next_table(items: &[NextItem], explain: bool, show_context: bool) -> String {
    let mut out = String::new();
    let title_w = items
        .iter()
        .map(|i| i.title.chars().count())
        .max()
        .unwrap_or(0)
        .clamp(10, 48);
    for it in items {
        let badge = tier_badge(it.tier);
        let crumbs = if it.ancestry.is_empty() {
            String::new()
        } else {
            let root = &it.ancestry[0].title;
            let parent = it.ancestry.last().map(|c| c.title.as_str()).unwrap_or("");
            if it.ancestry.len() == 1 {
                format!("‹ {root}")
            } else {
                format!("‹ {root} › … › {parent}")
            }
        };
        let ctx = if show_context {
            format!(" {:<8}", it.context.as_str())
        } else {
            String::new()
        };
        let _ = writeln!(
            out,
            "{}  {:<9}{} {:<title_w$}  {:<28} queued {:>4} · touched {:>4}",
            it.id,
            badge,
            ctx,
            truncate(&it.title, title_w),
            truncate(&crumbs, 28),
            humanize::duration(it.queued_secs),
            humanize::duration(it.untouched_secs),
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
        Status::Todo if d.blocked_by > 0 => "⏳",
        Status::Todo => "·",
        Status::Doing => "▶",
        Status::Waiting => "⌛",
        Status::Someday => "💤",
        Status::Done => "✓",
        Status::Dropped => "✗",
    };
    let mut extra = Vec::new();
    if d.blocked_by > 0 {
        extra.push(format!("blocked by {}", d.blocked_by));
    }
    if d.resurfaced_at.is_some() {
        extra.push("resurfaced".to_string());
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
                Status::Waiting => "⌛",
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
