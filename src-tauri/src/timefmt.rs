//! Turn WeChat's stored timestamps into readable local time.

use chrono::{Local, TimeZone};

/// WeChat stores Unix seconds in most tables, but some columns (e.g. `sequence`)
/// hold milliseconds. Normalize to seconds so both render correctly.
pub fn normalize_seconds(raw: i64) -> i64 {
    if raw.abs() >= 100_000_000_000 {
        raw / 1000
    } else {
        raw
    }
}

/// `YYYY-MM-DD HH:MM:SS` in the machine's local timezone; empty for no time.
pub fn format_local(ts_seconds: i64) -> String {
    if ts_seconds <= 0 {
        return String::new();
    }
    match Local.timestamp_opt(ts_seconds, 0) {
        chrono::LocalResult::Single(dt) => dt.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => String::new(),
    }
}

/// Timestamp suitable for file names: `YYYYMMDD_HHMMSS` (local time now).
pub fn stamp_now() -> String {
    Local::now().format("%Y%m%d_%H%M%S").to_string()
}

/// RFC3339 local time, used for the export's `exportedAt` field.
pub fn now_rfc3339() -> String {
    Local::now().to_rfc3339()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_millisecond_timestamps() {
        assert_eq!(normalize_seconds(1_700_000_000), 1_700_000_000);
        assert_eq!(normalize_seconds(1_700_000_000_000), 1_700_000_000);
        assert_eq!(normalize_seconds(0), 0);
    }

    #[test]
    fn formats_and_rejects_empty_timestamps() {
        assert!(format_local(0).is_empty());
        assert!(format_local(-5).is_empty());
        let s = format_local(1_700_000_000);
        // "YYYY-MM-DD HH:MM:SS" is exactly 19 chars regardless of timezone.
        assert_eq!(s.len(), 19);
        assert_eq!(&s[4..5], "-");
        assert_eq!(&s[10..11], " ");
    }

    #[test]
    fn stamp_is_filename_safe() {
        let s = stamp_now();
        assert_eq!(s.len(), 15);
        assert!(s.chars().all(|c| c.is_ascii_digit() || c == '_'));
    }
}
