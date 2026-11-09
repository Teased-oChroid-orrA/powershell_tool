//! Natural-language Find: a sentence becomes removable conditions (rules, `search_core::nl_rules`), then
//! `search_core::nl_find` retrieves and judges per condition. Nothing is generated; every match shows a cited passage
//! and a quote copied from it. The search runs on a blocking thread; results stream in through `Event`.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use search_core::nl_find::{self, Condition, Evidence, FileMatch};
use tokio::sync::mpsc;

use super::{act, file_name, semantic_search_roots, SearchAct, SearchSession};
use crate::gpu_viewer::editor::session::Tone;
use crate::gpu_viewer::menu::Entry;
use crate::gpu_viewer::ui::DockItem;

pub(super) enum Event {
    File {
        id: u64,
        file: FileMatch,
    },
    Done {
        id: u64,
        result: Result<Vec<FileMatch>, String>,
    },
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Stage {
    Idle,
    Searching,
}

pub(super) struct NlFind {
    pub sentence: String,
    /// The conditions the user can still edit; `None` until the sentence has been understood.
    pub conditions: Option<Vec<Condition>>,
    pub stage: Stage,
    pub files: Vec<FileMatch>,
    pub error: Option<String>,
    /// Bumped by every new request and by edits that make running work stale.
    generation: u64,
    cancel: Arc<AtomicBool>,
    tx: mpsc::UnboundedSender<Event>,
    rx: mpsc::UnboundedReceiver<Event>,
}

impl NlFind {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self {
            sentence: String::new(),
            conditions: None,
            stage: Stage::Idle,
            files: Vec::new(),
            error: None,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
        }
    }

    /// Forget results and running work (the sentence changed).
    pub fn reset(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation += 1;
        self.conditions = None;
        self.files.clear();
        self.error = None;
        self.stage = Stage::Idle;
    }
}

/// A search box entry written as a sentence ("surfaces that were blended and filled"), not as filters:
/// several words, a connecting word, and none of the filter syntax (`,` `;` `[` `(` `"`).
pub(super) fn looks_like_sentence(text: &str) -> bool {
    const CONNECTIVES: &[&str] = &[
        "that", "which", "where", "were", "was", "and", "but", "with", "not", "have", "has", "had",
    ];
    if text.contains([',', ';', '[', ']', '(', ')', '"']) {
        return false;
    }
    let words: Vec<String> = text
        .split_whitespace()
        .map(|w| w.to_lowercase())
        .collect();
    words.len() >= 5 && words.iter().any(|w| CONNECTIVES.contains(&w.as_str()))
}

impl SearchSession {
    /// Whether a sentence typed in the search box can be understood now (the semantic index is on).
    pub(super) fn nl_ready(&self) -> bool {
        self.st.config.index.semantic && self.nl.stage == Stage::Idle
    }

    /// Read the sentence as conditions (instant: rules, no model).
    pub(super) fn nl_understand(&mut self) {
        let sentence = self.nl.sentence.trim().to_string();
        if sentence.is_empty() {
            self.status = "Describe what to find first.".into();
            return;
        }
        if !self.st.config.index.semantic {
            self.status = "Enable Semantic index in the Index menu, then build the index.".into();
            return;
        }
        if self.nl.stage != Stage::Idle {
            self.status = "Wait for the current step to finish.".into();
            return;
        }
        self.nl.reset();
        match nl_find::parse(&sentence) {
            Ok(conditions) => {
                self.status = "Check the conditions, remove any that are wrong, then press Find.".into();
                self.nl.conditions = Some(conditions);
            }
            Err(error) => self.nl.error = Some(error),
        }
    }

    pub(super) fn nl_remove(&mut self, index: usize) {
        if self.nl.stage != Stage::Idle {
            return;
        }
        if let Some(list) = self
            .nl
            .conditions
            .as_mut()
            .filter(|list| index < list.len())
        {
            list.remove(index);
            self.nl.files.clear();
        }
    }

