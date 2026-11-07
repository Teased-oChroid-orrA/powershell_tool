use search_core::relevance::{FeedbackLabel, RelevanceFeedback};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

const VERSION: u32 = 1;
const MAX_FILE_BYTES: u64 = 64 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
struct FeedbackFile {
    version: u32,
    enabled: bool,
    examples: Vec<FeedbackRecord>,
}

impl Default for FeedbackFile {
    fn default() -> Self {
        Self {
            version: VERSION,
            enabled: false,
            examples: Vec::new(),
        }
    }
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Vote {
    Relevant,
    NotRelevant,
}

#[derive(Clone, Copy, Debug, Serialize, Deserialize)]
struct FeedbackRecord {
    vote: Vote,
    match_text_similarity: f32,
    filename_similarity: f32,
    exact_evidence: f32,
    proximity: Option<f32>,
    recency: f32,
}

impl FeedbackRecord {
    fn from_core(example: RelevanceFeedback) -> Self {
        Self {
            vote: match example.label {
                FeedbackLabel::Relevant => Vote::Relevant,
                FeedbackLabel::NotRelevant => Vote::NotRelevant,
            },
            match_text_similarity: example.match_text_similarity,
            filename_similarity: example.filename_similarity,
            exact_evidence: example.exact_evidence,
            proximity: example.proximity,
            recency: example.recency,
        }
    }

    fn into_core(self) -> RelevanceFeedback {
        RelevanceFeedback {
            label: match self.vote {
                Vote::Relevant => FeedbackLabel::Relevant,
                Vote::NotRelevant => FeedbackLabel::NotRelevant,
            },
            match_text_similarity: self.match_text_similarity,
            filename_similarity: self.filename_similarity,
            exact_evidence: self.exact_evidence,
            proximity: self.proximity,
            recency: self.recency,
        }
    }

    fn valid(self) -> bool {
        [
            Some(self.match_text_similarity),
            Some(self.filename_similarity),
            Some(self.exact_evidence),
            self.proximity,
            Some(self.recency),
        ]
        .into_iter()
        .flatten()
        .all(|value| value.is_finite() && (0.0..=1.0).contains(&value))
    }
}

type Cache = std::collections::HashMap<PathBuf, FeedbackFile>;
static CACHE: OnceLock<Mutex<Cache>> = OnceLock::new();

fn cache() -> Result<std::sync::MutexGuard<'static, Cache>, String> {
    CACHE
        .get_or_init(|| Mutex::new(Cache::new()))
        .lock()
        .map_err(|_| "relevance feedback lock is poisoned".into())
}

fn read(path: &Path) -> Result<FeedbackFile, String> {
    let bytes = match std::fs::read(path) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(FeedbackFile::default())
        }
        Err(error) => return Err(error.to_string()),
    };
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("relevance feedback file exceeds 64 KiB".into());
    }
    let file: FeedbackFile = serde_json::from_slice(&bytes).map_err(|error| error.to_string())?;
    if file.version != VERSION
        || file.examples.len() > search_core::relevance::MAX_FEEDBACK_EXAMPLES
        || file.examples.iter().any(|example| !example.valid())
    {
        return Err("relevance feedback file has unsupported or invalid data".into());
    }
    Ok(file)
}

fn snapshot_locked(path: &Path, cache: &mut Cache) -> Result<FeedbackFile, String> {
    if !cache.contains_key(path) {
        cache.insert(path.to_path_buf(), read(path)?);
    }
    Ok(cache.get(path).expect("feedback was loaded").clone())
}

fn save(path: &Path, file: &FeedbackFile) -> Result<(), String> {
    let bytes = serde_json::to_vec_pretty(file).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("relevance feedback file would exceed 64 KiB".into());
    }
    crate::persistence::write(path, bytes).map_err(|error| error.to_string())
}

pub fn enabled(path: &Path) -> Result<bool, String> {
    let mut cache = cache()?;
    Ok(snapshot_locked(path, &mut cache)?.enabled)
}

pub fn snapshot(path: &Path) -> Result<(bool, Vec<RelevanceFeedback>), String> {
    let mut cache = cache()?;
    let file = snapshot_locked(path, &mut cache)?;
    Ok((
        file.enabled,
        file.examples
            .into_iter()
            .map(FeedbackRecord::into_core)
            .collect(),
    ))
}

