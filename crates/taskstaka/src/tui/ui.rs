//! Rendering. Pure function of `App`; snapshot-tested with `TestBackend`.
//!
//! Next reads like the web's plates: the task in hand boxed at the top with
//! its notes and the four verbs that move it, then each story as a stack —
//! bright *now* rows, a folded `… N later` line, the story at the base.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Clear, Paragraph, Wrap};
use taskstaka_core::action::{derive, is_agent_step};
use taskstaka_core::{Status, Tier, humanize};

use super::app::{App, MessageKind, Mode, NodeKind, Row, View};

const SELECTED_BG: Color = Color::Rgb(40, 40, 50);

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let plate_h = plate_height(app, area);
    let [header, plate, list, detail, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Length(plate_h),
        Constraint::Min(4),
        Constraint::Length(detail_height(app, area)),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);
    if plate_h > 0 {
        draw_plate(f, app, plate);
    }
    draw_list(f, app, list);
    draw_detail(f, app, detail);
    draw_footer(f, app, footer);

    match &app.mode {
        Mode::Help => draw_help(f, area),
        Mode::Prompt(p) => {
            let line = Line::from(vec![
                Span::styled(format!(" {}: ", p.label()), Style::default().bold()),
                Span::raw(p.buf.as_str()),
                Span::styled("▏", Style::default().fg(Color::Yellow)),
            ]);
            f.render_widget(Clear, footer);
            f.render_widget(
                Paragraph::new(line).style(Style::default().bg(Color::DarkGray)),
                footer,
            );
        }
        Mode::ConfirmQuit => {
            f.render_widget(Clear, footer);
            f.render_widget(
                Paragraph::new(" quit? y/n ").style(Style::default().bg(Color::DarkGray).bold()),
                footer,
            );
        }
        Mode::Normal => {}
    }
}

/// The in-hand plate sits above the list on Next: title, where it sits, the
/// last few note lines, and the verbs. Zero when nothing is in hand.
fn plate_height(app: &App, area: Rect) -> u16 {
    if app.view != View::Next {
        return 0;
    }
    let Some(t) = app.in_hand() else { return 0 };
    let notes = note_lines(&t.body).len() as u16;
    // border + title + crumb + notes + verbs + border
    let want = 2 + 1 + 1 + notes + 1;
    want.min(area.height / 2).max(4)
}

fn detail_height(app: &App, area: Rect) -> u16 {
    let Some(t) = app.selected_task() else {
        return 0;
    };
    // The plate already shows the task in hand; no need to repeat it.
    if matches!(app.selected_row(), Some(Row::Plate(id)) if *id == t.id) {
        return 0;
    }
    (area.height / 3).clamp(4, 10)
}

