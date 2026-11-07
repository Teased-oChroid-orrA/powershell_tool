use crate::semantic_index::SemanticHit;

const MAX_PASSAGES: usize = 3;
const MAX_CHARS: usize = 2_400;

/// Select complete, cited excerpts for an extractive evidence brief.
/// Similarity only orders passages; it does not establish that a passage answers a question.
pub fn select_evidence(hits: &[SemanticHit]) -> Vec<&SemanticHit> {
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

    let mut citations = std::collections::HashSet::new();
    let mut excerpts = std::collections::HashSet::new();
    let mut candidates = Vec::new();
    for hit in ranked {
        let citation = (&hit.path, hit.first_line, hit.last_line, &hit.excerpt);
        if citations.insert(citation) && excerpts.insert(&hit.excerpt) {
            candidates.push(hit);
        }
    }

    let mut selected = Vec::new();
    let mut paths = std::collections::HashSet::new();
    let mut chars = 0;
    for pass in 0..2 {
        for hit in &candidates {
            if selected.len() == MAX_PASSAGES || (pass == 0 && paths.contains(hit.path.as_str())) {
                continue;
            }
            let size = hit.excerpt.chars().count();
            if chars + size > MAX_CHARS {
                continue;
            }
            if selected.contains(hit) {
                continue;
            }
            paths.insert(hit.path.as_str());
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
