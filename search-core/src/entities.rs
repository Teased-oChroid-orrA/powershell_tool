use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::OnceLock;

use regex::{Match, Regex};
use tokio_util::sync::CancellationToken;

use search_ml::{cosine_similarity, embed_tokens_batch, token_count, Embedding, TokenEmbedding};

const MAX_SCAN_BYTES: usize = 1_048_576;
const MAX_CANDIDATES: usize = 256;
const ENTITY_CONTEXT_TOKENS: usize = 240;
const INFERENCE_BATCH: usize = 16;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub enum EntityKind {
    PartNumber,
    Material,
    Dimension,
    Unit,
    FastenerCallout,
    Standard,
}

#[derive(Clone, Debug, PartialEq)]
pub struct EntityCandidate {
    pub bytes: Range<usize>,
    pub line_number: usize,
    pub shape_hints: Vec<EntityKind>,
    pub category_scores: Vec<EntityScore>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct EntityScore {
    pub kind: EntityKind,
    pub similarity: f32,
}

impl EntityCandidate {
    /// Highest-scoring neural category admitted by the exact pattern hints.
    pub fn neural_suggestion(&self) -> Option<&EntityScore> {
        self.category_scores
            .iter()
            .find(|score| self.shape_hints.contains(&score.kind))
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct EntityCoverage {
    pub total_lines: usize,
    pub scanned_lines: usize,
    pub scanned_bytes: usize,
    pub skipped_long_lines: usize,
    pub candidate_limit_reached: bool,
}

#[derive(Clone, Debug, Default, PartialEq)]
pub struct EntityExtraction {
    pub candidates: Vec<EntityCandidate>,
    pub coverage: EntityCoverage,
}

struct Rule {
    regex: Regex,
    capture: usize,
    kind: EntityKind,
}

const ANCHORS: [(EntityKind, &str, &str); 18] = [
    (
        EntityKind::PartNumber,
        "Part number PN-42-A identifies the replacement bracket.",
        "PN-42-A",
    ),
    (
        EntityKind::PartNumber,
        "The catalog lists component AX-21-7 for replacement.",
        "AX-21-7",
    ),
    (
        EntityKind::PartNumber,
        "P/N M8X1 identifies a bolt in the spare parts list.",
        "M8X1",
    ),
    (
        EntityKind::Material,
        "The material is 7075-T6 aluminum alloy.",
        "7075-T6",
    ),
    (
        EntityKind::Material,
        "Use Ti-6Al-4V titanium for the fitting.",
        "Ti-6Al-4V",
    ),
    (
        EntityKind::Material,
        "The bracket is machined from 17-4 PH stainless steel.",
        "17-4 PH",
    ),
    (
        EntityKind::Dimension,
        "The bore diameter is 0.25 in.",
        "0.25 in",
    ),
    (
        EntityKind::Dimension,
        "The shaft length measures 12 mm.",
        "12 mm",
    ),
    (
        EntityKind::Dimension,
        "Maintain 1/2 inch clearance.",
        "1/2 inch",
    ),
    (
        EntityKind::Unit,
        "The bore diameter is measured in 0.25 in.",
        "in",
    ),
    (EntityKind::Unit, "The length is 12 mm.", "mm"),
    (EntityKind::Unit, "Clearance is 1/2 inch.", "inch"),
    (
        EntityKind::FastenerCallout,
        "Install an M6x1 screw.",
        "M6x1",
    ),
    (
        EntityKind::FastenerCallout,
        "Use 1/4-28 UNF bolts.",
        "1/4-28 UNF",
    ),
    (
        EntityKind::FastenerCallout,
        "Select an M8x1.25 fastener.",
        "M8x1.25",
    ),
    (EntityKind::Standard, "Comply with ASTM A36.", "ASTM A36"),
    (
        EntityKind::Standard,
        "Piping design follows ASME B31.3.",
        "ASME B31.3",
    ),
    (
        EntityKind::Standard,
        "Use SAE J429 as the fastener standard.",
        "SAE J429",
    ),
];

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (r"(?i)(?:P\s*/\s*N|P\s*N|PART\s+(?:NUMBER|NO\.?))\s*[:#]?\s*([A-Z0-9][A-Z0-9._/-]{0,127})", 1, EntityKind::PartNumber),
            (r"(?i)\b[A-Z0-9]{1,12}(?:-[A-Z0-9]{1,12}){1,4}\b", 0, EntityKind::PartNumber),
            (r"(?i)\b(?:7075|7050|2024|6061)-?T\d\b|\b(?:17-4(?:\s*PH)?|15-5(?:\s*PH)?|316L?|304L?|A286|Ti-6Al-4V|Inconel\s+718)\b|\b(?:aluminum|aluminium|titanium|stainless\s+steel|magnesium|carbon\s+steel)\b", 0, EntityKind::Material),
            (r"(?i)\b(?:M\d+(?:\s*[x×]\s*\d+(?:\.\d+)?)?|\d+/\d+-\d+\s*(?:UNC|UNF|UNEF|UNS)|(?:NAS|MS)\d{3,5})\b", 0, EntityKind::FastenerCallout),
            (r"(?i)\b(?:ASTM\s+[A-Z]?\s*\d+(?:-\d+)?|ASME\s+[A-Z]\d+(?:\.\d+)?|SAE\s+[A-Z]\s*\d+|AMS\s+\d+(?:-\d+)?|MIL-(?:DTL|STD|PRF)-[A-Z0-9-]+|NAS\d{3,5})\b", 0, EntityKind::Standard),
            (r"(?i)([-+]?(?:\d+\s+\d+/\d+|\d+/\d+|\d+(?:\.\d+)?))\s*(in[·-]lb|ft[·-]lb|lbf[·-]in|mm|millimeters?|millimetres?|cm|inches?|in|ft|psi|ksi|MPa|GPa|lbf|lb|N(?:·m|m)?|°\s*[CF])\b", 0, EntityKind::Dimension),
            (r"(?i)([-+]?(?:\d+\s+\d+/\d+|\d+/\d+|\d+(?:\.\d+)?))\s*(in[·-]lb|ft[·-]lb|lbf[·-]in|mm|millimeters?|millimetres?|cm|inches?|in|ft|psi|ksi|MPa|GPa|lbf|lb|N(?:·m|m)?|°\s*[CF])\b", 2, EntityKind::Unit),
        ]
        .into_iter()
        .map(|(pattern, capture, kind)| Rule {
            regex: Regex::new(pattern).expect("static entity pattern is valid"),
            capture,
            kind,
        })
        .collect()
    })
}

