//! Pure formatting helpers. No IO, no GUI — these are unit-tested directly.
//!
//! The orchestrator keeps all display maths here so the UI layer stays thin
//! and the timestamp / size / resolution logic can be tested without a window.

/// Format a whole-second duration as a tabular `HH:MM:SS` timer.
pub fn format_duration(total_secs: u64) -> String {
    let h = total_secs / 3600;
    let m = (total_secs % 3600) / 60;
    let s = total_secs % 60;
    format!("{h:02}:{m:02}:{s:02}")
}

/// Human-readable byte size, e.g. `1.5 MB`. Uses binary (1024) steps.
pub fn format_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let mut value = bytes as f64;
    let mut unit = 0usize;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    format!("{value:.1} {}", UNITS[unit])
}

/// The single resolution + refresh chip shown in the UI, e.g. `3840×2160 · 144 Hz`.
pub fn format_res_hz(width: u32, height: u32, hz: u32) -> String {
    format!("{width}\u{00d7}{height} \u{00b7} {hz} Hz")
}

/// Output filename: `glass-YYYYmmdd-HHMMSS.mp4` from a UNIX timestamp (UTC).
pub fn timestamp_filename(epoch_secs: u64) -> String {
    let (y, mo, d, h, mi, s) = civil_from_unix(epoch_secs as i64);
    format!("glass-{y:04}{mo:02}{d:02}-{h:02}{mi:02}{s:02}.mp4")
}

/// Convert UNIX seconds to a UTC civil date/time. Pure calendar maths
/// (Howard Hinnant's `civil_from_days`), so no chrono dependency is needed.
fn civil_from_unix(secs: i64) -> (i64, u32, u32, u32, u32, u32) {
    let days = secs.div_euclid(86_400);
    let rem = secs.rem_euclid(86_400);
    let hour = (rem / 3600) as u32;
    let minute = ((rem % 3600) / 60) as u32;
    let second = (rem % 60) as u32;

    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = (z - era * 146_097) as u64; // [0, 146096]
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365; // [0, 399]
    let year = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100); // [0, 365]
    let mp = (5 * doy + 2) / 153; // [0, 11]
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32; // [1, 31]
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32; // [1, 12]
    let year = if month <= 2 { year + 1 } else { year };
    (year, month, day, hour, minute, second)
}

/// Shorten a path for the folder row, keeping the tail and eliding the head.
pub fn shorten_path(path: &str, max_chars: usize) -> String {
    let count = path.chars().count();
    if count <= max_chars {
        return path.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let tail: String = path.chars().skip(count - keep).collect();
    format!("\u{2026}{tail}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn duration_zero_is_midnight() {
        assert_eq!(format_duration(0), "00:00:00");
    }

    #[test]
    fn duration_carries_units() {
        assert_eq!(format_duration(3661), "01:01:01");
    }

    #[test]
    fn duration_max_day() {
        assert_eq!(format_duration(86_399), "23:59:59");
    }

    #[test]
    fn bytes_small_stays_integer() {
        assert_eq!(format_bytes(0), "0 B");
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn bytes_kilobyte_boundary() {
        assert_eq!(format_bytes(1024), "1.0 KB");
    }

    #[test]
    fn bytes_fractional() {
        assert_eq!(format_bytes(1536), "1.5 KB");
    }

    #[test]
    fn bytes_megabyte() {
        assert_eq!(format_bytes(1_048_576), "1.0 MB");
    }

    #[test]
    fn res_hz_uses_multiplication_sign() {
        assert_eq!(
            format_res_hz(3840, 2160, 144),
            "3840\u{00d7}2160 \u{00b7} 144 Hz"
        );
    }

    #[test]
    fn filename_epoch_is_1970() {
        assert_eq!(timestamp_filename(0), "glass-19700101-000000.mp4");
    }

    #[test]
    fn filename_known_timestamp() {
        // 1_700_000_000 == 2023-11-14 22:13:20 UTC
        assert_eq!(
            timestamp_filename(1_700_000_000),
            "glass-20231114-221320.mp4"
        );
    }

    #[test]
    fn filename_leap_day() {
        // 1_709_164_800 == 2024-02-29 00:00:00 UTC
        assert_eq!(
            timestamp_filename(1_709_164_800),
            "glass-20240229-000000.mp4"
        );
    }

    #[test]
    fn shorten_path_keeps_short() {
        assert_eq!(shorten_path("/home/a/Videos", 24), "/home/a/Videos");
    }

    #[test]
    fn shorten_path_elides_head() {
        let out = shorten_path("/very/long/path/that/exceeds/the/limit", 12);
        assert!(out.starts_with('\u{2026}'));
        assert_eq!(out.chars().count(), 12);
    }
}
