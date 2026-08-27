//! Parity tests against the same real embedded fixture files the C# test
//! harness uses (`tests/TextInFilesSearch.Tests/Fixtures/*`), reused
//! byte-identical rather than regenerated - see
//! `tests/TextInFilesSearch.Tests/Program.cs` Tests 14/27/28/29 for the C#
//! side of these same expectations.

use search_core::extraction::{
    extract_docx_lines, extract_pdf_lines, extract_pptx_lines, extract_xlsx_lines,
    extract_zip_archive_lines, pdf_extraction_looks_reliable,
};
use search_core::models::{FileSearchStatus, SearchSettings};
use search_core::orchestrator;

const TEST_DOCX: &[u8] = include_bytes!("../../tests/TextInFilesSearch.Tests/Fixtures/test.docx");
const TEST_PPTX: &[u8] = include_bytes!("../../tests/TextInFilesSearch.Tests/Fixtures/test.pptx");
const TEST_PDF: &[u8] = include_bytes!("../../tests/TextInFilesSearch.Tests/Fixtures/test.pdf");
const TEST_XLSX: &[u8] = include_bytes!("../../tests/TextInFilesSearch.Tests/Fixtures/test.xlsx");
const TEST_ZIP: &[u8] = include_bytes!("../../tests/TextInFilesSearch.Tests/Fixtures/test.zip");
const TEST_NOTES_PPTX: &[u8] =
    include_bytes!("../../tests/TextInFilesSearch.Tests/Fixtures/test_notes.pptx");

fn count_ci(haystack: &str, needle: &str) -> usize {
    let haystack_lower = haystack.to_lowercase();
    let needle_lower = needle.to_lowercase();
    if needle_lower.is_empty() {
        return 0;
    }
    haystack_lower.matches(&needle_lower).count()
}

#[test]
fn docx_fixture_finds_apple_and_banana() {
    let lines = extract_docx_lines(TEST_DOCX).expect("real python-docx fixture must extract");
    let joined = lines.join("\n");
    assert!(count_ci(&joined, "apple") >= 1, "expected 'apple' in: {joined}");
    assert!(count_ci(&joined, "banana") >= 1, "expected 'banana' in: {joined}");
}

#[test]
fn pptx_fixture_finds_apple() {
    let lines = extract_pptx_lines(TEST_PPTX).expect("real python-pptx fixture must extract");
    let joined = lines.join("\n");
    assert!(count_ci(&joined, "apple") >= 1, "expected 'apple' in: {joined}");
}

#[test]
fn pdf_fixture_ascii85_flate_chain_finds_apple_and_banana_reliably() {
    let (lines, truncated) = extract_pdf_lines(TEST_PDF, 15, None, false);
    let lines = lines.expect("real ReportLab PDF fixture (ASCII85+FlateDecode) must extract");
    assert!(!truncated);
    let joined = lines.join("\n");
    assert!(count_ci(&joined, "apple") >= 1, "expected 'apple' in: {joined}");
    assert!(count_ci(&joined, "banana") >= 1, "expected 'banana' in: {joined}");
    assert!(
        pdf_extraction_looks_reliable(&lines),
        "clean generated PDF text should look reliable, got: {joined}"
    );
}

#[test]
fn xlsx_fixture_finds_apple_and_banana_via_shared_strings() {
    let lines = extract_xlsx_lines(TEST_XLSX).expect("real xlsx fixture must extract");
    let joined = lines.join("\n");
    assert!(count_ci(&joined, "apple") >= 1, "expected 'apple' in: {joined}");
    assert!(count_ci(&joined, "banana") >= 1, "expected 'banana' in: {joined}");
}

#[test]
fn zip_fixture_finds_apple_in_plain_entry_and_banana_in_nested_docx() {
    let lines = extract_zip_archive_lines(TEST_ZIP, 2).expect("real zip fixture must extract");
    let joined = lines.join("\n");
    assert!(
        count_ci(&joined, "apple") >= 1,
        "expected 'apple' from the plain-text entry in: {joined}"
    );
    assert!(
        count_ci(&joined, "banana") >= 1,
        "expected 'banana' from the nested docx entry in: {joined}"
    );
}

#[test]
fn pptx_notes_fixture_finds_slide_notes_and_smartart_diagram_text() {
    let lines = extract_pptx_lines(TEST_NOTES_PPTX).expect("real pptx-with-notes fixture must extract");
    let joined = lines.join("\n");
    assert!(count_ci(&joined, "apple") >= 1, "expected slide text 'apple' in: {joined}");
    assert!(count_ci(&joined, "banana") >= 1, "expected speaker-notes text 'banana' in: {joined}");
    assert!(
        count_ci(&joined, "cherry") >= 1,
        "expected SmartArt diagram text 'cherry' in: {joined}"
    );
    assert!(joined.contains("notes ---"), "expected a speaker-notes section marker in: {joined}");
    assert!(
        joined.contains("SmartArt diagram"),
        "expected a SmartArt diagram section marker in: {joined}"
    );
}