    /// Join condition `index` with the next one (for a condition the rules split in two).
    pub(super) fn nl_merge(&mut self, index: usize) {
        if self.nl.stage != Stage::Idle {
            return;
        }
        let Some(list) = self
            .nl
            .conditions
            .as_mut()
            .filter(|list| index + 1 < list.len())
        else {
            return;
        };
        if list[index].negated != list[index + 1].negated {
            self.status =
                "Only conditions that are both wanted, or both unwanted, can be merged.".into();
            return;
        }
        let next = list.remove(index + 1);
        let first = &mut list[index];
        first.text = format!("{} {}", first.text, next.text);
        for term in next.terms {
            if !first.terms.contains(&term) {
                first.terms.push(term);
            }
        }
        self.nl.files.clear();
    }

    pub(super) fn nl_find(&mut self) {
        if self.nl.stage != Stage::Idle {
            return;
        }
        let Some(conditions) = self.nl.conditions.clone().filter(|list| !list.is_empty()) else {
            self.status = "Keep at least one condition.".into();
            return;
        };
        let roots = semantic_search_roots(&self.st.config);
        if roots.is_empty() {
            self.status = "Choose a folder before searching.".into();
            return;
        }
        self.nl.files.clear();
        self.nl.error = None;
        self.nl.stage = Stage::Searching;
        self.nl.cancel = Arc::new(AtomicBool::new(false));
        let (id, cancel, tx) = (
            self.nl.generation,
            self.nl.cancel.clone(),
            self.nl.tx.clone(),
        );
        let done_tx = tx.clone();
        self.status = "Checking the best candidates against each condition…".into();
        self.rt.spawn(async move {
            let result = tokio::task::spawn_blocking(move || {
                let retrieve = |query: &str, limit: usize| {
                    let mut hits = Vec::new();
                    let mut errors = Vec::new();
                    let mut indexed = 0usize;
                    for (root, settings) in &roots {
                        match search_core::semantic_index::search_directory_with_settings(
                            root, query, limit, settings,
                        ) {
                            Ok(mut found) => {
                                indexed += 1;
                                hits.append(&mut found);
                            }
                            Err(error) => errors.push(error),
                        }
                    }
                    if indexed == 0 {
                        return Err(errors.join("; "));
                    }
                    if roots.len() > 1 {
                        // One index's hits arrive in keyword-fused order, which cosine order would undo.
                        hits.sort_by(|a, b| {
                            b.similarity
                                .total_cmp(&a.similarity)
                                .then_with(|| a.path.cmp(&b.path))
                        });
                    }
                    hits.truncate(limit);
                    Ok(hits)
                };
                nl_find::find(&conditions, retrieve, &cancel, |file| {
                    let _ = tx.send(Event::File {
                        id,
                        file: file.clone(),
                    });
                })
                .map_err(|error| error.to_string())
            })
            .await
            .unwrap_or_else(|error| Err(format!("search task failed: {error}")));
            let _ = done_tx.send(Event::Done { id, result });
        });
    }

    pub(super) fn nl_cancel(&mut self) {
        self.nl.cancel.store(true, Ordering::Relaxed);
    }

    pub(super) fn nl_open(&mut self, file: usize, condition: usize) {
        let Some(evidence) = self.nl.files.get(file).and_then(|f| {
            f.evidence
                .get(condition)
                .and_then(Option::as_ref)
                .or_else(|| f.evidence.iter().flatten().next())
        }) else {
            return;
        };
        let path = evidence.hit.path.clone();
        self.status = format!(
            "Opening {} lines {}–{}.",
            file_name(&path),
            evidence.hit.first_line,
            evidence.hit.last_line
        );
        self.execute(vec![crate::app::Effect::OpenPath(path)]);
    }

    /// Fold finished background work in; true when something changed.
    pub(super) fn poll_nl(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.nl.rx.try_recv() {
            match event {
                Event::File { id, file } if id == self.nl.generation => {
                    self.nl.files.push(file);
                    changed = true;
                }
                Event::Done { id, result } if id == self.nl.generation => {
                    self.nl.stage = Stage::Idle;
                    match result {
                        Ok(files) => {
                            self.status = if files.is_empty() {
                                "No file had a passage stating every condition.".into()
                            } else {
                                format!("{} files with matching passages; open each citation to verify.", files.len())
                            };
                            self.nl.files = files;
                        }
                        Err(error) => self.nl.error = Some(error),
                    }
                    changed = true;
                }
                _ => {}
            }
        }
        changed
    }
}