pub fn set_enabled(path: &Path, enabled: bool) -> Result<(), String> {
    let mut cache = cache()?;
    let mut file = snapshot_locked(path, &mut cache)?;
    file.enabled = enabled;
    save(path, &file)?;
    cache.insert(path.to_path_buf(), file);
    Ok(())
}

pub fn record(path: &Path, example: RelevanceFeedback) -> Result<(), String> {
    if !FeedbackRecord::from_core(example).valid() {
        return Err("relevance feedback values must be finite and between 0 and 1".into());
    }
    let mut cache = cache()?;
    let mut file = snapshot_locked(path, &mut cache)?;
    if !file.enabled {
        return Err("enable local relevance feedback before saving a judgment".into());
    }
    file.examples.push(FeedbackRecord::from_core(example));
    let excess = file
        .examples
        .len()
        .saturating_sub(search_core::relevance::MAX_FEEDBACK_EXAMPLES);
    file.examples.drain(..excess);
    save(path, &file)?;
    cache.insert(path.to_path_buf(), file);
    Ok(())
}

pub fn reset(path: &Path) -> Result<(), String> {
    let mut cache = cache()?;
    let file = FeedbackFile::default();
    save(path, &file)?;
    cache.insert(path.to_path_buf(), file);
    let backup = path.with_extension("previous.json");
    match std::fs::remove_file(backup) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.to_string()),
    }
    Ok(())
}

pub fn export(path: &Path, destination: &Path) -> Result<(), String> {
    if path == destination {
        return Err("choose a separate file for feedback export".into());
    }
    let mut cache = cache()?;
    let file = snapshot_locked(path, &mut cache)?;
    let bytes = serde_json::to_vec_pretty(&file).map_err(|error| error.to_string())?;
    if bytes.len() as u64 > MAX_FILE_BYTES {
        return Err("relevance feedback export would exceed 64 KiB".into());
    }
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(destination)
        .map_err(|error| error.to_string())?;
    use std::io::Write;
    if let Err(error) = output.write_all(&bytes).and_then(|()| output.sync_all()) {
        let _ = std::fs::remove_file(destination);
        return Err(error.to_string());
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn example(label: FeedbackLabel) -> RelevanceFeedback {
        RelevanceFeedback {
            label,
            match_text_similarity: 0.8,
            filename_similarity: 0.6,
            exact_evidence: 0.5,
            proximity: None,
            recency: 0.4,
        }
    }

    #[test]
    fn feedback_is_opt_in_bounded_persisted_reset_and_exported_without_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search-feedback.json");
        let destination = dir.path().join("feedback-export.json");
        assert!(!enabled(&path).unwrap());
        assert!(record(&path, example(FeedbackLabel::Relevant)).is_err());
        set_enabled(&path, true).unwrap();
        record(&path, example(FeedbackLabel::Relevant)).unwrap();
        assert_eq!(snapshot(&path).unwrap().1.len(), 1);
        export(&path, &destination).unwrap();
        assert!(
            export(&path, &destination).is_err(),
            "export never overwrites a user file"
        );
        reset(&path).unwrap();
        assert!(!enabled(&path).unwrap());
        assert!(snapshot(&path).unwrap().1.is_empty());
        assert!(
            destination.exists(),
            "reset cannot remove copies the user exported"
        );
    }

    #[test]
    fn feedback_rejects_oversized_persisted_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search-feedback.json");
        std::fs::write(&path, vec![b' '; MAX_FILE_BYTES as usize + 1]).unwrap();
        assert!(enabled(&path).unwrap_err().contains("64 KiB"));
    }

    #[test]
    fn saved_feedback_is_bounded_and_contains_no_query_or_document_identity() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("search-feedback.json");
        set_enabled(&path, true).unwrap();
        for _ in 0..search_core::relevance::MAX_FEEDBACK_EXAMPLES + 1 {
            record(&path, example(FeedbackLabel::Relevant)).unwrap();
        }
        assert_eq!(
            snapshot(&path).unwrap().1.len(),
            search_core::relevance::MAX_FEEDBACK_EXAMPLES
        );
        let saved = std::fs::read_to_string(path).unwrap();
        assert!(
            !saved.contains("\"query\"")
                && !saved.contains("\"path\"")
                && !saved.contains("\"filename\"")
        );
    }
}
