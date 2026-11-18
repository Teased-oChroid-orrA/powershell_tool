//! Narrow: narrows the results already found by file type, folder, modified date and text quality (the sidebar's
//! "Narrow results"). It only hides rows of the run's results; it never searches again and never changes ranking.
//! Selections are stored as values (".pdf", a folder label), not positions, so they survive a changing result list;
//! the actions carry a position into the facet list built from the same results.
use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Local};
use search_core::models::FileSearchResult;

/// How many types and folders the sidebar lists (a selected one is always listed).
const MAX_LISTED: usize = 8;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Modified {
    #[default]
    Any,
    Week,
    Month,
    Year,
}

impl Modified {
    pub const ALL: [Modified; 4] = [
        Modified::Any,
        Modified::Week,
        Modified::Month,
        Modified::Year,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Modified::Any => "Any time",
            Modified::Week => "7 days",
            Modified::Month => "30 days",
            Modified::Year => "Year",
        }
    }

    /// The chip over the table for an active window.
    pub fn chip(self) -> &'static str {
        match self {
            Modified::Any => "Any time",
            Modified::Week => "Last 7 days",
            Modified::Month => "Last 30 days",
            Modified::Year => "Last year",
        }
    }

    fn days(self) -> Option<i64> {
        match self {
            Modified::Any => None,
            Modified::Week => Some(7),
            Modified::Month => Some(30),
            Modified::Year => Some(365),
        }
    }
}

/// How far the text read from a file can be trusted, from the result's own warning (`FileSearchResult::pdf_warning`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum Quality {
    Clean,
    Misread,
    Poor,
}

impl Quality {
    pub const ALL: [Quality; 3] = [Quality::Misread, Quality::Poor, Quality::Clean];

    pub fn label(self) -> &'static str {
        match self {
            Quality::Clean => "Clean text layer",
            Quality::Misread => "PDF text may be misread",
            Quality::Poor => "OCR, poorly read",
        }
    }

    pub fn of(r: &FileSearchResult) -> Quality {
        if !r.low_confidence_pdf {
            Quality::Clean
        } else if r.ocr_is_poor() {
            Quality::Poor
        } else {
            Quality::Misread
        }
    }
}

/// The extension of a result as the facet key (".pdf"), the same text `format::extension_breakdown` counts.
pub fn type_of(r: &FileSearchResult) -> String {
    std::path::Path::new(&r.full_name)
        .extension()
        .map(|e| format!(".{}", e.to_string_lossy().to_lowercase()))
        .unwrap_or_else(|| "(no extension)".to_string())
}

/// What the sidebar and the chips call a type key.
pub fn type_label(key: &str) -> String {
    key.strip_prefix('.')
        .map_or_else(|| key.to_string(), str::to_uppercase)
}

/// The folder of a result below the searched folder ("(top folder)" for the folder itself, the full path outside it).
pub fn folder_of(full_name: &str, root: &str) -> String {
    let folder = std::path::Path::new(full_name)
        .parent()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_default();
    match folder
        .strip_prefix(root)
        .map(|f| f.trim_start_matches(['/', '\\']).to_string())
    {
        Some(f) if f.is_empty() => "(top folder)".to_string(),
        Some(f) => f,
        None => folder,
    }
}

