# Issue #6 Phase 7: regex candidate filtering (§24)

Closes the one real gap left in issue #6's "Search Modes"/"Query Planner"
sections: regex-mode searches previously always fell back to a full,
unfiltered file scan even when a fast-search index existed for the
folder, because the index-first routing added in Phase 1 explicitly
excluded `use_regex` (correctly, at the time - there was no safe way yet
to narrow a regex search by trigram presence).

## Design: conservative literal-chunk extraction, not a regex analyzer

New module `search-core/src/regex_literals.rs`,
`required_literal_chunks(pattern: &str) -> Option<Vec<String>>`. Finds
substrings *guaranteed* to appear, contiguously, in any string the
pattern matches - the same "necessary but not sufficient" safety property
the existing plain-filter trigram narrowing relies on.

Deliberately not a general regex engine. It walks the pattern once and:

- Escaped literal punctuation (`\.`, `\+`, `\\`, ...) → literal text.
- `\d \w \s \b` and friends → safe run-terminator, contributes nothing.
- `. ^ $` → safe run-terminator.
- `* + ?` → drops the immediately-preceding atom **and splits the run
  there** (not just a drop - see below for why the split is required).
- `( ) [ ] { } |` → **bails immediately** (`None`, meaning "don't narrow,
  full scan") - groups, character classes, alternation, and bounded
  quantifiers are not analyzed at all, not even partially.
- A trailing unescaped `\` → bails.
- Chunks under 3 characters produce no trigrams and are dropped; if a
  filter ends up with zero usable chunks, the whole call returns `None`
  (mirrors `trigram_candidate_paths`'s existing "one bad filter falls back
  the whole query" behavior, not a silent per-filter skip).

### The correctness trap this design exists to avoid

An earlier draft just dropped the atom before a quantifier and kept
building the same literal run around it. That's wrong: `ab+c` matches
both "abc" and "abbc" - the latter does **not** contain "abc" as a
contiguous substring (it's "abb" then "bc"). Requiring "abc" would be an
unsound narrowing - it could exclude a file `orchestrator::run` (full
scan) would have found, exactly the class of bug this whole feature must
never introduce. Fix: split the run at the quantifier, so characters
before and after it are never treated as one required chunk. Verified two
ways per adversarial case, not just asserted: `colou?r` → chunks
`["colo"]` (not the wrong, naive `["color"]`), proven against a real
`fancy_regex::Regex` that both `"color"` and `"colour"` actually match and
both actually contain `"colo"`; `ab+c` → no usable chunks at all, proven
against `"abbc"` (a real match) which provably does *not* contain `"abc"`.

## Wiring (`native-search`, `app/src/state.rs`)

- `NativeSearchEngine::trigram_candidate_paths_for_chunk_sets(&[Vec<String>])` -
  same OR-across-filters/AND-within-filter boolean-query shape as the
  existing `trigram_candidate_paths`, refactored to share the query-build
  and searcher/doc-fetch tail (`must_all_trigrams`, `run_candidate_query`)
  rather than duplicating it. Each filter's chunk set contributes the
  union of every chunk's own trigrams (computed separately per chunk via
  the existing `trigrams_of`, never by concatenating chunks first - that
  would fabricate trigrams spanning the very gap the split above exists to
  avoid).
- `AppState::run_search`'s index-first routing (previously gated on
  `!use_regex`) now computes, for regex mode, `chunk_sets` via
  `filters.iter().map(required_literal_chunks).collect::<Option<Vec<_>>>()`
  - `Option`'s `FromIterator` short-circuits to `None` the instant any one
  filter fails to extract, which is exactly the desired "any uncertainty
  anywhere means fall back to a full scan" behavior, for free.

## Verification

`cargo test --workspace`: **167/167 passing** (app 8, native-search 28 +
13 ffi_smoke, search-cli 4, search-core 104 + 10 fixtures). New tests:
11 in `regex_literals.rs` (including the two adversarial-proof tests
above), 3 in `native-search/engine.rs` for
`trigram_candidate_paths_for_chunk_sets`, and one new end-to-end test in
`native_index.rs` - `regex_mode_index_first_routing_agrees_with_full_scan` -
mirroring the existing plain-filter agreement test: indexes 3 real files,
narrows via a regex pattern (`"eng.*mount"` → chunks `["eng","mount"]`),
runs both `orchestrator::run` (full scan) and `orchestrator::run_candidates`
(narrowed) against the same settings, and asserts they find exactly the
same hit files.
