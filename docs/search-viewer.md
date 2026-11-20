# Search "Open original" viewer tab

The Search workbench's Open button opens a viewer tab in the browser instead of handing the file to whatever program is
registered for its extension. The tab shows the file's text with the search's own highlighting, a match list, flags and a
whole-file ruler beside the scrollbar, filter chips, a matches-only view, find, go to line and `#L12-L18` links. The look
follows the approved mockup (`.audit/open-original-viewer-mockup.html`, v7). The report's look, highlighting and filtering
are unchanged: the viewer calls the report's own `highlight_ranges` and `FilterColors`.

Code: `search-core/src/viewer/` (`mod.rs` page building, `reader.rs` large files, `assets/viewer.{html,css,js}`, `tests.rs`).
App side: `app-tui/src/toolboxes/search/viewer.rs` (the effect), `Effect::OpenViewer`, `SearchAct::OpenSelected` and
`SearchAct::OpenInDefaultApp` in the Search window.

## How a file is shown (`viewer::plan_for`)

| File | Plan | What happens |
|---|---|---|
| `.docx` `.pptx` `.xlsx` `.zip` `.rtf`, plain text up to 1 MiB, UTF-16 text | Embedded | Extracted lines and hits are written into one self-contained page (works offline, survives moving). At most 200,000 lines; the page says when it cut. |
| Plain text over 1 MiB | Reader | `ReaderServer` serves blocks by byte offset on `127.0.0.1`; the file is never loaded whole. |
| `.pdf` | Pdf | The browser's own PDF viewer beside the match list (`#search=<word>`). The list comes from the extracted text. |
| Binary, folders, missing files | NoViewer | The caller opens the default app, as before. |

Any failure (page not writable, reader not started, extraction error) also falls back to the default app, so Open never
does less than before. The window keeps both: `Open` (Enter, double-click) is the viewer, `Open in default app` is the old
behaviour. The terminal toolbox's Open (`OpenPath`) was left as it was.

## The reader

- The index (`LineIndex`) stores the byte offset of every 256th line plus the line count. Measured 0.9 to 1.0 GB/s from the
  file cache (305 MB, 4 million lines: 0.30 to 0.34 s). Lines are decoded one at a time with `decode_text`.
- `GET /r/<token>/v/<id>` is the page; `/r/<token>/f/<id>/{lines,hits,find,native}` are its data: line ranges or a list of
  lines, the hit line numbers, a time-sliced regex scan (120 ms per request, the page loops), and "open the original"
  through the host's opener. The token is random per process; the `Host` header must be exactly `127.0.0.1:<port>`
  (anything else is 403, which blocks DNS rebinding); only GET is accepted (405).
- The page keeps a bounded LRU of 25-line blocks, shows skeleton rows while a block loads, and shows a "local reader
  stopped" overlay with Retry when the process is gone (checked by killing the server under a loaded page).
- The reader's hit list always comes from the search's results; it never rescans the file for hits.

## Verification (2026-11-19)

Real pages loaded in the built-in browser pane, no console errors:

- 800-line text file with three filters: rendering, slot colours, filter chips on and off, matches-only folds,
  match list, flags and ruler, Ctrl+G, Ctrl+F (literal and regex), `#L` links, a 12,000-character line (chip), non-ASCII
  text and a tab.
- 30 MB, 400,000-line log through the reader: scrolling to 25, 50, 99 and 100 percent, go to line, find across the whole file,
  307,537 matches in the list with two filters off, matches-only, and the reader-stopped overlay.
- 305 MB, 4,000,001-line log through the reader: the page scrolls in scaled mode and every position, including the last
  line and go to line 2,500,000, resolves. A first version capped the scroll height at 30 million pixels; this pane's
  browser tops out at 16,777,214 px, so the end of the file was unreachable. The cap is now 8 million pixels.
- A docx page (extracted text). The right-rail window marker was drawn one line low (the marker used the 1-based line as a
  0-based offset); fixed.
- Not verified in a browser: the PDF page. The pane serves pages over http and Chrome blocks an http page from loading a
  `file:` PDF; the real use opens the page from `file:`. The page's markup and the plan are covered by tests only.

## Deferred

- Wrap (the button is present and disabled).
- The left histogram.
- "By message" and "By range" groups.
- Structured office renderings (the docx/pptx/xlsx pages show the extracted text lines).
- Images.
- Scanning a large file for hits when the search had none (reader hits come from the results only).
- Retargeting the report's own "open original" links to the viewer.

## Things to remember

- Highlight spans are computed in Rust and sent as UTF-16 offsets, because JavaScript indexes strings by UTF-16 units.
  Do not recompute matches in the page.
- `plan_for` must check the UTF-16 byte order mark before `looks_binary`: UTF-16 text is full of NUL bytes and was classed
  as binary.
- The reader decodes each line separately; extraction decodes a whole file, so a file that mixes UTF-8 and Windows-1252
  lines can show different text for a line. Line numbering matches: a test compares the reader's lines with
  `split_lines(decode_text(..))` over a BOM, CR, CRLF and a CRLF straddling the read buffer.
- Find in the page uses the JavaScript regex engine, which differs from the Rust one in a few constructs.
- The window builds the viewer's settings from the current Find text (`build_settings`). Editing the filters after a run and
  then pressing Open changes which filters get chips and colours; the hit lines still come from the run.
- Embedded pages are written to `<temp>/toolbench-viewer/<stem>-<8 hex>.html`, one stable name per file, and are
  overwritten on the next Open; nothing deletes them.
