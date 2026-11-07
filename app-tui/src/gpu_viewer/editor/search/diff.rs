//! Bounded, exact comparison of extracted document lines.
use tokio_util::sync::CancellationToken;

pub(super) const MAX_BYTES: usize = 1_048_576;
const MAX_LINES: usize = 1_500;
const MAX_CELLS: usize = 2_250_000;
const MAX_ROWS: usize = 100;
const MAX_LINE_CHARS: usize = 240;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Side {
    Source,
    Candidate,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Row {
    pub side: Side,
    pub line: usize,
    pub text: String,
    pub clipped: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct LineDiff {
    pub added: usize,
    pub removed: usize,
    pub unchanged: usize,
    pub rows: Vec<Row>,
    pub omitted: usize,
}

pub(super) fn compare(
    source: &[String],
    candidate: &[String],
    cancelled: &CancellationToken,
) -> Result<LineDiff, String> {
    let bytes = source
        .iter()
        .chain(candidate)
        .fold(0usize, |total, line| total.saturating_add(line.len()));
    if bytes > MAX_BYTES {
        return Err("Extracted comparison text exceeds 1 MiB.".into());
    }
    if source.len() > MAX_LINES || candidate.len() > MAX_LINES {
        return Err("Extracted comparison exceeds 1,500 lines per file.".into());
    }
    let cells = source.len().saturating_mul(candidate.len());
    if cells > MAX_CELLS {
        return Err("Comparison exceeds the bounded work limit.".into());
    }

    let width = candidate.len() + 1;
    let mut lcs = vec![0u16; (source.len() + 1) * width];
    for i in (0..source.len()).rev() {
        if cancelled.is_cancelled() {
            return Err("Comparison cancelled.".into());
        }
        for j in (0..candidate.len()).rev() {
            let at = i * width + j;
            lcs[at] = if source[i] == candidate[j] {
                lcs[(i + 1) * width + j + 1] + 1
            } else {
                lcs[(i + 1) * width + j].max(lcs[i * width + j + 1])
            };
        }
    }

    let (mut i, mut j) = (0, 0);
    let mut diff = LineDiff {
        added: 0,
        removed: 0,
        unchanged: 0,
        rows: Vec::new(),
        omitted: 0,
    };
    while i < source.len() || j < candidate.len() {
        if cancelled.is_cancelled() {
            return Err("Comparison cancelled.".into());
        }
        if i < source.len() && j < candidate.len() && source[i] == candidate[j] {
            diff.unchanged += 1;
            i += 1;
            j += 1;
        } else if i < source.len()
            && (j == candidate.len() || lcs[(i + 1) * width + j] >= lcs[i * width + j + 1])
        {
            diff.removed += 1;
            push_row(&mut diff, Side::Source, i + 1, &source[i]);
            i += 1;
        } else {
            diff.added += 1;
            push_row(&mut diff, Side::Candidate, j + 1, &candidate[j]);
            j += 1;
        }
    }
    Ok(diff)
}

fn push_row(diff: &mut LineDiff, side: Side, line: usize, text: &str) {
    if diff.rows.len() == MAX_ROWS {
        diff.omitted += 1;
        return;
    }
    let mut chars = text.chars();
    let clipped_text: String = chars.by_ref().take(MAX_LINE_CHARS).collect();
    let clipped = chars.next().is_some();
    diff.rows.push(Row {
        side,
        line,
        text: clipped_text,
        clipped,
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn lines(values: &[&str]) -> Vec<String> {
        values.iter().map(|s| (*s).into()).collect()
    }

    #[test]
    fn compares_exact_lines_and_bounds_output() {
        let token = CancellationToken::new();
        let got = compare(
            &lines(&["same", "old", "tail"]),
            &lines(&["same", "new", "tail"]),
            &token,
        )
        .unwrap();
        assert_eq!((got.added, got.removed, got.unchanged), (1, 1, 2));
        assert_eq!(got.rows[0].side, Side::Source);
        assert_eq!(got.rows[1].side, Side::Candidate);
    }

    #[test]
    fn handles_identical_inserted_deleted_and_repeated_lines() {
        let token = CancellationToken::new();
        let same = lines(&["repeat", "repeat"]);
        let got = compare(&same, &same, &token).unwrap();
        assert_eq!((got.added, got.removed, got.unchanged), (0, 0, 2));
        assert_eq!(compare(&[], &lines(&["new"]), &token).unwrap().added, 1);
        assert_eq!(compare(&lines(&["old"]), &[], &token).unwrap().removed, 1);
        let repeated = compare(&lines(&["x", "x"]), &lines(&["x"]), &token).unwrap();
        assert_eq!(repeated.rows[0].line, 2);
    }

    #[test]
    fn rejects_oversized_inputs_and_reports_omitted_rows() {
        let token = CancellationToken::new();
        assert!(compare(&vec!["x".into(); MAX_LINES + 1], &[], &token).is_err());
        assert!(compare(&vec!["x".repeat(MAX_BYTES + 1)], &[], &token).is_err());
        let got = compare(&vec!["a".into(); MAX_ROWS + 1], &[], &token).unwrap();
        assert_eq!(got.rows.len(), MAX_ROWS);
        assert_eq!(got.omitted, 1);
        let clipped = compare(&[], &["x".repeat(MAX_LINE_CHARS + 1)], &token).unwrap();
        assert!(clipped.rows[0].clipped);
        assert_eq!(clipped.rows[0].text.chars().count(), MAX_LINE_CHARS);
        let cancelled = CancellationToken::new();
        cancelled.cancel();
        assert_eq!(
            compare(&lines(&["a"]), &lines(&["b"]), &cancelled).unwrap_err(),
            "Comparison cancelled."
        );
    }

    #[test]
    fn accepts_but_bounds_the_maximum_comparison_matrix() {
        let token = CancellationToken::new();
        let source = vec!["same".to_string(); MAX_LINES];
        let candidate = source.clone();
        let got = compare(&source, &candidate, &token).unwrap();
        assert_eq!(got.unchanged, MAX_LINES);
        assert!(got.rows.is_empty());
    }
}
