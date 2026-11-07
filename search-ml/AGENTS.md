# search-ml

## Purpose
Owns local neural inference helpers used by Search. Model weights and tokenizer files are bundled for offline use.

Does not own file traversal, text extraction, exact matching, persistent indexes, or UI. `search-core` owns search policy; `native-search` remains the exact Tantivy index; `app-tui` owns user actions and presentation.

## Current model
- `all-MiniLM-L6-v2` sentence encoder, Apache-2.0, 384-dimensional embeddings.
- `google/flan-t5-small` encoder-decoder, Apache-2.0, for bounded local answer generation from cited search evidence.
- Maximum input is 256 WordPiece tokens. Longer inputs are truncated by the matching tokenizer.
- Generated answers cap questions at 64 tokens and output at 48 tokens. They are untrusted; `search-core` supplies citations and the UI must show the unchanged source lines beside the answer.
- Runtime is pinned to the same `rten` 0.24.0 used by OCR, with `onnx_format` enabled for the bundled model.
- Model inference is CPU-only. Load lazily; a model failure must leave exact search usable.

## Contracts
- No runtime network access or model downloads.
- Do not treat embedding similarity as proof of synonymy, relevance, or engineering equivalence.
- Do not let model output alter exact matching or suppress exact results.
- Any model or tokenizer update must update the asset manifest and invalidate vectors from the prior model version.
- Text generation must stay separate from retrieval policy and exact matching. Keep all model assets bundled for offline use.
- Keep user data and corpus text local.

## Verification
- `cargo test -p search-ml` loads the bundled ONNX model and checks normalized embeddings and paraphrase similarity.
