# toolboxes/material_lookup/ — Material Lookup toolbox

> TL;DR: Searchable, filterable, sortable browser over every built-in material (17 curated typicals + 1,375 MIL-HDBK-5J conditions in `mechanics-core`), with a property panel and a side-by-side comparison of up to four marked materials. Also the picker overlay used by Lug Analysis (`picking: true`).

## Purpose
Owns: the pure query/sort/compare/report layer (`model.rs`), state + keys (`mod.rs`), rendering (`view.rs`).
Does not own: material data or any property conversion (`mechanics-core::materials`/`handbook`, `docs/material-handbook.md`); the multi-word matcher and the property panel (`widgets/material_detail.rs`, shared with the Bushing/Pressure Vessel pickers).

## Code Map
| Looking for... | Go to |
|---|---|
| `catalog()`, `groups()`, `Query`, `filter_sort`, `compare_rows`, `report_text` | `model.rs` |
| `MaterialLookupState`, `handle_key`, mark/compare/export | `mod.rs` |
| List + property/comparison panels | `view.rs` |

## Contracts
- Every printable key types into the search; commands are Ctrl chords and function keys (Ctrl+B basis, Ctrl+S sort, Ctrl+R reverse, Ctrl+O/Enter mark, Ctrl+X clear, Ctrl+V/F2 compare, Ctrl+E/F3 export), letters in both cases. Tab is left to the shell.
- Sorting by a property puts missing values last. Strengths in the comparison use the lowest grain direction. A tie for best marks every tied column; identical values mark none.
- Export goes through `Effect::WriteTextFileAndOpen` (`reports/material-lookup.txt`).

## Pitfalls
- Comparison/readout lines are trimmed per line by the paragraph widget: start rows with text.
- Curated typicals carry no density, so the weight-specific rows are empty for them (by design, not a bug).

## Navigation
Parent: `app-tui/AGENTS.md`. Data: `docs/material-handbook.md`.
