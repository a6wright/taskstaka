//! Compact human durations: "<1m", "45m", "3h", "2d", "6w". Precision drops
//! as the span grows; the panel needs a glance, not a stopwatch.

use jiff::Timestamp;

pub fn duration(secs: i64) -> String {
    let secs = secs.max(0);
    const MIN: i64 = 60;
    const HOUR: i64 = 60 * MIN;
    const DAY: i64 = 24 * HOUR;
    const WEEK: i64 = 7 * DAY;
    if secs < MIN {
        "<1m".into()
    } else if secs < HOUR {
        format!("{}m", secs / MIN)
    } else if secs < DAY {
        format!("{}h", secs / HOUR)
    } else if secs < 2 * WEEK {
        format!("{}d", secs / DAY)
    } else {
        format!("{}w", secs / WEEK)
    }
}

pub fn since(from: Timestamp, now: Timestamp) -> String {
    duration(now.as_second() - from.as_second())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn buckets() {
        assert_eq!(duration(5), "<1m");
        assert_eq!(duration(61), "1m");
        assert_eq!(duration(3 * 3600 + 5), "3h");
        assert_eq!(duration(2 * 86400), "2d");
        assert_eq!(duration(13 * 86400), "13d");
        assert_eq!(duration(15 * 86400), "2w");
        assert_eq!(duration(-5), "<1m");
    }
}

/// Journal lines are `- YYYY-MM-DD HH:MM [source] text` (the tag is optional).
/// Returns the text without the stamp, or the line unchanged if it is not one.
pub fn strip_journal_stamp(line: &str) -> &str {
    let l = line.trim();
    let Some(rest) = l.strip_prefix("- ") else {
        return l;
    };
    // "2026-09-29 14:00 " is 17 bytes of ASCII when it is a stamp.
    let is_stamp = rest.len() >= 17
        && rest.as_bytes()[..16]
            .iter()
            .enumerate()
            .all(|(i, b)| match i {
                4 | 7 => *b == b'-',
                10 => *b == b' ',
                13 => *b == b':',
                _ => b.is_ascii_digit(),
            })
        && rest.as_bytes()[16] == b' ';
    if !is_stamp {
        return l;
    }
    let text = rest[17..].trim_start();
    match text.strip_prefix('[') {
        Some(tagged) => match tagged.find("] ") {
            Some(i) => tagged[i + 2..].trim_start(),
            None => text,
        },
        None => text,
    }
}

/// `line` is a journal line (see `strip_journal_stamp`).
pub fn is_journal_line(line: &str) -> bool {
    strip_journal_stamp(line) != line.trim()
}

#[cfg(test)]
mod journal_tests {
    use super::*;

    #[test]
    fn strips_tagged_untagged_and_leaves_prose_alone() {
        assert_eq!(
            strip_journal_stamp("- 2026-09-29 14:00 [user] draft: hi [team] ok"),
            "draft: hi [team] ok"
        );
        assert_eq!(
            strip_journal_stamp("- 2026-09-29 14:00 paused: first X"),
            "paused: first X"
        );
        assert_eq!(strip_journal_stamp("- just a bullet"), "- just a bullet");
        assert_eq!(strip_journal_stamp("plain"), "plain");
        assert!(is_journal_line("- 2026-09-29 14:00 [agent] chased B"));
        assert!(!is_journal_line("- 2026 is a year"));
    }
}
