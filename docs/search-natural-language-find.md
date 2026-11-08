# Natural-language Find (Search workbench, feature `llm-find`)

Type a sentence such as "files where corrosion is blended, and the surface filled and faired". A local language model
(Qwen3-4B-Instruct-2507, Q4_K_M GGUF, llama.cpp on the CPU) splits it into conditions; the hybrid semantic index
retrieves passages for each condition; the model judges the best passages of the most promising files; results show
the unchanged cited passage, a quote the model copied from it, and a fit label. Off by default.

## Pieces

| Piece | Where |
|---|---|
| Trait, prompts, GBNF grammars, condition grounding, quote check | `search-llm/src/lib.rs` (no native code) |
| llama.cpp engine (feature `llama`) | `search-llm/src/engine.rs` |
| Orchestration, generic over `ConditionModel` (feature `nl-find` in `search-core`) | `search-core/src/nl_find.rs` |
| Panel: sentence input, model path, condition chips, Find, cited results (feature `llm-find` in `app-tui`) | `app-tui/src/gpu_viewer/editor/search/nl.rs`, `view.rs` |
| Real-model judge check | `search-llm/tests/judge_parity.rs` (set `SEARCH_LLM_GGUF`) |

## Rules the design keeps

- The model never writes answers. A "yes" is accepted only when its quote is a substring of the passage; a condition
  is kept only when at least half of its content words relate to words of the sentence, and its terms are limited to
  word forms of the sentence. A condition that is not entirely the user's words is marked "!" and shown unlit.
- The user edits the conditions before any search: click a pill to remove it, click "+" between two to merge them.
- Labels are "all matched", "one weak" (three or more wanted conditions, one unconfirmed) and "partial"; never a
  percentage. A negated condition only excludes a file. At least one condition must be wanted.
- Only the 10 best-ranked candidate files are judged (2 passages per file and condition) because one model call costs
  seconds on a laptop CPU. A file with no passage for a condition inside the top-100 retrieval is unconfirmed.
- The language model does not index or OCR. OCR is `ocrs`/`rten` (`ocr.rs`); embeddings are MiniLM (`search-ml`).

## Evidence (2026-11-07 and 2026-11-08, 8 GB Apple M1, CPU, heavily loaded machine)

- Judge, 41 pairs: Python bake-off 0.951 accuracy, 0 false positives; Rust engine now identical (0.951, 0 false
  positives). Both give "no" for the two cases "corrosion is blended" against "…blended out to a 0.020 in depth…" and
  "Light surface corrosion was removed by blending…": those are model answers, not a parity bug.
- Issue #0128 root cause: the judge grammar forced a compact `{"quote":`, the model's natural output is
  `{ "quote": `; with the compact form the quote came out garbled (", corrosion was removed…") and the verdict was
  discarded. Chat template and tokenization were byte-identical to the Python harness.
- Metal: `llama-cpp-sys-2` enables Metal on macOS even with zero GPU layers (about 26 s of shader compiling on first
  load). `.cargo/config.toml` sets `GGML_METAL=OFF`; changing it needs `cargo clean -p llama-cpp-sys-2`. After the
  change: load 10.8 s, parse of the example sentence 18 s, one judge call 8 s (noisy: load average 6 to 20).

## Windows / CI

`qualification.yml` and `app-tui-build.yml` build without `llm-find`, so CI never compiles llama.cpp; `search-llm`'s
default build is the trait only and `Cargo.lock` lists the llama crates because lock files are feature-agnostic.
Shipping `llm-find` on Windows is not verified: it needs CMake, a C++ toolchain and libclang (all present on hosted
Windows runners), and the static-CRT release needs llama.cpp built with `LLAMA_STATIC_CRT` (see its `build.rs`).
Add `--features llm-find` to `app-tui-build.yml` and run it by hand once before relying on it. The GGUF (about 2.4 GB)
is not shipped; the user sets its path in the panel.

## Not done

- Judge threshold tuning on the user's own wording; grounding thresholds are from synthetic bake-off data.
- Windows run of the panel; the model-path field is a typed path (no file picker).
