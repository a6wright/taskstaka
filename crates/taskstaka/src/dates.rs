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
        _ => {}
    }
    if let Some(rest) = s.strip_prefix('+') {
        let (num, unit) = rest.split_at(rest.trim_end_matches(char::is_alphabetic).len());
        let n: i64 = num
            .parse()
            .map_err(|_| format!("bad relative date {s:?}; try +3d or +2w"))?;
        let span = match unit {
            "d" | "" => n.days(),
            "w" => (n * 7).days(),
            "m" => n.months(),
            _ => return Err(format!("unknown unit {unit:?} in {s:?}; use d, w or m")),
        };
        return today.checked_add(span).map_err(|e| e.to_string());
    }
    s.parse::<Date>()
        .map_err(|_| format!("{s:?} is not a date; use YYYY-MM-DD, today, tomorrow or +Nd"))
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
        assert!(parse_date_at("friday", &now).is_err());
    }
}
