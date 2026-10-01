//! Log file names: `{ts}` and `{pid}` placeholders, so one static editor
//! configuration gives every server process its own log.

use std::path::{Path, PathBuf};

/// Replaces `{ts}` in `template` with `unix_seconds` as a UTC timestamp
/// (`20261001T093000Z`) and `{pid}` with `pid`.
pub fn expand(template: &Path, unix_seconds: u64, pid: u32) -> PathBuf {
    let text = template.to_string_lossy();
    if !text.contains("{ts}") && !text.contains("{pid}") {
        return template.to_path_buf();
    }
    PathBuf::from(
        text.replace("{ts}", &utc_stamp(unix_seconds))
            .replace("{pid}", &pid.to_string()),
    )
}

/// `unix_seconds` as `YYYYMMDDTHHMMSSZ`.
pub fn utc_stamp(unix_seconds: u64) -> String {
    let days = unix_seconds / 86_400;
    let rest = unix_seconds % 86_400;
    let (year, month, day) = civil_from_days(days);
    format!(
        "{year:04}{month:02}{day:02}T{:02}{:02}{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}

/// The proleptic Gregorian date `days` after 1970-01-01 (Howard Hinnant's
/// `civil_from_days`, for non-negative day counts).
fn civil_from_days(days: u64) -> (u64, u64, u64) {
    let z = days + 719_468;
    let era = z / 146_097;
    let doe = z % 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    let year = yoe + era * 400 + u64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::{expand, utc_stamp};

    #[test]
    fn stamps_known_instants() {
        assert_eq!(utc_stamp(0), "19700101T000000Z");
        assert_eq!(utc_stamp(951_782_400), "20000229T000000Z");
        // 2026-10-01T09:30:05Z
        assert_eq!(utc_stamp(1_790_847_005), "20261001T093005Z");
    }

    #[test]
    fn expands_placeholders() {
        let path = expand(Path::new("logs/lsp-{ts}-{pid}.jsonl"), 0, 42);
        assert_eq!(path, Path::new("logs/lsp-19700101T000000Z-42.jsonl"));
    }

    #[test]
    fn leaves_plain_paths_alone() {
        assert_eq!(expand(Path::new("a/b.jsonl"), 5, 1), Path::new("a/b.jsonl"));
    }
}
