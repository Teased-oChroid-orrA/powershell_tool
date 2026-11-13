//! Small, reviewable engineering vocabulary for query expansion.
//! Candidates are suggestions only; callers decide whether and how to apply them.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    Abbreviation,
    AlternateWording,
}

impl Relation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Abbreviation => "abbreviation",
            Self::AlternateWording => "alternate wording",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Suggestion {
    pub source: &'static str,
    pub term: &'static str,
    pub relation: Relation,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypoCorrection {
    pub original: String,
    pub replacement: String,
    pub query: String,
}

const GROUPS: &[(&[&str], Relation)] = &[
    (
        &["OD", "outer diameter", "outside diameter"],
        Relation::Abbreviation,
    ),
    (
        &["ID", "inner diameter", "inside diameter"],
        Relation::Abbreviation,
    ),
    (&["dia", "diam", "diameter"], Relation::Abbreviation),
    (
        &["UTS", "ultimate tensile strength"],
        Relation::Abbreviation,
    ),
    (&["nom", "nominal"], Relation::Abbreviation),
    (&["max", "maximum"], Relation::Abbreviation),
    (&["min", "minimum"], Relation::Abbreviation),
    (&["thk", "thickness"], Relation::Abbreviation),
    (
        &["yield strength", "yield stress"],
        Relation::AlternateWording,
    ),
    (&["threads per inch", "TPI"], Relation::Abbreviation),
];
const MAX_TYPO_TOKEN_CHARS: usize = 32;

/// Return equivalent engineering terms explicitly present in the local vocabulary.
/// Matching uses whole tokens, so `OD` does not match inside `node`.
pub fn suggestions(query: &str) -> Vec<Suggestion> {
    let words = query
        .to_lowercase()
        .chars()
        .map(|ch| if ch.is_alphanumeric() { ch } else { ' ' })
        .collect::<String>();
    let words = words.split_whitespace().collect::<Vec<_>>();
    let mut result = Vec::new();
    for (terms, relation) in GROUPS {
        let present = terms
            .iter()
            .filter(|term| contains_phrase(&words, term))
            .copied()
            .collect::<Vec<_>>();
        for source in &present {
            for term in *terms {
                if !present.iter().any(|p| p.eq_ignore_ascii_case(term))
                    && !result
                        .iter()
                        .any(|s: &Suggestion| s.term.eq_ignore_ascii_case(term))
                {
                    result.push(Suggestion {
                        source,
                        term,
                        relation: *relation,
                    });
                }
            }
        }
    }
    result
}

/// Suggest one-token corrections from the bounded local engineering vocabulary.
/// Callers show these only after a completed zero-result search and require a click.
pub fn typo_corrections(query: &str) -> Vec<TypoCorrection> {
    let vocabulary = GROUPS
        .iter()
        .flat_map(|(terms, _)| terms.iter())
        .flat_map(|term| term.split_whitespace())
        .filter(|word| word.chars().count() >= 4)
        .map(str::to_lowercase)
        .collect::<std::collections::HashSet<_>>();
    let mut cursor = 0;
    let words = query
        .split_whitespace()
        .map(|word| {
            let start = cursor
                + query[cursor..]
                    .find(word)
                    .expect("split token follows prior token");
            cursor = start + word.len();
            (word, start, cursor)
        })
        .collect::<Vec<_>>();
    let mut corrections = Vec::new();
    for (word, start, end) in &words {
        let token_len = word.chars().count();
        if !word.chars().all(char::is_alphabetic)
            || !(4..=MAX_TYPO_TOKEN_CHARS).contains(&token_len)
        {
            continue;
        }
        let lower = word.to_lowercase();
        if vocabulary.contains(&lower) {
            continue;
        }
        let max_distance = if lower.chars().count() > 7 { 2 } else { 1 };
        let mut nearest = vocabulary
            .iter()
            .filter_map(|candidate| {
                let distance = edit_distance(&lower, candidate);
                (distance <= max_distance).then_some((candidate, distance))
            })
            .collect::<Vec<_>>();
        nearest.sort_by_key(|(_, distance)| *distance);
        let Some((candidate, distance)) = nearest.first().copied() else {
            continue;
        };
        if nearest
            .iter()
            .filter(|(_, other_distance)| other_distance == &distance)
            .count()
            != 1
        {
            continue;
        }
        let replacement = if word.chars().all(|ch| !ch.is_lowercase()) {
            candidate.to_uppercase()
        } else {
            candidate.clone()
        };
        let corrected_query = format!("{}{}{}", &query[..*start], replacement, &query[*end..]);
        corrections.push(TypoCorrection {
            original: (*word).to_string(),
            replacement,
            query: corrected_query,
        });
        if corrections.len() == 3 {
            break;
        }
    }
    corrections
}

fn edit_distance(left: &str, right: &str) -> usize {
    let mut previous: Vec<usize> = (0..=right.chars().count()).collect();
    for (i, left_char) in left.chars().enumerate() {
        let mut current = vec![i + 1; previous.len()];
        for (j, right_char) in right.chars().enumerate() {
            current[j + 1] = (previous[j + 1] + 1)
                .min(current[j] + 1)
                .min(previous[j] + usize::from(left_char != right_char));
        }
        previous = current;
    }
    *previous.last().unwrap_or(&0)
}

fn contains_phrase(words: &[&str], phrase: &str) -> bool {
    let phrase = phrase.to_lowercase();
    let needle = phrase.split_whitespace().collect::<Vec<_>>();
    !needle.is_empty() && words.windows(needle.len()).any(|window| window == needle)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engineering_abbreviations_expand_only_on_token_boundaries() {
        let found = suggestions("OD and yield strength");
        assert!(found
            .iter()
            .any(|s| s.source == "OD" && s.term == "outer diameter"));
        assert!(found
            .iter()
            .any(|s| s.source == "yield strength" && s.term == "yield stress"));
        assert!(suggestions("node").is_empty());
    }

    #[test]
    fn terms_already_in_the_query_are_not_suggested_again() {
        let suggestions = suggestions("OD, outside diameter");
        assert!(suggestions
            .iter()
            .all(|s| s.term != "OD" && s.term != "outside diameter"));
        assert!(suggestions.iter().any(|s| s.term == "outer diameter"));
    }

    #[test]
    fn typo_corrections_require_a_unique_nearby_engineering_term() {
        let correction = typo_corrections("ultimate tensile strenght");
        assert_eq!(correction.len(), 1);
        assert_eq!(correction[0].replacement, "strength");
        assert_eq!(correction[0].query, "ultimate tensile strength");
        assert_eq!(
            typo_corrections("  ultimate  tensile strenght  ")[0].query,
            "  ultimate  tensile strength  "
        );
        assert!(typo_corrections("ordinary words").is_empty());
        assert!(typo_corrections(&"a".repeat(MAX_TYPO_TOKEN_CHARS + 1)).is_empty());
    }
}
