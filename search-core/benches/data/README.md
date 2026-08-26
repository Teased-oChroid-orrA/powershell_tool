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

## Not used for correctness testing

These files are extraction-*performance* fixtures only. `extract_lines_by_extension`'s
*correctness* is still proven against `tests/TextInFilesSearch.Tests/Fixtures/`'s
original, hand-verified fixtures - unrelated to this directory.
