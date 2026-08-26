# Issue #6 Phase 9: archive extraction bomb guards (§62)

## What was already there

`search-core/src/extraction.rs`'s nested-zip path (`extract_zip_archive_lines`/
`extract_zip_entries`, used for standalone `.zip` files, including nested
zips-inside-zips) already had real, working zip-bomb protection: a
recursion-depth cap (`max_depth`), a shared entry-count budget
(`ZIP_MAX_ENTRIES_SCANNED`), a per-entry declared-size cap
(`ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES`), and a total-uncompressed-bytes
budget shared across the whole scan tree (`ZIP_MAX_UNCOMPRESSED_BYTES_TOTAL`,
deliberately *not* reset per nesting level - resetting per level is a
classic zip-bomb-protection bypass). Path traversal protection was never
needed and isn't added here: entry names are only ever used as a display
label and for extension-dispatch, never to construct a filesystem path
for a write - this code is pure in-memory text extraction, so the
classic "zip-slip" vulnerability (writing an entry to
`some_dir.join(entry.name())` without checking for `../`) has no code
path to occur through. Macro/script execution is likewise structurally
impossible - extraction is regex tag-stripping over XML text, there is no
interpreter anywhere in this pipeline.

## The real gap: DOCX/PPTX/XLSX entry reads were unbounded

`extract_docx_lines`/`extract_pptx_lines`/`extract_xlsx_lines` all read
their zip entries (`word/document.xml`, each slide/notes/diagram entry,
sheet XML, shared strings) through a single shared helper,
`read_zip_entry_to_string` - which, unlike the nested-zip path, had no
size check at all: a plain `entry.read_to_string(&mut s)`. A small
on-disk `.docx`/`.pptx`/`.xlsx` whose one XML entry declares an extreme
uncompressed size (the standard "small compressed file, huge declared/
actual decompressed size" zip-bomb shape) could exhaust memory before the
orchestrator's `max_file_size_mb` gate is even relevant - that gate only
sees the small on-disk *compressed* file size, never the decompressed
content size.

Fixed by giving `read_zip_entry_to_string` the same two-layer defense the
nested-zip path already uses for its own entries: reject up front if the
entry's declared `size()` exceeds `ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES`
(reused, not a new constant), then bound the actual read itself with
`Read::take(ZIP_MAX_ENTRY_UNCOMPRESSED_BYTES)` as defense-in-depth against
a deliberately-wrong declared size, not just a trusted-header check. Also
upgraded the nested-zip path's own `entry.read_to_end` to the same
`.take()` bound - it already had the declared-size precheck, but not the
second layer.

## A second gap: PDF's FlateDecode had no inflated-size bound at all

`inflate_raw_deflate` (used for `/FlateDecode`d PDF content streams) had
no size bound whatsoever - unlike a zip entry, a PDF stream has no
"declared uncompressed size" header field to precheck against, and
`extract_pdf_lines`'s own `MAX_CONTENT_CHARS` truncation only trims the
buffer *after* `read_to_end` has already fully inflated it - too late to
prevent the allocation itself. Fixed with the same `Read::take` bound
(new `PDF_MAX_INFLATED_STREAM_BYTES` constant, 20MB, matching the zip
per-entry cap) directly on the `DeflateDecoder`'s read.

## Verification

`cargo test --workspace`: **174/174 passing** (app 8, native-search 42 +
13 ffi_smoke, search-cli 4, search-core 110 + 10 fixtures - all real
DOCX/PPTX/XLSX/PDF fixtures still extract correctly under the new bounds,
since real documents sit far below a 20MB single-entry/single-stream
cap). New test:
`extract_docx_lines_rejects_an_entry_declaring_more_than_the_size_cap` -
builds a *real* zip (via `zip::ZipWriter`, not fabricated bytes) whose
`word/document.xml` entry declares just over the cap using highly
compressible repeated bytes (tiny on-disk size, exactly the shape a real
zip bomb takes), and confirms `extract_docx_lines` rejects it via the
precheck rather than attempting to materialize the content - the test
itself runs in well under a second despite the 20MB+ logical payload,
which is the whole point.
