# Vendored `ocrs`

Upstream: <https://github.com/robertknight/ocrs>, version 0.12.2, license `MIT OR Apache-2.0` (Robert Knight).
Its MIT license text is in `LICENSE-MIT` here and, for releases, `licenses/ocrs-LICENSE-MIT.txt`.
Taken from the crates.io release so the build stays offline and the recognition output can be extended.

Changes against upstream 0.12.2 (everything else is the upstream source):

- `src/text_items.rs`: `TextChar` gains `confidence: f32`, the model's probability (0 to 1) for the character.
- `src/recognition.rs`: after CTC decoding, each emitted character gets the highest probability the model gave its label
  over the run of frames that decoded to it; `LineRecResult` carries those values to `text_lines_from_recognition_results`.
- `Cargo.toml`: examples, benches and most dev-dependencies removed; `publish = false`.

Why: the crate exposes no recognition score, so `search-core` could not tell a well-read scan from a poorly read one
(`search-core/AGENTS.md`, "OCR confidence"). Upstream's decoding, batching and character text are unchanged, so the
recognised text is byte-for-byte what 0.12.2 produces.
