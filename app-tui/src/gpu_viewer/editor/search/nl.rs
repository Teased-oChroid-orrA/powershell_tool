//! Natural-language Find: a sentence becomes removable conditions (local language model), then `search_core::nl_find`
//! retrieves and judges per condition. The model never writes answers; every match shows a cited passage and a quote.
//! Loading the model and every model call run on blocking threads; results stream in through `Event`.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use search_core::nl_find::{self, Evidence, FileMatch};
use search_llm::{Condition, ConditionModel};
use tokio::sync::mpsc;

use super::{act, file_name, semantic_search_roots, SearchAct, SearchSession};
use crate::gpu_viewer::editor::session::Tone;
use crate::gpu_viewer::menu::Entry;
use crate::gpu_viewer::ui::DockItem;

pub(super) type Model = Arc<dyn ConditionModel + Send + Sync>;
pub(super) type Loader = fn(&Path) -> Result<Model, String>;

pub(super) fn load_llama(path: &Path) -> Result<Model, String> {
    search_llm::LlamaEngine::load(path)
        .map(|engine| Arc::new(engine) as Model)
        .map_err(|error| error.to_string())
}

pub(super) enum Event {
    Parsed {
        id: u64,
        loaded: Option<(PathBuf, Model)>,
        result: Result<Vec<Condition>, String>,
    },
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
    Parsing,
    Searching,
}

pub(super) struct NlFind {
    pub sentence: String,
    /// The conditions the user can still edit; `None` until the sentence has been understood.
    pub conditions: Option<Vec<Condition>>,
    pub stage: Stage,
    pub files: Vec<FileMatch>,
    pub error: Option<String>,
    model: Option<(PathBuf, Model)>,
    loader: Loader,
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
            model: None,
            loader: load_llama,
            generation: 0,
            cancel: Arc::new(AtomicBool::new(false)),
            tx,
            rx,
        }
    }

    #[cfg(test)]
    pub fn with_loader(loader: Loader) -> Self {
        Self {
            loader,
            ..Self::new()
        }
    }

    /// Forget results and running work (the sentence or the model changed).
    pub fn reset(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        self.generation += 1;
        self.conditions = None;
        self.files.clear();
        self.error = None;
        self.stage = Stage::Idle;
    }

    pub fn forget_model(&mut self) {
        self.model = None;
        self.reset();
    }
}

/// What the user sees for a missing model: where it was looked for and what to do.
fn model_problem(path: &str) -> Option<String> {
    let path = path.trim();
    if path.is_empty() {
        Some("No language model set. Enter the location of a Qwen3-4B GGUF file below.".into())
    } else if !Path::new(path).is_file() {
        Some(format!(
            "Language model not found: {path}. Check the location below."
        ))
    } else {
        None
    }
}

impl SearchSession {
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
        if let Some(problem) = model_problem(&self.st.config.llm_model_path) {
            self.nl.error = Some(problem);
            return;
        }
        if self.nl.stage != Stage::Idle {
            self.status = "Wait for the current step to finish.".into();
            return;
        }
        self.nl.reset();
        self.nl.stage = Stage::Parsing;
        let id = self.nl.generation;
        let path = PathBuf::from(self.st.config.llm_model_path.trim());
        let cached = self
            .nl
            .model
            .as_ref()
            .filter(|(p, _)| *p == path)
            .map(|(_, m)| m.clone());
        let (loader, tx) = (self.nl.loader, self.nl.tx.clone());
        self.status = "Understanding your sentence…".into();
        self.rt.spawn(async move {
            let outcome = tokio::task::spawn_blocking(move || {
                let (model, loaded) = match cached {
                    Some(model) => (model, None),
                    None => {
                        let model = loader(&path)?;
                        (model.clone(), Some((path, model)))
                    }
                };
                let conditions = model.parse(&sentence)?;
                Ok::<_, String>((loaded, conditions))
            })
            .await
            .unwrap_or_else(|error| Err(format!("language model task failed: {error}")));
            let (loaded, result) = match outcome {
                Ok((loaded, conditions)) => (loaded, Ok(conditions)),
                Err(error) => (None, Err(error)),
            };
            let _ = tx.send(Event::Parsed { id, loaded, result });
        });
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

