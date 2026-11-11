//! Which files get embedded first, and how many per pass (P2-3, `docs/knowledge-engine-audit.md`).
//!
//! Embedding is the dominant indexing cost (about 14 ms per 800-character chunk for MiniLM on this Mac, against about
//! 2 ms per file for the exact index). The exact (keyword and trigram) index never waits for it: a file that is not
//! embedded yet is still found by exact search, and is embedded by a later pass. A pass embeds at most
//! [`EmbedPolicy::limit`] files, in priority order: folder order first (earlier folders first), then extension class
//! (prose before code before data). No limit means "embed everything", which is what every default build does.
//!
//! Pure functions, no model and no I/O, so the ordering is testable without the embedding model.

/// What kind of text a file holds, which decides how soon it is worth embedding. Declaration order is priority order.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum ExtClass {
    /// Documents and notes: the text similarity search is for.
    Prose,
    /// Everything not listed (config, unknown extensions, containers).
    Other,
    /// Source code: terse, identifier-heavy, well served by exact search.
    Code,
    /// Tables, logs and serialized data: large, repetitive, poor semantic yield.
    Data,
}

/// Class of an extension, with or without the leading dot, any case.
pub fn class_of(ext: &str) -> ExtClass {
    match ext.trim_start_matches('.').to_ascii_lowercase().as_str() {
        "txt" | "md" | "markdown" | "rst" | "adoc" | "org" | "tex" | "pdf" | "doc" | "docx" | "odt" | "rtf" | "pptx" | "html" | "htm" => ExtClass::Prose,
        "rs" | "py" | "js" | "mjs" | "ts" | "tsx" | "jsx" | "c" | "h" | "cc" | "cpp" | "hpp" | "cs" | "java" | "go" | "rb" | "php" | "swift" | "kt" | "sh" | "ps1" | "psm1" | "bat" | "lua" | "r" | "scala" | "pl" => ExtClass::Code,
        "csv" | "tsv" | "json" | "jsonl" | "ndjson" | "xml" | "log" | "sql" | "xlsx" | "xls" | "dat" => ExtClass::Data,
        _ => ExtClass::Other,
    }
}

/// How many files one indexing pass embeds, and in what order.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EmbedPolicy {
    /// Most files to embed in one pass; `None` embeds every file that needs it.
    pub limit: Option<usize>,
    /// Folders embedded before everything else, in the order given. A file under none of them comes last.
    pub priority_folders: Vec<String>,
}

fn normal(path: &str) -> String {
    let slashed = path.replace('\\', "/");
    if cfg!(windows) { slashed.to_lowercase() } else { slashed }
}

impl EmbedPolicy {
    /// Sort key of a file: (position of its priority folder, extension class). Smaller embeds sooner.
    pub fn rank(&self, path: &str, ext: &str) -> (usize, ExtClass) {
        let path = normal(path);
        let folder = self
            .priority_folders
            .iter()
            .position(|f| {
                let f = normal(f);
                let f = f.trim_end_matches('/');
                path.strip_prefix(f).is_some_and(|rest| rest.starts_with('/'))
            })
            .unwrap_or(self.priority_folders.len());
        (folder, class_of(ext))
    }

    /// The files of `needing` (path, extension) this pass embeds: the first `limit` by [`rank`](Self::rank), ties in
    /// the order given. Returns indices into `needing`, in priority order.
    pub fn select(&self, needing: &[(&str, &str)]) -> Vec<usize> {
        let mut order: Vec<usize> = (0..needing.len()).collect();
        order.sort_by_cached_key(|&i| self.rank(needing[i].0, needing[i].1));
        order.truncate(self.limit.unwrap_or(usize::MAX));
        order
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn prose_comes_before_code_before_data() {
        assert!(class_of(".PDF") < class_of("rs"));
        assert!(class_of("md") < class_of(".other"));
        assert!(class_of("rs") < class_of(".csv"));
        assert_eq!(class_of("weird"), ExtClass::Other);
    }

    #[test]
    fn folders_beat_extension_class_and_ties_keep_input_order() {
        let policy = EmbedPolicy { limit: None, priority_folders: vec!["/proj/specs".into(), "/proj/notes/".into()] };
        let files = [
            ("/proj/other/a.md", ".md"),
            ("/proj/notes/b.csv", ".csv"),
            ("/proj/specs/c.rs", ".rs"),
            ("/proj/specs/d.txt", ".txt"),
            ("/proj/specs-old/e.txt", ".txt"),
            ("/proj/other/f.md", ".md"),
        ];
        let picked: Vec<&str> = policy.select(&files).into_iter().map(|i| files[i].0).collect();
        assert_eq!(
            picked,
            ["/proj/specs/d.txt", "/proj/specs/c.rs", "/proj/notes/b.csv", "/proj/other/a.md", "/proj/specs-old/e.txt", "/proj/other/f.md"],
            "specs-old is not under specs; unlisted folders sort last, prose first"
        );
    }

    #[test]
    fn limit_takes_the_head_of_the_order_and_none_takes_all() {
        let files = [("/a/x.csv", ".csv"), ("/a/y.md", ".md"), ("/a/z.rs", ".rs")];
        let two = EmbedPolicy { limit: Some(2), ..Default::default() };
        assert_eq!(two.select(&files), vec![1, 2]);
        assert_eq!(EmbedPolicy::default().select(&files).len(), 3);
        assert!(EmbedPolicy { limit: Some(0), ..Default::default() }.select(&files).is_empty());
    }
}
