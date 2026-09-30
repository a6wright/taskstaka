//! Split, parse and render `---` YAML frontmatter around a markdown body.
//!
//! Parsing tolerates any valid YAML (Obsidian rewrites quoting, key order and
//! list style if its Properties UI is ever used); rendering is canonical.

use thiserror::Error;

use crate::model::Task;

#[derive(Debug, Error)]
pub enum FrontmatterError {
    #[error("file does not start with a `---` frontmatter fence")]
    MissingOpen,
    #[error("frontmatter opened with `---` but never closed")]
    MissingClose,
    #[error("invalid frontmatter: {0}")]
    Yaml(#[from] serde_norway::Error),
}

const FENCE: &str = "---";

/// Split a document into (yaml, body). The body has the newline after the
/// closing fence removed but is otherwise verbatim.
pub fn split(text: &str) -> Result<(&str, &str), FrontmatterError> {
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let rest = text
        .strip_prefix(FENCE)
        .and_then(|r| r.strip_prefix('\n').or_else(|| r.strip_prefix("\r\n")))
        .ok_or(FrontmatterError::MissingOpen)?;

    // Find a line that is exactly `---` (allowing trailing \r).
    let mut offset = 0;
    for line in rest.split_inclusive('\n') {
        if line.trim_end_matches(['\r', '\n']) == FENCE {
            let yaml = &rest[..offset];
            let body = &rest[offset + line.len()..];
            return Ok((yaml, body));
        }
        offset += line.len();
    }
    Err(FrontmatterError::MissingClose)
}

pub fn parse(text: &str) -> Result<Task, FrontmatterError> {
    let (yaml, body) = split(text)?;
    let mut task: Task = serde_norway::from_str(yaml)?;
    task.body = body.to_string();
    Ok(task)
}

pub fn render(task: &Task) -> String {
    // serde_norway does not emit the fences; we add them.
    let yaml = serde_norway::to_string(task).expect("Task always serializes");
    let mut out = String::with_capacity(yaml.len() + task.body.len() + 16);
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(&yaml);
    if !yaml.ends_with('\n') {
        out.push('\n');
    }
    out.push_str(FENCE);
    out.push('\n');
    out.push_str(&task.body);
    if !task.body.is_empty() && !task.body.ends_with('\n') {
        out.push('\n');
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Context, Priority, Status, TaskId};
    use jiff::Timestamp;
    use jiff::civil::date;

    fn sample() -> Task {
        let now: Timestamp = "2026-09-29T14:02:00Z".parse().unwrap();
        let mut t = Task::new(
            "k7x2mq".parse::<TaskId>().unwrap(),
            "Fix JWT expiry",
            Context::Work,
            now,
        );
        t.parent = Some("a1b2c3".parse().unwrap());
        t.priority = Some(Priority::High);
        t.due = Some(date(2026, 10, 3));
        t.tags = vec!["auth".into(), "backend".into()];
        t.body = "# Notes\n\nSome *markdown*.\n".into();
        t
    }

    #[test]
    fn roundtrip_is_identity() {
        let t = sample();
        let text = render(&t);
        let back = parse(&text).unwrap();
        assert_eq!(back, t);
    }

    #[test]
    fn render_is_canonical_and_omits_unset_optionals() {
        let text = render(&sample());
        insta::assert_snapshot!(text);
    }

    #[test]
    fn parses_obsidian_style_yaml() {
        // Obsidian's Properties UI quotes strings, uses block lists, and may
        // reorder keys. Dates may be written without time.
        let text = "---\ntags:\n  - auth\ntitle: \"Fix JWT expiry\"\nid: 'k7x2mq'\nstatus: waiting\nwaiting_on: \"Sam\"\nuntil: 2026-10-01\ncreated: 2026-09-29T14:02:00Z\nupdated: 2026-09-29T14:02:00Z\nstarted:\n---\nbody\n";
        let t = parse(text).unwrap();
        assert_eq!(t.title, "Fix JWT expiry");
        assert_eq!(t.status, Status::Waiting);
        assert_eq!(t.until, Some(date(2026, 10, 1)));
        assert_eq!(t.tags, vec!["auth"]);
        assert_eq!(t.started, None);
        assert_eq!(t.context, Context::Work, "context defaults when missing");
        assert_eq!(t.body, "body\n");
    }

    #[test]
    fn split_handles_crlf_and_bom() {
        let text = "\u{feff}---\r\nid: abc\r\n---\r\nhi";
        let (yaml, body) = split(text).unwrap();
        assert_eq!(yaml, "id: abc\r\n");
        assert_eq!(body, "hi");
    }

    #[test]
    fn errors_are_specific() {
        assert!(matches!(
            parse("no fence"),
            Err(FrontmatterError::MissingOpen)
        ));
        assert!(matches!(
            parse("---\nid: x\n"),
            Err(FrontmatterError::MissingClose)
        ));
        assert!(matches!(
            parse("---\nid: [\n---\n"),
            Err(FrontmatterError::Yaml(_))
        ));
    }
}
