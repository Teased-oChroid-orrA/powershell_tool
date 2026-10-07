# toolboxes/eccentric_bushing/ - Eccentric Bushing toolbox

> TL;DR: Thin bridge over the `eccentric-bushing` solver. Reads bore, ID, interference, friction, length, load and materials from the live Bushing Workbench model (`state.bushing.model`; nothing is entered twice) and adds the offset, load angle, housing (round boss, or the Bushing Workbench's edge distance as a plate) and the pin (clearance, friction, modulus, Poisson's ratio). Manual runs (`r` analyse, `m` max offset, `l` max load) on a worker (`Effect::RunEccentric` -> `AppEvent::EccentricFinished`).

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
- `handle_key` takes the Bushing model by reference: `app.rs` passes `&state.bushing.model` (also the mouse paths). A result is stored with the exact `Inputs` it ran for and marked stale when `build_input` differs; the export only describes fresh results.
- Every single-letter binding matches both cases (`'r' | 'R'`); tests cover the uppercase variants.
- Runs are manual, never automatic (seconds to tens of seconds); a second start while a job runs is ignored; a result for an older job id is dropped.

## Rows
Offset, load angle; housing (`Boss OD / Bore`, round boss or edge-limited plate), axial condition (plane stress / strain); pin (clearance, friction, E, nu), capacity basis (with the pin load / fit alone), direct spin check (on: also simulate the spin, 15-30 s more); the run actions. Inputs from the Bushing Workbench: bore, ID, interference, friction, length, load, materials, edge distance, minimum wall.

## Pitfalls
- `model::field_rows(advanced)`: basic inputs and the three Run rows first; axial condition, housing, pin modulus/nu, capacity basis and direct spin check sit behind `FieldRow::AdvancedSection` (`EccentricUi::advanced_open`). The Results headline folds the thin-wall check into the spin verdict, so `HOLDS` never hides a failing wall.
- The Bushing Workbench's `output.delta_total` is the diametral interference (the solver takes diametral); its `bore_tol.nominal` is the bore diameter.
- Material elasticity is converted from `mechanics_core::materials::Material` (`e_ksi * 1000`), never re-derived.
