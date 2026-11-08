# Search: OCR and PDF line quality (2026-11-08)

Question from the owner: stop gibberish-like lines (`l1|~ . ,; Il`, single-character lines, broken glyph runs) from reaching the index and the result snippets, and see how the local language model could be used in PDF scanning, indexing and OCR.

Short answer: the gibberish was mostly not OCR noise. Measured on the repository's real PDFs, the dominant source was the PDF text extractor itself (one "line" per kerned string fragment, and binary font programs scanned as if they were page text). Those are fixed at the source. A deterministic line scorer (no model, about 2.5 microseconds per line) removes what is left. Neither MiniLM nor the language model adds anything measurable over it, and the language model is far too slow to run per line.

## Where lines become index text

`extraction::extract_pdf_lines_capture` produces the lines of a PDF: text-layer strings, hex-string (CID) text resolved through `/ToUnicode`, and, only when the text layer is empty and the OCR setting is on, `ocr::ocr_image` lines per page image. Every consumer reads those same lines: the Tantivy body (`native_index::read_and_extract`, `lines.join("\n")`), the semantic passages (`semantic_index::embed_lines_with_pages`, with a parallel per-line `origins` vector for page citations), the live-search result cache (`lines_cache`) and the snippets, which are cut from the indexed or cached lines. So the right place to filter is the end of `extract_pdf_lines_capture`, keeping `origins` aligned; nothing downstream needs to change.

## What was measured

Inputs: `search-core/benches/data/{medium,large,xlarge}.pdf` and, with the `ocr` feature, `xlarge-scanned.pdf` (a real scan). These are the only real PDFs on the machine; the OCR sample is a clean synthetic document, so it under-represents real OCR noise.

| File | Lines before | After joining `TJ` arrays | After scorer | Notes |
|---|---|---|---|---|
| `medium.pdf` (CWEB manual) | 34,863 (12,762 single character, 21,569 of at most 2 characters) | 3,955 | 3,349 | 15.3% of joined lines dropped |
| `xlarge.pdf` (arXiv LLM survey) | 160,692 (22,499 single character, 52,462 of at most 2) | 25,475 | 23,391 | 8.2% dropped |
| `large.pdf` (a signed form) | 2,678 | 4,693 | 0 | the only "text" is embedded TrueType font programs; now reported as no extractable text |
| `xlarge-scanned.pdf`, OCR | 1,402 OCR lines | n/a | 1,350 | 52 dropped (3.7%): lone letters (`T`, `V`, `B`), punctuation, fragments `mP`, `mN`, `mI`, `Wn`, `in?tu` |

Real gibberish classes found, in order of volume:

1. Kerned text split into fragments. A `TJ` array such as `[(Do)-32(c)1(um)-1(en)31(ta)-1(ti)-1(on)]TJ` emitted one line per string: `Do`, `c`, `um`, `en`, `ta`. About 62% of `medium.pdf` lines were two characters or shorter. These words were also unsearchable. Fix: `extraction::text_run_re` and `join_tj_array` join the array into one line, with a space where the adjustment is -150 thousandths of an em or more negative (kerns seen are 0 to -90, word spaces -250 to -450).
2. Binary data scanned as page text. Any decompressed stream containing the bytes `Tj` or `TJ` had its `(...)` runs emitted, including font programs, which gave certificate and glyph-table residue (`ÿÎi¹ô!fÜæìF=`). Fix: `looks_like_content_stream` requires at least 60% printable ASCII (random bytes are about 40%, real content streams over 90%).
3. Encoding damage that made good text look like binary. Windows-1252 quotes, dashes and bullets arrive as C1 bytes (`\x93models,\x94`); Computer Modern ligatures arrive as `\x0c` (fi) and friends (`\x0cles` for files). `line_quality::repair_encoding` maps both back. Font-specific ligature codes in other fonts (`\x01nancial`, `\x02rst`) cannot be recovered without parsing the font's `/Differences`; the stray control code is removed so the line is clean but the word stays damaged.
4. Lone punctuation, accents and single letters from math and glyph tables (`,`, `(`, `´`, `n`, `x`).
5. OCR speckle (the original example class): stroke confusions, lone letters, symbol fragments.

## The scorer (`search-core/src/line_quality.rs`)

It classifies each whitespace token as good (a word, a number, a part number, a unit), a capital-letter label, neutral (operators and punctuation), or noise (stroke confusions such as `l1` and `Il`, symbols wedged inside a short run of letters, repeated lowercase letters, vowel-less long runs, tokens full of Latin-1 accents, random case with accents, digit runs salted with `!%#$&`). A line is dropped when it has no good token, when control characters or Latin-1 junk dominate, or when it has one or two good tokens next to a mass of noise. It is kept as soon as one token looks like content, which is what protects `MIL-STD-1553`, `AN3-5A`, `0.125 in`, `Ftu = 62 ksi`, `3/8-24 UNF`, `σ = F/A`, units without vowels (`psf`, `lbf`, `rpm`), roman numerals, plural acronyms (`MLLMs`, `GPUs`), TeX and C code lines, URLs and accented European words. OCR lines are judged slightly harder than text-layer lines: a lone capital letter is kept in a text layer (it is usually a symbol) but dropped from OCR. A document whose only lines are lone capitals is treated as having no text.

