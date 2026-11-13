# Extractive summary grounding

- Runtime: `search-ml` bundles all-MiniLM-L6-v2 ONNX and tokenizer; `embed_batch` returns local normalized sentence embeddings with 384 dimensions and 256-token model limit. No generative model exists.
- Search policy: `search-core/src/neural_search.rs` owns ranking over bounded candidates; it already ranks cited evidence and preserves exact text.
- UI state: `SearchSession` in `app-tui/src/gpu_viewer/editor/search/mod.rs` owns preview path/generation, tokio runtime, event channels, and stale-request guards. It can schedule summary work off-thread.
- Presentation: `view::cards` constructs the Index panel. Related excerpts and question evidence already display original path/line citations.
- Constraint: summary must select original extracted sentences and cite extracted line numbers. Similarity ranks salience only. It must not invent prose, alter exact results, or imply source-page citations unavailable from extraction.
