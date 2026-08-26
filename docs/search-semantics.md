# Search semantics (issue #6 §45 "Unicode Handling", §46 "Search Semantics")

What this app actually does when it matches text, written down explicitly
so behavior is documented rather than only inferable from source. Applies
to the literal-scan path (`orchestrator`/`matching.rs`) and, where noted,
the index-first/fast-search path (`native-search`/Tantivy).

## Case sensitivity

Always case-insensitive, everywhere, by default - no setting turns this
off. Ordinal/Unicode-aware case folding, not ASCII-only:

- Literal mode: `line.to_lowercase().contains(&filter.to_lowercase())` -
  `str::to_lowercase()` applies full Unicode case folding (handles
  non-ASCII letters correctly - "İ"/"i" Turkish-dotted-I edge cases
  included, since Rust's lowercasing tables are locale-independent
  Unicode default case folding, not locale-sensitive).
- Whole-word and regex mode: `fancy_regex::RegexBuilder::case_insensitive(true)`,
  same Unicode-aware folding via the underlying regex engine.
- Index-first path (Tantivy): the `LowerCaser` token filter, applied both
  at index time and query time via the same registered tokenizer
  instance - see `native-search/src/engine.rs`'s `trigrams_of` doc
  comment for why "same instance" specifically matters here (byte-for-
  byte identical splitting is what makes the trigram narrowing sound).

## Unicode normalization - NOT applied, a known, documented gap

No NFC/NFD/NFKC/NFKD normalization happens anywhere in the matching
pipeline. A filter typed as `"café"` (NFC - "é" as one codepoint, U+00E9)
will **not** match a document containing the visually-identical but
differently-encoded `"café"` stored as NFD ("e" + combining acute accent
U+0301, two codepoints) - `to_lowercase()` and regex matching both operate
on the actual codepoint sequence, not a normalized form. This is a real,
honest limitation, not a silent-but-acceptable one: it can cause a real
match to be missed if the document and the filter happen to use different
normalization forms of the same visible character. In practice this is
rare for ASCII-only or Windows-authored content (Windows text editors
overwhelmingly produce NFC), more plausible for text that started on
macOS (HFS+ historically normalized filenames to NFD) or came through
certain Unicode-heavy pipelines. No normalization step was added because
it wasn't something this session found evidence of actually causing a
problem for this app's real users - adding one speculatively would be
exactly the "solve a problem without evidence" pattern this project's own
philosophy warns against. If it's ever reported as a real issue, the fix
is straightforward: normalize both filter and line to NFC (via the
`unicode-normalization` crate) before comparing, in exactly one place
(`matching.rs`'s literal/whole-word/regex prep functions).

## Invalid byte sequences / non-UTF-8 content

Handled, not corrupted (epic §45's explicit requirement). Encoding
detection order (`file_reader`/`extraction::decode_text`):

1. UTF-8 BOM present → strict UTF-8 decode.
2. No BOM → attempt strict UTF-8 decode.
3. Strict UTF-8 fails → Windows-1252 fallback decode (`encoding_rs`),
   never a lossy/replacement-character UTF-8 decode that would silently
   turn invalid bytes into `U+FFFD` and corrupt surrounding text.

This mirrors the original C# tool's own encoding-detection order exactly
(BOM → UTF-8 → Windows-1252), not a new design - kept for parity, and
because Windows-1252 is far more likely than any other single-byte
encoding to be what a Windows-authored legacy text file that isn't valid
UTF-8 actually is.

## Punctuation and tokenization

- **Literal mode**: pure substring matching - "eng" matches "engine",
  "engineering", "reengineer". No tokenization at all.
- **Whole-word mode**: lookaround-based boundary check
  (`(?<![\p{L}\p{N}_])filter(?![\p{L}\p{N}_])`), not `\b` - `\b` treats
  punctuation as always a word boundary, which breaks a punctuation-edged
  filter like `"C#"` (the `#` would need to itself be non-word-adjacent on
  both sides for `\b` to place a boundary correctly around it; lookaround
  checking against the Unicode letter/number/underscore classes directly
  gets this right). This is *why* `fancy-regex` is used throughout instead
  of the `regex` crate (which has no lookaround support, by design) - see
  CLAUDE.md's design-decisions section.
- **Regex mode**: whatever the user's pattern says - full `fancy-regex`
  syntax (a superset of `regex`'s, including lookaround/backreferences),
  always compiled case-insensitive (see above).
- **Index-first path (Tantivy)**: a real tokenizer (word-boundary based,
  not substring) for the default query-parser search, so `"eng"` alone
  would *not* match `"engine"` there - a deliberate, documented semantic
  difference from the literal-scan default (see "Indexed vs. raw-scan
  parity" below for how this app avoids that difference ever silently
  producing missed results).

## Phrase, wildcard, fuzzy semantics (index-first path only)

The literal-scan path has no separate "phrase"/"wildcard"/"fuzzy" modes -
those are index-first (Tantivy `QueryParser`) concepts:

- **Phrase**: `"quoted text"` - exact token sequence.
- **Prefix/wildcard**: `term*` - **only works on multi-word quoted
  phrases** (`"the engin"*`), not a bare single word. Tantivy 0.26.1's
  `generate_literals_for_str` rejects a single-token phrase-prefix query
  outright (`PhrasePrefixRequiresAtLeastTwoTerms`), and a bare unquoted
  `engin*` silently becomes an ordinary exact-term query for the literal
  text `"engin"` - zero results against `"engine"`, no error. This was
  discovered empirically during this session (not assumed from docs) and
  is exercised by `native-search`'s
  `search_supports_prefix_wildcard_on_multi_word_phrases_only` test -
  read that test before changing anything about wildcard query handling.
- **Fuzzy**: Damerau-Levenshtein edit-distance tolerance via
  `QueryParser::set_field_fuzzy`, exposed as `search_fuzzy`.
- **Regex candidate filtering**: see `docs/issue-6-phase-7.md` -
  `regex_literals::required_literal_chunks` extracts a conservative,
  provably-safe literal substring requirement from a regex pattern for
  index-first narrowing; falls back to a full scan whenever it can't be
  sure.

## Path and filename matching

- Filters never match against the file path or filename - only against
  extracted line content. (Filename/title *ranking boosts* exist on the
  index-first fuzzy/relevance path - `native-search::engine::build_parser`
  boosts the `filename`/`title` fields 3x/2x - but that's relevance
  ordering, not a separate "does this match" predicate.)
- `exclude_folders` matches whole path segments, not a raw substring
  (`node_mod` must not exclude `node_modules_backup`) - already covered
  by an existing test
  (`orchestrator::tests::exclude_folder_prunes_whole_path_segment_not_substring`).

## Indexed vs. raw-scan parity

The one place this app deliberately lets index-first and raw-scan modes
differ (Tantivy's word-tokenized default search vs. literal substring
matching) is bridged, not left as a silent trap: `native-search`'s
trigram field (a second, separately-tokenized field, 3-character n-grams)
is used purely as a *candidate pre-filter* before every real search still
runs the exact same, unchanged literal/whole-word/regex line-scan against
the narrowed candidate file list. The trigram narrowing is a *safe
superset* (see `trigram_candidate_paths`'s doc comment for the proof) -
it can only ever over-select candidate files, never under-select them -
so the index-first path always produces results identical to a full scan
over the same settings, proven by
`native_index::tests::index_first_routing_agrees_with_full_scan` and its
regex-mode sibling. The only user-visible difference between the two
paths is speed, never correctness.
