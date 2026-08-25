# ADR-009: FFI Serialization Strategy

Status: Accepted (implemented in `native-search/src/ffi.rs` / `NativeSearchInterop.cs`)

## Problem

Section 16 requires a narrow, stable ABI that doesn't expose Rust-specific
types (`String`, `Vec<T>`, `Result<T>`, trait objects) directly across the
boundary, with explicit ownership. Search results are a variable-length
list of structured records (`SearchHit`) — some encoding has to carry that
across the FFI boundary.

## Alternatives considered

1. **Fixed-layout C structs**, one `SearchHit` struct mirrored field-for-
   field in both languages, returned as a raw array + count.
2. **A binary schema/codegen format** (Protobuf, FlatBuffers, Cap'n Proto).
3. **JSON**, serialized to a byte buffer, handed across as `(ptr, len)`.

## Evidence

- Tantivy's own dependency tree already includes `serde` (used throughout
  its schema/document types) — adding `serde_json` on the Rust side is not
  a new dependency *category*, just one more crate in an already-JSON-
  adjacent stack.
- `System.Text.Json` is part of the .NET base class library — zero new
  NuGet package on the C# side, consistent with `Core`'s zero-third-party-
  package invariant (`CLAUDE.md`).
- `docs/ffi.md`'s own "Buffers" convention (a `(ptr, len)` allocation the
  Rust side owns until `ns_free_buffer` releases it) already works for any
  byte payload — JSON doesn't need a different buffer-ownership story than
  a raw struct array would.
- Section 23 explicitly prohibits unnecessary complexity absent a
  demonstrated need; a result set of ~9 scalar fields per hit, at the scale
  this app searches (a local file index, not millions of QPS), does not
  demonstrate a need for a binary/codegen serialization format.

## Decision

JSON. `ns_search` serializes `Vec<SearchHit>` via `serde_json::to_vec`,
returns the bytes as `(ptr, len)`, and `NativeSearchService.Search`
deserializes via `System.Text.Json` with `JsonNamingPolicy.SnakeCaseLower`
(matching serde's default `snake_case` field-name output against C#'s
PascalCase record properties — see `NativeSearchModels.cs`'s doc comment).

## Consequences

- No fixed C struct layout to keep in sync across a 32/64-bit or alignment-
  sensitive boundary — the JSON schema (field names/types) is the only
  contract to maintain, and it's readable/debuggable by inspecting a raw
  buffer dump if something goes wrong.
- Per-call JSON parse/serialize overhead exists but is negligible relative
  to the search itself at this app's scale (a local file-search tool, not
  a high-QPS service) — not benchmarked separately because Section 23's
  own guidance is not to add complexity without a demonstrated need, and
  no need has been demonstrated.
- The JSON schema is an implicit contract between `engine::SearchHit`
  (Rust) and `NativeSearchHit` (C#) — renaming or retyping a field on
  either side without updating the other silently breaks deserialization
  (a missing/null field, not a compile error). This is the one real cost
  of choosing JSON over a schema-checked binary format; documented as a
  known tradeoff, not fixed by any compile-time enforcement in this pass.
- If future query results need genuinely large payloads (e.g. returning
  full document bodies instead of small metadata + score) at a scale where
  JSON overhead becomes measurable, that's a concrete, demonstrated need
  Rejected alternative 2 could be revisited against — not a preemptive
  optimization taken now.

## Rejected alternatives

- **Fixed-layout C structs** — rejected: every field addition/reorder
  becomes an ABI-breaking change requiring exact struct-layout agreement
  (packing, alignment, string encoding) on both sides, with no compiler
  check that they match — a classic FFI foot-gun Section 16's "avoid
  exposing Rust-specific types" guidance is implicitly steering away from,
  even though it doesn't name this alternative directly.
- **Protobuf/FlatBuffers/Cap'n Proto** — rejected per Section 23's "do not
  overengineer": these add a codegen build step, a schema-definition
  language to maintain in a third place (beyond the Rust struct and the C#
  record), and a new dependency category, for a result shape simple enough
  that JSON already solves it with tooling already present on both sides.
