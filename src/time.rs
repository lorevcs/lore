//! Minimal Unix-nanoseconds to UTC formatting, so Lore needs no date dependency.

/// Format Unix nanoseconds as `YYYY-MM-DD HH:MM:SS UTC`. Sub-second precision is
/// dropped from the display, but kept in the stored value so intent recorded in
/// the same second still sorts in the order it happened.
pub fn format_ns(nanos: u64) -> String {
    let secs = nanos / 1_000_000_000;
    let days = (secs / 86_400) as i64;
    let rem = secs % 86_400;
    let (hh, mm, ss) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    let (y, m, d) = civil_from_days(days);
    format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02} UTC")
}

/// Convert a count of days since 1970-01-01 to a (year, month, day) triple.
/// Howard Hinnant's `civil_from_days`, valid for any day we will ever see.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let year = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    (if month <= 2 { year + 1 } else { year }, month, day)
}

#[cfg(test)]
mod tests {
    use super::*;

    const S: u64 = 1_000_000_000; // nanoseconds per second

    #[test]
    fn epoch() {
        assert_eq!(format_ns(0), "1970-01-01 00:00:00 UTC");
    }

    #[test]
    fn known_timestamps() {
        assert_eq!(format_ns(1_000_000_000 * S), "2001-09-09 01:46:40 UTC");
        assert_eq!(format_ns(1_700_000_000 * S), "2023-11-14 22:13:20 UTC");
    }

    #[test]
    fn drops_sub_second() {
        assert_eq!(
            format_ns(1_700_000_000 * S + 999_999_999),
            "2023-11-14 22:13:20 UTC"
        );
    }

    #[test]
    fn handles_leap_day() {
        // 2020-02-29 was a leap day; 1582934400 is its midnight UTC.
        assert_eq!(format_ns(1_582_934_400 * S), "2020-02-29 00:00:00 UTC");
    }
}