fn add_candidate(
    found: &mut BTreeMap<(usize, usize), (usize, Vec<EntityKind>)>,
    matched: Match<'_>,
    base: usize,
    line_number: usize,
    kind: EntityKind,
) -> bool {
    let key = (base + matched.start(), base + matched.end());
    if let Some((_, kinds)) = found.get_mut(&key) {
        if !kinds.contains(&kind) {
            kinds.push(kind);
            kinds.sort_unstable();
        }
        false
    } else if found.len() == MAX_CANDIDATES {
        true
    } else {
        found.insert(key, (line_number, vec![kind]));
        false
    }
}

pub fn extract_candidates(text: &str) -> EntityExtraction {
    let mut found = BTreeMap::new();
    let mut coverage = EntityCoverage {
        total_lines: text.lines().count().max(1),
        ..Default::default()
    };
    let mut offset = 0;
    let mut stopped = false;
    for (line_index, line) in text.split_inclusive('\n').enumerate() {
        let remaining = MAX_SCAN_BYTES.saturating_sub(coverage.scanned_bytes);
        if line.len() > remaining {
            coverage.skipped_long_lines += 1;
            offset += line.len();
            continue;
        }
        coverage.scanned_bytes += line.len();
        coverage.scanned_lines += 1;
        for rule in rules() {
            for captures in rule.regex.captures_iter(line) {
                let Some(matched) = captures.get(rule.capture) else {
                    continue;
                };
                if add_candidate(&mut found, matched, offset, line_index + 1, rule.kind) {
                    coverage.candidate_limit_reached = true;
                    stopped = true;
                    break;
                }
            }
            if stopped {
                break;
            }
        }
        offset += line.len();
        if stopped {
            break;
        }
    }
    let candidates = found
        .into_iter()
        .map(
            |((start, end), (line_number, shape_hints))| EntityCandidate {
                bytes: start..end,
                line_number,
                shape_hints,
                category_scores: Vec::new(),
            },
        )
        .collect();
    EntityExtraction {
        candidates,
        coverage,
    }
}

