//! UTC timestamps in the one form the bench stores: `YYYY-MM-DDTHH:MM:SSZ`. Fixed width, so
//! string order is time order.

use std::time::{SystemTime, UNIX_EPOCH};

/// Whether `text` is a well-formed `YYYY-MM-DDTHH:MM:SSZ` timestamp.
pub fn is_utc(text: &str) -> bool {
    let bytes = text.as_bytes();
    if bytes.len() != 20 {
        return false;
    }
    let shape = b"0000-00-00T00:00:00Z";
    let shaped = bytes
        .iter()
        .zip(shape)
        .all(|(byte, expected)| match expected {
            b'0' => byte.is_ascii_digit(),
            other => byte == other,
        });
    if !shaped {
        return false;
    }
    let field = |range: std::ops::Range<usize>| text[range].parse::<u32>().unwrap_or(u32::MAX);
    (1..=12).contains(&field(5..7))
        && (1..=31).contains(&field(8..10))
        && field(11..13) <= 23
        && field(14..16) <= 59
        && field(17..19) <= 59
}

/// The current time as `YYYY-MM-DDTHH:MM:SSZ`.
pub fn now_utc() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64);
    format_utc(seconds)
}

/// Formats Unix seconds as `YYYY-MM-DDTHH:MM:SSZ` (Hinnant's civil-from-days).
pub fn format_utc(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let of_day = seconds.rem_euclid(86_400);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        of_day / 3600,
        of_day % 3600 / 60,
        of_day % 60
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_and_validates() {
        assert_eq!(format_utc(0), "1970-01-01T00:00:00Z");
        assert_eq!(format_utc(1_577_836_800), "2020-01-01T00:00:00Z");
        assert_eq!(format_utc(951_782_400), "2000-02-29T00:00:00Z");
        assert_eq!(format_utc(1_795_123_456), "2026-11-19T21:24:16Z");
        assert!(is_utc(&now_utc()));
        for bad in [
            "2026-11-19 18:44:16Z",
            "2026-11-19T18:44:16",
            "2026-13-19T18:44:16Z",
            "2026-11-19T24:00:00Z",
            "2026-11-19T18:44:16+00:00",
        ] {
            assert!(!is_utc(bad), "{bad}");
        }
    }
}
