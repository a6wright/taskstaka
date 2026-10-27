//! Notes are Markdown (everyone writes it anyway). One parser, two
//! renderers: HTML for the web pane and styled lines for the TUI. The CLI
//! prints the source, which reads fine in a terminal.
//!
//! Journal lines (`- YYYY-MM-DD HH:MM [source] text`, see
//! `humanize::strip_journal_stamp`) are the task's activity, not its notes;
//! `notes_of` drops them so each surface can show them separately.

use pulldown_cmark::{CodeBlockKind, Event, Options, Parser, Tag, TagEnd};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use taskstaka_core::humanize::is_journal_line;

fn options() -> Options {
    Options::ENABLE_TABLES
        | Options::ENABLE_STRIKETHROUGH
        | Options::ENABLE_TASKLISTS
        | Options::ENABLE_SMART_PUNCTUATION
}

/// The body without its journal lines.
pub fn notes_of(body: &str) -> String {
    body.lines()
        .filter(|l| !is_journal_line(l))
        .collect::<Vec<_>>()
        .join("\n")
        .trim()
        .to_string()
}

/// Markdown → HTML for the web. Raw HTML in the source is shown as text, not
/// interpreted: notes come from the user, the agent and `quick`, and none of
/// them need script in a note. Links get `target=_blank` on the client side.
pub fn to_html(md: &str) -> String {
    let events = Parser::new_ext(md, options()).map(|e| match e {
        Event::Html(s) | Event::InlineHtml(s) => Event::Text(s),
        e => e,
    });
    let mut out = String::with_capacity(md.len() * 2);
    pulldown_cmark::html::push_html(&mut out, events);
    out
}

/// Markdown → styled lines for ratatui. Paragraphs become one `Line` each
/// (the widget wraps); lists get bullets or numbers, code and quotes an
/// indent, headings bold, links their text with the URL dimmed after it.
pub fn to_lines(md: &str) -> Vec<Line<'static>> {
    let mut r = Lines::default();
    for ev in Parser::new_ext(md, options()) {
        r.event(ev);
    }
    r.flush();
    while r.out.last().is_some_and(|l| l.width() == 0) {
        r.out.pop();
    }
    r.out
}

#[derive(Default)]
struct Lines {
    out: Vec<Line<'static>>,
    cur: Vec<Span<'static>>,
    /// Open styles, innermost last.
    styles: Vec<Style>,
    /// Open lists: `Some(n)` is an ordered list whose next item is `n`.
    lists: Vec<Option<u64>>,
    quote: usize,
    code_block: bool,
    /// A link's destination, shown after its text when they differ.
    link: Option<(String, String)>,
    cell: bool,
}

const CODE: Style = Style::new().fg(Color::Cyan);
const DIM: Style = Style::new().fg(Color::DarkGray);

impl Lines {
    fn style(&self) -> Style {
        self.styles
            .iter()
            .fold(Style::default(), |acc, s| acc.patch(*s))
    }

    fn prefix(&self) -> String {
        let mut p = "│ ".repeat(self.quote);
        if self.code_block {
            p.push_str("  ");
        } else if !self.lists.is_empty() {
            p.push_str(&"  ".repeat(self.lists.len()));
        }
        p
    }

    fn push(&mut self, text: impl Into<String>) {
        let style = self.style();
        self.push_styled(text, style);
    }

    fn push_styled(&mut self, text: impl Into<String>, style: Style) {
        let text = text.into();
        if text.is_empty() {
            return;
        }
        if self.cur.is_empty() {
            let p = self.prefix();
            if !p.is_empty() {
                self.cur.push(Span::styled(p, DIM));
            }
        }
        self.cur.push(Span::styled(text, style));
    }

    fn flush(&mut self) {
        if !self.cur.is_empty() {
            self.out.push(Line::from(std::mem::take(&mut self.cur)));
        }
    }

    /// A blank line before a block, unless we are at the top or just had one.
    fn gap(&mut self) {
        self.flush();
        if self.out.last().is_some_and(|l| l.width() > 0) {
            self.out.push(Line::from(""));
        }
    }

    fn event(&mut self, ev: Event<'_>) {
        match ev {
            Event::Start(tag) => self.start(tag),
            Event::End(tag) => self.end(tag),
            Event::Text(s) => {
                if self.code_block {
                    for l in s.lines() {
                        self.push_styled(l.to_string(), CODE);
                        self.flush();
                    }
                } else {
                    self.push(s.into_string());
                }
            }
            Event::Code(s) => self.push_styled(s.into_string(), CODE),
            Event::Html(s) | Event::InlineHtml(s) => self.push(s.trim_end().to_string()),
            Event::SoftBreak => self.push(" "),
            Event::HardBreak => self.flush(),
            Event::Rule => {
                self.gap();
                self.push_styled("───", DIM);
                self.flush();
            }
            Event::TaskListMarker(done) => self.push(if done { "☑ " } else { "☐ " }),
            Event::FootnoteReference(s) => self.push_styled(format!("[{s}]"), DIM),
            _ => {}
        }
    }

