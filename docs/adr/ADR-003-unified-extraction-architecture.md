# ADR-003: Unified Extraction Architecture

Status: Proposed

## Problem

Section 3 of the epic asks for a Rust `TextExtractor` trait abstraction
covering PDF, DOCX, PPTX, XLSX, RTF, TXT, MD, CSV, TSV, JSON, XML, YAML,
TOML, INI, ENV, LOG, HTML, CSS/SCSS/LESS, source code, and ZIP/archive
contents, using mature Rust crates rather than custom parsers.

## Evidence

- Every one of those formats is already extracted today, in C#, zero
  third-party dependencies, in `TextExtractionService.cs` /
  `FileReaderService.cs`, backed by 60 integration tests and multiple
  previously-fixed extraction bugs (see `docs/native-search-assessment.md`
  and `CLAUDE.md`'s "Bug classes already found and fixed once").
- ADR-001 already decided the Rust module's initial scope is indexing/search
  only, consuming already-extracted text from .NET.

## Decision

No Rust extraction layer is built in this phase. The "unified extraction
abstraction" the epic asks for already exists — as `TextExtractionService`'s
per-format methods plus `FileReaderService`'s plain-text path — and is
reused as-is via ADR-001's boundary (.NET extracts, Rust indexes what .NET
hands it).

This ADR exists to record that Section 3 was evaluated and consciously
deferred, not overlooked — re-open it only if a concrete requirement emerges
that the existing extraction can't serve (e.g., a background indexer that
needs to extract text without the .NET UI in the loop first).

## Consequences

- No PDF/DOCX/XLSX/RTF/ZIP crates get evaluated or vendored in this phase —
  removes a large chunk of Section 22's "verify every crate" burden until/
  unless this decision is revisited.
- The native module's input contract is simple: `(doc_id, path,
  extracted_text, metadata)`, not raw file bytes — narrows the FFI surface
  (serves Section 16 directly).
- If this is revisited later, the comparison is existing-tested-C# vs.
  candidate-Rust-crates on a per-format basis, not a wholesale swap.

## Rejected alternatives

- **Build the Rust extraction trait now, in parallel with existing C#
  extraction** — rejected: doubles the maintenance surface for identical
  functionality with no near-term consumer, directly against Section 23's
  "do not overengineer" and Section 1's "do not duplicate existing
  functionality without justification."
