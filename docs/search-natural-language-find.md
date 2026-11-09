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
- Only the 10 best-ranked candidate files are judged (up to 8 passages per file and condition for a strong candidate, 1 for a file whose best passage is under half as similar as the best file's) because one model call costs
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

## Corpus test (2026-11-08, real model, 14 mixed files)

Corpus: 14 public files in 10 types (the FAA advisory circular AC 43.13-1B PDF of 21 MB, two NASA reports, a novel as
`.txt`/`.html`/`.zip`, a CSV, a README, `.docx`, `.xlsx`, `.pptx`), run with `corpus_probe`
(`FEA_CORPUS`, `FEA_MODEL`, `FEA_SENTENCE`, `FEA_QUERIES`; `cargo test -p app-tui --features llm-find corpus_probe -- --ignored --nocapture`).

- The sentence "find surfaces that were blended and fastener holes were spot faced" first returned no file at all.
  Three causes, all fixed: keyword scoring matched only the words as typed (the manual says "blending", "blended");
  the judge saw the two passages with the highest cosine (a contents page, "bonding surfaces") instead of the ones
  holding the condition's word forms; and the 800-character passage buried the one relevant sentence for a 4B model.
  Now word forms are stemmed in `lexical::terms`, `judge_pair` judges the passages with the most word-form coverage
  first (3 per pair), and `nl_find::focus` hands the model only the best sentences of the passage.
- The judge prompt said a requirement counts as not met; a manual's "should be faired or blended" then never matched.
  It now accepts instructions and still rejects negations, unmet requirements, other items and other properties
  of the same item. Eval (45 pairs, 4 added): 0.933 and 1 false positive before and after.
- Result now: the FAA manual, fit Partial, quote "Rework depressions by forming smoothly blended dish-outs, using a
  ratio of 20:1, length to depth." No file says "spot faced" (the manual has one "counterbore"), so the second
  condition is correctly unconfirmed. MiniLM does not bridge that jargon: "spot faced" retrieves unrelated passages.
- Negative result: a similarity floor for file lines in Ask. On this corpus, cosine of passages that do not answer
  (0.27 to 0.31, a novel and a CSV for "counterbore") overlaps cosine of passages that do (0.28 to 0.43), so no floor
  separates them. Not shipped.
- Cost: parse 30 s, find about 2.5 minutes for two conditions on a loaded 8 GB M1 in a debug build.
- Extraction note: the PDF has soft hyphens at line ends (`re­ moved`); `focus` rejoins them for the model, but the
  keyword index still sees split words.

## Second corpus round (2026-11-08): "search fasteners which size is specified?"

- The parser kept only the clause ("size specified"). `restore_subject` puts the noun before the first relative word
  back in front of each condition ("fasteners size specified"); the prompt carries an example of it.
- The model judged correctly (it accepts "AN3-5A bolts with a 3/16 inch diameter" and rejects fastener text with no
  size). Retrieval was the weak point: "fasteners size specified" is abstract, so it retrieves generic fastener text.
  Each condition now carries a model-written `example` sentence ("The fastener size is specified as M8 x 1.25."),
  used only to retrieve more passages; passages are still judged against the condition text, so nothing the model
  invents is ever searched or reported as a condition.
- Judging order: retrieval order and word-form coverage are interleaved, up to 8 passages for a strong candidate
  file, stopping at the first confirmation. `wanted_stems` ignores one- and two-character and numeric stems (the `x`
  and `8` of "M8 x 1.25" matched scraps of every passage). `focus` keeps the best sentences with their neighbours.
- When nothing is confirmed, up to 5 files are shown as `Fit::Unconfirmed` ("Closest passages (not confirmed)"),
  only those whose best passage is at least half as similar as the best file's (and at least 0.2); a search is no
  longer a dead end. They are not judged and are labelled so.
- Result on the 14-file corpus: the FAA manual, AllMatched, 90 s; the cited line ("Solid shank rivets ... size of
  shank") names size as an attribute rather than giving one, so it is a loose match.
- Model calls: the engine keeps its context and reuses the KV cache for the common prompt prefix
  (`search-llm/src/engine.rs`, `Session`). Judge calls on the 45-pair set: mean 5.5 s against about 9.5 s, same
  accuracy (0.933, 1 false positive).
- PDF words broken over two lines by a soft hyphen (`re` + U+00AD, then `moved`) are joined in
  `line_quality::join_soft_hyphens`. This changes extracted text, so `INDEX_SEMANTIC_VERSION` is 4,
  `CITATION_VERSION` 3 and `EXTRACTION_VERSION` 2: existing indexes rebuild once (11 minutes for the 21 MB FAA PDF).

## Windows / CI

`llm-find` is a default feature of `app-tui` since 2026-11-08 (owner request), so every default `app-tui` build and the
CI jobs that build it now compile llama.cpp (CMake and a C++ toolchain required; first build takes minutes). With no
model path set the feature does nothing and Search behaves as before. `search-llm`'s own default build is the trait only.
Not yet verified on the hosted Windows runners: run `app-tui-build.yml` by hand and check `qualification.yml`.
Shipping `llm-find` on Windows is not verified: it needs CMake, a C++ toolchain and libclang (all present on hosted
Windows runners), and the static-CRT release needs llama.cpp built with `LLAMA_STATIC_CRT` (see its `build.rs`).
Add `--features llm-find` to `app-tui-build.yml` and run it by hand once before relying on it. The GGUF (about 2.4 GB)
is not shipped; the user sets its path in the panel.

## Not done

- Judge threshold tuning on the user's own wording; grounding thresholds are from synthetic bake-off data.
- Windows run of the panel; the model-path field is a typed path (no file picker).

## 2026-11-08 update: the Qwen / `search-llm` design was replaced by `nl_rules`

Everything above that describes the Qwen3-4B judge, the `search-llm` crate, the `llm-find` feature and the GGUF model path
is historical. The crate was removed from the working tree and Natural-language Find is now rule-based:
`search_core::nl_rules` (no model, no download, no CMake/C++ toolchain) owns the parsing, and `app-tui`'s `nl.rs`
(feature `nl-find`) owns the panel. The bake-off numbers and the failure analysis above remain the reason a generative
judge was not kept (17.5 s per parse on an 8 GB machine, one false positive class that rules avoid). The "Windows / CI"
and "Not done" sections no longer apply. Current behaviour and limits: `app-tui/src/gpu_viewer/editor/search/AGENTS.md`.
