//! The Search help: the unchanged lines of `help.txt`, ranked against a question and cited by section.
//! Nothing is generated; a question the help does not cover finds no lines.

use std::sync::OnceLock;

use search_core::neural_search::LineIndex;

const TEXT: &str = include_str!("help.txt");

/// Questions offered before the first one is asked.
pub(super) const EXAMPLES: [&str; 4] = [
    "How do I leave a folder out of a search?",
    "What does the semantic index need?",
    "Why is searching scanned PDFs slow?",
    "What does one weak mean?",
];

/// A top line less similar to the question than this is not about the Search app.
const MIN_SIMILARITY: f32 = 0.3;
const SHOWN: usize = 2;

/// One help line and the section it is cited from.
#[derive(Debug, Clone, PartialEq)]
pub(super) struct HelpAnswer {
    pub section: String,
    pub line: String,
    pub similarity: f32,
}

struct Help {
    index: LineIndex,
    /// For each line of `index`, its section title.
    sections: Vec<String>,
}

/// `(section title, line)` for each fact of the help text; `#` lines are maintainer comments.
fn parse(text: &str) -> Vec<(String, String)> {
    let mut section = String::new();
    let mut lines = Vec::new();
    for raw in text.lines() {
        let line = raw.trim();
        if let Some(title) = line.strip_prefix("## ") {
            section = title.trim().to_string();
        } else if !line.is_empty() && !line.starts_with('#') {
            lines.push((section.clone(), line.to_string()));
        }
    }
    lines
}

fn help() -> Result<&'static Help, String> {
    static HELP: OnceLock<Result<Help, String>> = OnceLock::new();
    HELP.get_or_init(|| {
        let (sections, lines): (Vec<_>, Vec<_>) = parse(TEXT).into_iter().unzip();
        LineIndex::build(lines)
            .map(|index| Help { index, sections })
            .map_err(|error| error.to_string())
    })
    .as_ref()
    .map_err(Clone::clone)
}

/// The help lines that answer `question`, best first; empty when the help does not cover it.
pub(super) fn answer(question: &str) -> Result<Vec<HelpAnswer>, String> {
    let help = help()?;
    let ranked = help
        .index
        .rank(question, SHOWN)
        .map_err(|error| error.to_string())?;
    Ok(ranked
        .into_iter()
        .filter(|found| found.similarity >= MIN_SIMILARITY)
        .map(|found| HelpAnswer {
            section: help.sections[found.index].clone(),
            line: help.index.lines()[found.index].clone(),
            similarity: found.similarity,
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_keeps_facts_under_their_section_and_skips_comments() {
        let lines = parse("## A\n# src: x\nOne fact.\n\n## B\nTwo fact.\n");
        assert_eq!(
            lines,
            vec![("A".into(), "One fact.".into()), ("B".into(), "Two fact.".into())]
        );
    }

    #[test]
    fn the_help_text_has_sections_and_no_line_outside_one() {
        let lines = parse(TEXT);
        assert!(lines.len() > 100, "{} lines", lines.len());
        assert!(lines.iter().all(|(section, _)| !section.is_empty()));
    }
}
