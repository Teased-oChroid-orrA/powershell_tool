# Saved-search alert design grounding

Goal: implement opt-in, local saved-search alerts for newly matching and materially changed files. Use the existing bundled neural model for semantic change comparison. Keep exact query parsing, matching, and hit membership unchanged. All alert data remains offline.

## Existing ownership and flow

- `app-tui/src/toolboxes/search/persistence.rs` owns `PersistedSearchSettings`, `RecentSearch`, `SavedPreset`, and `PersistedFile`; settings are written atomically to the app data JSON file.
- `app-tui/src/toolboxes/search/mod.rs::SearchSession::run` delegates to the shared `toolboxes::search::run_or_notify` effect path. Search completion flows through `toolboxes::search::apply_event`; exact results remain in `SearchRunState.results`.
- `app-tui/src/gpu_viewer/editor/search/mod.rs::SearchSession::poll` accepts `AppEvent`s; its event reducer must not move filesystem or model work onto the UI thread. Existing semantic and entity operations use async workers and reject stale completion keys.
- `search-core/src/neural_search.rs` owns policy for existing bundled neural embeddings; `search-ml` owns MiniLM ONNX inference. `SearchRunState` results include matched line text, but only while a run is in memory.
- `SearchToolConfig` and `PersistedSearchSettings` already persist local query settings and folder paths. Recent searches and presets have distinct purposes; do not overload them without evidence.
- No app scheduler currently exists. A first complete alert lifecycle may check saved alerts when the user explicitly runs the same saved alert. State the lifecycle clearly; don't claim background monitoring if none runs.

## Hard constraints

- Exact search stays the source of truth for result membership; semantic similarity may label a change but cannot add/remove exact hits.
- Alerts must be opt-in and distinguish first baseline capture from later notifications.
- Store no extracted source text. A bounded vector/fingerprint per matched file is sufficient; local paths are needed to identify changed files.
- Bound number of alerts, files per baseline, vector bytes, and file-read/extraction work. Keep old settings files readable and writes crash-safe.
- Handle same alert, same query scope, and repeated checks idempotently. Ignore stale async results after query or scope changes.
- No network access, no model download, no new model family, no new background daemon.

## Design candidates to produce

1. Compact baseline embedded with each saved alert in a separate versioned sidecar file.
2. Dedicated local SQLite alert store reusing existing dependencies and transaction patterns.

Each candidate must specify a domain shape, lifecycle/UI entry points, bounded semantic comparison, failure behavior, migration compatibility, and testable invariants.
