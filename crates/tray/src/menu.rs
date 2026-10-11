//! What the menu bar shows, as data. Pure so it can be tested without a
//! display; `main.rs` turns a `MenuSpec` into muda items.

use taskstaka_core::action::StoryPlan;
use taskstaka_core::view::PlateView;

/// The task on the plate (CONTEXT: The plate).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Plate {
    pub id: String,
    pub title: String,
    pub in_hand: bool,
    /// The note body, so the menu shows what to do at a glance.
    pub notes: String,
}

/// A row you can do now, with the story it is for when that is not itself.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: String,
    pub title: String,
    pub story: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Model {
    Offline,
    Online {
        plate: Option<Plate>,
        rows: Vec<Row>,
    },
}

impl Model {
    /// From the wire: the plate and the per-story plan (`now` rows only).
    pub fn online(plate: Option<PlateView>, stories: &[StoryPlan]) -> Model {
        let plate = plate.map(|p| Plate {
            id: p.task.task.id.to_string(),
            title: p.task.task.title.clone(),
            in_hand: p.in_hand,
            notes: p.task.body.clone(),
        });
        let rows = stories
            .iter()
            .flat_map(|s| {
                s.now.iter().map(move |r| Row {
                    id: r.id.to_string(),
                    title: r.title.clone(),
                    story: (r.id != s.id).then(|| s.title.clone()),
                })
            })
            .collect();
        Model::Online { plate, rows }
    }
}

/// The bar title and the menu, top to bottom.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuSpec {
    pub title: String,
    pub items: Vec<Item>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Item {
    /// Greyed, not clickable.
    Label(String),
    /// `id` is what the click reports: `done:<id>`, `putback:<id>`,
    /// `start:<id>`, `task:<id>`, `quick`, `open`, `quit`.
    Action {
        id: String,
        text: String,
    },
    Separator,
}

/// How many characters of title fit in the bar before it starts crowding
/// the other status items.
pub const BAR_TITLE_CHARS: usize = 40;
/// How many *now* rows the menu lists before summarising the rest.
pub const MAX_ROWS: usize = 10;
/// Notes wrap at this many characters per menu line. Nothing is cut: the
/// menu grows with the note, so everything to do is in view.
pub const NOTE_WRAP: usize = 72;

pub fn build(model: &Model, hotkey_label: &str) -> MenuSpec {
    let mut items = Vec::new();
    let title = match model {
        Model::Offline => {
            items.push(Item::Label("daemon not running".into()));
            "taskstaka · offline".to_string()
        }
        Model::Online { plate, rows } => {
            let title = match plate {
                Some(p) => {
                    let (word, mark) = if p.in_hand {
                        ("In hand", "▶")
                    } else {
                        ("Up next", "○")
                    };
                    items.push(Item::Label(format!("{word}: {}", p.title)));
                    // Title | notes | actions, each behind a rule.
                    items.push(Item::Separator);
                    for line in note_lines(&p.notes) {
                        items.push(Item::Label(line));
                    }
                    items.push(Item::Separator);
                    items.push(action(format!("done:{}", p.id), "Done"));
                    if p.in_hand {
                        items.push(action(format!("putback:{}", p.id), "Put back"));
                    } else {
                        items.push(action(format!("start:{}", p.id), "Start"));
                    }
                    format!("{mark} {}", truncate(&p.title, BAR_TITLE_CHARS))
                }
                None => {
                    items.push(Item::Label("nothing ready".into()));
                    "taskstaka".to_string()
                }
            };
            if !rows.is_empty() {
                items.push(Item::Separator);
                for r in rows.iter().take(MAX_ROWS) {
                    let text = match &r.story {
                        Some(s) => format!("{s} › {}", r.title),
                        None => r.title.clone(),
                    };
                    items.push(action(format!("task:{}", r.id), text));
                }
                if rows.len() > MAX_ROWS {
                    items.push(Item::Label(format!("… {} more", rows.len() - MAX_ROWS)));
                }
            }
            title
        }
    };
    items.push(Item::Separator);
    items.push(action("quick", format!("Quick add…   {hotkey_label}")));
    items.push(action("open", "Open taskstaka"));
    items.push(Item::Separator);
    items.push(action("quit", "Quit"));
    MenuSpec { title, items }
}

/// The note body as menu lines: blank lines dropped, heading marks stripped,
/// list markers turned into bullets, wrapped at `NOTE_WRAP`. Empty notes say so.
pub fn note_lines(body: &str) -> Vec<String> {
    let mut out = Vec::new();
    for raw in body.lines() {
        let line = raw.trim();
        if line.is_empty() {
            continue;
        }
        let line = line.trim_start_matches('#').trim_start();
        let line = ["- [ ] ", "- [x] ", "- [X] ", "- ", "* ", "+ "]
            .iter()
            .find_map(|m| line.strip_prefix(m))
            .map(|rest| format!("• {rest}"))
            .unwrap_or_else(|| line.to_string());
        out.extend(wrap(&line, NOTE_WRAP));
    }
    if out.is_empty() {
        out.push("no notes yet".to_string());
    }
    out
}

