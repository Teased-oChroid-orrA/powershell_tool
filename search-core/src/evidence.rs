use crate::semantic_index::SemanticHit;

/// How much one selection may take: passages, passages per file, and characters in all.
#[derive(Clone, Copy)]
struct Limits {
    passages: usize,
    per_file: usize,
    chars: usize,
}

/// The short list quoted under a search: three passages, sources first, 2,400 characters.
const SHORT: Limits = Limits { passages: 3, per_file: usize::MAX, chars: 2_400 };
/// The brief (P4-1): six passages, at most two per file, 4,800 characters.
const BRIEF: Limits = Limits { passages: 6, per_file: 2, chars: 4_800 };

/// Select complete, cited excerpts for an extractive evidence brief.
/// Similarity only orders passages; it does not establish that a passage answers a question.
pub fn select_evidence(hits: &[SemanticHit]) -> Vec<&SemanticHit> {
    select_evidence_with(hits, None)
}

/// [`select_evidence`] with a near-duplicate limit (word-3-gram Jaccard; `None` drops only identical passages, which is what
/// search uses). Measured for the brief on the five benchmark sets, dropping near-duplicates loses on MS MARCO, so it is not
/// applied (`docs/search-benchmarks.md`).
pub fn select_evidence_with(hits: &[SemanticHit], near_duplicate: Option<f32>) -> Vec<&SemanticHit> {
    let ranked = ranked_hits(hits);

    pick_in_order(ranked, near_duplicate, SHORT)
}

fn ranked_hits(hits: &[SemanticHit]) -> Vec<&SemanticHit> {
    let mut ranked = hits.iter().collect::<Vec<_>>();
    ranked.sort_by(|left, right| {
        right
            .similarity
            .total_cmp(&left.similarity)
            .then_with(|| left.path.cmp(&right.path))
            .then_with(|| left.first_line.cmp(&right.first_line))
            .then_with(|| left.last_line.cmp(&right.last_line))
            .then_with(|| left.excerpt.cmp(&right.excerpt))
    });
    ranked
}

/// Passages of one file in a [`brief`], in reading order.
#[derive(Debug)]
pub struct BriefFile<'a> {
    pub path: &'a str,
    pub passages: Vec<&'a SemanticHit>,
}

/// Files of one folder in a [`brief`], best first.
#[derive(Debug)]
pub struct BriefFolder<'a> {
    pub folder: &'a str,
    pub files: Vec<BriefFile<'a>>,
}

/// Extractive multi-document brief (P4-1): the best cited passages of `hits` (up to six, at most two per file,
/// every file first before a file gets a second passage), grouped by folder and file. Folders and files come best
/// first by their strongest passage; passages within a file in line order. Nothing is generated or reworded: every
/// passage is a stored excerpt with its path and lines. Similarity only orders; it does not show a passage answers
/// anything.
pub fn brief(hits: &[SemanticHit]) -> Vec<BriefFolder<'_>> {
    let picked = pick_in_order(ranked_hits(hits), None, BRIEF);
    let mut folders: Vec<BriefFolder> = Vec::new();
    for hit in picked {
        let folder = crate::catalog_edges::parent_folder(&hit.path);
        let fi = match folders.iter().position(|f| f.folder == folder) {
            Some(i) => i,
            None => {
                folders.push(BriefFolder { folder, files: Vec::new() });
                folders.len() - 1
            }
        };
        let files = &mut folders[fi].files;
        match files.iter_mut().find(|f| f.path == hit.path) {
            Some(file) => file.passages.push(hit),
            None => files.push(BriefFile { path: &hit.path, passages: vec![hit] }),
        }
    }
    // `picked` was in rank order, so folders and files were created best first; only passages need reading order.
    for file in folders.iter_mut().flat_map(|f| f.files.iter_mut()) {
        file.passages.sort_by_key(|p| (p.first_line, p.last_line));
    }
    folders
}

fn pick_in_order(ranked: Vec<&SemanticHit>, near_duplicate: Option<f32>, limits: Limits) -> Vec<&SemanticHit> {
    let mut shingles: Vec<std::collections::HashSet<u64>> = Vec::new();
    let mut citations = std::collections::HashSet::new();
    let mut excerpts = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for hit in ranked {
        let citation = (&hit.path, hit.first_line, hit.last_line, &hit.excerpt);
        if !citations.insert(citation) || !excerpts.insert(&hit.excerpt) {
            continue;
        }
        // A passage that repeats one already kept (a copied paragraph, an overlapping chunk) would use a slot of the
        // brief for nothing; the file ranking is not touched.
        if let Some(limit) = near_duplicate {
            let own = crate::ranking::shingles(&hit.excerpt);
            if shingles.iter().any(|kept| crate::ranking::jaccard(&own, kept) >= limit) {
                continue;
            }
            shingles.push(own);
        }
        candidates.push(hit);
    }

    let mut selected: Vec<&SemanticHit> = Vec::new();
    let mut per_path: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    let mut chars = 0;
    for pass in 0..2 {
        for hit in &candidates {
            let taken = per_path.get(hit.path.as_str()).copied().unwrap_or(0);
            if selected.len() == limits.passages || (pass == 0 && taken > 0) || taken >= limits.per_file {
                continue;
            }
            let size = hit.excerpt.chars().count();
            if chars + size > limits.chars {
                continue;
            }
            if selected.contains(hit) {
                continue;
            }
            *per_path.entry(hit.path.as_str()).or_insert(0) += 1;
            chars += size;
            selected.push(*hit);
        }
    }
    selected
}

