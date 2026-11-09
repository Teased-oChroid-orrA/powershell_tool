//! Opt-in "Refine results": a second embedding model (bge-small, not bundled) re-scores the leading semantic hits'
//! excerpts and the order becomes the reciprocal-rank fusion of the fast ranking and the bge ranking
//! (`search_core::rank_fusion`, the benchmarked `rerank-rrf`; `docs/search-benchmarks.md`).
//!
//! Nothing here runs unless the user clicks Refine. The model lives in a folder holding `model.onnx` and
//! `tokenizer.json` (see [`model_dir`]); loading and scoring run on a blocking thread and report through
//! [`RefineEvent`], so the window never waits.
use super::SemanticSearchKey;
use search_core::semantic_index::SemanticHit;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use tokio::sync::mpsc;

/// Environment variable naming the model folder.
pub(super) const MODEL_ENV: &str = "TOOLBENCH_RERANK_MODEL";
/// Folder name looked for next to the executable and under `<app data>/fea-editor/`.
pub(super) const MODEL_FOLDER: &str = "rerank-model";

pub(super) enum RefineEvent {
    Progress { key: SemanticSearchKey, done: usize, total: usize },
    Done { key: SemanticSearchKey, result: Result<Vec<SemanticHit>, String> },
}

/// State of the Refine step, owned by the session.
pub(super) struct Refine {
    pub(super) tx: mpsc::UnboundedSender<RefineEvent>,
    pub(super) rx: mpsc::UnboundedReceiver<RefineEvent>,
    pub(super) running: bool,
    /// Refined order for the semantic search it was computed for.
    pub(super) hits: Option<(SemanticSearchKey, Vec<SemanticHit>)>,
}

impl Default for Refine {
    fn default() -> Self {
        let (tx, rx) = mpsc::unbounded_channel();
        Self { tx, rx, running: false, hits: None }
    }
}

fn has_model(dir: &Path) -> bool {
    dir.join("model.onnx").is_file() && dir.join("tokenizer.json").is_file()
}

/// The model folder: `env` (`TOOLBENCH_RERANK_MODEL`), else `rerank-model/` beside the executable, else
/// `<app data>/fea-editor/rerank-model/`. The first folder holding both files wins; otherwise the error says
/// where to put them.
pub(super) fn resolve_model_dir(
    env: Option<OsString>,
    exe_dir: Option<&Path>,
    app_data: Option<&Path>,
) -> Result<PathBuf, String> {
    let env = env.filter(|v| !v.is_empty()).map(PathBuf::from);
    let beside_exe = exe_dir.map(|d| d.join(MODEL_FOLDER));
    let in_app_data = app_data.map(|d| d.join("fea-editor").join(MODEL_FOLDER));
    let candidates = env.iter().chain(beside_exe.iter()).chain(in_app_data.iter());
    if let Some(found) = candidates.clone().find(|dir| has_model(dir)) {
        return Ok(found.clone());
    }
    let target = beside_exe.as_ref().or(in_app_data.as_ref()).map(|d| d.display().to_string());
    let mut message = String::from("Refine needs the second search model. ");
    if let Some(env) = &env {
        message.push_str(&format!("{MODEL_ENV} points to {}, which lacks model.onnx or tokenizer.json. ", env.display()));
    }
    match (target, in_app_data.as_ref().filter(|d| Some(*d) != beside_exe.as_ref())) {
        (Some(folder), Some(other)) => message.push_str(&format!(
            "Put model.onnx and tokenizer.json in {folder} (or {}).",
            other.display()
        )),
        (Some(folder), None) => message.push_str(&format!("Put model.onnx and tokenizer.json in {folder}.")),
        _ => message.push_str(&format!("Set {MODEL_ENV} to a folder holding model.onnx and tokenizer.json.")),
    }
    Err(message)
}

/// [`resolve_model_dir`] for this process.
pub(super) fn model_dir() -> Result<PathBuf, String> {
    let exe_dir = std::env::current_exe().ok().and_then(|p| p.parent().map(Path::to_path_buf));
    resolve_model_dir(std::env::var_os(MODEL_ENV), exe_dir.as_deref(), crate::paths::app_data_dir().as_deref())
}