/// Greedy word wrap by character count; a single long word gets its own line.
fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut cur = String::new();
    for word in text.split_whitespace() {
        let fits = cur.chars().count() + 1 + word.chars().count() <= width;
        if cur.is_empty() {
            cur.push_str(word);
        } else if fits {
            cur.push(' ');
            cur.push_str(word);
        } else {
            lines.push(std::mem::take(&mut cur));
            cur.push_str(word);
        }
    }
    if !cur.is_empty() {
        lines.push(cur);
    }
    lines
}

fn action(id: impl Into<String>, text: impl Into<String>) -> Item {
    Item::Action {
        id: id.into(),
        text: text.into(),
    }
}

/// Cut to `max` characters, the last one an ellipsis when something was cut.
pub fn truncate(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        return s.to_string();
    }
    let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
    out.push('…');
    out
}
#[cfg(test)]
mod tests {
    use super::*;

    fn plate(id: &str, title: &str, in_hand: bool) -> Plate {
        Plate {
            id: id.into(),
            title: title.into(),
            in_hand,
            notes: String::new(),
        }
    }

    #[test]
    fn notes_come_before_the_actions_so_you_see_what_to_do() {
        let mut p = plate("abc234", "Renew the cert", true);
        p.notes = "Run certbot on web-1.\nThen restart nginx.".into();
        let t = texts(&build(
            &Model::Online {
                plate: Some(p),
                rows: vec![],
            },
            "⌥Space",
        ));
        assert_eq!(t[0], "[In hand: Renew the cert]");
        assert_eq!(t[1], "---", "a rule between the title and the notes");
        assert_eq!(t[2], "[Run certbot on web-1.]");
        assert_eq!(t[3], "[Then restart nginx.]");
        assert_eq!(t[4], "---", "and one before the actions");
        assert!(t[5].starts_with("done:"), "{t:?}");
    }

    #[test]
    fn long_note_lines_wrap_and_nothing_is_cut() {
        let mut p = plate("abc234", "T", false);
        let long = "word ".repeat(30).trim_end().to_string(); // 149 chars
        p.notes = format!(
            "{long}\n{}",
            (1..=10)
                .map(|i| format!("line {i}"))
                .collect::<Vec<_>>()
                .join("\n")
        );
        let t = texts(&build(
            &Model::Online {
                plate: Some(p),
                rows: vec![],
            },
            "⌥Space",
        ));
        let notes: Vec<&String> = t[2..].iter().take_while(|x| x.starts_with('[')).collect();
        assert!(
            notes.iter().all(|l| l.chars().count() <= NOTE_WRAP + 2),
            "{notes:?}"
        );
        // 149 chars wrap to 3 lines at 72; then the ten short lines, all kept.
        assert_eq!(notes.len(), 3 + 10, "{notes:?}");
        assert_eq!(notes.last().unwrap().as_str(), "[line 10]");
    }

    #[test]
    fn empty_notes_say_so_and_markdown_bullets_are_kept_readable() {
        let p = plate("abc234", "T", true);
        let t = texts(&build(
            &Model::Online {
                plate: Some(p),
                rows: vec![],
            },
            "⌥Space",
        ));
        assert_eq!(t[2], "[no notes yet]");
        let mut p = plate("abc234", "T", true);
        p.notes = "- [ ] call Priya\n\n## Context\nsome text".into();
        let t = texts(&build(
            &Model::Online {
                plate: Some(p),
                rows: vec![],
            },
            "⌥Space",
        ));
        assert_eq!(t[2], "[• call Priya]");
        assert_eq!(
            t[3], "[Context]",
            "blank lines dropped, heading marks stripped"
        );
        assert_eq!(t[4], "[some text]");
    }

    fn row(id: &str, title: &str, story: Option<&str>) -> Row {
        Row {
            id: id.into(),
            title: title.into(),
            story: story.map(Into::into),
        }
    }

    fn texts(spec: &MenuSpec) -> Vec<String> {
        spec.items
            .iter()
            .map(|i| match i {
                Item::Label(t) => format!("[{t}]"),
                Item::Action { id, text } => format!("{id}: {text}"),
                Item::Separator => "---".into(),
            })
            .collect()
    }

    #[test]
    fn offline_says_so_and_still_offers_the_web_and_quit() {
        let spec = build(&Model::Offline, "⌥Space");
        assert_eq!(spec.title, "taskstaka · offline");
        let t = texts(&spec);
        assert!(t.contains(&"[daemon not running]".to_string()), "{t:?}");
        assert!(t.contains(&"open: Open taskstaka".to_string()));
        assert!(t.contains(&"quit: Quit".to_string()));
        assert!(!t.iter().any(|x| x.starts_with("done:")));
    }

    #[test]
    fn nothing_ready_is_calm() {
        let spec = build(
            &Model::Online {
                plate: None,
                rows: vec![],
            },
            "⌥Space",
        );
        assert_eq!(spec.title, "taskstaka");
        assert!(texts(&spec).contains(&"[nothing ready]".to_string()));
    }

