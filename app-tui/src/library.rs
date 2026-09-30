//! Generic user-added library infrastructure: import/export (JSON),
//! duplicate pruning, conflict detection, and free-form labels - shared by
//! every toolbox-local catalog that lets a user add/import their own
//! entries alongside a built-in list (Bushing Workbench's reamer catalog,
//! housing/bushing material catalog, and Bushing ID presets). One
//! implementation, reused rather than copy-pasted per catalog - see root
//! `CLAUDE.md`'s "avoid parallel implementations of the same... concept"
//! rule.
//!
//! Schema: a library file is `{ "items": [ { ...T's own fields...,
//! "labels": ["Preferred", ...] } ] }` - `labels` defaults to `[]` when
//! absent, so a plain "just the data, no labels" export from another tool
//! still imports cleanly. See `app-tui/data/*.sample.json` for concrete
//! examples per catalog.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

/// One library entry: the caller's own data type plus this crate's
/// (optional) free-form labels. `#[serde(flatten)]` on `item` is what makes
/// `T`'s fields appear as siblings of `labels` in the JSON, not nested
/// under an `"item"` key - so a hand-written or externally-exported file
/// (e.g. a plain reamer-catalog CSV-to-JSON conversion with no `labels`
/// key at all) still round-trips.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LibraryItem<T> {
    #[serde(flatten)]
    pub item: T,
    #[serde(default)]
    pub labels: Vec<String>,
}

impl<T> LibraryItem<T> {
    pub fn new(item: T) -> Self {
        Self { item, labels: Vec::new() }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct LibraryFile<T> {
    #[serde(default = "Vec::new")]
    items: Vec<LibraryItem<T>>,
}

/// Parses a library JSON document into its items. Returns a human-readable
/// error (shown directly in a toolbox's status/toast) rather than a raw
/// `serde_json::Error` - callers never need to know this is JSON at all.
pub fn import_json<T: DeserializeOwned>(text: &str) -> Result<Vec<LibraryItem<T>>, String> {
    let file: LibraryFile<T> = serde_json::from_str(text).map_err(|e| format!("Invalid library file: {e}"))?;
    Ok(file.items)
}

pub fn export_json<T: Serialize + Clone>(items: &[LibraryItem<T>]) -> String {
    let file = LibraryFile { items: items.to_vec() };
    serde_json::to_string_pretty(&file).unwrap_or_default()
}

/// One incoming item's classification against an existing catalog, keyed by
/// `key` (e.g. a reamer's size label, a material's name) - the caller
/// decides what "the same entry" means, since it varies per catalog.
pub enum ImportOutcome<T> {
    /// No existing entry shares this key - a brand new item, added as-is.
    Added(LibraryItem<T>),
    /// An existing entry shares the key AND has identical data - not a
    /// conflict, just a possible label addition. `new_labels` is already
    /// deduplicated against the existing entry's own labels; empty means
    /// the imported item was a byte-for-byte duplicate (data and labels
    /// both already present) and was silently dropped, per "detect and
    /// prune exact duplicates".
    LabelsMerged { existing_index: usize, new_labels: Vec<String> },
    /// An existing entry shares the key but the data itself differs - a
    /// real conflict. The caller must ask the user: keep existing, or
    /// overwrite with the imported entry (which replaces data AND labels).
    Conflict { existing_index: usize, incoming: LibraryItem<T> },
}

/// Classifies every incoming item against `existing` in one pass. Applying
/// the result: `Added` items get pushed, `LabelsMerged` extends the named
/// existing entry's `labels`, and `Conflict` items are queued for
/// interactive resolution (see `ConflictQueue` below) - nothing in
/// `existing` is mutated by this function itself.
pub fn classify_import<T: PartialEq, K: Eq>(existing: &[LibraryItem<T>], incoming: Vec<LibraryItem<T>>, key: impl Fn(&T) -> K) -> Vec<ImportOutcome<T>> {
    incoming
        .into_iter()
        .filter_map(|inc| match existing.iter().enumerate().find(|(_, e)| key(&e.item) == key(&inc.item)) {
            None => Some(ImportOutcome::Added(inc)),
            Some((idx, existing_entry)) if existing_entry.item == inc.item => {
                let new_labels: Vec<String> = inc.labels.into_iter().filter(|l| !existing_entry.labels.contains(l)).collect();
                if new_labels.is_empty() {
                    None // byte-for-byte duplicate (data + labels) - pruned
                } else {
                    Some(ImportOutcome::LabelsMerged { existing_index: idx, new_labels })
                }
            }
            Some((idx, _)) => Some(ImportOutcome::Conflict { existing_index: idx, incoming: inc }),
        })
        .collect()
}

/// A user's choice for one conflicting import - `Overwrite` replaces the
/// existing entry's data AND labels with the imported one's.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConflictResolution {
    KeepExisting,
    Overwrite,
}

/// Drives interactive resolution of the `Conflict` outcomes from a
/// `classify_import` call - a toolbox picker owns one of these while
/// conflicts remain unresolved, same "own state, own key routing" pattern
/// every other picker overlay in this crate uses (`ReamerPickerState`,
/// `MaterialPickerState`, ...).
pub struct ConflictQueue<T> {
    pending: Vec<(usize, LibraryItem<T>)>,
    cursor: usize,
}

impl<T: Clone> ConflictQueue<T> {
    pub fn new(outcomes: Vec<ImportOutcome<T>>, existing: &mut Vec<LibraryItem<T>>) -> (Self, usize, usize) {
        let mut added = 0;
        let mut labels_merged = 0;
        let mut pending = Vec::new();
        for outcome in outcomes {
            match outcome {
                ImportOutcome::Added(item) => {
                    existing.push(item);
                    added += 1;
                }
                ImportOutcome::LabelsMerged { existing_index, new_labels } => {
                    if let Some(e) = existing.get_mut(existing_index) {
                        e.labels.extend(new_labels);
                    }
                    labels_merged += 1;
                }
                ImportOutcome::Conflict { existing_index, incoming } => pending.push((existing_index, incoming)),
            }
        }
        (Self { pending, cursor: 0 }, added, labels_merged)
    }