/// Re-order `hits` (best first) with the model in `dir`: the first `REFINE_TOP` are re-scored from their excerpts and
/// fused with their current order; the rest keep their place. `progress(done, total)` is called as passages are embedded.
pub(super) fn refine_hits(
    dir: &Path,
    query: &str,
    mut hits: Vec<SemanticHit>,
    mut progress: impl FnMut(usize, usize),
) -> Result<Vec<SemanticHit>, String> {
    let top = hits.len().min(search_core::rank_fusion::REFINE_TOP);
    if top < 2 {
        return Ok(hits);
    }
    progress(0, top);
    let texts: Vec<&str> = hits[..top].iter().map(|h| h.excerpt.as_str()).collect();
    let scores = search_core::second_model::score_passages(dir, query, &texts, &mut progress)?;
    let order = search_core::rank_fusion::fuse_top(&scores, hits.len());
    let mut slots: Vec<Option<SemanticHit>> = hits.drain(..).map(Some).collect();
    Ok(order.into_iter().filter_map(|i| slots[i].take()).collect())
}

impl super::SearchSession {
    /// Start refining the current semantic results on a blocking thread.
    pub(super) fn refine_results(&mut self) {
        if self.refine.running {
            self.status = "Refining is already running.".into();
            return;
        }
        let Some(key) = self.semantic_request.clone() else {
            self.status = "Search semantically first, then refine the results.".into();
            return;
        };
        if self.current_semantic_hits().is_none() {
            self.status = "Search semantically first, then refine the results.".into();
            return;
        }
        let dir = match model_dir() {
            Ok(dir) => dir,
            Err(message) => {
                self.status = message;
                return;
            }
        };
        let roots = super::semantic_search_roots(&self.st.config);
        let query = key.query.clone();
        self.refine.running = true;
        self.refine.hits = None;
        self.status = "Loading the second model…".into();
        let tx = self.refine.tx.clone();
        self.rt.spawn(async move {
            let worker_key = key.clone();
            let worker_tx = tx.clone();
            let result = tokio::task::spawn_blocking(move || {
                // Same candidate pool the benchmark scored: the leading REFINE_TOP hits of the fast search.
                let mut hits = Vec::new();
                let mut errors = Vec::new();
                for (root, settings) in roots {
                    match search_core::semantic_index::search_directory_with_settings(
                        &root,
                        &query,
                        search_core::rank_fusion::REFINE_TOP,
                        &settings,
                    ) {
                        Ok(mut found) => hits.append(&mut found),
                        Err(error) => errors.push(error),
                    }
                }
                if hits.is_empty() && !errors.is_empty() {
                    return Err(errors.join("; "));
                }
                hits.sort_by(|a, b| b.similarity.total_cmp(&a.similarity).then_with(|| a.path.cmp(&b.path)));
                hits.truncate(search_core::rank_fusion::REFINE_TOP);
                refine_hits(&dir, &query, hits, |done, total| {
                    let _ = worker_tx.send(RefineEvent::Progress { key: worker_key.clone(), done, total });
                })
            })
            .await
            .unwrap_or_else(|error| Err(format!("refine task failed: {error}")));
            let _ = tx.send(RefineEvent::Done { key, result });
        });
    }

    /// Fold finished Refine work into the session; true when something changed.
    pub(super) fn poll_refine(&mut self) -> bool {
        let mut changed = false;
        while let Ok(event) = self.refine.rx.try_recv() {
            match event {
                RefineEvent::Progress { key, done, total } => {
                    if self.semantic_request.as_ref() == Some(&key) {
                        self.status = format!("Refining results: {done} of {total} passages read…");
                        changed = true;
                    }
                }
                RefineEvent::Done { key, result } => {
                    self.refine.running = false;
                    if self.semantic_request.as_ref() != Some(&key) {
                        continue;
                    }
                    match result {
                        Ok(hits) => {
                            self.status = format!("Refined the top {} results.", hits.len().min(search_core::rank_fusion::REFINE_TOP));
                            self.refine.hits = Some((key, hits));
                        }
                        Err(error) => self.status = format!("Refine: {error}"),
                    }
                    changed = true;
                }
            }
        }
        changed
    }