    #[test]
    fn in_hand_leads_with_the_task_and_offers_done_and_put_back() {
        let m = Model::Online {
            plate: Some(plate("abc234", "Renew the cert", true)),
            rows: vec![row("abc234", "Renew the cert", None)],
        };
        let spec = build(&m, "⌥Space");
        assert_eq!(spec.title, "▶ Renew the cert");
        let t = texts(&spec);
        assert_eq!(t[0], "[In hand: Renew the cert]");
        assert!(t.contains(&"done:abc234: Done".to_string()));
        assert!(t.contains(&"putback:abc234: Put back".to_string()));
        assert!(!t.iter().any(|x| x.starts_with("start:")));
    }

    #[test]
    fn up_next_offers_start_instead_of_put_back() {
        let m = Model::Online {
            plate: Some(plate("abc234", "Renew the cert", false)),
            rows: vec![],
        };
        let spec = build(&m, "⌥Space");
        assert_eq!(spec.title, "○ Renew the cert");
        let t = texts(&spec);
        assert_eq!(t[0], "[Up next: Renew the cert]");
        assert!(t.contains(&"start:abc234: Start".to_string()));
        assert!(t.contains(&"done:abc234: Done".to_string()));
        assert!(!t.iter().any(|x| x.starts_with("putback:")));
    }

    #[test]
    fn now_rows_show_their_story_and_open_the_task() {
        let m = Model::Online {
            plate: None,
            rows: vec![
                row("aaa234", "Ask B about TTL", Some("Fix DNS")),
                row("bbb234", "Write the memo", None),
            ],
        };
        let t = texts(&build(&m, "⌥Space"));
        assert!(
            t.contains(&"task:aaa234: Fix DNS › Ask B about TTL".to_string()),
            "{t:?}"
        );
        assert!(t.contains(&"task:bbb234: Write the memo".to_string()));
    }

    #[test]
    fn at_most_ten_rows_then_a_count() {
        let rows: Vec<Row> = (0..14)
            .map(|i| row(&format!("r{i:05}"), &format!("Row {i}"), None))
            .collect();
        let t = texts(&build(&Model::Online { plate: None, rows }, "⌥Space"));
        assert_eq!(t.iter().filter(|x| x.starts_with("task:")).count(), 10);
        assert!(t.contains(&"[… 4 more]".to_string()), "{t:?}");
    }

    #[test]
    fn quick_add_names_the_hotkey() {
        let t = texts(&build(&Model::Offline, "⌃⌥Space"));
        assert!(
            t.contains(&"quick: Quick add…   ⌃⌥Space".to_string()),
            "{t:?}"
        );
    }

    #[test]
    fn long_titles_are_cut_for_the_bar_but_not_the_menu() {
        let long = "A very long task title that would eat the whole menu bar if shown";
        let m = Model::Online {
            plate: Some(plate("abc234", long, true)),
            rows: vec![],
        };
        let spec = build(&m, "⌥Space");
        assert_eq!(spec.title, "▶ A very long task title that would eat t…");
        assert_eq!(spec.title.chars().count(), 2 + 40);
        assert_eq!(texts(&spec)[0], format!("[In hand: {long}]"));
    }

    #[test]
    fn model_is_built_from_the_wire_types() {
        let plate: Option<taskstaka_core::view::PlateView> = serde_json::from_str(
            r#"{"in_hand":true,"task":{"id":"abc234","title":"Renew the cert","status":"doing","context":"work","created":"2026-09-29T15:00:00Z","updated":"2026-09-29T15:00:00Z","body":"do it carefully","derived":{"blocked_by":0,"deferred_by":null,"followup":false,"actionable":true,"resurfaced_at":null,"orphaned":false,"waiting_on_summary":null},"children":[],"ancestry":[]}}"#,
        )
        .unwrap();
        let stories: Vec<taskstaka_core::action::StoryPlan> = serde_json::from_str(
            r#"[{"id":"sss234","title":"Fix DNS","context":"work","now":[{"id":"abc234","title":"Renew the cert","status":"doing","context":"work","tier":"normal","reason":"","ancestry":[{"id":"sss234","title":"Fix DNS"}],"created":"2026-09-29T15:00:00Z","updated":"2026-09-29T15:00:00Z","started":null,"due":null,"queued_secs":0,"untouched_secs":0}],"later":[],"later_agent":0,"waiting_on":[],"stuck":null},
               {"id":"ttt234","title":"Write the memo","context":"work","now":[{"id":"ttt234","title":"Write the memo","status":"todo","context":"work","tier":"normal","reason":"","ancestry":[],"created":"2026-09-29T15:00:00Z","updated":"2026-09-29T15:00:00Z","started":null,"due":null,"queued_secs":0,"untouched_secs":0}],"later":[],"later_agent":0,"waiting_on":[],"stuck":null}]"#,
        )
        .unwrap();
        let Model::Online { plate, rows } = Model::online(plate, &stories) else {
            panic!("online");
        };
        let p = plate.unwrap();
        assert_eq!((p.id.as_str(), p.in_hand), ("abc234", true));
        assert_eq!(p.notes, "do it carefully");
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].story.as_deref(), Some("Fix DNS"));
        assert_eq!(rows[1].story, None, "a one-task story is just itself");
    }
}
