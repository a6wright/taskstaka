//! Rendering. Pure function of `App`; snapshot-tested with `TestBackend`.

use ratatui::Frame;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Clear, Paragraph, Wrap};
use taskstaka_core::action::derive;
use taskstaka_core::{Status, Tier, humanize};

use super::app::{App, MessageKind, Mode, Row};

pub fn draw(f: &mut Frame, app: &App) {
    let area = f.area();
    let [header, list, detail, footer] = Layout::vertical([
        Constraint::Length(2),
        Constraint::Min(5),
        Constraint::Length(detail_height(app, area)),
        Constraint::Length(1),
    ])
    .areas(area);

    draw_header(f, app, header);
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

fn detail_height(app: &App, area: Rect) -> u16 {
    if app.selected_task().is_none() {
        return 0;
    }
    // Enough for header lines plus a few body lines; shrink on tiny terminals.
    (area.height / 3).clamp(4, 10)
}

fn draw_header(f: &mut Frame, app: &App, area: Rect) {
    let ctx = match app.ctx {
        None => "all".to_string(),
        Some(c) => c.to_string(),
    };
    let s = &app.summary;
    let mut spans = vec![
        Span::styled(" taskstaka ", Style::default().bold().fg(Color::Cyan)),
        Span::styled(format!("· {ctx} · "), Style::default().fg(Color::DarkGray)),
    ];
    if !app.connected {
        spans.push(Span::styled(
            " DISCONNECTED ",
            Style::default().fg(Color::Black).bg(Color::Red).bold(),
        ));
    }
    let status = if s.open == 0 {
        Span::styled("nothing open", Style::default().fg(Color::DarkGray))
    } else if s.actionable == 0 {
        Span::styled(
            format!(
                "nothing actionable · {} blocked · {} waiting — go unblock something",
                s.blocked, s.waiting
            ),
            Style::default().fg(Color::Yellow).bold(),
        )
    } else {
        Span::styled(
            format!(
                "{} actionable · {} doing · {} blocked · {} waiting · {} someday",
                s.actionable, s.doing, s.blocked, s.waiting, s.someday
            ),
            Style::default().fg(Color::Gray),
        )
    };
    spans.push(status);
    let line1 = Line::from(spans);

    let waiting = taskstaka_core::action::waiting_on(&app.tree, &app.now, app.ctx);
    let line2 = if let Some((_, msg)) = app
        .message
        .as_ref()
        .filter(|(k, _)| *k == MessageKind::Error)
    {
        Line::from(Span::styled(
            format!(" {msg}"),
            Style::default().fg(Color::Red).bold(),
        ))
    } else if let Some(g) = waiting.first() {
        let first = &g.items[0];
        let more = waiting.len() - 1;
        let chased = match first.wait.chases {
            0 => String::new(),
            n => format!(" · {n}×"),
        };
        Line::from(Span::styled(
            format!(
                " ⌛ waiting on {} · {} · {}{chased}{}",
                g.person,
                first.wait.title,
                taskstaka_core::humanize::duration(first.since_secs),
                if more > 0 {
                    format!("  (+{more} more · b)")
                } else {
                    String::new()
                }
            ),
            Style::default().fg(Color::Cyan),
        ))
    } else if let Some(n) = app.nudges.first() {
        Line::from(Span::styled(
            format!(" ⚑ {n}"),
            Style::default().fg(Color::Magenta),
        ))
    } else if let Some((kind, msg)) = &app.message {
        let style = match kind {
            MessageKind::Info => Style::default().fg(Color::Gray),
            MessageKind::Success => Style::default().fg(Color::Green),
            MessageKind::Error => Style::default().fg(Color::Red).bold(),
        };
        Line::from(Span::styled(format!(" {msg}"), style))
    } else {
        Line::from("")
    };
    f.render_widget(Paragraph::new(vec![line1, line2]), area);
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
        Tier::FollowUp => "follow-up",
        Tier::DueSoon => "due soon",
        Tier::High => "high",
        Tier::Normal => "",
        Tier::Low => "low",
    }
}

