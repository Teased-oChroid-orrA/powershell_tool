# toolboxes/eccentric_bushing/ - Eccentric Bushing toolbox

> TL;DR: Thin bridge over the `eccentric-bushing` solver. The terminal toolbox reads bore, ID, interference, friction, length, load and materials from a default `BushingModel` held by `AppState`. It adds offset, load angle, housing and pin inputs. Manual runs (`r` analyse, `m` max offset, `l` max load) use a worker (`Effect::RunEccentric` -> `AppEvent::EccentricFinished`). #242 tracks a case handoff after the terminal Bushing Workbench's retirement.

> The native window has a second front end over this toolbox: `gpu_viewer/editor/eccentric/` (node there). It reuses `EccentricState`, `model.rs` and `view::display_value` unchanged (`display_value` is `pub(crate)` for it). Keep this toolbox free of window types.

## Purpose
Owns: UI state, key routing, rendering, report text. Does not own: any mechanics (`eccentric-bushing/AGENTS.md`).

## Code Map
| Looking for... | Go to |
|---|---|
| `Task`/`Output`, `build_input` (Bushing model -> solver input), `EccentricUi`, rows, report text | `model.rs` |
| `EccentricState`, `start`/`finish`, `handle_key(state, &BushingModel, key)` | `mod.rs` |
| Fields + results rendering (`d` pressure profile) | `view.rs` |
| Tests (key routing in both cases, stale results, one real solver run on the default model) | `tests.rs` |

## Contracts
- `handle_key` takes the Bushing model by reference: `app.rs` passes its default model (also in mouse paths). A result is stored with the exact `Inputs` it ran for and marked stale when `build_input` differs; the export only describes fresh results.
- Every single-letter binding matches both cases (`'r' | 'R'`); tests cover the uppercase variants.
- Keys (both cases): `r` analyse, `m` max offset, `l` max load, `s` margin-versus-offset sweep (9 offsets in parallel, bar chart), `v` solve and write the FE fields as `.vtu` (`<app data>/reports/eccentric-bushing-fields.vtu`), `x` CSV of the pressure profile and the sweep (`eccentric-bushing.csv`), `e` text report, `d` profile, `c` cancel.
- Each run has a `Control` (`model::new_control`): a cancel flag, a 10 min deadline for searches and sweeps (`SEARCH_BUDGET`) and a `Progress` record the view reads live (stage, last step with the force the pin carries, solves done, the bracket a search has so far). A stopped search is a normal result with `OffsetLimit::halted`: the view and report show the verified bracket (`model::halted_note`). A cancelled analysis is the error "cancelled"; `c` also clears the queue.
- `EccentricState` keeps a bounded `FitCache` and attaches it to each job's `Control`. Analyse, Max load, Max offset, sweeps and field export reuse a solved interference fit when its fit inputs agree; changing geometry, fit materials or friction causes a new fit.
- During Max offset, `a` accepts the current lower bound only when it has been verified on the selected capacity basis. With pin-load credit, a fit-only bracket is shown as preliminary until a loaded search verifies a bound; it cannot be accepted. The result remains labelled as an early stop with its upper bound; `c` cancels and clears the queue. Bracket progress is in physical inches (or lbf for Max load), converted at the solver entry point.
- A run asked for while one is going is queued (same task once, at most 4) and starts when the job finishes (`finish` returns the effects). The last 6 analyses are kept (`history`, an identical rerun replaces the newest) and listed against the latest.
- `Analysis::loaded_failure` shows as a warning under the results.
- Runs are manual, never automatic (seconds to tens of seconds); a second start while a job runs is ignored; a result for an older job id is dropped.

## Rows
Offset, load angle; housing (`Boss OD / Bore`, round boss or edge-limited plate), axial condition (plane stress / strain); pin (clearance, friction, E, nu), capacity basis (with the pin load / fit alone), direct spin check (on: also simulate the spin, 15-30 s more); the run actions. Inputs from the default Bushing model: bore, ID, interference, friction, length, load, materials, edge distance, minimum wall.

## Pitfalls
- `model::field_rows(advanced)`: basic inputs and the three Run rows first; axial condition, housing, pin modulus/nu, capacity basis and direct spin check sit behind `FieldRow::AdvancedSection` (`EccentricUi::advanced_open`). The Results headline folds the thin-wall check into the spin verdict, so `HOLDS` never hides a failing wall.
- The Bushing Workbench's `output.delta_total` is the diametral interference (the solver takes diametral); its `bore_tol.nominal` is the bore diameter.
- Material elasticity is converted from `mechanics_core::materials::Material` (`e_ksi * 1000`), never re-derived.