fn quote_line(evidence: &Evidence) -> String {
    format!(
        "“{}” · {}",
        evidence.quote.trim(),
        super::view::semantic_citation(&evidence.hit)
    )
}

/// The sentence's conditions as pills: click one to remove it, click the + between two to merge them.
pub(super) fn chips(s: &SearchSession) -> Vec<Entry> {
    let Some(conditions) = &s.nl.conditions else {
        return Vec::new();
    };
    let locked = s.nl.stage != Stage::Idle;
    let mut chips = Vec::new();
    for (i, condition) in conditions.iter().enumerate() {
        let mark = if condition.negated { "not " } else { "" };
        let entry = Entry::new(
            format!("{mark}{}  ×", condition.text),
            "click to remove",
            act(SearchAct::NlRemove(i)),
        )
        .checked(true);
        chips.push(entry.when(if locked {
            Err("wait for the current step".to_string())
        } else {
            Ok(())
        }));
        if i + 1 < conditions.len() && condition.negated == conditions[i + 1].negated {
            chips.push(
                Entry::new(
                    "+",
                    "merge with the next condition",
                    act(SearchAct::NlMerge(i)),
                )
                .when(if locked {
                    Err("wait for the current step".to_string())
                } else {
                    Ok(())
                }),
            );
        }
    }
    chips
}

/// The card's rows: the sentence's conditions as removable chips, then the files with their evidence.
pub(super) fn items(s: &SearchSession) -> Vec<DockItem> {
    let nl = &s.nl;
    let mut items = Vec::new();
    if let Some(error) = &nl.error {
        items.push(DockItem::text(error.clone(), Tone::Warn));
    }
    match nl.stage {
        Stage::Searching => {
            items.push(DockItem::text(
                format!(
                    "Checking candidates… {} files confirmed so far",
                    nl.files.len()
                ),
                Tone::Dim,
            ));
            items.push(DockItem::row(
                "Stop",
                Tone::Normal,
                Some(act(SearchAct::NlCancel)),
                false,
            ));
        }
        Stage::Idle => {}
    }
    if nl.conditions.as_ref().is_some_and(|list| !list.is_empty()) && nl.stage == Stage::Idle {
        items.push(DockItem::row(
            "Find with these conditions",
            Tone::Title,
            Some(act(SearchAct::NlFind)),
            false,
        ));
    }
    if !nl.files.is_empty() {
        let wanted: Vec<&Condition> = nl
            .conditions
            .iter()
            .flatten()
            .filter(|c| !c.negated)
            .collect();
        let unconfirmed = nl.files.iter().all(|file| file.fit == nl_find::Fit::Unconfirmed);
        if unconfirmed {
            items.push(DockItem::Heading("Closest passages (not confirmed)".into()));
            items.push(DockItem::text(
                "No passage states every condition. These are the best matches by meaning and wording; read each in context.",
                Tone::Warn,
            ));
        } else {
            items.push(DockItem::Heading("Files with matching passages".into()));
            items.push(DockItem::text(
                "Each quote is copied from the cited passage. Open the source to confirm.",
                Tone::Dim,
            ));
        }
        for (file_index, file) in nl.files.iter().enumerate() {
            items.push(DockItem::row(
                format!("{} · {}", file_name(&file.path), file.fit.label()),
                Tone::Normal,
                Some(act(SearchAct::NlOpen(file_index, 0))),
                false,
            ));
            for (condition_index, evidence) in file.evidence.iter().enumerate() {
                let name = wanted.get(condition_index).map_or("", |c| c.text.as_str());
                match evidence {
                    Some(evidence) => items.push(DockItem::row(
                        format!("   {}  {name}: {}", if unconfirmed { "?" } else { "ok" }, quote_line(evidence)),
                        Tone::Dim,
                        Some(act(SearchAct::NlOpen(file_index, condition_index))),
                        false,
                    )),
                    None => items.push(DockItem::text(
                        format!("   --  {name}: not confirmed"),
                        Tone::Warn,
                    )),
                }
            }
        }
    }
    items
}
