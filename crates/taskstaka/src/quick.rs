//! Quick capture: one line of text becomes a new root task.
//!
//! `buy milk`                       a task
//! `Ask Priya about certs @Priya`   waiting on Priya (follow-up in two days)
//! `Ask Priya about certs @Priya +3d`   ... follow up in three days
//! `#devops renew the cert`         in the project devops (first or last word only,
//!                                  so "fix #123 in the parser" keeps its number)
//!
//! Always a root: placing it under a story is a later, deliberate move.

use anyhow::{Result, bail};
use taskstaka_core::Context;
use taskstaka_core::command::AddTask;

use crate::dates;

pub fn parse(text: &str, context: Option<Context>) -> Result<AddTask> {
    let mut words: Vec<&str> = text.split_whitespace().collect();
    let mut waiting_on = None;
    let mut until = None;

    // `@Name`, optionally followed by a date, at the end.
    let at = |w: &str| w.len() > 1 && w.starts_with('@');
    if let Some(&last) = words.last()
        && at(last)
    {
        waiting_on = Some(last[1..].to_string());
        words.pop();
    } else if words.len() >= 2 && at(words[words.len() - 2]) {
        let date = words[words.len() - 1];
        if let Ok(d) = dates::parse_date(date) {
            waiting_on = Some(words[words.len() - 2][1..].to_string());
            until = Some(d);
            words.truncate(words.len() - 2);
        }
    }

    // `#project` as the first or last word.
    let tag = |w: &str| w.len() > 1 && w.starts_with('#');
    let mut project = None;
    if let Some(&first) = words.first()
        && tag(first)
    {
        project = Some(first[1..].to_string());
        words.remove(0);
    } else if let Some(&last) = words.last()
        && tag(last)
    {
        project = Some(last[1..].to_string());
        words.pop();
    }

    let title = words.join(" ");
    if title.is_empty() {
        bail!("give the task a title, for example: renew the cert #devops");
    }
    Ok(AddTask {
        title,
        context,
        waiting_on,
        until,
        project,
        ..Default::default()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_plain_line_is_a_title() {
        let a = parse("  buy   milk ", None).unwrap();
        assert_eq!(a.title, "buy milk");
        assert!(a.parent.is_none() && a.waiting_on.is_none() && a.project.is_none());
    }

    #[test]
    fn at_name_waits_on_them_with_an_optional_date() {
        let a = parse("Ask about certs @Priya", None).unwrap();
        assert_eq!(a.title, "Ask about certs");
        assert_eq!(a.waiting_on.as_deref(), Some("Priya"));
        assert!(a.until.is_none());
        let a = parse("Ask about certs @Priya +3d", None).unwrap();
        assert_eq!(a.waiting_on.as_deref(), Some("Priya"));
        assert!(a.until.is_some());
        // Not at the end: just words.
        assert_eq!(parse("email @bob about it", None).unwrap().waiting_on, None);
    }

    #[test]
    fn hash_project_only_at_the_ends() {
        let a = parse("#devops renew the cert", None).unwrap();
        assert_eq!(
            (a.title.as_str(), a.project.as_deref()),
            ("renew the cert", Some("devops"))
        );
        let a = parse("renew the cert #devops", None).unwrap();
        assert_eq!(
            (a.title.as_str(), a.project.as_deref()),
            ("renew the cert", Some("devops"))
        );
        let a = parse("fix #123 in the parser", None).unwrap();
        assert_eq!(
            (a.title.as_str(), a.project),
            ("fix #123 in the parser", None)
        );
        let a = parse("renew the cert #devops @Priya", None).unwrap();
        assert_eq!(a.project.as_deref(), Some("devops"));
        assert_eq!(a.waiting_on.as_deref(), Some("Priya"));
    }

    #[test]
    fn empty_is_refused_and_context_is_passed_through() {
        assert!(parse("   ", None).is_err());
        assert!(parse("#devops", None).is_err());
        assert_eq!(
            parse("x", Some(Context::Personal)).unwrap().context,
            Some(Context::Personal)
        );
    }
}