/// The searched folder as `folder_of` wants it (no trailing separator).
pub fn root_of(search_path: &str) -> String {
    search_path.trim().trim_end_matches(['/', '\\']).to_string()
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Narrow {
    pub types: BTreeSet<String>,
    pub folders: BTreeSet<String>,
    pub modified: Modified,
    pub quality: BTreeSet<Quality>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Dim {
    Type,
    Folder,
    Modified,
    Quality,
}

/// The four facet values of one result.
struct Facts {
    ty: String,
    folder: String,
    age_days: i64,
    quality: Quality,
}

impl Facts {
    fn of(r: &FileSearchResult, root: &str, now: DateTime<Local>) -> Facts {
        Facts {
            ty: type_of(r),
            folder: folder_of(&r.full_name, root),
            age_days: now.signed_duration_since(r.modified).num_days(),
            quality: Quality::of(r),
        }
    }
}

impl Narrow {
    pub fn is_empty(&self) -> bool {
        *self == Narrow::default()
    }

    /// Whether `r` stays in the table.
    pub fn matches(&self, r: &FileSearchResult, root: &str, now: DateTime<Local>) -> bool {
        self.is_empty() || self.passes(&Facts::of(r, root, now), None)
    }

    /// Passes every dimension except `skip` (a facet counts what its own choice would add).
    fn passes(&self, f: &Facts, skip: Option<Dim>) -> bool {
        let on = |d: Dim| skip != Some(d);
        (!on(Dim::Type) || self.types.is_empty() || self.types.contains(&f.ty))
            && (!on(Dim::Folder) || self.folders.is_empty() || self.folders.contains(&f.folder))
            && (!on(Dim::Modified) || self.modified.days().is_none_or(|d| f.age_days <= d))
            && (!on(Dim::Quality) || self.quality.is_empty() || self.quality.contains(&f.quality))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Value {
    /// The key a selection stores (".pdf", a folder label).
    pub key: String,
    pub label: String,
    /// Results with this value that pass every other choice.
    pub count: usize,
    pub on: bool,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Facets {
    pub types: Vec<Value>,
    pub folders: Vec<Value>,
    pub modified: Vec<(Modified, usize)>,
    pub quality: Vec<(Quality, usize)>,
    /// Results shown with the current choices, and all of them.
    pub shown: usize,
    pub total: usize,
}

impl Facets {
    pub fn build(
        results: &[FileSearchResult],
        narrow: &Narrow,
        root: &str,
        now: DateTime<Local>,
    ) -> Facets {
        let facts: Vec<Facts> = results.iter().map(|r| Facts::of(r, root, now)).collect();
        let list = |dim: Dim,
                    key: fn(&Facts) -> &String,
                    selected: &BTreeSet<String>,
                    label: fn(&str) -> String| {
            let mut totals: BTreeMap<&str, usize> = BTreeMap::new();
            for f in &facts {
                *totals.entry(key(f).as_str()).or_default() += 1;
            }
            // Most common first (over all results, so the order does not move as choices change), then by name.
            let mut order: Vec<(&str, usize)> = totals.into_iter().collect();
            order.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(b.0)));
            let mut keys: Vec<String> = order
                .iter()
                .take(MAX_LISTED)
                .map(|(k, _)| k.to_string())
                .collect();
            for k in selected {
                if !keys.contains(k) {
                    keys.push(k.clone());
                }
            }
            keys.into_iter()
                .map(|k| Value {
                    count: facts
                        .iter()
                        .filter(|f| key(f) == &k && narrow.passes(f, Some(dim)))
                        .count(),
                    on: selected.contains(&k),
                    label: label(&k),
                    key: k,
                })
                .collect::<Vec<_>>()
        };
        Facets {
            types: list(Dim::Type, |f| &f.ty, &narrow.types, type_label),
            folders: list(Dim::Folder, |f| &f.folder, &narrow.folders, str::to_string),
            modified: Modified::ALL
                .iter()
                .map(|&m| {
                    let counted = Narrow {
                        modified: m,
                        ..narrow.clone()
                    };
                    (m, facts.iter().filter(|f| counted.passes(f, None)).count())
                })
                .collect(),
            quality: Quality::ALL
                .iter()
                .map(|&q| {
                    (
                        q,
                        facts
                            .iter()
                            .filter(|f| f.quality == q && narrow.passes(f, Some(Dim::Quality)))
                            .count(),
                    )
                })
                .collect(),
            shown: facts.iter().filter(|f| narrow.passes(f, None)).count(),
            total: facts.len(),
        }
    }
}
