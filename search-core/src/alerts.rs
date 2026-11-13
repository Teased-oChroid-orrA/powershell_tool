//! Bounded, local neural snapshots for explicit saved-search alert checks.
//! Exact search remains the sole source of alert membership.

use std::path::{Path, PathBuf};

use base64::Engine as _;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::models::{FileSearchResult, FileSearchStatus, SearchRunResult, SearchSettings};

const SCHEMA_VERSION: u32 = 1;
const MAX_ALERT_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_SAVED_ALERTS: usize = 32;
const MAX_FILES: usize = 2_000;
const MAX_EVIDENCE_BYTES: usize = 2 * 1024;
const ID_PREFIX: &str = "alert-";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SavedSearchAlert {
    pub schema_version: u32,
    pub id: String,
    pub name: String,
    pub settings: SearchSettings,
    scope_signature: String,
    pub baseline: Option<Vec<BaselineFile>>,
    pub last_checked_utc: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BaselineFile {
    pub path: String,
    pub modified_ns: i64,
    pub size: i64,
    /// Digest of bounded exact-hit and whole-document samples; source text is not stored.
    fingerprint: String,
    /// Base64 encoded signed 8-bit MiniLM vector. Stores no source text.
    embedding: String,
}

/// Bounded, transient exact-result data prepared before asynchronous inference.
#[derive(Debug, Clone)]
pub struct AlertRun {
    files: Vec<AlertFile>,
}

#[derive(Debug, Clone)]
struct AlertFile {
    path: String,
    modified_ns: i64,
    size: i64,
    evidence: String,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AlertChange {
    New { path: String },
    Updated { path: String, cosine_distance: f32 },
}

impl AlertRun {
    pub fn from_search_result(run: &SearchRunResult) -> Result<Self, String> {
        ensure_complete(run)?;
        Ok(Self {
            files: run
                .file_results
                .iter()
                .filter(|result| result.status == FileSearchStatus::Hit)
                .map(|result| AlertFile {
                    path: result.full_name.clone(),
                    modified_ns: modified_ns(result),
                    size: result.file_length,
                    evidence: sample_evidence(result),
                })
                .collect(),
        })
    }
}

impl SavedSearchAlert {
    pub fn create(name: String, settings: SearchSettings) -> Result<Self, String> {
        if name.trim().is_empty() {
            return Err("alert name must not be empty".into());
        }
        let id = format!("{ID_PREFIX}{}", new_alert_id());
        let scope_signature = scope_signature(&settings)?;
        Ok(Self {
            schema_version: SCHEMA_VERSION,
            id,
            name,
            settings,
            scope_signature,
            baseline: None,
            last_checked_utc: None,
        })
    }

    /// Capture only exact-hit context. Incomplete scans cannot replace a baseline.
    pub fn capture_baseline(&mut self, run: &AlertRun) -> Result<(), String> {
        let mut candidate = self.clone();
        candidate.baseline = Some(embed_results(&run.files)?);
        candidate.last_checked_utc = Some(Utc::now().to_rfc3339());
        ensure_size(&candidate)?;
        *self = candidate;
        Ok(())
    }

    /// Check exact hits, then advance baseline only after full success.
    pub fn check_and_update(
        &mut self,
        settings: &SearchSettings,
        run: &AlertRun,
    ) -> Result<Vec<AlertChange>, String> {
        if scope_signature(settings)? != self.scope_signature {
            return Err("search settings differ from this alert; baseline was kept".into());
        }
        let baseline = self
            .baseline
            .as_ref()
            .ok_or("capture a baseline before checking this alert")?;
        let previous: std::collections::HashMap<&str, &BaselineFile> = baseline
            .iter()
            .map(|file| (file.path.as_str(), file))
            .collect();
        let mut next_baseline = Vec::new();
        let mut pending = Vec::new();
        for result in &run.files {
            let fingerprint = evidence_fingerprint(&result.evidence);
            match previous.get(result.path.as_str()) {
                Some(old)
                    if old.size == result.size
                        && old.modified_ns == result.modified_ns
                        && old.fingerprint == fingerprint =>
                {
                    next_baseline.push((*old).clone());
                }
                old => pending.push((result, old.copied(), fingerprint)),
            }
        }
        let mut changes = Vec::new();
        for chunk in pending.chunks(16) {
            let evidence = chunk
                .iter()
                .map(|(file, _, _)| bounded_evidence(&file.evidence))
                .collect::<Result<Vec<_>, _>>()?;
            let inputs = evidence.iter().map(String::as_str).collect::<Vec<_>>();
            let embeddings = search_ml::embed_batch(&inputs).map_err(|e| e.to_string())?;
            for ((result, old, fingerprint), embedding) in chunk.iter().zip(embeddings) {
                let quantized = quantize(embedding.values());
                if let Some(old) = old {
                    let previous = decode_embedding(&old.embedding)?;
                    let distance = cosine_distance(&previous, &quantized)?;
                    changes.push(AlertChange::Updated {
                        path: result.path.clone(),
                        cosine_distance: distance,
                    });
                } else {
                    changes.push(AlertChange::New {
                        path: result.path.clone(),
                    });
                }
                next_baseline.push(BaselineFile {
                    path: result.path.clone(),
                    modified_ns: result.modified_ns,
                    size: result.size,
                    fingerprint: fingerprint.clone(),
                    embedding: encode_embedding(&quantized),
                });
            }
        }
        let mut candidate = self.clone();
        candidate.baseline = Some(next_baseline);
        candidate.last_checked_utc = Some(Utc::now().to_rfc3339());
        ensure_size(&candidate)?;
        *self = candidate;
        Ok(changes)
    }

    pub fn save(&self, path: &Path) -> Result<(), String> {
        self.validate()?;
        ensure_size(self)?;
        let bytes = serde_json::to_vec(self).map_err(|e| e.to_string())?;
        let parent = path.parent().ok_or("alert path has no parent directory")?;
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        let temp = temporary_path(path);
        std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
        if let Err(error) = std::fs::rename(&temp, path) {
            let _ = std::fs::remove_file(&temp);
            return Err(error.to_string());
        }
        Ok(())
    }

    pub fn load(path: &Path) -> Result<Self, String> {
        let size = std::fs::metadata(path).map_err(|e| e.to_string())?.len();
        if size > MAX_ALERT_BYTES {
            return Err("alert snapshot exceeds the 2 MiB storage limit".into());
        }
        let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
        let alert: Self = serde_json::from_slice(&bytes).map_err(|e| e.to_string())?;
        alert.validate()?;
        if let Some(files) = &alert.baseline {
            if files.len() > MAX_FILES {
                return Err("alert baseline exceeds the 2,000 file limit".into());
            }
            for file in files {
                let _ = decode_embedding(&file.embedding)?;
                let _ = decode_fingerprint(&file.fingerprint)?;
            }
        }
        Ok(alert)
    }

    pub fn validate(&self) -> Result<(), String> {
        if self.schema_version != SCHEMA_VERSION || !self.id.starts_with(ID_PREFIX) {
            return Err("unsupported or invalid alert snapshot".into());
        }
        if scope_signature(&self.settings)? != self.scope_signature {
            return Err("alert scope does not match its saved settings".into());
        }
        if self
            .baseline
            .as_ref()
            .is_some_and(|files| files.len() > MAX_FILES)
        {
            return Err("alert baseline exceeds the 2,000 file limit".into());
        }
        Ok(())
    }

    pub fn path_in(directory: &Path, id: &str) -> Result<PathBuf, String> {
        if !id.starts_with(ID_PREFIX) || id.contains(['/', '\\']) || id.contains("..") {
            return Err("invalid alert id".into());
        }
        Ok(directory.join(format!("{id}.json")))
    }
}

fn ensure_complete(run: &SearchRunResult) -> Result<(), String> {
    if run.was_dry_run
        || run.summary.enumeration_errors > 0
        || run.summary.skipped_read_error > 0
        || run.summary.skipped_unexpected_error > 0
    {
        return Err("incomplete search cannot capture or update an alert baseline".into());
    }
    let count = run
        .file_results
        .iter()
        .filter(|r| r.status == FileSearchStatus::Hit)
        .count();
    if count > MAX_FILES {
        return Err("alert baseline exceeds the 2,000 matching file limit".into());
    }
    Ok(())
}

fn embed_results(results: &[AlertFile]) -> Result<Vec<BaselineFile>, String> {
    if results.len() > MAX_FILES {
        return Err("alert baseline exceeds the 2,000 matching file limit".into());
    }
    let mut records = Vec::with_capacity(results.len());
    for chunk in results.chunks(16) {
        let evidence = chunk.iter().map(|r| r.evidence.clone()).collect::<Vec<_>>();
        let fingerprints = evidence
            .iter()
            .map(|text| evidence_fingerprint(text))
            .collect::<Vec<_>>();
        let model_texts = evidence
            .iter()
            .map(|text| bounded_evidence(text))
            .collect::<Result<Vec<_>, _>>()?;
        let inputs = model_texts.iter().map(String::as_str).collect::<Vec<_>>();
        let embeddings = search_ml::embed_batch(&inputs).map_err(|e| e.to_string())?;
        for ((result, fingerprint), embedding) in chunk.iter().zip(fingerprints).zip(embeddings) {
            let quantized = quantize(embedding.values());
            records.push(BaselineFile {
                path: result.path.clone(),
                modified_ns: result.modified_ns,
                size: result.size,
                fingerprint,
                embedding: encode_embedding(&quantized),
            });
        }
    }
    Ok(records)
}

fn quantize(values: &[f32]) -> Vec<i8> {
    values
        .iter()
        .map(|value| (value * 127.0).round().clamp(-127.0, 127.0) as i8)
        .collect()
}

fn encode_embedding(values: &[i8]) -> String {
    let raw: Vec<u8> = values.iter().map(|value| *value as u8).collect();
    base64::engine::general_purpose::STANDARD.encode(raw)
}

fn bounded_evidence(evidence: &str) -> Result<String, String> {
    let mut text = evidence.to_string();
    let mut len = text.len();
    while search_ml::token_count(&text).map_err(|e| e.to_string())? > 224 {
        len /= 2;
        while !text.is_char_boundary(len) {
            len -= 1;
        }
        text.truncate(len);
    }
    Ok(text)
}

fn sample_evidence(result: &FileSearchResult) -> String {
    let mut text = String::new();
    for hit in sample_hits(&result.hits, 3) {
        for line in [
            hit.before.as_deref(),
            Some(hit.match_line.as_str()),
            hit.after.as_deref(),
        ]
        .into_iter()
        .flatten()
        {
            append_bounded_line(&mut text, line, 96);
        }
    }
    for index in sample_indices(result.lines_cache.len(), 6) {
        append_bounded_line(&mut text, &result.lines_cache[index], 72);
    }
    text
}

fn sample_indices(len: usize, limit: usize) -> Vec<usize> {
    if len == 0 {
        return Vec::new();
    }
    let count = len.min(limit);
    (0..count)
        .map(|i| {
            if count == 1 {
                0
            } else {
                i * (len - 1) / (count - 1)
            }
        })
        .collect()
}

fn sample_hits(hits: &[crate::models::LineHit], limit: usize) -> Vec<&crate::models::LineHit> {
    sample_indices(hits.len(), limit)
        .into_iter()
        .map(|i| &hits[i])
        .collect()
}

fn append_bounded_line(text: &mut String, line: &str, max_chars: usize) {
    if text.len() >= MAX_EVIDENCE_BYTES {
        return;
    }
    let mut end = line
        .char_indices()
        .nth(max_chars)
        .map_or(line.len(), |(offset, _)| offset)
        .min(MAX_EVIDENCE_BYTES.saturating_sub(text.len()));
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    let separator = usize::from(!text.is_empty());
    if text.len() + separator + end > MAX_EVIDENCE_BYTES {
        end = MAX_EVIDENCE_BYTES.saturating_sub(text.len() + separator);
        while !line.is_char_boundary(end) {
            end -= 1;
        }
    }
    if !text.is_empty() {
        text.push('\n');
    }
    text.push_str(&line[..end]);
}

fn evidence_fingerprint(text: &str) -> String {
    base64::engine::general_purpose::STANDARD.encode(Sha256::digest(text.as_bytes()))
}

fn decode_fingerprint(fingerprint: &str) -> Result<(), String> {
    let bytes = base64::engine::general_purpose::STANDARD
        .decode(fingerprint)
        .map_err(|e| e.to_string())?;
    if bytes.len() != 32 {
        return Err("saved alert has an invalid evidence fingerprint".into());
    }
    Ok(())
}

fn decode_embedding(encoded: &str) -> Result<Vec<i8>, String> {
    let raw = base64::engine::general_purpose::STANDARD
        .decode(encoded)
        .map_err(|e| e.to_string())?;
    let values: Vec<i8> = raw.into_iter().map(|value| value as i8).collect();
    let decoded: Vec<f32> = values
        .iter()
        .map(|value| f32::from(*value) / 127.0)
        .collect();
    let norm = decoded
        .iter()
        .map(|value| value * value)
        .sum::<f32>()
        .sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err("saved alert has a zero or invalid embedding".into());
    }
    search_ml::Embedding::from_values(decoded.into_iter().map(|value| value / norm).collect())
        .map_err(|_| "saved alert has an invalid embedding size".to_string())?;
    Ok(values)
}

fn cosine_distance(left: &[i8], right: &[i8]) -> Result<f32, String> {
    if left.len() != right.len() || left.is_empty() {
        return Err("embedding dimensions do not match".into());
    }
    let (mut dot, mut l2, mut r2) = (0f64, 0f64, 0f64);
    for (&a, &b) in left.iter().zip(right) {
        let (a, b) = (f64::from(a), f64::from(b));
        dot += a * b;
        l2 += a * a;
        r2 += b * b;
    }
    if l2 == 0.0 || r2 == 0.0 {
        return Err("saved alert has a zero embedding".into());
    }
    Ok((1.0 - dot / (l2.sqrt() * r2.sqrt())).clamp(0.0, 2.0) as f32)
}

fn modified_ns(result: &FileSearchResult) -> i64 {
    result.modified.timestamp_nanos_opt().unwrap_or_default()
}

#[derive(Serialize)]
struct Scope<'a> {
    search_path: &'a str,
    filters: &'a [String],
    filter_groups: &'a [crate::models::FilterGroup],
    exclude_filters: &'a [String],
    match_mode: crate::models::MatchMode,
    proximity_lines: i32,
    exclude_scope: crate::models::ExcludeScope,
    whole_word: bool,
    use_regex: bool,
    extensions: &'a Option<Vec<String>>,
    exclude_folders: &'a [String],
    include_hidden: bool,
    max_file_size_mb: f64,
    ocr_scanned_pdfs: bool,
}

