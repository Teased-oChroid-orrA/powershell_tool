//! Small presentation-formatting helpers shared across the Search Files
//! toolbox's Run and Settings views. Pure functions, no ratatui/crossterm
//! dependency - formatting numbers/strings needs neither.

use std::collections::HashMap;

use search_core::models::FileSearchResult;

/// Human-readable byte size, e.g. `"0 B"`, `"240 KB"`, `"1.2 MB"`.
/// `FileSearchResult.file_length` is `i64` (can't be negative in practice,
/// but the type allows it - clamp to 0 rather than panicking on an
/// unexpected negative value from a corrupt cache entry or similar).
pub fn format_bytes(bytes: i64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    let bytes = bytes.max(0) as f64;
    if bytes < 1.0 {
        return "0 B".to_string();
    }
    let exponent = (bytes.log(1024.0).floor() as usize).min(UNITS.len() - 1);
    let value = bytes / 1024f64.powi(exponent as i32);
    if exponent == 0 {
        format!("{value:.0} B")
    } else {
        format!("{value:.1} {}", UNITS[exponent])
    }
}

/// Human-readable elapsed time for the in-flight ticker, e.g. `"0.4s"`,
/// `"4.2s"`, `"1m 12s"` once a file has been in flight over a minute.
pub fn format_elapsed(seconds: f64) -> String {
    let seconds = seconds.max(0.0);
    if seconds < 60.0 {
        format!("{seconds:.1}s")
    } else {
        let total = seconds.round() as u64;
        let minutes = total / 60;
        let rest = total % 60;
        format!("{minutes}m {rest}s")
    }
}

/// Top-6 extensions among `results`, most-common first, ties broken
/// alphabetically. Ports `app/src/components.rs::extension_breakdown`'s
/// aggregation logic - only the input type differs: the original takes
/// `app/`'s local `FileResultView` (which has a bare `file_name` field);
/// this operates directly on `search_core::models::FileSearchResult`,
/// whose only name field is `full_name` (the full path) - `Path::extension`
/// gives the same answer either way, since it only looks at the last path
/// component's suffix.
pub fn extension_breakdown(results: &[FileSearchResult]) -> Vec<(String, usize)> {
    let mut counts: HashMap<String, usize> = HashMap::new();
    for r in results {
        let ext = std::path::Path::new(&r.full_name)
            .extension()
            .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
            .unwrap_or_else(|| "(no extension)".to_string());
        *counts.entry(ext).or_insert(0) += 1;
    }
    let mut breakdown: Vec<(String, usize)> = counts.into_iter().collect();
    breakdown.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    breakdown.truncate(6);
    breakdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Local;
    use search_core::models::FileSearchStatus;

    fn fixture(full_name: &str) -> FileSearchResult {
        FileSearchResult {
            full_name: full_name.to_string(),
            status: FileSearchStatus::Hit,
            hits: Vec::new(),
            created: Local::now(),
            modified: Local::now(),
            file_length: 0,
            lines_cache: Vec::new(),
            total_line_count: 0,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }

    #[test]
    fn zero_bytes_formats_as_0_b() {
        assert_eq!(format_bytes(0), "0 B");
    }

    #[test]
    fn negative_bytes_clamp_to_0_b() {
        assert_eq!(format_bytes(-5), "0 B");
    }

    #[test]
    fn small_byte_counts_stay_in_bytes() {
        assert_eq!(format_bytes(512), "512 B");
    }

    #[test]
    fn kilobyte_range_formats_with_one_decimal() {
        assert_eq!(format_bytes(240 * 1024), "240.0 KB");
    }

    #[test]
    fn megabyte_range_formats_with_one_decimal() {
        // 1.2 MB, rounded to one decimal place.
        let bytes = (1.2 * 1024.0 * 1024.0) as i64;
        assert_eq!(format_bytes(bytes), "1.2 MB");
    }

    #[test]
    fn under_a_minute_uses_seconds_with_one_decimal() {
        assert_eq!(format_elapsed(0.0), "0.0s");
        assert_eq!(format_elapsed(0.4), "0.4s");
        assert_eq!(format_elapsed(4.2), "4.2s");
        assert_eq!(format_elapsed(59.9), "59.9s");
    }

    #[test]
    fn at_exactly_sixty_seconds_switches_to_minutes() {
        assert_eq!(format_elapsed(60.0), "1m 0s");
    }

    #[test]
    fn over_a_minute_formats_as_minutes_and_seconds() {
        assert_eq!(format_elapsed(72.0), "1m 12s");
    }

    #[test]
    fn negative_elapsed_clamps_to_zero() {
        assert_eq!(format_elapsed(-1.0), "0.0s");
    }

    #[test]
    fn breakdown_counts_and_sorts_descending() {
        let results = vec![
            fixture("a.pdf"),
            fixture("b.pdf"),
            fixture("c.docx"),
            fixture("d.pdf"),
            fixture("e.txt"),
        ];
        let breakdown = extension_breakdown(&results);
        assert_eq!(breakdown[0], (".pdf".to_string(), 3));
        assert_eq!(breakdown[1].1, 1);
        assert_eq!(breakdown[2].1, 1);
    }

    #[test]
    fn breakdown_ties_break_alphabetically() {
        let results = vec![fixture("a.docx"), fixture("b.txt")];
        let breakdown = extension_breakdown(&results);
        assert_eq!(breakdown, vec![(".docx".to_string(), 1), (".txt".to_string(), 1)]);
    }

    #[test]
    fn breakdown_handles_missing_extension() {
        let results = vec![fixture("README")];
        let breakdown = extension_breakdown(&results);
        assert_eq!(breakdown, vec![("(no extension)".to_string(), 1)]);
    }

    #[test]
    fn breakdown_is_truncated_to_top_six() {
        let names = ["a.a1", "b.a2", "c.a3", "d.a4", "e.a5", "f.a6", "g.a7"];
        let results: Vec<_> = names.iter().map(|n| fixture(n)).collect();
        assert_eq!(extension_breakdown(&results).len(), 6);
    }
}