    fn start(&mut self, tag: Tag<'_>) {
        match tag {
            Tag::Paragraph => {
                // Inside a list item the first paragraph continues the bullet's line.
                if self.cur.is_empty() {
                    self.gap();
                }
            }
            Tag::Heading { .. } => {
                self.gap();
                self.styles.push(Style::new().add_modifier(Modifier::BOLD));
            }
            Tag::BlockQuote(_) => {
                self.gap();
                self.quote += 1;
            }
            Tag::CodeBlock(kind) => {
                self.gap();
                if let CodeBlockKind::Fenced(lang) = kind
                    && !lang.is_empty()
                {
                    self.push_styled(lang.into_string(), DIM);
                    self.flush();
                }
                self.code_block = true;
            }
            Tag::List(start) => {
                if self.lists.is_empty() {
                    self.gap();
                } else {
                    self.flush();
                }
                self.lists.push(start);
            }
            Tag::Item => {
                self.flush();
                let depth = self.lists.len();
                let marker = match self.lists.last_mut() {
                    Some(Some(n)) => {
                        let m = format!("{n}. ");
                        *n += 1;
                        m
                    }
                    _ => "• ".to_string(),
                };
                // The item's own indent is one level less than its content's.
                let p = format!("{}{}", "│ ".repeat(self.quote), "  ".repeat(depth - 1));
                if !p.is_empty() {
                    self.cur.push(Span::styled(p, DIM));
                }
                self.cur.push(Span::styled(marker, DIM));
            }
            Tag::Emphasis => self
                .styles
                .push(Style::new().add_modifier(Modifier::ITALIC)),
            Tag::Strong => self.styles.push(Style::new().add_modifier(Modifier::BOLD)),
            Tag::Strikethrough => self
                .styles
                .push(Style::new().add_modifier(Modifier::CROSSED_OUT)),
            Tag::Link { dest_url, .. } | Tag::Image { dest_url, .. } => {
                self.styles.push(
                    Style::new()
                        .fg(Color::Cyan)
                        .add_modifier(Modifier::UNDERLINED),
                );
                self.link = Some((dest_url.into_string(), String::new()));
            }
            Tag::Table(_) => self.gap(),
            Tag::TableHead | Tag::TableRow => self.flush(),
            Tag::TableCell => {
                if !self.cell {
                    self.cell = true;
                } else {
                    self.push_styled(" │ ", DIM);
                }
            }
            _ => {}
        }
    }

    fn end(&mut self, tag: TagEnd) {
        match tag {
            TagEnd::Paragraph | TagEnd::Item | TagEnd::TableRow => self.flush(),
            TagEnd::TableHead => {
                self.flush();
                self.cell = false;
                self.push_styled("───", DIM);
                self.flush();
            }
            TagEnd::Heading(_) => {
                self.styles.pop();
                self.flush();
            }
            TagEnd::Emphasis | TagEnd::Strong | TagEnd::Strikethrough => {
                self.styles.pop();
            }
            TagEnd::BlockQuote(_) => {
                self.flush();
                self.quote -= 1;
            }
            TagEnd::CodeBlock => {
                self.flush();
                self.code_block = false;
            }
            TagEnd::List(_) => {
                self.flush();
                self.lists.pop();
            }
            TagEnd::Link | TagEnd::Image => {
                self.styles.pop();
                if let Some((url, _)) = self.link.take() {
                    let text: String = self
                        .cur
                        .iter()
                        .map(|s| s.content.as_ref())
                        .collect::<String>();
                    if !text.trim_end().ends_with(&url) {
                        self.push_styled(format!(" ({url})"), DIM);
                    }
                }
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn text(lines: &[Line<'_>]) -> Vec<String> {
        lines
            .iter()
            .map(|l| l.spans.iter().map(|s| s.content.as_ref()).collect())
            .collect()
    }

    #[test]
    fn notes_drop_journal_lines() {
        let body = "Do the thing\n\n- 2026-09-29 14:00 [user] draft: hi\n- a real bullet\n";
        assert_eq!(notes_of(body), "Do the thing\n\n- a real bullet");
    }

    #[test]
    fn html_renders_markdown_and_neutralises_raw_html() {
        let h = to_html(
            "A **bold** [link](https://x.y) and `code`\n\n- [ ] todo\n- [x] done\n\n<script>alert(1)</script>",
        );
        assert!(h.contains("<strong>bold</strong>"));
        assert!(h.contains(r#"<a href="https://x.y">link</a>"#));
        assert!(h.contains("<code>code</code>"));
        assert!(h.contains(r#"type="checkbox""#));
        assert!(!h.contains("<script>"));
        assert!(h.contains("&lt;script&gt;"));
    }

    #[test]
    fn lines_for_the_terminal() {
        let md = "# Plan\n\nFirst **this**, then\nthat.\n\n- one\n- two\n  - nested\n\n1. a\n2. b\n\n```sh\ncargo build\n```\n\n> quoted\n\nSee [docs](https://x.y/docs) or <https://x.y>.";
        let lines = to_lines(md);
        assert_eq!(
            text(&lines),
            [
                "Plan",
                "",
                "First this, then that.",
                "",
                "• one",
                "• two",
                "  • nested",
                "",
                "1. a",
                "2. b",
                "",
                "sh",
                "  cargo build",
                "",
                "│ quoted",
                "",
                "See docs (https://x.y/docs) or https://x.y.",
            ]
        );
        assert!(
            lines[0].spans[0]
                .style
                .add_modifier
                .contains(Modifier::BOLD)
        );
        assert_eq!(lines[12].spans[1].style, CODE);
    }

    #[test]
    fn tables_and_task_lists() {
        let md = "| a | b |\n|---|---|\n| 1 | 2 |\n\n- [x] done\n- [ ] open";
        assert_eq!(
            text(&to_lines(md)),
            ["a │ b", "───", "1 │ 2", "", "• ☑ done", "• ☐ open"]
        );
    }

    #[test]
    fn plain_text_is_one_line_per_paragraph() {
        assert_eq!(text(&to_lines("just a note")), ["just a note"]);
        assert!(to_lines("").is_empty());
        assert!(to_lines("   \n\n").is_empty());
    }
}