    /// Join condition `index` with the next one (for a condition the model split in two).
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
        first.grounding = first.grounding.min(next.grounding);
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
        let Some(model) = self.nl.model.as_ref().map(|(_, model)| model.clone()) else {
            self.status = "Understand the sentence first.".into();
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
                    hits.sort_by(|a, b| {
                        b.similarity
                            .total_cmp(&a.similarity)
                            .then_with(|| a.path.cmp(&b.path))
                    });
                    hits.truncate(limit);
                    Ok(hits)
                };
                nl_find::find(&*model, &conditions, retrieve, &cancel, |file| {
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
                Event::Parsed { id, loaded, result } if id == self.nl.generation => {
                    if let Some(loaded) = loaded {
                        self.nl.model = Some(loaded);
                    }
                    self.nl.stage = Stage::Idle;
                    match result {
                        Ok(conditions) if conditions.is_empty() => {
                            self.nl.error = Some(
                                "No condition could be taken from that sentence. Rephrase it."
                                    .into(),
                            );
                        }
                        Ok(conditions) => {
                            self.status =
                                "Check the conditions, remove any that are wrong, then press Find."
                                    .into();
                            self.nl.conditions = Some(conditions);
                        }
                        Err(error) => self.nl.error = Some(error),
                    }
                    changed = true;
                }
                Event::File { id, file } if id == self.nl.generation => {
                    self.nl.files.push(file);
                    changed = true;
                }
                Event::Done { id, result } if id == self.nl.generation => {
                    self.nl.stage = Stage::Idle;
                    match result {
                        Ok(files) => {
                            self.status = if files.is_empty() {
                                "No file had a passage the model confirmed.".into()
                            } else {
                                format!("{} files with confirmed evidence; open each citation to verify.", files.len())
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

/// The sentence's conditions as pills: click one to remove it, click the + between two to merge them. A condition that
/// is only partly made of the user's words is marked "!" and left unlit, so it reads as "check me".
pub(super) fn chips(s: &SearchSession) -> Vec<Entry> {
    let Some(conditions) = &s.nl.conditions else {
        return Vec::new();
    };
    let locked = s.nl.stage != Stage::Idle;
    let mut chips = Vec::new();
    for (i, condition) in conditions.iter().enumerate() {
        let mark = if condition.negated { "not " } else { "" };
        let review = if condition.needs_review() { "! " } else { "" };
        let entry = Entry::new(
            format!("{review}{mark}{}  ×", condition.text),
            "click to remove",
            act(SearchAct::NlRemove(i)),
        )
        .checked(!condition.needs_review());
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
    if let Some(problem) = model_problem(&s.st.config.llm_model_path) {
        items.push(DockItem::text(problem, Tone::Warn));
    }
    if let Some(error) = &nl.error {
        items.push(DockItem::text(error.clone(), Tone::Warn));
    }
    match nl.stage {
        Stage::Parsing => items.push(DockItem::text("Reading your sentence…", Tone::Dim)),
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
        items.push(DockItem::Heading("Files with confirmed evidence".into()));
        items.push(DockItem::text(
            "The model checked these passages; each quote is verified to be in the cited text. Open the source to confirm.",
            Tone::Dim,
        ));
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
                        format!("   ok  {name}: {}", quote_line(evidence)),
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

/// A stand-in model for tests and rendered snapshots: splits a sentence at " and ", judges by the condition's first word.
#[cfg(test)]
pub(super) mod fake {
    use super::*;
    use search_llm::Verdict;

    pub struct Fake;
    impl ConditionModel for Fake {
        fn parse(&self, sentence: &str) -> Result<Vec<Condition>, String> {
            Ok(sentence
                .split(" and ")
                .map(|part| Condition {
                    text: part.trim().to_string(),
                    terms: part.split_whitespace().map(str::to_string).collect(),
                    negated: false,
                    grounding: 1.0,
                })
                .collect())
        }
        fn judge(&self, condition: &str, passage: &str) -> Result<Verdict, String> {
            let word = condition
                .split_whitespace()
                .next()
                .unwrap_or("")
                .to_lowercase();
            let hit = passage.to_lowercase().contains(&word);
            Ok(Verdict {
                matches: hit,
                quote: if hit {
                    passage.to_string()
                } else {
                    String::new()
                },
            })
        }
    }
    pub fn loader(_: &Path) -> Result<Model, String> {
        Ok(Arc::new(Fake))
    }
}
