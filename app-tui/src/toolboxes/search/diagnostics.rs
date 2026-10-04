//! The post-search "where did the time go" summary and the fast-index health
//! snapshot shown under the Run view. Pure data + formatting: the runner fills
//! it in, `run_view` draws `lines()`. Counts, sizes and timings only - never
//! file names, folder names or search terms, so the same text is safe to log.

use std::path::Path;
use std::time::Duration;

use search_core::native_index::{NarrowOutcome, INDEX_SEMANTIC_VERSION};

use crate::format::format_bytes;

use super::indexing::format_eta;

/// Segment count above which the index is probably still merging in the
/// background after a big build (measured: 256 right after a 100k-document
/// commit, 4 once settled). Tantivy exposes no merge progress, so this is a
/// hint, never a percentage.
const MERGING_SEGMENT_HINT: usize = 32;

/// Cheap, accurate facts about the index a search just used.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct IndexHealth {
    pub docs: u64,
    pub segments: usize,
    pub size_bytes: u64,
    /// Since the index was last committed (`meta.json` modified time).
    pub updated_ago: Option<Duration>,
    pub new_or_changed: usize,
    pub removed: usize,
    pub semantic_version: u32,
}

impl IndexHealth {
    /// Blocking: lists the index folder. Run it off the async workers.
    pub fn read(index_dir: &Path, narrowed: &NarrowOutcome) -> Self {
        let updated_ago = std::fs::metadata(index_dir.join("meta.json"))
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok());
        IndexHealth {
            docs: narrowed.index_docs,
            segments: narrowed.segments,
            size_bytes: dir_size(index_dir),
            updated_ago,
            new_or_changed: narrowed.needs_update,
            removed: narrowed.removed,
            semantic_version: INDEX_SEMANTIC_VERSION,
        }
    }
}

/// Background merges delete segment files while this walks, so an entry that
/// vanishes is skipped, not an error.
fn dir_size(path: &Path) -> u64 {
    let Ok(entries) = std::fs::read_dir(path) else { return 0 };
    entries
        .filter_map(|e| e.ok())
        .filter_map(|e| e.metadata().ok().map(|m| if m.is_dir() { dir_size(&e.path()) } else { m.len() }))
        .sum()
}

/// What one search run did, summed over its roots.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct SearchDiagnostics {
    pub roots: usize,
    /// Roots whose file list was narrowed by the index.
    pub roots_narrowed: usize,
    /// Why the most recent non-narrowed root was fully scanned.
    pub full_scan_reason: Option<String>,
    /// Files a full scan would have processed (narrowed roots only).
    pub corpus: usize,
    pub candidates: usize,
    pub walk_ms: u64,
    pub freshness_ms: u64,
    pub lookup_ms: u64,
    /// Time in the exact line scan itself.
    pub verify_ms: u64,
    pub total_ms: u64,
    pub verified: i32,
    pub cache_reused: i32,
    pub matched_files: usize,
    pub health: Option<IndexHealth>,
}

impl SearchDiagnostics {
    pub fn record_narrowed(&mut self, out: &NarrowOutcome, candidates: usize) {
        self.roots_narrowed += 1;
        self.corpus += out.scannable;
        self.candidates += candidates;
        self.walk_ms += out.walk_ms;
        self.freshness_ms += out.freshness_ms;
        self.lookup_ms += out.query_ms;
    }

    /// Display lines: result, timing, and (when an index was used) health.
    /// `last_build_failed` is the failed-file count of the most recent index
    /// build, when there was one.
    pub fn lines(&self, last_build_failed: Option<i32>) -> Vec<String> {
        let mut lines = Vec::new();
        let cache = if self.cache_reused > 0 { format!(" · {} from result cache", group(self.cache_reused as u64)) } else { String::new() };
        if self.roots_narrowed > 0 {
            let reduction = if self.corpus == 0 { 0.0 } else { 100.0 * (1.0 - self.candidates as f64 / self.corpus as f64) };
            let roots = if self.roots > 1 { format!(" · index used for {}/{} root(s)", self.roots_narrowed, self.roots) } else { String::new() };
            lines.push(format!(
                "Corpus {} files · index candidates {} ({reduction:.1}% fewer) · verified {} · {} file(s) with matches{cache}{roots}",
                group(self.corpus as u64),
                group(self.candidates as u64),
                group(self.verified.max(0) as u64),
                group(self.matched_files as u64),
            ));
            lines.push(format!(
                "Time: index lookup {} ms (walk {}, freshness {}) · exact verification {} ms · total {} ms",
                self.lookup_ms, self.walk_ms, self.freshness_ms, self.verify_ms, self.total_ms
            ));
        } else {
            let reason = self.full_scan_reason.as_deref().unwrap_or("index not used");
            lines.push(format!(
                "Full scan ({reason}) · verified {} file(s) · {} with matches{cache}",
                group(self.verified.max(0) as u64),
                group(self.matched_files as u64)
            ));
            lines.push(format!("Time: exact verification {} ms · total {} ms", self.verify_ms, self.total_ms));
        }
        if let Some(h) = &self.health {
            let merging = if h.segments > MERGING_SEGMENT_HINT { " (many - background merge likely still running)" } else { "" };
            let updated = h.updated_ago.map_or(String::new(), |d| format!(" · updated {} ago", format_eta(d.as_secs_f64())));
            let failed = match last_build_failed {
                Some(n) if n > 0 => format!(" · last build: {n} file(s) failed"),
                _ => String::new(),
            };
            lines.push(format!(
                "Index: {} docs · {} segment(s){merging} · {}{updated} · {} new/changed, {} removed · format v{}{failed}",
                group(h.docs),
                h.segments,
                format_bytes(h.size_bytes as i64),
                h.new_or_changed,
                h.removed,
                h.semantic_version
            ));
        }
        lines
    }
}

