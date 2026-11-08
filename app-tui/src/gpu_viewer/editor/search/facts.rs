//! Questions about the search folder itself ("how many files are here?") are answered by
//! looking at the folder, not by ranking text: a count is a fact to compute, not a line to quote.
//! The count uses the walk and the file-type scoping a search of the same folder uses, so it
//! agrees with what a search would read.

use std::collections::BTreeMap;
use std::path::Path;

use search_core::file_reader::enumerate_files_safely;
use search_core::models::SearchSettings;
use search_core::orchestrator::filter_by_extension;
use tokio_util::sync::CancellationToken;

/// Types shown in the breakdown.
const TYPES_SHOWN: usize = 6;

/// What a question asks to be counted: every file, or the files of some types.
#[derive(Debug, PartialEq, Eq)]
struct Count {
    types: Vec<String>,
    label: Option<String>,
}

/// Plain names for groups of extensions.
const NAMES: &[(&str, &[&str])] = &[
    ("word", &[".docx", ".doc"]),
    ("excel", &[".xlsx", ".xls", ".csv"]),
    ("spreadsheet", &[".xlsx", ".xls", ".csv"]),
    ("powerpoint", &[".pptx"]),
    ("slide", &[".pptx"]),
    ("image", &[".png", ".jpg", ".jpeg", ".tif", ".tiff"]),
    ("text", &[".txt", ".md", ".log"]),
];

fn count_request(question: &str) -> Option<Count> {
    let lower = question.to_lowercase();
    let words: Vec<&str> = lower
        .split(|c: char| !c.is_alphanumeric() && c != '.')
        .map(|w| w.trim_matches('.'))
        .filter(|w| !w.is_empty())
        .collect();
    let asks_count = lower.contains("how many") || lower.contains("number of") || words.contains(&"count");
    let about_files = words
        .iter()
        .any(|w| matches!(w.trim_end_matches('s'), "file" | "document" | "doc" | "pdf" | "item"))
        || NAMES.iter().any(|(name, _)| words.iter().any(|w| w.trim_end_matches('s') == *name));
    if !asks_count || !about_files {
        return None;
    }
    for (name, extensions) in NAMES {
        if words.iter().any(|w| w.trim_end_matches('s') == *name) {
            return Some(Count {
                types: extensions.iter().map(|e| e.to_string()).collect(),
                label: Some((*name).to_string()),
            });
        }
    }
    // A bare extension word: "pdf", "dwg", ".step".
    let typed = words.iter().find(|w| {
        let w = w.trim_end_matches('s');
        (2..=5).contains(&w.len())
            && !matches!(w, "file" | "doc" | "many" | "item" | "this" | "that" | "the" | "are" | "there" | "in" | "of")
            && search_core::models::extension_catalog::all_extensions().contains(&format!(".{w}"))
    });
    Some(match typed {
        Some(word) => {
            let word = word.trim_end_matches('s');
            Count {
                types: vec![format!(".{word}")],
                label: Some(word.to_string()),
            }
        }
        None => Count {
            types: Vec::new(),
            label: None,
        },
    })
}

/// One computed answer with where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Fact {
    pub text: String,
    pub source: String,
}

fn extension_of(path: &Path) -> String {
    path.extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_else(|| "(none)".into())
}

fn thousands(n: usize) -> String {
    let digits = n.to_string();
    let mut out = String::new();
    for (i, c) in digits.chars().enumerate() {
        if i > 0 && (digits.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

fn megabytes(bytes: i64) -> String {
    format!("{:.1} MB", bytes.max(0) as f64 / 1_048_576.0)
}

/// The facts that answer `question` about `settings.search_path`; `None` when the question does not
/// ask for a count (or the folder cannot be read).
pub(super) fn answer(question: &str, settings: &SearchSettings) -> Option<Vec<Fact>> {
    let request = count_request(question)?;
    let root = settings.search_path.as_str();
    let (files, errors) = enumerate_files_safely(
        root,
        settings.include_hidden,
        &settings.exclude_folders,
        &CancellationToken::new(),
        None,
    )
    .ok()?;
    let source = format!("Counted in {root}");
    let searchable = filter_by_extension(files.clone(), settings).len();
    let mut kinds: BTreeMap<String, (usize, i64)> = BTreeMap::new();
    for file in &files {
        let entry = kinds.entry(extension_of(&file.path)).or_default();
        entry.0 += 1;
        entry.1 += file.length;
    }
    let mut facts = Vec::new();
    if !request.types.is_empty() {
        let (n, bytes) = request
            .types
            .iter()
            .filter_map(|t| kinds.get(t))
            .fold((0, 0), |a, b| (a.0 + b.0, a.1 + b.1));
        let label = request.label.as_deref().unwrap_or("matching");
        facts.push(Fact {
            text: format!("{} {label} files ({}).", thousands(n), megabytes(bytes)),
            source: source.clone(),
        });
    }
    let total_bytes: i64 = files.iter().map(|f| f.length).sum();
    facts.push(Fact {
        text: format!(
            "{} files in total ({}); {} are of the types a search reads.",
            thousands(files.len()),
            megabytes(total_bytes),
            thousands(searchable)
        ),
        source,
    });
    let mut ranked: Vec<_> = kinds.into_iter().collect();
    ranked.sort_by(|a, b| b.1 .0.cmp(&a.1 .0).then_with(|| a.0.cmp(&b.0)));
    let more = ranked.len().saturating_sub(TYPES_SHOWN);
    let mut by_type: Vec<String> = ranked
        .iter()
        .take(TYPES_SHOWN)
        .map(|(ext, (n, _))| format!("{ext} {}", thousands(*n)))
        .collect();
    if more > 0 {
        by_type.push(format!("{more} more types"));
    }
    if !by_type.is_empty() {
        facts.push(Fact {
            text: format!("By type: {}.", by_type.join(", ")),
            source: format!("Counted in {root}"),
        });
    }
    if errors > 0 {
        facts.push(Fact {
            text: format!("{errors} folders could not be read and are not counted."),
            source: format!("Counted in {root}"),
        });
    }
    Some(facts)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counting_questions_are_recognised_and_others_are_not() {
        assert_eq!(count_request("how many files in this search directory"), Some(Count { types: vec![], label: None }));
        assert_eq!(count_request("How many PDFs are there?").unwrap().types, vec![".pdf"]);
        assert_eq!(count_request("number of word documents").unwrap().label.as_deref(), Some("word"));
        assert!(count_request("how do I exclude a folder").is_none());
        assert!(count_request("what files does Search read").is_none());
    }

    #[test]
    fn the_folder_is_counted_by_type_and_scoped_like_a_search() {
        let dir = std::env::temp_dir().join(format!("facts-{}", std::process::id()));
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        for name in ["a.txt", "b.txt", "sub/c.pdf", "sub/d.xyzzy"] {
            std::fs::write(dir.join(name), "x").unwrap();
        }
        let settings = SearchSettings {
            search_path: dir.to_string_lossy().into_owned(),
            ..SearchSettings::default()
        };
        let facts = answer("how many files are in this folder", &settings).unwrap();
        std::fs::remove_dir_all(&dir).unwrap();
        assert!(facts[0].text.starts_with("4 files in total"), "{facts:?}");
        assert!(facts[0].text.contains("3 are of the types"), "{facts:?}");
        assert!(facts[1].text.contains(".txt 2"), "{facts:?}");
        assert!(facts[0].source.starts_with("Counted in "));
    }
}
