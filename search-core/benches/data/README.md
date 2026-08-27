# Benchmark-only sample documents

Real-world-sized DOCX/PPTX/XLSX/RTF/PDF files used **only** by
`search-core/benches/discovery_and_extraction.rs`'s per-format
extraction benchmark - not correctness fixtures (those live in
`tests/TextInFilesSearch.Tests/Fixtures/` and stay tiny/hand-crafted on
purpose, reused byte-identical by `search-core/tests/fixtures.rs`).

## Why these exist

The original per-format benchmark used the 1-4KB correctness fixtures,
which are far smaller than a typical real document - not representative
of real extraction cost. These are pulled from established open-source
projects' own test-data corpora (not a random file-sharing site), each
file is a real, normally-authored document (not a synthetic filler
string), covering a `medium` (~150KB-450KB) and `large` (~1MB-3MB) tier
per format alongside the existing tiny correctness fixtures, giving three
real size tiers instead of one.

## Source and license

All files: **Apache License 2.0** (each source project's license),
downloaded 2026-08-26 directly from each project's `trunk`/`main` branch
test-data directory (verified via each file's exact byte count matching
what GitHub's API reported before download, and `file(1)` confirming each
is genuinely the claimed format - not re-hosted from a third party).

| File | Source | Original path |
|---|---|---|
| `medium.docx`, `large.docx` | [apache/poi](https://github.com/apache/poi) | `test-data/document/WordWithAttachments.docx`, `test-data/document/saut_page.docx` |
| `medium.pptx`, `large.pptx` | [apache/poi](https://github.com/apache/poi) | `test-data/slideshow/themes.pptx`, `test-data/slideshow/KEY02.pptx` |
| `medium.xlsx`, `large.xlsx` | [apache/poi](https://github.com/apache/poi) | `test-data/spreadsheet/picture.xlsx`, `test-data/spreadsheet/58325_db.xlsx` |
| `medium.rtf`, `large.rtf` | [apache/tika](https://github.com/apache/tika) | `.../tika-parser-microsoft-module/src/test/resources/test-documents/testRTFRegularImages.rtf`, `testRTFEmbeddedFiles.rtf` |
| `medium.pdf`, `large.pdf` | [apache/pdfbox](https://github.com/apache/pdfbox) | `pdfbox/src/test/resources/input/cweb.pdf`, `.../pdmodel/interactive/form/PDFBOX-5784.pdf` |

Apache POI, Tika, and PDFBox are all Apache Software Foundation projects
whose test-data directories exist specifically to give document-format
parsers (exactly this codebase's `search-core::extraction` module) real
files to test against - the same reason this benchmark needs them, not
files scraped from an unrelated site.

## `xlarge` tier (~10MB+), added 2026-08-26

Added in response to a follow-up request to also benchmark files in the
~10MB+ range and concurrent/mixed-format search
(`search-core/benches/concurrent_extraction.rs`). None of POI/Tika/PDFBox's
own test-data corpora had a real file this large for every format, so two
additional legitimate sources were used: [sample-files.com](https://sample-files.com)
(a site whose stated purpose is hosting real, freely-downloadable sample
files of exactly this kind - verified by downloading directly and checking
`file(1)` output and content, not assumed from the page) and
[arXiv.org](https://arxiv.org) (freely-redistributable academic PDFs).

| File | Bytes | Source | Notes |
|---|---|---|---|
| `xlarge.docx` | 11,317,142 | sample-files.com, `large-doc.docx` | Real, image-heavy DOCX; extracts successfully. |
| `xlarge.pdf` | 5,853,703 | arXiv.org, paper [2303.18223](https://arxiv.org/abs/2303.18223) | Real, text-heavy PDF; extracts successfully (verified via `search-cli` - 7114 real hits on a test filter). The "representative" xlarge PDF. |
| `xlarge-scanned.pdf` | 38,589,556 | sample-files.com, `large-doc.pdf` | Real PDF, but image-only (scanned pages, `/Im1 Do` content streams, zero `Tj`/`TJ` text-showing operators, raw uncompressed `/DeviceRGB`/`/FlateDecode` page images at 2479x3509). Without the `ocr` Cargo feature/setting enabled, this extractor correctly returns no text - kept deliberately as a real "large file, zero extractable text" edge case for concurrency/robustness testing, not a bug. **Also** the fixture the OCR feature was built and proven against (`extraction::tests::ocr_extracts_real_text_from_the_real_scanned_pdf_fixture`, `#[cfg(feature = "ocr")]`) - with OCR enabled, real readable text ("PDF", "Testing", "sample-files.com", ...) is correctly extracted from it. |
| `xlarge-recordheavy.xlsx` | 12,364,136 | apache/tika, `test-documents/testRecordSizeExceeded.xlsx` | Deliberately pathological Tika test fixture: its single worksheet entry decompresses from 12.4MB to ~328MB. Correctly rejected by `search-core`'s `ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES` (20MB) zip-bomb guard before any real parse work happens - the guard working as designed, not a bug. Kept as a real-world pathological-compression-ratio edge case. |

No real ~10MB+ PPTX or RTF was found from a source this project treats as
legitimate - `large.pptx` (2.28MB) and `large.rtf` (1.23MB) stay the
biggest real tiers for those two formats. Documented as a real gap in
`discovery_and_extraction.rs`'s `format_fixtures()`, not silently skipped.

## Not used for correctness testing

These files are extraction-*performance* fixtures only. `extract_lines_by_extension`'s
*correctness* is still proven against `tests/TextInFilesSearch.Tests/Fixtures/`'s
original, hand-verified fixtures - unrelated to this directory.
