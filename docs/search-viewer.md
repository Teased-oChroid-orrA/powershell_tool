# Search "Open original" viewer tab

The Search workbench's Open button opens a viewer tab in the browser instead of handing the file to whatever program is
registered for its extension. The tab shows the file's text with the search's own highlighting, a match list, flags and a
whole-file ruler beside the scrollbar, a density histogram above the match list, filter chips, a matches-only view, wrap,
find, go to line and `#L12-L18` links; Office files also get a structured view. The look
follows the approved mockup (`.audit/open-original-viewer-mockup.html`, v7). The report's look, highlighting and filtering
are unchanged: the viewer calls the report's own `highlight_ranges` and `FilterColors`.

Code: `search-core/src/viewer/` (`mod.rs` page building, `reader.rs` large files, `structure.rs` Office structure, `assets/viewer.{html,css,js}`, `tests.rs`).
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
- `GET /r/<token>/v/<id>` is the page; `/r/<token>/f/<id>/{lines,hits,find,scan,native}` are its data: line ranges or a list
  of lines (`&spans=0` leaves the highlight ranges out, which is much cheaper), the hit line numbers, a time-sliced regex
  scan for the page's Find box and another that finds the search's own hits (`scan`), each 120 ms per request with the page
  looping, and "open the original" through the host's opener. The token is random per process; the `Host` header must be exactly `127.0.0.1:<port>`
  (anything else is 403, which blocks DNS rebinding); only GET is accepted (405).
- The page keeps a bounded LRU of 25-line blocks, shows skeleton rows while a block loads, and shows a "local reader
  stopped" overlay with Retry when the process is gone (checked by killing the server under a loaded page).
- The reader's hit list comes from the search's results. When the search listed none (or none for these filters) the page
  offers "Scan the file for these filters" (also in the `...` menu): `GET .../scan?from=N` runs the search's own matcher
  (`apply_line_matching`: one unit per filter group, any-line mode, the line excludes) over 4,096-line chunks for about
  120 ms per request and merges what it finds into the view's hit set, so chips, counts, the list, the ruler, the
  histogram and the text highlights fill in as it goes. It can be cancelled and resumed from where it stopped; the hits stay
  on the server for the next page load of the same file. Proximity and all-lines modes are not rescanned.

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

## Wrap, histogram, groups, structured views

- **Wrap** (`W`, the Wrap button) wraps long lines in text and code pages. Row heights are measured, kept in a prefix-sum
  array so the scrollbar, the ruler and the window marker stay exact, and toggling wrap, resizing, changing the font size or
  the rail width keeps the first visible line (the selected line if it is on screen) in place. It is for embedded pages
  only: a reader file keeps one line per row so its block windows stay uniform, and the button says so. The structured view
  wraps by itself.
- **Histogram**: above the match list when the file is large (a reader file, or 2,000 lines and more). It shows the hit
  density over the whole file with the window and the current hit marked; a click jumps there.
- **By message / By range** (above the list, next to All hits). By message groups the hit lines that differ only in digits
  and spacing (the first 400 characters of the line with digits removed is the key), biggest group first, top 300 shown,
  at most the first 100,000 hits grouped; a click filters the list to that message (a pill above the list clears it). The
  group label is the first hit's text with the match in bold. By range counts hits per block of 100, 1,000, ... lines (the
  smallest block size that gives 60 blocks or fewer); a click goes to the block's first hit. The reader sends the hit texts
  400 at a time without highlight ranges (`spans=0`) and then one request with ranges for the 300 label lines: 20,031 hits
  grouped in 12 s with ranges for every hit (release build) and in 0.57 s this way.
- **Structured view** (`structure.rs`, `cfg.structure`, the Text / Structured switch, on by default when available):
  - `.docx`: headings, title, list bullets, paragraphs and tables (the first row is the header) in pages of about 40 lines.
    The descriptors come from the extractor's own pipeline run with marker characters, so they align line for line with the
    text the search matched.
  - `.pptx`: one card per slide with the title, bullets, notes and SmartArt text; the slide's pictures sit under the title.
  - `.xlsx`: one grid per sheet in chunks of 200 rows, column headers 1, 2, 3, per-cell highlights.
  - Pictures in docx and pptx (png, jpg, gif, bmp, webp; at most 3 MB each, 12 MB and 60 pictures per file; emf, svg and
    linked pictures are skipped) are embedded as data URIs.
  - Every line keeps its number, hit and selection colours, `#L` links and click to select. Go to line, find, the filter chips,
    the match list, the ruler and the font size all work on it, and switching to Text keeps the position.
  - Skipped for pages above 60,000 lines (`STRUCTURE_MAX_LINES`); those show the text view only.

## Verification (2026-11-20)

Real pages in the built-in browser pane, no console errors:

- Embedded 1,460-hit log: wrap on and off with a long-line fixture (the first visible line stays within about two lines),
  wrap with matches-only and with a font-size change (not re-checked after a window resize); the histogram draws, clicking jumps and the window and
  cursor markers follow; By range (buckets, click goes to the first hit); By message (groups, click filters the list, pill
  and "713 of 1,460").
- Reader, 3.7 MB, 40,001 lines, no hits listed: the empty state with the scan button, scan, cancel and resume from the `...`
  menu, chips and counts filling, highlights appearing in the text, the scan menu item disabling when done. A file identical
  to a searched one scans to the same 20,031 matching lines the search found. By message on the 20,031 hits takes 0.57 s;
  the histogram and By range work on a reader file; Wrap is disabled with an explanation.
- docx: headings, list, table with a header row, page cards, highlight and current-line outlines, go to line, `#L12`,
  find (`Pressure` found in the title), filters off and on, A+ / A-, light theme, a 420 px wide window.
- pptx: four slide cards with their pictures under the titles, bullet hits and current line, switching Text and Structured
  keeps the line. xlsx: two sheets of 449 rows in 200-row chunks, per-cell highlights, go to line 700 in the second sheet,
  filter chips on and off (565 hit lines, 283 with one filter off), clicking a line sets `#L5`.
- Not verified: the structured view on very large real Office files (the cap is by line count, not measured on one).

## Decided, not built: the report's own "Open original" links

The report's `Open original` link still opens the file with the default program. The report is one self-contained file by
design (the banner is embedded for that reason and the footer says so). Pointing the link at viewer pages would mean writing
those pages next to the report, in a sibling folder that has to travel with it, or growing the report by every hit file's
text. Both change what the report is, and an opened report that moves or is mailed would lose its links. That is a product
decision, so it is not built. The Search window's Open button is the viewer entry point. If it is wanted later, the cheap route is a bounded sibling folder `<report>_files/` of viewer pages built from
`FileSearchResult::lines_cache` (embedded plan only, a cap on files and bytes), with the file link kept for everything else.

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