fn draw_list(f: &mut Frame, app: &App, area: Rect) {
    if app.rows.is_empty() {
        let msg = if app.summary.open == 0 {
            "Nothing open. Press a to add a task."
        } else {
            "Nothing actionable. Press v for the tree, or b for what is waiting or blocked."
        };
        f.render_widget(
            Paragraph::new(Line::from(Span::styled(
                format!("  {msg}"),
                Style::default().fg(Color::DarkGray),
            ))),
            area,
        );
        return;
    }

    let width = area.width as usize;
    // Keep the selection visible.
    let visible = area.height as usize;
    let start = if app.selected >= visible {
        app.selected + 1 - visible
    } else {
        0
    };

    let mut lines = Vec::with_capacity(visible);
    for (i, row) in app.rows.iter().enumerate().skip(start).take(visible) {
        let selected = i == app.selected;
        let line = match row {
            Row::Section(title) => Line::from(Span::styled(
                format!(" ── {title} "),
                Style::default()
                    .fg(Color::DarkGray)
                    .add_modifier(Modifier::ITALIC),
            )),
            Row::Item(it) => {
                let badge = tier_badge(it.tier);
                let mut crumb = match it.ancestry.first() {
                    Some(root) => format!(" ‹ {}", root.title),
                    None => String::new(),
                };
                if let Some(due) = it.due
                    && matches!(it.tier, Tier::Overdue | Tier::DueSoon)
                {
                    crumb.push_str(&format!(" · due {due}"));
                }
                let ages = format!(
                    " {:>4} · {:>4} ",
                    humanize::duration(it.queued_secs),
                    humanize::duration(it.untouched_secs)
                );
                let fixed = 2 + 10 + ages.chars().count();
                let title_room = width.saturating_sub(fixed);
                let title = fit(&format!("{}{}", it.title, crumb), title_room);
                let style = tier_style(it.tier);
                Line::from(vec![
                    Span::raw(if selected { "› " } else { "  " }),
                    Span::styled(format!("{badge:<9} "), style),
                    Span::styled(
                        pad(&title, title_room),
                        if selected {
                            Style::default().bold()
                        } else {
                            Style::default()
                        },
                    ),
                    Span::styled(ages, Style::default().fg(Color::DarkGray)),
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
                use super::app::NodeKind;
                let (mark, style) = match kind {
                    NodeKind::Free => ("›", Style::default()),
                    NodeKind::Doing => ("▶", Style::default().fg(Color::Yellow)),
                    NodeKind::Blocked => ("⏳", Style::default().fg(Color::DarkGray)),
                    NodeKind::Waiting => ("⌛", Style::default().fg(Color::Cyan)),
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
                let fixed =
                    2 + indent.chars().count() + 2 + state_s.chars().count() + age.chars().count();
                let title_room = width.saturating_sub(fixed);
                let title_style = match kind {
                    NodeKind::Free | NodeKind::Doing => Style::default(),
                    _ => Style::default().fg(Color::DarkGray),
                };
                Line::from(vec![
                    Span::raw(if selected { "› " } else { "  " }),
                    Span::raw(indent),
                    Span::styled(format!("{mark} "), style),
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
            Row::Stuck {
                title, why, root, ..
            } => {
                let crumb = root.as_ref().map(|r| format!(" ‹ {r}")).unwrap_or_default();
                let why_s = format!(" {why} ");
                let title_room = width.saturating_sub(2 + 10 + why_s.chars().count());
                Line::from(vec![
                    Span::raw(if selected { "› " } else { "  " }),
                    Span::styled(format!("{:<9} ", "·"), Style::default().fg(Color::DarkGray)),
                    Span::styled(
                        pad(&fit(&format!("{title}{crumb}"), title_room), title_room),
                        Style::default().fg(Color::DarkGray),
                    ),
                    Span::styled(why_s, Style::default().fg(Color::DarkGray).italic()),
                ])
            }
        };
        let line = if selected {
            line.style(Style::default().bg(Color::Rgb(40, 40, 50)))
        } else {
            line
        };
        lines.push(line);
    }
    f.render_widget(Paragraph::new(lines), area);
}

fn draw_detail(f: &mut Frame, app: &App, area: Rect) {
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
        facts.push(format!("blocked by {}", d.blocked_by));
    }
    if let Some(at) = d.resurfaced_at {
        facts.push(format!("resurfaced {} ago", humanize::since(at, now)));
    }
    if let Some(ws) = &d.waiting_on_summary {
        facts.push(format!(
            "waiting on {} via '{}' since {}",
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
            "waiting on {w}{}",
            t.until.map(|u| format!(" until {u}")).unwrap_or_default()
        ));
    }
    lines.push(Line::from(Span::styled(
        format!(" {}", facts.join(" · ")),
        Style::default().fg(Color::Gray),
    )));

    let kids: Vec<_> = app.tree.children(&t.id).collect();
    if !kids.is_empty() {
        let mut spans = vec![Span::styled(
            " subtasks: ",
            Style::default().fg(Color::DarkGray),
        )];
        for k in kids {
            let (mark, style) = match k.status {
                Status::Done => ("✓", Style::default().fg(Color::Green)),
                Status::Dropped => ("✗", Style::default().fg(Color::DarkGray)),
                Status::Doing => ("▶", Style::default().fg(Color::Yellow)),
                Status::Waiting => ("⌛", Style::default().fg(Color::Cyan)),
                Status::Someday => ("💤", Style::default().fg(Color::DarkGray)),
                Status::Todo => ("·", Style::default()),
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
        .borders(ratatui::widgets::Borders::TOP)
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
        super::app::View::Tree => {
            " tree · v ranked · j/k · ⇧↑↓←→ move · d done · s start · w wait · f chase · a/A add · h completed · c ctx · ? · q"
        }
        super::app::View::Next if app.show_stuck => {
            " ranked · v tree · j/k · d done · s start · w wait · f chase · a/A add · e edit · b hide stuck · c ctx · ? · q"
        }
        super::app::View::Next => {
            " ranked · v tree · j/k · d done · s start · w wait · f chase · a/A add · e edit · b stuck · c ctx · ? · q"
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
        Line::from(" j / k, ↑ / ↓     move        g / G   first / last"),
        Line::from(" d                done        x       drop"),
        Line::from(" s                start       o       reopen (back to todo)"),
        Line::from(" w                waiting on … (append ' @ +3d' for a follow-up)"),
        Line::from(" z                someday     t       touch, with an optional note"),
        Line::from(" a                add task    A       add subtask under selection"),
        Line::from(" e / Enter        open the note in $EDITOR"),
        Line::from(" b                show / hide waiting & blocked tasks"),
        Line::from(" c                cycle context: all → work → personal"),
        Line::from(" ⇧↑ / ⇧↓          move among siblings · ⇧→ nest under row above · ⇧← out"),
        Line::from(" h                show / hide completed (tree view)"),
        Line::from(" r                refresh     q       quit"),
        Line::from(""),
        Line::from(Span::styled(
            " Rows are what needs action, best first. A ↑ ready row is a task whose",
            Style::default().fg(Color::Gray),
        )),
        Line::from(Span::styled(
            " subtasks all closed and that you have not touched since.",
            Style::default().fg(Color::Gray),
        )),
        Line::from(""),
        Line::from(Span::styled(
            " any key to close",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let h = (text.len() as u16 + 2).min(area.height);
    let w = 76.min(area.width);
    let popup = Rect {
        x: area.x + (area.width.saturating_sub(w)) / 2,
        y: area.y + (area.height.saturating_sub(h)) / 2,
        width: w,
        height: h,
    };
    f.render_widget(Clear, popup);
    f.render_widget(Paragraph::new(text).block(Block::bordered()), popup);
}

fn fit(s: &str, max: usize) -> String {
    let n = s.chars().count();
    if n <= max {
        s.to_string()
    } else if max == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(max - 1).collect();
        t.push('…');
        t
    }
}

fn pad(s: &str, width: usize) -> String {
    let n = s.chars().count();
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

    fn render(app: &App, w: u16, h: u16) -> String {
        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
        term.draw(|f| draw(f, app)).unwrap();
        term.backend().to_string()
    }

    #[test]
    fn panel_snapshot() {
        let (tree, now) = super::super::app::tests::fixture();
        let mut app = App::new(tree, None, now);
        app.view = super::super::app::View::Next;
        app.rebuild();
        insta::assert_snapshot!(render(&app, 100, 18));
    }

    #[test]
    fn panel_with_stuck_and_help() {
        let (tree, now) = super::super::app::tests::fixture();
        let mut app = App::new(tree, None, now);
        app.view = super::super::app::View::Next;
        app.rebuild();
        app.show_stuck = true;
        app.rebuild();
        insta::assert_snapshot!("panel_stuck", render(&app, 100, 22));
        app.mode = Mode::Help;
        insta::assert_snapshot!("panel_help", render(&app, 100, 24));
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
        assert!(out.contains("go unblock"), "{out}");
    }
}

#[cfg(test)]
mod tree_view_snapshot {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    #[test]
    fn tree_view_snapshot() {
        let (tree, now) = super::super::app::tests::fixture();
        let app = App::new(tree, None, now);
        let mut term = Terminal::new(TestBackend::new(100, 20)).unwrap();
        term.draw(|f| draw(f, &app)).unwrap();
        insta::assert_snapshot!("panel_tree", term.backend().to_string());
    }
}