    pub fn is_empty(&self) -> bool {
        self.cursor >= self.pending.len()
    }

    /// The conflict currently awaiting a decision: `(existing_index,
    /// &incoming)`.
    pub fn current(&self) -> Option<(usize, &LibraryItem<T>)> {
        self.pending.get(self.cursor).map(|(idx, item)| (*idx, item))
    }

    /// Resolves the current conflict and advances. `apply_to_all_remaining`
    /// resolves every not-yet-seen conflict the same way in one call - "[A]
    /// Apply to all remaining".
    pub fn resolve(&mut self, existing: &mut [LibraryItem<T>], resolution: ConflictResolution, apply_to_all_remaining: bool) {
        let end = if apply_to_all_remaining { self.pending.len() } else { self.cursor + 1 };
        for (idx, incoming) in &self.pending[self.cursor..end] {
            if resolution == ConflictResolution::Overwrite {
                if let Some(e) = existing.get_mut(*idx) {
                    *e = (*incoming).clone();
                }
            }
        }
        self.cursor = end;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Deserialize, Serialize};

    #[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
    struct Widget {
        name: String,
        value: f64,
    }

    fn item(name: &str, value: f64, labels: &[&str]) -> LibraryItem<Widget> {
        LibraryItem { item: Widget { name: name.to_string(), value }, labels: labels.iter().map(|s| s.to_string()).collect() }
    }

    #[test]
    fn import_export_round_trips() {
        let items = vec![item("a", 1.0, &["Preferred"]), item("b", 2.0, &[])];
        let json = export_json(&items);
        let parsed: Vec<LibraryItem<Widget>> = import_json(&json).unwrap();
        assert_eq!(parsed, items);
    }

    #[test]
    fn import_json_missing_labels_key_defaults_to_empty() {
        let json = r#"{ "items": [ { "name": "a", "value": 1.0 } ] }"#;
        let parsed: Vec<LibraryItem<Widget>> = import_json(&json).unwrap();
        assert_eq!(parsed, vec![item("a", 1.0, &[])]);
    }

    #[test]
    fn import_json_rejects_malformed_input_with_a_readable_error() {
        let err = import_json::<Widget>("not json").unwrap_err();
        assert!(err.contains("Invalid library file"));
    }

