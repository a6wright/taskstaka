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
