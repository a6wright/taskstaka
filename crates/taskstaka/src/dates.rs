//! Lenient date input for flags: ISO dates plus a few relative forms.

use jiff::civil::Date;
use jiff::{ToSpan, Zoned};

pub fn parse_date(s: &str) -> Result<Date, String> {
    parse_date_at(s, &Zoned::now())
}

fn parse_date_at(s: &str, now: &Zoned) -> Result<Date, String> {
    let today = now.date();
    let s = s.trim();
    match s {
        "today" | "tod" => return Ok(today),
        "tomorrow" | "tom" => return today.checked_add(1.day()).map_err(|e| e.to_string()),
        "yesterday" | "yest" => return today.checked_sub(1.day()).map_err(|e| e.to_string()),
        _ => {}
    }
    // `+3d` is three days ahead, `-3d` three days back.
    if let Some((rest, sign)) = s
        .strip_prefix('+')
        .map(|r| (r, 1))
        .or_else(|| s.strip_prefix('-').map(|r| (r, -1)))
    {
        let (num, unit) = rest.split_at(rest.trim_end_matches(char::is_alphabetic).len());
        let n: i64 = num
            .parse::<i64>()
            .map(|n| n * sign)
            .map_err(|_| format!("bad relative date {s:?}; try +3d, -3d or +2w"))?;
        let span = match unit {
            "d" | "" => n.days(),
            "w" => (n * 7).days(),
            "m" => n.months(),
            _ => return Err(format!("unknown unit {unit:?} in {s:?}; use d, w or m")),
        };
        return today.checked_add(span).map_err(|e| e.to_string());
    }
    s.parse::<Date>().map_err(|_| {
        format!("{s:?} is not a date; use YYYY-MM-DD, today, tomorrow, yesterday, +Nd or -Nd")
    })
}

/// The day-log range from the `--on` / `--since` / `--until` flags (the MCP
/// `log` tool takes the same three): `on` is one day; otherwise `since`
/// (default six days back) to `until` (default today).
pub fn log_range(
    on: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
) -> Result<(Date, Date), String> {
    log_range_at(on, since, until, &Zoned::now())
}

fn log_range_at(
    on: Option<&str>,
    since: Option<&str>,
    until: Option<&str>,
    now: &Zoned,
) -> Result<(Date, Date), String> {
    if let Some(d) = on {
        let d = parse_date_at(d, now)?;
        return Ok((d, d));
    }
    let to = parse_date_at(until.unwrap_or("today"), now)?;
    let from = match since {
        Some(s) => parse_date_at(s, now)?,
        None => to.checked_sub(6.days()).map_err(|e| e.to_string())?,
    };
    if from > to {
        return Err(format!("since ({from}) is after until ({to})"));
    }
    Ok((from, to))
}

#[cfg(test)]
mod tests {
    use super::*;
    use jiff::civil::date;

    #[test]
    fn forms() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        assert_eq!(
            parse_date_at("2026-10-03", &now).unwrap(),
            date(2026, 10, 3)
        );
        assert_eq!(parse_date_at("today", &now).unwrap(), date(2026, 9, 29));
        assert_eq!(parse_date_at("tomorrow", &now).unwrap(), date(2026, 9, 30));
        assert_eq!(parse_date_at("+3d", &now).unwrap(), date(2026, 10, 2));
        assert_eq!(parse_date_at("+1w", &now).unwrap(), date(2026, 10, 6));
        assert_eq!(parse_date_at("+1m", &now).unwrap(), date(2026, 10, 29));
        assert_eq!(parse_date_at("yesterday", &now).unwrap(), date(2026, 9, 28));
        assert_eq!(parse_date_at("-6d", &now).unwrap(), date(2026, 9, 23));
        assert_eq!(parse_date_at("-1w", &now).unwrap(), date(2026, 9, 22));
        assert!(parse_date_at("friday", &now).is_err());
    }

    #[test]
    fn log_ranges() {
        let now: Zoned = "2026-09-29T15:00:00[UTC]".parse().unwrap();
        assert_eq!(
            log_range_at(None, None, None, &now).unwrap(),
            (date(2026, 9, 23), date(2026, 9, 29)),
            "a week ending today"
        );
        assert_eq!(
            log_range_at(Some("yesterday"), None, None, &now).unwrap(),
            (date(2026, 9, 28), date(2026, 9, 28))
        );
        assert_eq!(
            log_range_at(None, Some("-1d"), Some("today"), &now).unwrap(),
            (date(2026, 9, 28), date(2026, 9, 29))
        );
        assert!(log_range_at(None, Some("tomorrow"), None, &now).is_err());
    }
}