#[cfg(test)]
mod tests {
    use super::select_evidence;
    use crate::semantic_index::SemanticHit;

    fn hit(path: &str, first_line: usize, excerpt: &str, similarity: f32) -> SemanticHit {
        SemanticHit {
            path: path.into(),
            first_line,
            last_line: first_line,
            excerpt: excerpt.into(),
            similarity,
            page: None,
        }
    }

    #[test]
    fn selects_three_deduplicated_citations_with_source_diversity() {
        let hits = vec![
            hit("a.txt", 1, "top", 0.99),
            hit("a.txt", 2, "second from a", 0.98),
            hit("b.txt", 3, "other source", 0.90),
            hit("b.txt", 3, "other source", 0.90),
            hit("c.txt", 4, "third source", 0.80),
        ];

        let selected = select_evidence(&hits);

        assert_eq!(
            selected
                .iter()
                .map(|h| h.excerpt.as_str())
                .collect::<Vec<_>>(),
            ["top", "other source", "third source"]
        );
        assert_eq!(selected[0].first_line, 1);
    }

    #[test]
    fn brief_groups_by_folder_and_file_best_first_with_two_passages_per_file() {
        let hits = vec![
            hit("/p/a/one.txt", 9, "one late", 0.80),
            hit("/p/b/two.txt", 3, "two top", 0.99),
            hit("/p/a/one.txt", 2, "one early", 0.95),
            hit("/p/a/one.txt", 5, "one third", 0.94),
            hit("/p/a/three.txt", 1, "three", 0.90),
            hit("/q/four.txt", 1, "four", 0.85),
        ];
        let brief = super::brief(&hits);
        let shape: Vec<(&str, Vec<(&str, Vec<&str>)>)> = brief
            .iter()
            .map(|f| (f.folder, f.files.iter().map(|x| (x.path, x.passages.iter().map(|p| p.excerpt.as_str()).collect())).collect()))
            .collect();
        assert_eq!(
            shape,
            vec![
                ("/p/b", vec![("/p/b/two.txt", vec!["two top"])]),
                // one.txt: 0.95 and 0.94 kept (two per file), 0.80 left out; passages in reading order
                ("/p/a", vec![("/p/a/one.txt", vec!["one early", "one third"]), ("/p/a/three.txt", vec!["three"])]),
                ("/q", vec![("/q/four.txt", vec!["four"])]),
            ]
        );
    }

    #[test]
    fn brief_caps_passages_and_characters_and_never_cuts_a_quote() {
        let many: Vec<SemanticHit> = (0..10).map(|i| hit(&format!("/d/f{i}.txt"), 1, &format!("short {i}"), 1.0 - i as f32 / 100.0)).collect();
        let n: usize = super::brief(&many).iter().flat_map(|f| &f.files).map(|f| f.passages.len()).sum();
        assert_eq!(n, 6);
        let big = "y".repeat(3_000);
        let hits = vec![hit("/d/a.txt", 1, &big, 1.0), hit("/d/b.txt", 1, &big.replace('y', "z"), 0.9), hit("/d/c.txt", 1, "tiny", 0.8)];
        let kept: Vec<&str> = super::brief(&hits).iter().flat_map(|f| &f.files).flat_map(|f| &f.passages).map(|p| p.path.as_str()).collect();
        assert_eq!(kept, ["/d/a.txt", "/d/c.txt"], "the second 3,000-character quote would pass 4,800 and is skipped whole");
    }

    #[test]
    fn the_short_list_is_unchanged_by_the_brief_limits() {
        let hits: Vec<SemanticHit> = (0..5).map(|i| hit("/d/same.txt", i + 1, &format!("p{i}"), 1.0 - i as f32 / 100.0)).collect();
        assert_eq!(select_evidence(&hits).len(), 3, "short list: three passages even from one file");
    }

    #[test]
    fn near_duplicate_limit_is_opt_in_and_search_keeps_copies() {
        let copy = "the stud torque for the m8 flange bolts is 25 newton metres when dry and 20 when oiled";
        let hits = vec![
            hit("a.txt", 1, copy, 0.99),
            hit("b.txt", 7, &format!("{copy} (revision 2)"), 0.98),
            hit("c.txt", 4, "different passage about gasket thickness", 0.80),
        ];

        let kept: Vec<_> = super::select_evidence_with(&hits, Some(crate::ranking::NEAR_DUPLICATE)).iter().map(|h| h.path.as_str()).collect();
        assert_eq!(kept, ["a.txt", "c.txt"]);
        let all: Vec<_> = select_evidence(&hits).iter().map(|h| h.path.as_str()).collect();
        assert_eq!(all, ["a.txt", "b.txt", "c.txt"]);
    }

    #[test]
    fn skips_passages_over_budget_without_cutting_quotes() {
        let oversized = "x".repeat(2_401);
        let hits = vec![
            hit("a.txt", 1, &oversized, 1.0),
            hit("b.txt", 2, "whole quote", 0.9),
        ];

        let selected = select_evidence(&hits);

        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].excerpt, "whole quote");
    }
}