fn scope_signature(settings: &SearchSettings) -> Result<String, String> {
    serde_json::to_string(&Scope {
        search_path: &settings.search_path,
        filters: &settings.filters,
        filter_groups: &settings.filter_groups,
        exclude_filters: &settings.exclude_filters,
        match_mode: settings.match_mode,
        proximity_lines: settings.proximity_lines,
        exclude_scope: settings.exclude_scope,
        whole_word: settings.whole_word,
        use_regex: settings.use_regex,
        extensions: &settings.extensions,
        exclude_folders: &settings.exclude_folders,
        include_hidden: settings.include_hidden,
        max_file_size_mb: settings.max_file_size_mb,
        ocr_scanned_pdfs: settings.ocr_scanned_pdfs,
    })
    .map_err(|e| e.to_string())
}

fn ensure_size(alert: &SavedSearchAlert) -> Result<(), String> {
    let bytes = serde_json::to_vec(alert).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_ALERT_BYTES {
        Err("alert snapshot exceeds the 2 MiB storage limit".into())
    } else {
        Ok(())
    }
}

fn temporary_path(path: &Path) -> PathBuf {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{}.{}.tmp", std::process::id(), seq));
    path.with_file_name(name)
}

fn new_alert_id() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let time = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    format!(
        "{time:032x}-{:016x}",
        NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{LineHit, SearchRunSummary};
    use chrono::Local;

    fn result(path: &str, text: &str, modified_ns: i64) -> FileSearchResult {
        FileSearchResult {
            full_name: path.into(),
            status: FileSearchStatus::Hit,
            hits: vec![LineHit {
                line_number: 4,
                before: Some("engineering analysis".into()),
                match_line: text.into(),
                after: Some("approved for review".into()),
                matched_filters: vec!["engineering".into()],
            }],
            created: Local::now(),
            modified: chrono::DateTime::from_timestamp(
                modified_ns.div_euclid(1_000_000_000),
                modified_ns.rem_euclid(1_000_000_000) as u32,
            )
            .unwrap()
            .with_timezone(&Local),
            file_length: 100,
            lines_cache: vec![text.into()],
            total_line_count: 1,
            proximity_min_range: None,
            low_confidence_pdf: false,
            error_message: None,
        }
    }

    fn run(results: Vec<FileSearchResult>) -> SearchRunResult {
        SearchRunResult {
            file_results: results,
            summary: SearchRunSummary::default(),
            ..Default::default()
        }
    }

    fn alert_run(results: Vec<FileSearchResult>) -> AlertRun {
        AlertRun::from_search_result(&run(results)).unwrap()
    }

    #[test]
    fn alerts_report_new_and_updated_exact_paths_with_neural_scores() {
        let settings = SearchSettings {
            search_path: "/records".into(),
            filters: vec!["engineering".into()],
            ..Default::default()
        };
        let mut alert =
            SavedSearchAlert::create("Engineering changes".into(), settings.clone()).unwrap();
        alert
            .capture_baseline(&alert_run(vec![result(
                "/records/a.txt",
                "fatigue crack propagated along upper flange",
                1_700_000_000_000_000_000,
            )]))
            .unwrap();

        let unchanged = alert_run(vec![result(
            "/records/a.txt",
            "fatigue crack propagated along upper flange",
            1_700_000_000_000_000_001,
        )]);
        let touched = alert.check_and_update(&settings, &unchanged).unwrap();
        assert!(touched.iter().any(
            |change| matches!(change, AlertChange::Updated { path, .. } if path == "/records/a.txt")
        ));
        assert!(alert
            .check_and_update(&settings, &unchanged)
            .unwrap()
            .is_empty());

        let changed = alert_run(vec![
            result(
                "/records/a.txt",
                "hydraulic pump pressure at 3000 psi causes leakage",
                1_700_000_000_000_000_002,
            ),
            result(
                "/records/b.txt",
                "new exact engineering record",
                1_700_000_000_000_000_000,
            ),
        ]);
        let changes = alert.check_and_update(&settings, &changed).unwrap();
        assert!(changes
            .iter()
            .any(|c| matches!(c, AlertChange::New { path } if path == "/records/b.txt")));
        assert!(changes
            .iter()
            .any(|c| matches!(c, AlertChange::Updated { path, .. } if path == "/records/a.txt")));
        assert!(alert
            .check_and_update(&settings, &changed)
            .unwrap()
            .is_empty());
    }

    #[test]
    fn sampled_document_changes_are_reported_even_when_match_line_is_unchanged() {
        let settings = SearchSettings {
            search_path: "/records".into(),
            filters: vec!["engineering".into()],
            ..Default::default()
        };
        let mut alert =
            SavedSearchAlert::create("Engineering changes".into(), settings.clone()).unwrap();
        let mut before = result(
            "/records/a.txt",
            "engineering report remains unchanged",
            1_700_000_000_000_000_000,
        );
        before.lines_cache = vec!["engineering report remains unchanged".into(); 20];
        before.lines_cache[19] = "maximum allowable stress is 100 MPa".into();
        alert.capture_baseline(&alert_run(vec![before])).unwrap();

        let mut after = result(
            "/records/a.txt",
            "engineering report remains unchanged",
            1_700_000_000_000_000_000,
        );
        after.lines_cache = vec!["engineering report remains unchanged".into(); 20];
        after.lines_cache[19] = "maximum allowable stress is 150 MPa".into();
        let changes = alert
            .check_and_update(&settings, &alert_run(vec![after]))
            .unwrap();
        assert!(changes.iter().any(
            |change| matches!(change, AlertChange::Updated { path, .. } if path == "/records/a.txt")
        ));
    }

    #[test]
    fn incomplete_or_changed_scope_cannot_update_alert_baseline() {
        let settings = SearchSettings {
            search_path: "/records".into(),
            filters: vec!["engineering".into()],
            ..Default::default()
        };
        let mut alert =
            SavedSearchAlert::create("Engineering changes".into(), settings.clone()).unwrap();
        let prior = alert_run(vec![result(
            "/records/a.txt",
            "fatigue report",
            1_700_000_000_000_000_000,
        )]);
        alert.capture_baseline(&prior).unwrap();
        let baseline = alert.baseline.clone();
        let mut partial = run(vec![]);
        partial.summary.enumeration_errors = 1;
        assert!(AlertRun::from_search_result(&partial).is_err());
        assert_eq!(alert.baseline, baseline);
        let changed_scope = SearchSettings {
            search_path: "/other".into(),
            ..settings.clone()
        };
        assert!(alert.check_and_update(&changed_scope, &prior).is_err());
    }

    #[test]
    fn sidecar_round_trips_and_rejects_path_traversal() {
        let dir = tempfile::tempdir().unwrap();
        let settings = SearchSettings {
            search_path: "/records".into(),
            filters: vec!["engineering".into()],
            ..Default::default()
        };
        let mut alert = SavedSearchAlert::create("Engineering changes".into(), settings).unwrap();
        alert
            .capture_baseline(&alert_run(vec![result(
                "/records/a.txt",
                "fatigue report",
                1_700_000_000_000_000_000,
            )]))
            .unwrap();
        let path = SavedSearchAlert::path_in(dir.path(), &alert.id).unwrap();
        alert.save(&path).unwrap();
        let loaded = SavedSearchAlert::load(&path).unwrap();
        assert_eq!(loaded.name, alert.name);
        assert_eq!(loaded.baseline, alert.baseline);
        assert!(SavedSearchAlert::path_in(dir.path(), "alert-../outside").is_err());
    }
}