    #[test]
    fn classify_import_adds_a_brand_new_key() {
        let existing = vec![item("a", 1.0, &[])];
        let outcomes = classify_import(&existing, vec![item("b", 2.0, &[])], |w| w.name.clone());
        assert!(matches!(outcomes.as_slice(), [ImportOutcome::Added(w)] if w.item.name == "b"));
    }

    #[test]
    fn classify_import_prunes_a_byte_for_byte_duplicate() {
        let existing = vec![item("a", 1.0, &["Preferred"])];
        let outcomes = classify_import(&existing, vec![item("a", 1.0, &["Preferred"])], |w| w.name.clone());
        assert!(outcomes.is_empty(), "an exact duplicate (same data, same labels) must be silently pruned");
    }

    #[test]
    fn classify_import_merges_new_labels_on_matching_data() {
        let existing = vec![item("a", 1.0, &["Preferred"])];
        let outcomes = classify_import(&existing, vec![item("a", 1.0, &["Preferred", "Custom"])], |w| w.name.clone());
        match outcomes.as_slice() {
            [ImportOutcome::LabelsMerged { existing_index: 0, new_labels }] => assert_eq!(new_labels, &["Custom".to_string()]),
            other => panic!("expected a LabelsMerged outcome, got {other:?}", other = other.len()),
        }
    }

    #[test]
    fn classify_import_flags_a_real_conflict_on_same_key_different_data() {
        let existing = vec![item("a", 1.0, &[])];
        let outcomes = classify_import(&existing, vec![item("a", 999.0, &[])], |w| w.name.clone());
        assert!(matches!(outcomes.as_slice(), [ImportOutcome::Conflict { existing_index: 0, incoming }] if incoming.item.value == 999.0));
    }

    #[test]
    fn conflict_queue_applies_added_and_merged_outcomes_immediately() {
        let mut existing = vec![item("a", 1.0, &[])];
        let outcomes = vec![ImportOutcome::Added(item("b", 2.0, &[])), ImportOutcome::LabelsMerged { existing_index: 0, new_labels: vec!["Preferred".to_string()] }];
        let (queue, added, merged) = ConflictQueue::new(outcomes, &mut existing);
        assert_eq!(added, 1);
        assert_eq!(merged, 1);
        assert!(queue.is_empty());
        assert_eq!(existing.len(), 2);
        assert_eq!(existing[0].labels, vec!["Preferred".to_string()]);
    }

    #[test]
    fn keep_existing_leaves_data_untouched() {
        let mut existing = vec![item("a", 1.0, &[])];
        let outcomes = vec![ImportOutcome::Conflict { existing_index: 0, incoming: item("a", 999.0, &["Custom"]) }];
        let (mut queue, _, _) = ConflictQueue::new(outcomes, &mut existing);
        assert!(!queue.is_empty());
        queue.resolve(&mut existing, ConflictResolution::KeepExisting, false);
        assert!(queue.is_empty());
        assert_eq!(existing[0].item.value, 1.0);
    }

    #[test]
    fn overwrite_replaces_data_and_labels() {
        let mut existing = vec![item("a", 1.0, &[])];
        let outcomes = vec![ImportOutcome::Conflict { existing_index: 0, incoming: item("a", 999.0, &["Custom"]) }];
        let (mut queue, _, _) = ConflictQueue::new(outcomes, &mut existing);
        queue.resolve(&mut existing, ConflictResolution::Overwrite, false);
        assert_eq!(existing[0].item.value, 999.0);
        assert_eq!(existing[0].labels, vec!["Custom".to_string()]);
    }

    #[test]
    fn apply_to_all_remaining_resolves_every_pending_conflict_the_same_way() {
        let mut existing = vec![item("a", 1.0, &[]), item("b", 1.0, &[])];
        let outcomes = vec![
            ImportOutcome::Conflict { existing_index: 0, incoming: item("a", 111.0, &[]) },
            ImportOutcome::Conflict { existing_index: 1, incoming: item("b", 222.0, &[]) },
        ];
        let (mut queue, _, _) = ConflictQueue::new(outcomes, &mut existing);
        queue.resolve(&mut existing, ConflictResolution::Overwrite, true);
        assert!(queue.is_empty());
        assert_eq!(existing[0].item.value, 111.0);
        assert_eq!(existing[1].item.value, 222.0);
    }
}