/// Journal lines are `- date time [source] text`; strip the stamp so the
/// plate reads like notes, newest last, at most three.
fn note_lines(body: &str) -> Vec<String> {
    let lines: Vec<String> = body
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(|l| {
            let l = l.trim_start_matches("- ");
            match l.find("] ") {
                Some(i) if l.starts_with(char::is_numeric) => l[i + 2..].to_string(),
                _ => l.to_string(),
            }
        })
        .collect();
    let n = lines.len();
    lines.into_iter().skip(n.saturating_sub(3)).collect()
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let ctx = match app.ctx {
        None => "all".to_string(),
        Some(c) => c.to_string(),
    };
    let mut spans = vec![
        Span::styled(" taskstaka ", Style::default().bold().fg(Color::Cyan)),
        Span::styled(format!("· {ctx} "), Style::default().fg(Color::DarkGray)),
    ];
    if !app.connected {
        spans.push(Span::styled(
            " DISCONNECTED ",
            Style::default().fg(Color::Black).bg(Color::Red).bold(),
        ));
    }
    for (i, v) in View::ALL.iter().enumerate() {
        let count = match v {
            View::Waiting => app.summary.waiting,
            View::Someday => app.summary.someday,
            View::Nudges => app.nudges.len(),
            View::Next | View::Outline => 0,
        };
        let label = if count > 0 {
            format!(" {} {}·{} ", i + 1, v.label(), count)
        } else {
            format!(" {} {} ", i + 1, v.label())
        };
        spans.push(Span::styled(
            label,
            if *v == app.view {
                Style::default().fg(Color::Black).bg(Color::Cyan).bold()
            } else {
                Style::default().fg(Color::Gray)
            },
        ));
    }
    let line1 = Line::from(spans);

    let s = &app.summary;
    let line2 = if let Some((kind, msg)) = &app.message {
        let style = match kind {
            MessageKind::Info => Style::default().fg(Color::Gray),
            MessageKind::Success => Style::default().fg(Color::Green),
            MessageKind::Error => Style::default().fg(Color::Red).bold(),
        };
        Line::from(Span::styled(format!(" {msg}"), style))
    } else if let Some(n) = app.nudges.first().filter(|_| app.view != View::Nudges) {
        Line::from(Span::styled(
            format!(" ⚑ {}  (5 for all)", n.message),
            Style::default().fg(Color::Magenta),
        ))
    } else if s.open == 0 {
        Line::from(Span::styled(
            " nothing open",
            Style::default().fg(Color::DarkGray),
        ))
    } else if s.actionable == 0 {
        Line::from(Span::styled(
            format!(
                " nothing you can do right now · {} blocked · {} waiting — chase someone or unblock something",
                s.blocked, s.waiting
            ),
            Style::default().fg(Color::Yellow).bold(),
        ))
    } else {
        Line::from(Span::styled(
            format!(
                " {} you can do now · {} doing · {} blocked · {} waiting on people · {} someday",
                s.actionable, s.doing, s.blocked, s.waiting, s.someday
            ),
            Style::default().fg(Color::Gray),
        ))
    };
    f.render_widget(Paragraph::new(vec![line1, line2]), area);
}

fn draw_plate(f: &mut Frame, app: &App, area: Rect) {
    let Some(t) = app.in_hand() else { return };
    let selected = matches!(app.selected_row(), Some(Row::Plate(id)) if *id == t.id);
    let now = app.now.timestamp();
    let mut lines: Vec<Line> = Vec::new();

    let since = humanize::since(t.started.unwrap_or(t.created), now);
    lines.push(Line::from(vec![
        Span::styled(" ▶ in hand  ", Style::default().fg(Color::Yellow).bold()),
        Span::styled(t.title.clone(), Style::default().bold()),
        Span::styled(
            format!("   started {since}"),
            Style::default().fg(Color::DarkGray),
        ),
    ]));

    let anc = app.tree.ancestors(&t.id);
    let path: Vec<&str> = anc.iter().rev().map(|a| a.title.as_str()).collect();
    let mut crumb = if path.is_empty() {
        " a story of its own".to_string()
    } else {
        format!(" so that › {}", path.join(" › "))
    };
    if is_agent_step(t) {
        crumb.push_str("   🤖 Claude's step — prompt it, then check");
    }
    lines.push(Line::from(Span::styled(
        crumb,
        Style::default().fg(Color::DarkGray),
    )));

    let notes = note_lines(&t.body);
    if notes.is_empty() {
        lines.push(Line::from(Span::styled(
            " no notes yet · t to add one, e to open",
            Style::default().fg(Color::DarkGray).italic(),
        )));
    } else {
        for n in notes {
            lines.push(Line::from(Span::styled(
                format!(" ≡ {n}"),
                Style::default().fg(Color::Gray),
            )));
        }
    }
    lines.push(Line::from(Span::styled(
        " d done · p put back · i first I need to… · w wait on someone",
        Style::default().fg(Color::Yellow),
    )));

    let border = if selected {
        Style::default().fg(Color::Yellow)
    } else {
        Style::default().fg(Color::DarkGray)
    };
    let block = Block::bordered().border_style(border);
    let para = Paragraph::new(lines).block(block);
    f.render_widget(
        if selected {
            para.style(Style::default().bg(SELECTED_BG))
        } else {
            para
        },
        area,
    );
}