// ------------------------------------------------------------------
// End-to-end through the full orchestrator - mirrors
// tests/TextInFilesSearch.Tests/Program.cs Tests 14/27/28/29 exactly
// (same fixtures, same filters, same expected hit counts).
// ------------------------------------------------------------------

fn settings_for(dir: &std::path::Path, filters: &[&str]) -> SearchSettings {
    SearchSettings {
        search_path: dir.to_string_lossy().into_owned(),
        output_folder: dir.to_string_lossy().into_owned(),
        filters: filters.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

#[tokio::test]
async fn orchestrator_end_to_end_docx_pptx_pdf_fixtures() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.docx"), TEST_DOCX).unwrap();
    std::fs::write(dir.path().join("test.pptx"), TEST_PPTX).unwrap();
    std::fs::write(dir.path().join("test.pdf"), TEST_PDF).unwrap();

    let settings = settings_for(dir.path(), &["apple", "banana"]);
    let result = orchestrator::run(settings, None, tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();

    let docx = result.file_results.iter().find(|r| r.full_name.ends_with("test.docx")).unwrap();
    assert_eq!(docx.status, FileSearchStatus::Hit);
    let docx_matches: usize = docx.hits.iter().map(|h| h.matched_filters.len()).sum();
    assert_eq!(docx_matches, 2, "DOCX: real python-docx file finds both apple and banana");

    let pptx = result.file_results.iter().find(|r| r.full_name.ends_with("test.pptx")).unwrap();
    assert_eq!(pptx.status, FileSearchStatus::Hit, "PPTX: real python-pptx file finds apple");

    let pdf = result.file_results.iter().find(|r| r.full_name.ends_with("test.pdf")).unwrap();
    assert_eq!(pdf.status, FileSearchStatus::Hit);
    let pdf_matches: usize = pdf.hits.iter().map(|h| h.matched_filters.len()).sum();
    assert_eq!(pdf_matches, 2, "PDF: real ReportLab file (ASCII85+FlateDecode chain) finds both apple and banana");
    assert!(!pdf.low_confidence_pdf, "PDF: extraction confidence looks reliable for clean generated text");
}

#[tokio::test]
async fn orchestrator_end_to_end_xlsx_fixture() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.xlsx"), TEST_XLSX).unwrap();

    let settings = settings_for(dir.path(), &["apple", "banana"]);
    let result = orchestrator::run(settings, None, tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();

    let xlsx = result.file_results.iter().find(|r| r.full_name.ends_with("test.xlsx")).unwrap();
    assert_eq!(xlsx.status, FileSearchStatus::Hit);
    let matches: usize = xlsx.hits.iter().map(|h| h.matched_filters.len()).sum();
    assert_eq!(matches, 2, "XLSX: finds both apple and banana via shared strings");
}

#[tokio::test]
async fn orchestrator_end_to_end_zip_fixture() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test.zip"), TEST_ZIP).unwrap();

    let settings = settings_for(dir.path(), &["apple", "banana"]);
    let result = orchestrator::run(settings, None, tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();

    let zip = result.file_results.iter().find(|r| r.full_name.ends_with("test.zip")).unwrap();
    assert_eq!(zip.status, FileSearchStatus::Hit);
    let matches: usize = zip.hits.iter().map(|h| h.matched_filters.len()).sum();
    assert_eq!(matches, 2, "ZIP: finds apple (plain entry) and banana (nested docx entry)");
}

#[tokio::test]
async fn orchestrator_end_to_end_pptx_notes_fixture() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("test_notes.pptx"), TEST_NOTES_PPTX).unwrap();

    let settings = settings_for(dir.path(), &["apple", "banana", "cherry"]);
    let result = orchestrator::run(settings, None, tokio_util::sync::CancellationToken::new())
        .await
        .unwrap();

    let pptx = result.file_results.iter().find(|r| r.full_name.ends_with("test_notes.pptx")).unwrap();
    assert_eq!(pptx.status, FileSearchStatus::Hit);
    let matches: usize = pptx.hits.iter().map(|h| h.matched_filters.len()).sum();
    assert_eq!(
        matches, 3,
        "PPTX: finds slide text (apple), speaker notes (banana), and SmartArt diagram text (cherry)"
    );
}