fn span_embedding(tokens: &[TokenEmbedding], span: Range<usize>) -> Result<Embedding, String> {
    let mut values = vec![0.0; 384];
    let mut count = 0;
    for token in tokens
        .iter()
        .filter(|token| token.byte_range.start < span.end && span.start < token.byte_range.end)
    {
        for (sum, value) in values.iter_mut().zip(token.embedding.values()) {
            *sum += value;
        }
        count += 1;
    }
    if count == 0 {
        return Err("The model returned no vectors for a candidate span.".into());
    }
    for value in &mut values {
        *value /= count as f32;
    }
    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
    if !norm.is_finite() || norm <= f32::EPSILON {
        return Err("The model returned an invalid candidate vector.".into());
    }
    values.iter_mut().for_each(|value| *value /= norm);
    Embedding::from_values(values).map_err(|error| error.to_string())
}

fn prototypes() -> Result<&'static [(EntityKind, Embedding)], String> {
    static PROTOTYPES: OnceLock<Result<Vec<(EntityKind, Embedding)>, String>> = OnceLock::new();
    PROTOTYPES
        .get_or_init(|| {
            let mut grouped = BTreeMap::<EntityKind, Vec<Embedding>>::new();
            for batch in ANCHORS.chunks(INFERENCE_BATCH) {
                let texts = batch.iter().map(|(_, text, _)| *text).collect::<Vec<_>>();
                let vectors = embed_tokens_batch(&texts).map_err(|error| error.to_string())?;
                for ((kind, text, value), tokens) in batch.iter().zip(vectors) {
                    let start = text.find(value).ok_or("Invalid entity prototype span")?;
                    let vector = span_embedding(&tokens, start..start + value.len())?;
                    grouped.entry(*kind).or_default().push(vector);
                }
            }
            grouped
                .into_iter()
                .map(|(kind, examples)| {
                    let mut values = vec![0.0; 384];
                    for example in &examples {
                        for (sum, value) in values.iter_mut().zip(example.values()) {
                            *sum += value / examples.len() as f32;
                        }
                    }
                    let norm = values.iter().map(|value| value * value).sum::<f32>().sqrt();
                    if !norm.is_finite() || norm <= f32::EPSILON {
                        return Err("The model returned an invalid category prototype.".into());
                    }
                    values.iter_mut().for_each(|value| *value /= norm);
                    Embedding::from_values(values)
                        .map(|prototype| (kind, prototype))
                        .map_err(|error| error.to_string())
                })
                .collect()
        })
        .as_deref()
        .map_err(Clone::clone)
}

fn move_left(text: &str, start: usize, limit: usize, count: usize) -> usize {
    if count == 0 {
        return start;
    }
    text[limit..start]
        .char_indices()
        .rev()
        .nth(count.saturating_sub(1))
        .map_or(limit, |(index, _)| limit + index)
}

