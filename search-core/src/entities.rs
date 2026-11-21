use std::collections::BTreeMap;
use std::ops::Range;
use std::sync::OnceLock;

use regex::Regex;
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

impl EntityKind {
    /// Stable storage name (the catalog keys on it; never rename).
    pub fn as_str(self) -> &'static str {
        match self {
            EntityKind::PartNumber => "part_number",
            EntityKind::Material => "material",
            EntityKind::Dimension => "dimension",
            EntityKind::Unit => "unit",
            EntityKind::FastenerCallout => "fastener_callout",
            EntityKind::Standard => "standard",
        }
    }
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

/// A part-number cue ("P/N", "PN", "part number", "part number (P/N)"), without the token after it.
const PART_CUE: &str = r"(?i)\b(?:P\s*/\s*Ns?|P\s*Ns?|PART\s+(?:NUMBERS?|NO\.?))\s*(?:\(\s*P\s*/\s*Ns?\s*\)\s*)?[:#]?";

struct Rule {
    regex: Regex,
    capture: usize,
    kind: EntityKind,
    /// The match must contain a digit: "work-hour" and the "is" of "part number is not installed" are not part numbers.
    needs_digit: bool,
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

/// A number and a unit: group 1 is the number, group 2 the unit. Units the FAA directives actually use (`daN`, `lbs`,
/// `inch-pounds`, `Hz`, `degrees`, ...) sit beside the common ones; time (hours, days) is not a dimension.
const QUANTITY: &str = r"(?i)([-+]?(?:\d+\s+\d+/\d+|\d+/\d+|\d{1,3}(?:,\d{3})+(?:\.\d+)?|\d+(?:\.\d+)?))\s*(in[·-]lbs?|ft[·-]lbs?|lbf[·-]in|inch[·-]pounds?|daN(?:[·.-]?m)?|mm|millimeters?|millimetres?|cm|inches?|in|ft|feet|psi|ksi|MPa|GPa|Hz|rpm|degrees?|lbf|lbs?|N(?:·m|m)?|°\s*[CF])\b";

fn rules() -> &'static [Rule] {
    static RULES: OnceLock<Vec<Rule>> = OnceLock::new();
    RULES.get_or_init(|| {
        [
            (format!(r"{PART_CUE}\s*([A-Z0-9][A-Z0-9._/-]{{0,127}})").as_str(), 1, EntityKind::PartNumber, true),
            (r"(?i)\b[A-Z0-9]{1,12}(?:-[A-Z0-9]{1,12}){1,4}\b", 0, EntityKind::PartNumber, true),
            (r"(?i)\b(?:7075|7050|2024|6061)-?T\d\b|\b(?:17-4(?:\s*PH)?|15-5(?:\s*PH)?|316L?|304L?|A286|Ti-6Al-4V|Inconel\s+718)\b|\b(?:aluminum|aluminium|titanium|stainless\s+steel|magnesium|carbon\s+steel)\b", 0, EntityKind::Material, false),
            (r"(?i)\b(?:M\d+(?:\s*[x×]\s*\d+(?:\.\d+)?)?|\d+/\d+-\d+\s*(?:UNC|UNF|UNEF|UNS)|(?:NAS|MS)\d{3,5})\b", 0, EntityKind::FastenerCallout, false),
            (r"(?i)\b(?:ASTM\s+[A-Z]?\s*\d+(?:-\d+)?|ASME\s+[A-Z]\d+(?:\.\d+)?|SAE\s+[A-Z]\s*\d+|AMS\s+\d+(?:-\d+)?|MIL-(?:DTL|STD|PRF)-[A-Z0-9-]+|NAS\d{3,5})\b", 0, EntityKind::Standard, false),
            (QUANTITY, 0, EntityKind::Dimension, false),
            (QUANTITY, 2, EntityKind::Unit, false),
        ]
        .into_iter()
        .map(|(pattern, capture, kind, needs_digit)| Rule {
            regex: Regex::new(pattern).expect("static entity pattern is valid"),
            capture,
            kind,
            needs_digit,
        })
        .collect()
    })
}