Drops are also applied before the OCR fallback, so a PDF whose text layer is all junk now falls through to OCR instead of being indexed as junk. Per-file progress reporting is unchanged: the filter is a pass over lines already produced, and the progress callbacks and OCR per-image timeout checks are untouched.

Cost: 2.4 to 2.8 microseconds per line on this machine (release build), about 60 ms for the 23,000 lines of `xlarge.pdf` against 270 ms for the extraction itself, with no allocation for lines without control characters.

## Evaluation set and honest limits

`search-core/test-data/line-quality-labels.tsv` holds 307 real lines: 198 to keep, 88 to drop and 21 known misses. They were sampled from the dumps above (random, enriched for short and odd lines) and labelled by me while reviewing the filter's decisions line by line, so agreement on the keep and drop sets is by construction and is a regression guard, not an accuracy estimate. The informative findings are from the review itself: I read every dropped line that contained three or more letters plus a random 70 of the other dropped lines; none was real text except two mangled words that lost a ligature (`reection,"`, label withheld); of the odd-looking lines it kept, 21 were noise I could not get it to drop. The known misses are: glyph-index fragments that look like acronyms (`BH`, `BTH`, `BTND`, `XW`), ASCII glyph runs (`%(."+2"`), and three garbled OCR lines made of word-shaped tokens (`w lavascri  eI  it Vou we`, `EEON SEANE`). Nothing short of a language model or a dictionary can see the last class. Real OCR noise from poor scans was not available, so recall on it is unmeasured; the false-drop side, which matters more, was checked as described above on three documents and on the OCR output.

Deliberately not done:

- Hyphen joining across lines (`optimiza-` / `tion`). Without a dictionary it cannot tell it from `high-` / `temperature`, and removing the hyphen would break an exact search for the compound.
- A word list or character n-gram model. It would catch the garbled-OCR class but flags engineering vocabulary and part numbers.
- Applying the scorer to DOCX, PPTX, XLSX, RTF, ZIP or plain text. They are not OCR or glyph-decoding sources and the evidence above does not cover them.

Behaviour changes to expect: PDFs report far fewer lines (line numbers in PDF hits shift, and phrases split across kerned strings now match); a PDF with only binary or font data is reported as no extractable text; the whole Tantivy index rebuilds once (`INDEX_SEMANTIC_VERSION` 2 to 3), PDF passages re-embed (`CITATION_VERSION` 1 to 2) and the result cache is dropped (`cache::EXTRACTION_VERSION`, new, part of the fingerprint).

## Can MiniLM or the language model help?

MiniLM (measured). `search-ml` embeddings are L2-normalised, so the norm is exactly 1.0 for every line and carries no signal. Cosine to the centroid of the good lines, the natural outlier test, gave an area under the ROC curve of 0.54 over all noise versus text (chance is 0.5) and 0.63 on only the 21 lines the scorer keeps. At a threshold that catches 7 of those 21, it throws away 55 of 198 good lines. It also costs 4.8 ms per line, about 2,000 times the scorer. It gives nothing.

Language model (not measured on lines). There is no GGUF on this machine and `llama-cpp-python` is not installed, so I did not run Qwen3-4B. The recorded figure (`.audit/llm-condition-bakeoff-2026-11-07.md`) is 3.4 to 9.5 s per call on CPU. Classifying every line would be 23,000 calls for one 144-page paper; even 40 lines per prompt is about 600 calls, around 1.7 hours for one file, against 0.3 seconds for the scorer. That rules out index-time use outright, whatever its accuracy. Its one plausible contribution is the class the scorer cannot see, garbled OCR made of word-shaped tokens, and there it could only label (keep or drop), never rewrite: rewriting OCR text would introduce words that are not on the page and break the exact-text guarantee. Verdict: the model adds nothing over the heuristic for this problem at any affordable cost. If real noisy scans appear, the sensible use is offline: label a few hundred sampled OCR lines to extend `line-quality-labels.tsv` and re-tune thresholds, once, by hand.

## Follow-ups

- OCR recognition confidence (`.audit/issue-238-ocr-confidence.todo.md`, in progress elsewhere): when a calibrated per-line score exists, combine it with `Source::Ocr` in `clean_lines`; it is the only other cheap signal and it would address the garbled-but-word-shaped class.
- Font `/Differences` parsing to recover non-Computer-Modern ligature codes.
- More real scans for the label set; re-measure drop and miss rates when they exist.