fn move_right(text: &str, end: usize, limit: usize, count: usize) -> usize {
    if count == 0 {
        return end;
    }
    text[end..limit]
        .char_indices()
        .nth(count.saturating_sub(1))
        .map_or(limit, |(index, ch)| end + index + ch.len_utf8())
}

fn context(text: &str, span: Range<usize>) -> Result<(String, Range<usize>), String> {
    let line_start = text[..span.start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[span.end..]
        .find('\n')
        .map_or(text.len(), |index| span.end + index);
    let mut start = move_left(text, span.start, line_start, 128);
    let mut end = move_right(text, span.end, line_end, 128);
    while token_count(&text[start..end]).map_err(|error| error.to_string())? > ENTITY_CONTEXT_TOKENS
    {
        let left_chars = text[start..span.start].chars().count();
        let right_chars = text[span.end..end].chars().count();
        if left_chars == 0 && right_chars == 0 {
            return Err("A candidate exceeds the model's token limit.".into());
        }
        if left_chars >= right_chars && left_chars > 0 {
            start = move_right(text, start, span.start, 16);
        } else {
            let remaining = right_chars.saturating_sub(16);
            end = move_right(text, span.end, end, remaining);
        }
    }
    Ok((
        text[start..end].to_string(),
        span.start - start..span.end - start,
    ))
}

pub fn extract_entities(
    text: &str,
    cancel: &CancellationToken,
) -> Result<EntityExtraction, String> {
    if cancel.is_cancelled() {
        return Err("Entity analysis cancelled.".into());
    }
    let mut extraction = extract_candidates(text);
    if extraction.candidates.is_empty() {
        return Ok(extraction);
    }
    let category_prototypes = prototypes()?;
    let contexts = extraction
        .candidates
        .iter()
        .map(|candidate| context(text, candidate.bytes.clone()))
        .collect::<Result<Vec<_>, _>>()?;
    for (batch_index, batch) in contexts.chunks(INFERENCE_BATCH).enumerate() {
        if cancel.is_cancelled() {
            return Err("Entity analysis cancelled.".into());
        }
        let inputs = batch
            .iter()
            .map(|(text, _)| text.as_str())
            .collect::<Vec<_>>();
        let encoded = embed_tokens_batch(&inputs).map_err(|error| error.to_string())?;
        if cancel.is_cancelled() {
            return Err("Entity analysis cancelled.".into());
        }
        for (offset, (tokens, (_, span))) in encoded.iter().zip(batch).enumerate() {
            let vector = span_embedding(tokens, span.clone())?;
            let candidate = &mut extraction.candidates[batch_index * INFERENCE_BATCH + offset];
            let mut scores = category_prototypes
                .iter()
                .map(|(kind, prototype)| {
                    cosine_similarity(&vector, prototype)
                        .map(|similarity| EntityScore {
                            kind: *kind,
                            similarity,
                        })
                        .ok_or_else(|| {
                            "The model returned mismatched vector dimensions.".to_string()
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            scores.sort_by(|left, right| {
                right
                    .similarity
                    .total_cmp(&left.similarity)
                    .then(left.kind.cmp(&right.kind))
            });
            candidate.category_scores = scores;
        }
    }
    Ok(extraction)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn finds_all_engineering_shapes_with_overlapping_dimensions_and_units() {
        let text = "µ P/N AX-42-7; 7075-T6 aluminum; bore 0.25 in; use M6x1 per ASTM A36.";
        let result = extract_candidates(text);
        let has = |needle: &str, kind| {
            result.candidates.iter().any(|candidate| {
                text.get(candidate.bytes.clone()) == Some(needle)
                    && candidate.shape_hints.contains(&kind)
            })
        };
        assert!(has("AX-42-7", EntityKind::PartNumber));
        assert!(has("7075-T6", EntityKind::Material));
        assert!(has("0.25 in", EntityKind::Dimension));
        assert!(has("in", EntityKind::Unit));
        assert!(has("M6x1", EntityKind::FastenerCallout));
        assert!(has("ASTM A36", EntityKind::Standard));
        assert!(result
            .candidates
            .iter()
            .all(|candidate| text.get(candidate.bytes.clone()).is_some()));
    }

    #[test]
    fn material_grades_need_grade_markers_and_explicit_part_labels_win_as_hints() {
        let text = "Report year 2024. Use P/N 2024-T3 and material Ti-6Al-4V.";
        let result = extract_candidates(text);
        let year = text.find("2024.").unwrap();
        assert!(!result.candidates.iter().any(|candidate| {
            candidate.bytes.start == year && candidate.shape_hints.contains(&EntityKind::Material)
        }));
        let part = result
            .candidates
            .iter()
            .find(|candidate| text.get(candidate.bytes.clone()) == Some("2024-T3"))
            .unwrap();
        assert!(part.shape_hints.contains(&EntityKind::PartNumber));
        assert!(part.shape_hints.contains(&EntityKind::Material));
    }

    #[test]
    fn engineering_evaluation_specs_find_nas_ams_and_torque_entities() {
        let text = "Fastener NAS6204; torque 95 in·lb per AMS 5643.";
        let result = extract_candidates(text);
        let has = |needle: &str, kind| {
            result.candidates.iter().any(|candidate| {
                text.get(candidate.bytes.clone()) == Some(needle)
                    && candidate.shape_hints.contains(&kind)
            })
        };
        assert!(has("NAS6204", EntityKind::FastenerCallout));
        assert!(has("NAS6204", EntityKind::Standard));
        assert!(has("AMS 5643", EntityKind::Standard));
        assert!(has("95 in·lb", EntityKind::Dimension));
        assert!(has("in·lb", EntityKind::Unit));
    }

    #[test]
    fn scans_only_complete_lines_and_caps_candidate_count() {
        let long = format!("{}\nnext P/N AX-2", "x".repeat(MAX_SCAN_BYTES));
        let skipped = extract_candidates(&long);
        assert_eq!(skipped.coverage.skipped_long_lines, 1);
        let next_line = skipped
            .candidates
            .iter()
            .find(|candidate| &long[candidate.bytes.clone()] == "AX-2")
            .unwrap();
        assert_eq!(next_line.line_number, 2);

        let many = (0..MAX_CANDIDATES + 5)
            .map(|i| format!("P/N AX-{i}\n"))
            .collect::<String>();
        let bounded = extract_candidates(&many);
        assert_eq!(bounded.candidates.len(), MAX_CANDIDATES);
        assert!(bounded.coverage.candidate_limit_reached);
    }

    #[test]
    fn neural_extraction_ranks_all_categories_without_changing_source_spans() {
        let text = "µ P/N AX-42-7; 7075-T6 aluminum; bore 0.25 in; use M6x1 per ASTM A36.";
        let exact = extract_candidates(text);
        let result = extract_entities(text, &CancellationToken::new()).unwrap();
        assert_eq!(
            result
                .candidates
                .iter()
                .map(|candidate| &candidate.bytes)
                .collect::<Vec<_>>(),
            exact
                .candidates
                .iter()
                .map(|candidate| &candidate.bytes)
                .collect::<Vec<_>>()
        );
        assert!(result.candidates.iter().all(|candidate| {
            candidate.category_scores.len() == 6
                && candidate
                    .category_scores
                    .iter()
                    .all(|score| score.similarity.is_finite())
                && candidate
                    .neural_suggestion()
                    .is_some_and(|score| candidate.shape_hints.contains(&score.kind))
                && text.get(candidate.bytes.clone()).is_some()
        }));
    }

    #[test]
    fn neural_extraction_stops_when_cancelled_before_work() {
        let cancel = CancellationToken::new();
        cancel.cancel();
        assert!(extract_entities("P/N AX-42", &cancel)
            .unwrap_err()
            .contains("cancelled"));
    }
}
