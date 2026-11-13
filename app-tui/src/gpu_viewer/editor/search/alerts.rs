use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::persistence::PersistedSearchSettings;

const VERSION: u32 = 1;
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub(super) struct Record {
    version: u32,
    pub alert: search_core::alerts::SavedSearchAlert,
    pub ui_settings: PersistedSearchSettings,
}

impl Record {
    pub fn new(
        alert: search_core::alerts::SavedSearchAlert,
        ui_settings: PersistedSearchSettings,
    ) -> Self {
        Self {
            version: VERSION,
            alert,
            ui_settings,
        }
    }

    fn validate(&self) -> Result<(), String> {
        if self.version != VERSION {
            return Err("unsupported Search alert record version".into());
        }
        self.alert.validate()
    }
}

pub fn directory(settings_file: &Path) -> PathBuf {
    settings_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("search-alerts")
}

pub fn load_all(directory: &Path) -> (Vec<Record>, Vec<String>) {
    let mut records = Vec::new();
    let mut errors = Vec::new();
    let entries = match std::fs::read_dir(directory) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return (records, errors),
        Err(error) => return (records, vec![error.to_string()]),
    };
    let mut files = entries
        .filter_map(Result::ok)
        .map(|entry| entry.path())
        .filter(|path| {
            path.extension()
                .is_some_and(|extension| extension == "json")
        })
        .collect::<Vec<_>>();
    files.sort();
    if files.len() > search_core::alerts::MAX_SAVED_ALERTS {
        return (
            records,
            vec![format!(
                "{} alert files found; limit is {}. Files were left untouched.",
                files.len(),
                search_core::alerts::MAX_SAVED_ALERTS
            )],
        );
    }
    for path in files {
        let result: Result<Record, String> = (|| {
            let size = std::fs::metadata(&path).map_err(|e| e.to_string())?.len();
            if size > MAX_BYTES {
                return Err("alert file exceeds 2 MiB".into());
            }
            let record: Record =
                serde_json::from_slice(&std::fs::read(&path).map_err(|e| e.to_string())?)
                    .map_err(|e| e.to_string())?;
            record.validate()?;
            let expected =
                search_core::alerts::SavedSearchAlert::path_in(directory, &record.alert.id)?;
            if expected != path {
                return Err("alert id does not match its filename".into());
            }
            Ok(record)
        })();
        match result {
            Ok(record) => records.push(record),
            Err(error) => errors.push(format!("{}: {error}", path.display())),
        }
    }
    (records, errors)
}

pub fn save(directory: &Path, record: &Record) -> Result<(), String> {
    record.validate()?;
    let path = search_core::alerts::SavedSearchAlert::path_in(directory, &record.alert.id)?;
    let bytes = serde_json::to_vec(record).map_err(|e| e.to_string())?;
    if bytes.len() as u64 > MAX_BYTES {
        return Err("alert file exceeds 2 MiB".into());
    }
    std::fs::create_dir_all(directory).map_err(|e| e.to_string())?;
    let temp = path.with_extension(format!("{}.tmp", std::process::id()));
    std::fs::write(&temp, bytes).map_err(|e| e.to_string())?;
    if let Err(error) = std::fs::rename(&temp, &path) {
        let _ = std::fs::remove_file(temp);
        return Err(error.to_string());
    }
    Ok(())
}

pub fn remove(directory: &Path, id: &str) -> Result<(), String> {
    let path = search_core::alerts::SavedSearchAlert::path_in(directory, id)?;
    std::fs::remove_file(path).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::search::{model::SearchToolConfig, persistence::PersistedSearchSettings};

    fn sample_record() -> Record {
        let config = SearchToolConfig {
            search_path: "/tmp/records".into(),
            filters_text: "fatigue".into(),
            search_paths_extra: vec!["/tmp/archive".into()],
            ..Default::default()
        };
        let alert = search_core::alerts::SavedSearchAlert::create(
            "fatigue".into(),
            crate::toolboxes::search::model::build_settings(&config),
        )
        .unwrap();
        Record::new(alert, PersistedSearchSettings::from(&config))
    }

    #[test]
    fn sidecar_saves_replaces_loads_and_removes_with_ui_settings() {
        let temp = tempfile::tempdir().unwrap();
        let mut record = sample_record();
        save(temp.path(), &record).unwrap();
        record.ui_settings.output_folder = "/tmp/reports".into();
        save(temp.path(), &record).unwrap();

        let (loaded, errors) = load_all(temp.path());
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded[0].alert.id, record.alert.id);
        assert_eq!(loaded[0].ui_settings.search_paths_extra, ["/tmp/archive"]);
        assert_eq!(loaded[0].ui_settings.output_folder, "/tmp/reports");

        remove(temp.path(), &record.alert.id).unwrap();
        assert!(load_all(temp.path()).0.is_empty());
    }
}