/// `1234567` -> `1,234,567`.
fn group(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn group_inserts_thousands_separators() {
        assert_eq!(group(0), "0");
        assert_eq!(group(999), "999");
        assert_eq!(group(1_000), "1,000");
        assert_eq!(group(104_331), "104,331");
        assert_eq!(group(1_234_567), "1,234,567");
    }

    fn narrowed() -> SearchDiagnostics {
        let mut d = SearchDiagnostics { roots: 1, verified: 1_397, matched_files: 17, verify_ms: 380, total_ms: 610, ..Default::default() };
        d.record_narrowed(
            &NarrowOutcome { scannable: 104_331, walk_ms: 100, freshness_ms: 60, query_ms: 42, ..Default::default() },
            1_382,
        );
        d
    }

    #[test]
    fn narrowed_summary_reports_corpus_candidates_reduction_and_timings() {
        let lines = narrowed().lines(None);
        assert_eq!(lines.len(), 2, "no health line without an index snapshot");
        assert_eq!(
            lines[0],
            "Corpus 104,331 files · index candidates 1,382 (98.7% fewer) · verified 1,397 · 17 file(s) with matches"
        );
        assert_eq!(lines[1], "Time: index lookup 42 ms (walk 100, freshness 60) · exact verification 380 ms · total 610 ms");
    }

    #[test]
    fn full_scan_summary_says_why_and_omits_index_timings() {
        let d = SearchDiagnostics {
            roots: 1,
            full_scan_reason: Some("short or regex filter".into()),
            verified: 50,
            matched_files: 3,
            verify_ms: 20,
            total_ms: 25,
            ..Default::default()
        };
        let lines = d.lines(None);
        assert_eq!(lines[0], "Full scan (short or regex filter) · verified 50 file(s) · 3 with matches");
        assert_eq!(lines[1], "Time: exact verification 20 ms · total 25 ms");
    }

    #[test]
    fn health_line_flags_a_high_segment_count_and_a_failed_build() {
        let mut d = narrowed();
        d.health = Some(IndexHealth {
            docs: 104_331,
            segments: 256,
            size_bytes: 3 * 1024 * 1024,
            updated_ago: Some(Duration::from_secs(7_200)),
            new_or_changed: 3,
            removed: 1,
            semantic_version: 2,
        });
        let line = d.lines(Some(4)).pop().unwrap();
        assert!(line.contains("104,331 docs") && line.contains("256 segment(s) (many - background merge likely still running)"), "{line}");
        assert!(line.contains("3.0 MB") && line.contains("updated 2h00m ago") && line.contains("3 new/changed, 1 removed"), "{line}");
        assert!(line.contains("format v2") && line.ends_with("last build: 4 file(s) failed"), "{line}");

        d.health.as_mut().unwrap().segments = 4;
        assert!(!d.lines(Some(0)).pop().unwrap().contains("merge"), "settled index, clean build: no warnings");
    }

    #[test]
    fn multi_root_and_cache_notes_appear_only_when_relevant() {
        let mut d = narrowed();
        d.roots = 3;
        d.cache_reused = 1_200;
        let first = d.lines(None).remove(0);
        assert!(first.contains("1,200 from result cache") && first.contains("index used for 1/3 root(s)"), "{first}");
    }

    #[test]
    fn empty_corpus_does_not_divide_by_zero() {
        let mut d = SearchDiagnostics { roots: 1, ..Default::default() };
        d.record_narrowed(&NarrowOutcome::default(), 0);
        assert!(d.lines(None)[0].contains("0.0% fewer"));
    }

    #[test]
    fn health_read_reports_size_and_age_of_a_real_folder() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("meta.json"), "{}").unwrap();
        std::fs::write(dir.path().join("seg.bin"), vec![0u8; 1000]).unwrap();
        let h = IndexHealth::read(dir.path(), &NarrowOutcome { index_docs: 7, segments: 2, needs_update: 1, removed: 2, ..Default::default() });
        assert_eq!((h.docs, h.segments, h.new_or_changed, h.removed), (7, 2, 1, 2));
        assert_eq!(h.size_bytes, 1002);
        assert!(h.updated_ago.is_some());
    }
}