    /// The cited excerpts to show for the current semantic search: the refined order when Refine has run for it,
    /// otherwise the similarity order.
    pub(super) fn evidence_hits<'a>(&'a self, hits: &'a [SemanticHit]) -> Vec<&'a SemanticHit> {
        match &self.refine.hits {
            Some((key, refined)) if self.semantic_request.as_ref() == Some(key) => search_core::evidence::select_ordered(refined),
            _ => search_core::evidence::select_evidence(hits),
        }
    }

    pub(super) fn refined(&self) -> bool {
        self.refine.hits.as_ref().is_some_and(|(key, _)| self.semantic_request.as_ref() == Some(key))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dir_with_model(root: &Path, sub: &str) -> PathBuf {
        let d = root.join(sub);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(d.join("model.onnx"), b"x").unwrap();
        std::fs::write(d.join("tokenizer.json"), b"{}").unwrap();
        d
    }

    #[test]
    fn env_beats_exe_folder_beats_app_data() {
        let t = tempfile::tempdir().unwrap();
        let env = dir_with_model(t.path(), "env");
        let exe = t.path().join("exe");
        let exe_model = dir_with_model(&exe, MODEL_FOLDER);
        let data = t.path().join("data");
        let data_model = dir_with_model(&data.join("fea-editor"), MODEL_FOLDER);
        let got = |e: Option<&Path>, x: Option<&Path>, d: Option<&Path>| resolve_model_dir(e.map(|p| p.as_os_str().to_owned()), x, d);
        assert_eq!(got(Some(&env), Some(&exe), Some(&data)).unwrap(), env);
        assert_eq!(got(None, Some(&exe), Some(&data)).unwrap(), exe_model);
        assert_eq!(got(Some(&t.path().join("nope")), Some(&exe), Some(&data)).unwrap(), exe_model);
        assert_eq!(got(None, Some(&t.path().join("nope")), Some(&data)).unwrap(), data_model);
    }

    #[test]
    fn a_folder_missing_a_file_is_skipped_and_the_message_names_where_to_put_them() {
        let t = tempfile::tempdir().unwrap();
        let exe = t.path().join("exe");
        std::fs::create_dir_all(exe.join(MODEL_FOLDER)).unwrap();
        std::fs::write(exe.join(MODEL_FOLDER).join("model.onnx"), b"x").unwrap();
        let data = t.path().join("data");
        let message = resolve_model_dir(None, Some(&exe), Some(&data)).unwrap_err();
        assert!(message.contains("model.onnx and tokenizer.json"), "{message}");
        assert!(message.contains(&exe.join(MODEL_FOLDER).display().to_string()), "{message}");
        assert!(message.contains(&data.join("fea-editor").join(MODEL_FOLDER).display().to_string()), "{message}");
    }

    #[test]
    fn a_bad_env_folder_is_named() {
        let message = resolve_model_dir(Some("/no/such/dir".into()), None, None).unwrap_err();
        assert!(message.contains(MODEL_ENV) && message.contains("/no/such/dir"), "{message}");
    }

    #[test]
    fn fewer_than_two_hits_need_no_model() {
        let out = refine_hits(Path::new("/no/model"), "q", Vec::new(), |_, _| panic!("no progress")).unwrap();
        assert!(out.is_empty());
    }

    /// Real model: `TOOLBENCH_RERANK_MODEL=<bge-small dir> cargo test -p app-tui refine -- --ignored`.
    #[test]
    #[ignore = "needs the bge-small model folder in TOOLBENCH_RERANK_MODEL"]
    fn refine_with_the_real_model_puts_the_relevant_passage_first() {
        let dir = model_dir().expect("model folder");
        let hit = |name: &str, excerpt: &str, similarity: f32| SemanticHit {
            path: name.into(),
            first_line: 1,
            last_line: 1,
            excerpt: excerpt.into(),
            similarity,
            page: None,
        };
        let hits = vec![
            hit("a", "The cafeteria menu changes every Tuesday and includes soup.", 0.9),
            hit("b", "Titanium fasteners resist galling when torqued with anti-seize compound.", 0.8),
            hit("c", "Parking permits are renewed each January at the front desk.", 0.7),
        ];
        let mut steps = Vec::new();
        let out = refine_hits(&dir, "how do I prevent galling of titanium bolts", hits, |d, t| steps.push((d, t))).unwrap();
        assert_eq!(out.len(), 3);
        assert_eq!(out[0].path, "b");
        assert_eq!(steps.last(), Some(&(3, 3)));
    }
}