pub fn tier_style(t: Tier) -> Style {
    match t {
        Tier::Overdue => Style::default().fg(Color::Red).bold(),
        Tier::Resurfaced => Style::default().fg(Color::Magenta).bold(),
        Tier::Doing => Style::default().fg(Color::Yellow),
        Tier::FollowUp => Style::default().fg(Color::Cyan),
        Tier::DueSoon => Style::default().fg(Color::LightYellow),
        Tier::High => Style::default().fg(Color::Green),
        Tier::Normal => Style::default(),
        Tier::Low => Style::default().fg(Color::DarkGray),
    }
}

fn tier_badge(t: Tier) -> &'static str {
    match t {
        Tier::Overdue => "OVERDUE",
        Tier::Resurfaced => "↑ ready",
        Tier::Doing => "▶ doing",
        Tier::FollowUp => "✉ follow up",
        Tier::DueSoon => "due soon",
        Tier::High => "high",
        Tier::Normal | Tier::Low => "",
    }
}

fn empty_message(app: &App) -> &'static str {
    match app.view {
        View::Next if app.summary.open == 0 => "Nothing open. Press a to add a task.",
        View::Next => {
            "Nothing you can do right now. 2 for the outline, 3 for who you are waiting on."
        }
        View::Outline => "Nothing here. Press a to add a task; A adds under the selection.",
        View::Waiting => "Not waiting on anyone. w on a task hands it to a person.",
        View::Someday => "Nothing parked. z parks a task; a here parks an idea.",
        View::Nudges => "No nudges. They appear when something sits too long or resurfaces.",
    }
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    let width = area.width as usize;
    // Plate rows render above the list, so count only what is drawn here.
    let drawn: Vec<(usize, &Row)> = app
        .rows
        .iter()
        .enumerate()
        .filter(|(_, r)| !matches!(r, Row::Plate(_)))
        .collect();
    if drawn.is_empty() {
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {}", empty_message(app)),
                Style::default().fg(Color::DarkGray),
            ))),
            area,
        );
        return;
    }

    let visible = area.height as usize;
    let sel_pos = drawn
        .iter()
        .position(|(i, _)| *i == app.selected)
        .unwrap_or(0);
    let start = if sel_pos >= visible {
        sel_pos + 1 - visible
    } else {
        0
    };

    let in_hand = app.in_hand().map(|t| t.id.clone());
    let mut lines = Vec::with_capacity(visible);
    for (i, row) in drawn.into_iter().skip(start).take(visible) {
        let selected = i == app.selected;
        let cursor = if selected { "› " } else { "  " };
        let line = match row {
            Row::Plate(_) => continue,
            Row::Section(title) => Line::from(Span::styled(
                format!(" ── {title} "),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Row::Item(it) => {
                let agent = app.tree.get(&it.id).is_some_and(is_agent_step);
                let mut badge = if in_hand.as_ref() == Some(&it.id) {
                    "▶ in hand".to_string()
                } else {
                    tier_badge(it.tier).to_string()
                };
                if agent {
                    badge = if badge.is_empty() {
                        "🤖 Claude".into()
                    } else {
                        format!("{badge} · 🤖")
                    };
                }
                let mut crumb = String::new();
                if it.ancestry.len() > 1 {
                    // The story is the base line; show only the step above.
                    crumb = format!(" ‹ {}", it.ancestry.last().unwrap().title);
                }
                if let Some(due) = it.due
                    && matches!(it.tier, Tier::Overdue | Tier::DueSoon)
                {
                    crumb.push_str(&format!(" · due {due}"));
                }
                let badge_s = if badge.is_empty() {
                    String::new()
                } else {
                    format!(" {badge} ")
                };
                let age = format!(" {:>4} ", humanize::duration(it.queued_secs));
                let fixed = 4 + cells(&badge_s) + cells(&age);
                let title_room = width.saturating_sub(fixed);
                let style = if in_hand.as_ref() == Some(&it.id) {
                    Style::default().fg(Color::Yellow)
                } else {
                    tier_style(it.tier)
                };
                Line::from(vec![
                    Span::raw(cursor),
                    Span::styled("☐ ", Style::default().fg(Color::Gray)),
                    Span::styled(
                        pad(
                            &fit(&format!("{}{}", it.title, crumb), title_room),
                            title_room,
                        ),
                        if selected {
                            Style::default().bold()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(badge_s, style),
                    Span::styled(age, Style::default().fg(Color::DarkGray)),
                ])
            }
            Row::LaterToggle {
                count,
                agent,
                expanded,
                peek,
                ..
            } => {
                let arrow = if *expanded { "▾" } else { "▸" };
                let robots = if *agent > 0 {
                    format!(" · {agent} 🤖")
                } else {
                    String::new()
                };
                let text = if *expanded {
                    format!("  {arrow} {count} later{robots}")
                } else {
                    format!("  {arrow} … {count} later{robots}: {peek}")
                };
                Line::from(vec![
                    Span::raw(cursor),
                    Span::styled(
                        fit(&text, width.saturating_sub(2)),
                        Style::default().fg(Color::DarkGray),
                    ),
                ])
            }
            Row::Later(l) => {
                let mark = match l.status {
                    Status::Waiting => "✉",
                    Status::Someday => "💤",
                    Status::Doing => "▶",
                    _ if l.open_under > 0 => "▽",
                    _ => "·",
                };
                let robot = if l.agent { " 🤖" } else { "" };
                let why = format!(" {} ", l.why);
                let title_room = width.saturating_sub(8 + cells(robot) + cells(&why));
                Line::from(vec![
                    Span::raw(cursor),
                    Span::styled(format!("    {mark} "), Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        pad(&fit(&format!("{}{robot}", l.title), title_room), title_room),
                        if selected {
                            Style::default().fg(Color::Gray).bold()
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                    Span::styled(why, Style::default().fg(Color::DarkGray).italic()),
                ])
            }
            Row::Story {
                title,
                now,
                later,
                note,
                ..
            } => {
                let mut tail = match (now, later) {
                    (0, _) => String::new(),
                    (_, 0) => " · last steps".to_string(),
                    _ => format!(" · {later} later"),
                };
                if let Some(n) = note {
                    tail.push_str(&format!(" · {n}"));
                }
                let tail_s = format!("{tail} ");
                let title_room = width.saturating_sub(12 + cells(&tail_s));
                let style = if *now == 0 {
                    Style::default().fg(Color::DarkGray)
                } else {
                    Style::default().fg(Color::Gray)
                };
                Line::from(vec![
                    Span::raw(cursor),
                    Span::styled("▔▔ so that ", style),
                    Span::styled(
                        pad(&fit(title, title_room), title_room),
                        if selected { style.bold() } else { style },
                    ),
                    Span::styled(tail_s, Style::default().fg(Color::DarkGray)),
                ])
            }
            Row::Node {
                depth,
                title,
                state,
                kind,
                age_secs,
                ..
            } => {
                let (mark, style) = match kind {
                    NodeKind::Free => ("☐", Style::default()),
                    NodeKind::Doing => ("▶", Style::default().fg(Color::Yellow)),
                    NodeKind::Blocked => ("▽", Style::default().fg(Color::DarkGray)),
                    NodeKind::Waiting => ("✉", Style::default().fg(Color::Cyan)),
                    NodeKind::Someday => ("💤", Style::default().fg(Color::DarkGray)),
                    NodeKind::Closed => ("✓", Style::default().fg(Color::DarkGray)),
                };
                let indent = "  ".repeat(*depth);
                let age = format!(" {:>4} ", humanize::duration(*age_secs));
                let state_s = if state.is_empty() {
                    String::new()
                } else {
                    format!(" {state} ")
                };
                let mark_s = format!("{mark} ");
                let fixed = 2 + cells(&indent) + cells(&mark_s) + cells(&state_s) + cells(&age);
                let title_room = width.saturating_sub(fixed);
                let title_style = match kind {
                    NodeKind::Free | NodeKind::Doing => Style::default(),
                    _ => Style::default().fg(Color::DarkGray),
                };
                Line::from(vec![
                    Span::raw(cursor),
                    Span::raw(indent),
                    Span::styled(mark_s, style),
                    Span::styled(
                        pad(&fit(title, title_room), title_room),
                        if selected {
                            title_style.bold()
                        } else {
                            title_style
                        },
                    ),
                    Span::styled(state_s, style),
                    Span::styled(age, Style::default().fg(Color::DarkGray)),
                ])
            }
            Row::Person {
                name,
                count,
                due,
                oldest_secs,
            } => {
                let mut text = format!(
                    " {name}  · {count} · oldest {}",
                    humanize::duration(*oldest_secs)
                );
                if *due {
                    text.push_str(" · follow up due");
                }
                Line::from(Span::styled(
                    text,
                    if *due {
                        Style::default().fg(Color::Cyan).bold()
                    } else {
                        Style::default().fg(Color::Cyan)
                    },
                ))
            }
            Row::Wait(w) => {
                let mut tail = format!(" {}", humanize::duration(w.since_secs));
                if w.wait.chases > 0 {
                    tail.push_str(&format!(" · {}×", w.wait.chases));
                }
                if let Some(u) = w.wait.until {
                    tail.push_str(&format!(" · {}", if w.due_now { "due now" } else { "" }));
                    if !w.due_now {
                        tail.push_str(&format!("follow up {u}"));
                    }
                }
                tail.push(' ');
                let crumb = w
                    .root_title
                    .as_ref()
                    .map(|r| format!(" ‹ {r}"))
                    .unwrap_or_default();
                let title_room = width.saturating_sub(6 + cells(&tail));
                Line::from(vec![
                    Span::raw(cursor),
                    Span::styled("  ✉ ", Style::default().fg(Color::Cyan)),
                    Span::styled(
                        pad(
                            &fit(&format!("{}{crumb}", w.wait.title), title_room),
                            title_room,
                        ),
                        if selected {
                            Style::default().bold()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(
                        tail,
                        if w.due_now {
                            Style::default().fg(Color::Cyan).bold()
                        } else {
                            Style::default().fg(Color::DarkGray)
                        },
                    ),
                ])
            }
            Row::Nudge(n) => {
                let text = format!("⚑ {}", n.message);
                Line::from(vec![
                    Span::raw(cursor),
                    Span::styled(
                        fit(&text, width.saturating_sub(2)),
                        if selected {
                            Style::default().fg(Color::Magenta).bold()
                        } else {
                            Style::default().fg(Color::Magenta)
                        },
                    ),
                ])
            }
        };
        let line = if selected {
            line.style(Style::default().bg(SELECTED_BG))
        } else {
            line
        };
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
    if area.height == 0 {
        return;
    }
    let Some(t) = app.selected_task() else { return };
    let now = app.now.timestamp();
    let d = derive(&app.tree, t, app.now.date());
    let mut lines: Vec<Line> = Vec::new();

    let anc = app.tree.ancestors(&t.id);
    let path: Vec<String> = anc.iter().rev().map(|a| a.title.clone()).collect();
    let mut head = vec![Span::styled(
        format!(" {} ", t.id),
        Style::default().fg(Color::DarkGray),
    )];
    if !path.is_empty() {
        head.push(Span::styled(
            format!("{} › ", path.join(" › ")),
            Style::default().fg(Color::DarkGray),
        ));
    }
    head.push(Span::styled(t.title.clone(), Style::default().bold()));
    if is_agent_step(t) {
        head.push(Span::styled(
            "  🤖 Claude",
            Style::default().fg(Color::Magenta),
        ));
    }
    lines.push(Line::from(head));

    let mut facts = vec![
        format!("{}", t.status),
        format!("{}", t.context),
        format!("queued {}", humanize::since(t.created, now)),
        format!("touched {}", humanize::since(t.updated, now)),
    ];
    if let Some(s) = t.started {
        facts.push(format!("doing for {}", humanize::since(s, now)));
    }
    if let Some(due) = t.due {
        facts.push(format!("due {due}"));
    }
    if let Some(p) = t.priority {
        facts.push(format!("priority {p}"));
    }
    if d.blocked_by > 0 {
        facts.push(format!("▽ blocked by {}", d.blocked_by));
    }
    if let Some(at) = d.resurfaced_at {
        facts.push(format!("↑ ready since {}", humanize::since(at, now)));
    }
    if let Some(ws) = &d.waiting_on_summary {
        facts.push(format!(
            "✉ {} via '{}' · {}",
            ws.person,
            ws.title,
            humanize::since(ws.since, now)
        ));
    }
    if !t.chases.is_empty() {
        facts.push(format!("chased {}×", t.chases.len()));
    }
    if let Some(w) = &t.waiting_on {
        facts.push(format!(
            "✉ {w}{}",
            t.until
                .map(|u| format!(" · follow up {u}"))
                .unwrap_or_default()
        ));
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", facts.join(" · ")),
        Style::default().fg(Color::Gray),
    )));

    let kids: Vec<_> = app.tree.children(&t.id).collect();
    if !kids.is_empty() {
        let mut spans = vec![Span::styled(
            " first: ",
            Style::default().fg(Color::DarkGray),
        )];
        for k in kids {
            let (mark, style) = match k.status {
                Status::Done => ("✓", Style::default().fg(Color::Green)),
                Status::Dropped => ("✗", Style::default().fg(Color::DarkGray)),
                Status::Doing => ("▶", Style::default().fg(Color::Yellow)),
                Status::Waiting => ("✉", Style::default().fg(Color::Cyan)),
                Status::Someday => ("💤", Style::default().fg(Color::DarkGray)),
                Status::Todo => ("☐", Style::default()),
            };
            spans.push(Span::styled(format!("{mark} {}  ", k.title), style));
        }
        lines.push(Line::from(spans));
    }

    let body = t.body.trim();
    if !body.is_empty() {
        lines.push(Line::from(""));
        for l in body
            .lines()
            .take(area.height.saturating_sub(lines.len() as u16 + 1) as usize)
        {
            lines.push(Line::from(format!(" {l}")));
        }
    }

    let block = Block::new()
        .borders(Borders::TOP)
        .border_style(Style::default().fg(Color::DarkGray));
    f.render_widget(
        Paragraph::new(lines)
            .block(block)
            .wrap(Wrap { trim: false }),
        area,
    );
}

fn draw_footer(f: &mut Frame, app: &App, area: Rect) {
    let keys = match app.view {
        View::Next => {
            " 1-5 views · j/k · l later · d done · s start · p put back · i first I need to… · w wait · f chase · a add · e edit · ? · q"
        }
        View::Outline => {
            " 1-5 views · j/k · ⇧arrows move · d done · s start · w wait · z someday · a/A add · i first · h completed · ? · q"
        }
        View::Waiting => {
            " 1-5 views · j/k · f chase · u unblock (they replied) · d done · e edit · ? · q"
        }
        View::Someday => {
            " 1-5 views · j/k · u revive · a park an idea · A add under · d done · x drop · ? · q"
        }
        View::Nudges => {
            " 1-5 views · j/k · Enter open the task · d done · s start · f chase · r refresh · ? · q"
        }
    };
    f.render_widget(
        Paragraph::new(Line::from(Span::styled(
            keys,
            Style::default().fg(Color::DarkGray),
        ))),
        area,
    );
}

fn draw_help(f: &mut Frame, area: Rect) {
    let text = vec![
        Line::from(Span::styled(" taskstaka — keys ", Style::default().bold())),
        Line::from(""),
        Line::from(" 1 Next  2 Outline  3 Waiting on  4 Someday  5 Nudges     v cycles"),
        Line::from(
            " j / k, ↑ / ↓     move        g / G   first / last     l   fold / unfold later",
        ),
        Line::from(" d                done        x       drop             o   reopen"),
        Line::from(" s                start (takes it in hand)   p   put back (empties the hand)"),
        Line::from(" i                first I need to… — a prerequisite under the selection;"),
        Line::from(
            "                  from the task in hand it goes in hand and pauses the one above",
        ),
        Line::from(" w                wait on someone (append ' @ +3d' for a follow-up date)"),
        Line::from(
            " f                chase them    u   unblock / revive — it is ready, you are on it",
        ),
        Line::from(" z                someday      t       touch, with an optional note"),
        Line::from(
            " a                add task (on Someday: park an idea)    A   add under selection",
        ),
        Line::from(" e / Enter        open the note in $EDITOR"),
        Line::from(" ⇧↑ / ⇧↓          move among siblings · ⇧→ nest under row above · ⇧← out"),
        Line::from(" h                show / hide completed (outline)    c   cycle context"),
        Line::from(" r                refresh     q       quit"),
        Line::from(""),
        Line::from(Span::styled(
            " Nesting means 'first this'; siblings are independent. Next shows the task in",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            " hand, then per story what you can do now; '… N later' is what is still required.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let h = (text.len() as u16 + 2).min(area.height);
    let w = 86.min(area.width);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(text).block(Block::bordered()), popup);
}

/// Display width, so emoji (🤖, 💤) count as the two cells they take.
fn cells(s: &str) -> usize {
    Span::raw(s).width()
}

fn fit(s: &str, max: usize) -> String {
    if cells(s) <= max {
        return s.to_string();
    }
    if max == 0 {
        return String::new();
    }
    let mut t = String::new();
    for c in s.chars() {
        if cells(&t) + cells(&c.to_string()) > max - 1 {
            break;
        }
        t.push(c);
    }
    t.push('…');
    t
}

fn pad(s: &str, width: usize) -> String {
    let n = cells(s);
    if n >= width {
        s.to_string()
    } else {
        format!("{s}{}", " ".repeat(width - n))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers};

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().to_string()
    }

    fn press(app: &mut App, c: char) {
        app.handle_key(KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE));
    }

    #[test]
    fn next_plate_and_stacks() {
        let (tree, now) = super::super::app::tests::fixture();
        let mut app = App::new(tree, None, now);
        insta::assert_snapshot!("panel_next", render(&app, 100, 24));
        press(&mut app, 'l');
        press(&mut app, 'j');
        insta::assert_snapshot!("panel_next_expanded", render(&app, 100, 26));
    }

    #[test]
    fn other_views() {
        let (tree, now) = super::super::app::tests::fixture();
        let mut app = App::new(tree, None, now);
        press(&mut app, '2');
        insta::assert_snapshot!("panel_outline", render(&app, 100, 20));
        press(&mut app, '3');
        insta::assert_snapshot!("panel_waiting", render(&app, 100, 14));
        press(&mut app, '4');
        insta::assert_snapshot!("panel_someday", render(&app, 100, 14));
        app.mode = Mode::Help;
        insta::assert_snapshot!("panel_help", render(&app, 100, 28));
    }

    #[test]
    fn empty_and_all_blocked_states() {
        let now: jiff::Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        let app = App::new(taskstaka_core::Tree::new(), None, now.clone());
        let out = render(&app, 80, 8);
        assert!(out.contains("Nothing open"), "{out}");

        let mut t = taskstaka_core::Task::new(
            "stry00".parse().unwrap(),
            "Story",
            taskstaka_core::Context::Work,
            now.timestamp(),
        );
        t.status = Status::Waiting;
        t.waiting_on = Some("Bob".into());
        let app = App::new(taskstaka_core::Tree::from_tasks([t]), None, now);
        let out = render(&app, 80, 8);
        assert!(out.contains("chase someone"), "{out}");
        assert!(
            out.contains("so that Story"),
            "the stuck story still shows: {out}"
        );
    }

    #[test]
    fn note_lines_strip_journal_stamps() {
        let body = "- 2026-09-29 14:00 [user] draft: hi team\n\n- 2026-09-29 15:00 [agent] found the template\nplain line\n";
        assert_eq!(
            note_lines(body),
            ["draft: hi team", "found the template", "plain line"]
        );
    }
}