fn add_candidate(
    found: &mut BTreeMap<(usize, usize), (usize, Vec<EntityKind>)>,
    matched: Range<usize>,
    base: usize,
    line_number: usize,
    kind: EntityKind,
) -> bool {
    let key = (base + matched.start, base + matched.end);
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

/// The next item of a part-number list that starts at `from`: after a comma, "or", "and" or a combination, a token of
/// at least four characters that contains a digit ("P/N 200181 or 704A34112012, 704A34112013"). Stops at the first
/// thing that is not an item, so prose after the list ("or inspect") is never taken.
/// A token without the sentence punctuation its character class swallowed ("P/N 200181." names "200181").
fn trim_token(line: &str, range: Range<usize>) -> Range<usize> {
    let kept = line[range.clone()].trim_end_matches(['.', '/', '-']).len();
    range.start..range.start + kept
}

fn next_list_item(line: &str, from: usize) -> Option<Range<usize>> {
    static ITEM: OnceLock<Regex> = OnceLock::new();
    let item = ITEM.get_or_init(|| {
        Regex::new(r"(?i)\A\s*(?:,\s*(?:(?:and|or)\s+)?|(?:and/or|and|or)\s+)([A-Z0-9][A-Z0-9._/-]{3,127})")
            .expect("static list pattern is valid")
    });
    let found = item.captures(line.get(from..)?)?.get(1)?;
    found
        .as_str()
        .bytes()
        .any(|byte| byte.is_ascii_digit())
        .then(|| trim_token(line, from + found.start()..from + found.end()))
}

/// Part-number cue at the very end of a line, and the token that starts the next one: AD text wraps between "P/N" and
/// its number, and the scan is per line.
fn wrapped_part_number(previous_ends_in_cue: bool, line: &str) -> Option<Range<usize>> {
    static START: OnceLock<Regex> = OnceLock::new();
    if !previous_ends_in_cue {
        return None;
    }
    let start = START.get_or_init(|| Regex::new(r"(?i)\A\s*([A-Z0-9][A-Z0-9._/-]{0,127})").expect("static pattern is valid"));
    let token = start.captures(line)?.get(1)?;
    token.as_str().bytes().any(|byte| byte.is_ascii_digit()).then(|| trim_token(line, token.range()))
}

fn ends_in_part_cue(line: &str) -> bool {
    static END: OnceLock<Regex> = OnceLock::new();
    END.get_or_init(|| Regex::new(&format!(r"{PART_CUE}\s*\z")).expect("static pattern is valid"))
        .is_match(line)
}

/// Words that may follow "in" as a unit ("0.5 in thick", "0.25 in x 2 in"); any other word makes it the preposition.
const IN_UNIT_FOLLOWERS: [&str; 18] = [
    "x", "by", "thick", "thickness", "long", "length", "wide", "width", "deep", "depth", "high", "height", "diameter", "dia",
    "radius", "max", "maximum", "minimum",
];

/// Where the quantity "number unit" starts, or `None` when it is not one. A bare `N` and `in` are also a
/// compass letter or model suffix and a preposition, and a number glued to a letter or hyphenated into an identifier belongs to that
/// identifier: on the FAA directives "2701 N Forum Drive", "frame No. 9 in accordance", "SA-365N" and "2013-12-06 in
/// lieu" were 2,500 of the 1,700 Dimension candidates (docs/knowledge-engine-audit.md, 2026-11-21). A leading hyphen
/// after a digit is a range ("5-6 Nm" is 6 Nm), not a minus sign.
fn quantity_start(line: &str, number: Range<usize>, unit: Range<usize>) -> Option<usize> {
    let mut start = number.start;
    let before = line[..start].chars().next_back();
    if line[number.clone()].starts_with('-') {
        match before {
            Some(c) if c.is_alphabetic() => return None,
            Some(c) if c.is_ascii_digit() || c == '.' => start += 1,
            _ => {}
        }
    } else if before.is_some_and(char::is_alphabetic) {
        return None;
    }
    let spaced = number.end != unit.start;
    let rest = line[unit.end..].trim_start();
    let reads_as_unit = match line[unit.clone()].to_ascii_lowercase().as_str() {
        // "SA 365 N" (a helicopter model) and "2701 N Forum" read the same as "50 N"; only "50N" and "12 N/mm" are newtons ("Room 6N-321" is not).
        "n" => (spaced && rest.starts_with(['/', '·'])) || (!spaced && !line[unit.end..].starts_with('-')),
        "in" if spaced => {
            let word: String = rest.chars().take_while(|c| c.is_alphabetic()).collect::<String>().to_lowercase();
            // At a line end the sentence usually goes on ("in\n lieu of"), so only a stop or a unit word counts.
            !rest.is_empty() && (word.is_empty() || IN_UNIT_FOLLOWERS.contains(&word.as_str()))
        }
        _ => true,
    };
    reads_as_unit.then_some(start)
}

pub fn extract_candidates(text: &str) -> EntityExtraction {
    let mut found = BTreeMap::new();
    let mut coverage = EntityCoverage {
        total_lines: text.lines().count().max(1),
        ..Default::default()
    };
    let mut offset = 0;
    let mut stopped = false;
    let mut cue_ended_previous_line = false;
    for (line_index, line) in text.split_inclusive('\n').enumerate() {
        let remaining = MAX_SCAN_BYTES.saturating_sub(coverage.scanned_bytes);
        if line.len() > remaining {
            coverage.skipped_long_lines += 1;
            offset += line.len();
            cue_ended_previous_line = false;
            continue;
        }
        coverage.scanned_bytes += line.len();
        coverage.scanned_lines += 1;
        if let Some(range) = wrapped_part_number(cue_ended_previous_line, line) {
            if add_candidate(&mut found, range, offset, line_index + 1, EntityKind::PartNumber) {
                coverage.candidate_limit_reached = true;
                break;
            }
        }
        cue_ended_previous_line = ends_in_part_cue(line);
        for rule in rules() {
            for captures in rule.regex.captures_iter(line) {
                let Some(matched) = captures.get(rule.capture) else {
                    continue;
                };
                if rule.needs_digit && !matched.as_str().bytes().any(|byte| byte.is_ascii_digit()) {
                    continue;
                }
                let mut range = if rule.capture == 1 && rule.kind == EntityKind::PartNumber {
                    trim_token(line, matched.range())
                } else {
                    matched.range()
                };
                if matches!(rule.kind, EntityKind::Dimension | EntityKind::Unit) {
                    if let (Some(number), Some(unit)) = (captures.get(1), captures.get(2)) {
                        let Some(start) = quantity_start(line, number.range(), unit.range()) else {
                            continue;
                        };
                        if rule.kind == EntityKind::Dimension {
                            range.start = start;
                        }
                    }
                }
                if add_candidate(&mut found, range, offset, line_index + 1, rule.kind) {
                    coverage.candidate_limit_reached = true;
                    stopped = true;
                    break;
                }
                // "P/N A or B, C" names every item of the list, not only the one after the cue.
                if rule.capture == 1 && rule.kind == EntityKind::PartNumber {
                    let mut end = matched.end();
                    while let Some(item) = next_list_item(line, end) {
                        end = item.end;
                        if add_candidate(&mut found, item, offset, line_index + 1, rule.kind) {
                            coverage.candidate_limit_reached = true;
                            stopped = true;
                            break;
                        }
                    }
                    if stopped {
                        break;
                    }
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

    fn quantities(text: &str) -> Vec<String> {
        extract_candidates(text)
            .candidates
            .iter()
            .filter(|candidate| candidate.shape_hints.contains(&EntityKind::Dimension))
            .map(|candidate| text[candidate.bytes.clone()].to_string())
            .collect()
    }

    // Found on the FAA directives: "2701 N Forum Drive" read as 2701 newtons, "No. 9 in accordance" as 9 inches.
    #[test]
    fn a_street_direction_and_the_word_in_are_not_units() {
        assert!(quantities("contact Airbus Helicopters, Inc., 2701 N Forum Drive, Grand Prairie, TX; 2701 N. Forum").is_empty());
        assert!(quantities("installed on the frame No. 9 in accordance with the bulletin, or 3 in the area").is_empty());
        assert!(quantities("Room 6N-321, models SA 365 N, SA-365N, AS-365N2 and B672 mm; for AD 2013-12-06 in\n lieu of").is_empty());
        assert_eq!(quantities("range 5-6 Nm and 1,500 ft"), ["6 Nm", "1,500 ft"]);
        assert_eq!(quantities("a 50N load and 12 N/mm, 0.25 in diameter, 3 in x 4 in, 2 in."), ["50N", "12 N", "0.25 in", "3 in", "4 in", "2 in"]);
    }

    #[test]
    fn units_the_directives_use_are_dimensions() {
        let found = quantities(
            "Apply 5.6 daN.m, 6.8 daN·m or 3 daN (24.7 lbs), 564 inch-pounds, 12 Hz, 10 degrees, 1,000 ft and 10 feet.",
        );
        for expected in ["5.6 daN.m", "6.8 daN·m", "3 daN", "24.7 lbs", "564 inch-pounds", "12 Hz", "10 degrees", "10 feet"] {
            assert!(found.iter().any(|f| f == expected), "{expected} missing from {found:?}");
        }
        assert!(quantities("within 15 hours TIS or 7 days, 4 work-hours").is_empty());
    }

    fn part_numbers(text: &str) -> Vec<(usize, String)> {
        extract_candidates(text)
            .candidates
            .iter()
            .filter(|candidate| candidate.shape_hints.contains(&EntityKind::PartNumber))
            .map(|candidate| (candidate.line_number, text[candidate.bytes.clone()].to_string()))
            .collect()
    }

    // Found by labelling FAA Airworthiness Directives (docs/knowledge-engine-audit.md, 2026-11-20): the cue was
    // dropped after "(P/N)", a list named only its first part number, and the sentence period stayed in the span.
    #[test]
    fn part_number_cues_cover_a_parenthesised_pn_a_list_and_a_closing_period() {
        let found = part_numbers("with a gearbox, part number (P/N) 200181 or 704A34112012, installed.");
        assert_eq!(found.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["200181", "704A34112012"]);
        let found = part_numbers("Pall elements P/Ns CE01303F2, CE01303F2B, and CE01301F2B were omitted. Replace P/N 4639.");
        let texts: Vec<_> = found.iter().map(|(_, t)| t.as_str()).collect();
        for expected in ["CE01303F2", "CE01303F2B", "CE01301F2B", "4639"] {
            assert!(texts.contains(&expected), "{expected} missing from {texts:?}");
        }
    }

    #[test]
    fn a_part_number_list_stops_at_prose_and_short_numbers() {
        let found = part_numbers("Install P/N L671M5040205 or inspect the rod within 100 hours, 12 times, and 2 more.");
        assert_eq!(found.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["L671M5040205"]);
    }

    #[test]
    fn a_part_number_may_start_the_line_after_its_cue() {
        let found = part_numbers("remove coupling tube, P/N\n   C631A1002101, from service.");
        assert_eq!(found, [(2, "C631A1002101".to_string())]);
        assert!(part_numbers("see the P/N\nwork-hour table").is_empty());
        assert!(part_numbers("this part number is not installed").is_empty());
    }

    #[test]
    fn hyphenated_words_without_a_digit_are_not_part_numbers() {
        let found = part_numbers("$85 per work-hour; time-in-service (TIS); AX-42-7 and 44307-480-1 stay.");
        assert_eq!(found.iter().map(|(_, t)| t.as_str()).collect::<Vec<_>>(), ["AX-42-7", "44307-480-1"]);
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
