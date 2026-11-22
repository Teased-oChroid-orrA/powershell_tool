# GS Engineering Toolbench

**One offline desktop app for aircraft-structures engineering work: a finite-element modeller, joint and fastener calculators, a material library and a document search engine that reads the files engineers actually keep.**

Written in Rust. Runs on Windows, macOS and Linux. Needs no internet connection, no administrator rights and no runtime installed on the machine that runs it.

[Quick start](#quick-start) · [Workbenches](#workbenches) · [Search](#search-engine) · [FEA stack](#the-fea-stack) · [Architecture](#architecture) · [Build and test](#build-and-test) · [Documentation](#documentation-map)

---

## Contents

- [At a glance](#at-a-glance)
- [Quick start](#quick-start)
- [Workbenches](#workbenches)
- [The dashboard window](#the-dashboard-window)
- [Search engine](#search-engine)
- [The FEA stack](#the-fea-stack)
- [Solid modelling and meshing](#solid-modelling-and-meshing)
- [Engineering solvers](#engineering-solvers)
- [Architecture](#architecture)
- [Build and test](#build-and-test)
- [Continuous integration](#continuous-integration)
- [Target environment and packaging](#target-environment-and-packaging)
- [Working in this repository](#working-in-this-repository)
- [Documentation map](#documentation-map)

---

## At a glance

| | |
|---|---|
| **Twelve workbenches** | FEA, frame and truss, assembly joints, bushing, bushing stackup, eccentric bushing, fastener hole, lug, preload, pressure vessel, material lookup, search |
| **Native 3D front end** | Direct `wgpu` rendering with `egui` controls; one dashboard window hosts every workbench as a tab |
| **Terminal front end** | A `ratatui` interface with four terminal toolboxes, for machines with no graphics adapter |
| **General FE kernel** | 2D and 3D elements, sparse solvers, contact with friction, plasticity, modal, buckling, transient, heat conduction, sensitivities, verified adaptive solves |
| **Native solid kernel** | STEP import and editing, tetrahedral meshing and assembly feature recognition in Rust; no CAD library is linked |
| **Search engine** | Text, Office, zip, rtf and pdf files, optional OCR, a persistent Tantivy index, local semantic retrieval, streamed HTML / CSV / JSON reports |
| **Headless CLI** | `search-cli`, the same search engine without a window |
| **Offline by design** | Models, tokenizers and data tables are bundled; nothing is fetched at run time |

> [!NOTE]
> Rust is the only implementation in the tree. Every crate builds and its tests run on any platform; only the native window needs a graphics driver.

---

## Quick start

You need the Rust toolchain from [rustup](https://rustup.rs). The native window needs a working graphics driver (Metal, Vulkan or Direct3D 12).

```sh
# Open the native dashboard (every workbench, in one window)
cargo run --release -p app-tui -- --gui

# Terminal interface, for machines with no graphics adapter
cargo run -p app-tui

# Headless search from the command line
cargo run --release -p search-cli -- /path/to/folder --filter foo --filter bar
```

A terminal-only build leaves out `wgpu`, `winit` and `egui`:

```sh
cargo build --release -p app-tui --no-default-features
```

On Windows the application starts in the dashboard when launched with no arguments.

---

## Workbenches

The dashboard groups the twelve workbenches into three sections. Each one opens inside the dashboard window; Shift-click a tile (or use **New window**) to give a workbench its own process.

### Joints and fasteners

| Workbench | What it does |
|---|---|
| **Fastener Hole** | Hole, fastener and edge-distance geometry for a fastened hole. |
| **Assembly Joints** | Reads a STEP assembly and shows its holes, fastener sites (including bolts that bridge a gap between bodies) and contact faces on the model, with advice and the reason for each item. It decides nothing; the user chooses what to apply. |
| **Bushing** | Interference-fit bushing: straight, flanged and countersunk. Lamé contact pressure and hoop stress, tolerance stack, install force, margins of safety, an aircraft reamer catalog, edge-distance cross-checks and verified fixes. A finite-element view of the housing and bushing is available from the window. |
| **Bushing Stackup** | Line-reamed through holes and per-member bushing bore preparation across a stack of members. |
| **Eccentric Bushing** | Spin capacity of a bushing whose bore is offset from its outer diameter, in a round boss or an edge-limited plate. Friction torque of the fit against the applied moment, the largest offset or load the fit still holds, a meshed elastic pin and a direct spin simulation. |
| **Preload Analysis** | Threaded-fastener preload: V-thread torque, bearing friction, torque-preload equilibrium, fastener and member compliance, nut rotation, stress state, service-load, separation and slip checks, and uncertainty. Member stiffness can come from VDI 2230 segments, a pressure-cone integral or a finite-element model. |

### Structures and simulation

| Workbench | What it does |
|---|---|
| **FEA Workbench** | Define a 2D sketch, an extruded solid, a STEP solid or an imported mesh; apply supports and loads; solve; contour the result. Includes interference-fit bushings with contact, thermal studies, adaptive meshing and failure guidance. |
| **Frame and Truss** | Nodes, truss and frame members, supports and loads. |
| **Lug Analysis** | Pin-loaded lug on the general FE kernel: contact with Coulomb friction, optional bushing with a solved interference fit, and plasticity through to collapse. |
| **Pressure Vessel** | Exact Lamé stress, von Mises / Tresca / maximum-principal failure margins, minimum wall per criterion, external-pressure and span buckling, UG-27 heads, a shell-to-head junction FE, and a supported-tube line run. |

### Documents and data

| Workbench | What it does |
|---|---|
| **Search** | The [search engine](#search-engine) with a results table, live progress, an in-window preview and an original-file viewer. |
| **Material Lookup** | About 1,400 materials. Type to search, refine by basis, form, property limits and group, mark materials into a compare basket. |

> [!TIP]
> Choose the model's unit system in the **Units** dropdown. Conversion preserves the physical model, and STEP coordinates are normalised to millimetres. An exaggerated deformation plot is always labelled separately from true scale, and every result carries its convergence, equilibrium and formulation checks.

---

## The dashboard window

The dashboard is a shell that hosts workbenches as tabs ([ADR-024](docs/adr/ADR-024-in-window-workbenches.md)).

- **Nav strip.** One tab per open workbench, with a dot for unsaved edits and a spinner while a solve runs. `Ctrl/Cmd+J` opens the switcher; `Ctrl+K` is each workbench's own command palette.
- **Parked workbenches keep working.** A solve finishes in the background while another tab is in front, and each page keeps its scroll position, folded cards and selection.
- **Hand-offs stay in the window.** Bushing and Eccentric Bushing send a joint to the FEA Workbench and back; the stackup and fastener transfers work the same way.
- **Closing asks first** when a tab has unsaved edits or a running solve.
- **Left rail** with section filters, recent files and the theme switch (the theme follows the time of day).

---

## Search engine

Search finds keyword filters across a folder tree and produces a report. Its first design rule is that it never goes silent: progress is reported per file and by a background ticker, so a slow PDF never looks like a hang.

### What it reads

| Format | Notes |
|---|---|
| Plain text, logs, code, config, data | Encoding detection: BOM, then strict UTF-8, then Windows-1252. Dozens of extensions, with a type-to-filter picker and custom extensions. |
| `.docx`, `.pptx`, `.xlsx` | Slides include speaker notes and SmartArt text. |
| `.zip` | Including nested archives and Office documents inside them, with zip-bomb and deflate-bomb guards. |
| `.rtf` | |
| `.pdf` | Including CID-keyed / Type0 fonts through `/ToUnicode` maps. Optional OCR for scanned pages, in pure Rust and fully offline. Page citations come from the extracted page map. |

### How it matches

- **Match modes:** any line; all filters present somewhere in the file; proximity (all filters within N lines).
- **Filter semantics:** case-insensitive literal substring (default), whole-word matching (so a filter such as `C#` works standing alone), or full regular expressions. An invalid pattern produces an error that names the exact filter.
- **Exclude filters** with line or whole-file scope.
- **Highlighting** in the report, the preview and the viewer comes from one shared routine.

### Fast re-search and semantic retrieval

- **Persistent index.** An optional Tantivy index per searched folder narrows full scans through a trigram candidate filter. It is always a safe superset pre-filter: the exact line scanner stays the sole authority on what is a match. Files are skipped when their modified time and size are unchanged, and an optional folder watcher updates only the paths that changed.
- **Local semantic search.** A bundled `all-MiniLM-L6-v2` encoder (384 dimensions, CPU-only, Apache-2.0) feeds a hybrid retriever that fuses vector rank with BM25, so part numbers and spec codes such as `M8` still find `M8x1.25`.
- **Natural-language Find.** Type a sentence; rules split it into conditions you can edit, each condition is retrieved on its own, and a file is labelled by its weakest condition. Nothing is generated: every result shows the unchanged cited passage.
- **Related documents.** Result rows carry a cue for identical copies and other versions of a file, and the detail pane lists related files with the reason for each.
- **Entities and catalog.** Part numbers, materials, dimensions, units, fastener callouts and standards are extracted into a local catalog for linking documents.
- **Saved-search alerts** compare a stored baseline with a fresh run and report what changed.

### Robustness and export

- Retry with backoff and a per-file timeout for locked or slow files, symlink-safe and cancellable directory walking, dry-run mode, an incremental result cache keyed by the settings that affect matching, and a persistent extraction-failure log.
- Streaming **HTML** reports (dark mode, table of contents, per-filter chart, match highlighting), plus **CSV** (with formula-injection neutralisation), **JSON** and **JSONL**, all streamed rather than built in memory.
- An **Open original** viewer shows a file with the search's own highlighting, a match list and ruler, and handles very large text files by byte offset without loading them whole.

### Command line

`search-cli` exposes the same engine:

```sh
search-cli /path/to/folder --filter torque --filter preload --mode proximity
search-cli /path/to/folder --filter "M8x\d" --regex --output-folder ./reports
search-cli --interactive            # guided menu
search-cli --help                   # every option
```

Maintenance flags cover index verification, orphan removal, clearing the result cache and listing recorded extraction failures. Run `search-cli --help` for the exact set.

### Evidence, not claims

Any change to retrieval or ranking is measured with `search-core/examples/retrieval_eval.rs` on BEIR, MS MARCO, Cranfield and TREC data before it is adopted. Methods and results are in [`docs/search-benchmarks.md`](docs/search-benchmarks.md); the formal matching contract is in [`docs/search-semantics.md`](docs/search-semantics.md).

---

## The FEA stack

Three crates form the simulation core, and every structural workbench sits on them.

```text
solid-kernel   STEP / B-rep / surface mesh        (what the part looks like)
      │
fea-problem    Problem schema, mesh, solve, results   (what you are asking)
      │
fea-core       Elements, assembly, solvers, contact   (how it is solved)
```

### `fea-core`: the kernel

- **Elements:** 2D plane stress, plane strain and axisymmetric (Tri3/6, Quad4/8/9); 3D (Tet4/10, Hex8/20/27).
- **Solvers:** colour-parallel symmetric assembly, sparse Cholesky (AMD or nested dissection) through `faer`, and PCG with a smoothed-aggregation multigrid preconditioner for large 3D models.
- **Constraints and materials:** multi-point constraints, reference-point rigid coupling, isotropic and anisotropic elasticity, plasticity.
- **Loads:** point, surface (constant or field), body and thermal.
- **Contact:** augmented-Lagrangian contact with Coulomb friction and interference fits.
- **Analyses:** modal, buckling, transient, heat conduction, sensitivities, and verified adaptive solves driven by stress-error estimates.
- **Output:** nodal stress (averaged or SPR), `.vtu` export, structured mesh generators.

### `fea-problem`: the problem layer

A serialisable `Problem`: a 2D sketch with holes, an extrusion, a STEP solid or an imported Gmsh / Abaqus mesh, plus materials, supports, loads and mesh settings. It owns edge naming, mesh generation, the solve driver and its checks, result fields, a raster of any field for terminal display, text reports, and `joint::member_compliance` for bolt-joint stacks. Assembly advice and multi-body meshing live here as well.

### `lug-solver`, `eccentric-bushing` and `edge-check`

These are domain models built on the kernel: a pin-loaded lug (the legacy condensed solver is kept for comparison), the offset-bore spin model, and independent cross-checks of the bushing edge-distance requirement (stress superposition, tabulated allowables, and a contact FE).

Design notes are in [`docs/fea-core.md`](docs/fea-core.md), [`docs/adr/ADR-012`](docs/adr/ADR-012-general-fea-kernel.md) and [`docs/adr/ADR-013`](docs/adr/ADR-013-eigen-and-transient-methods.md).

---

## Solid modelling and meshing

`solid-kernel` is a native Rust B-rep kernel. **No CAD library is linked**, so no CAD-library licence notice is needed and the build has no native CAD dependency ([ADR-023](docs/adr/ADR-023-retire-opencascade.md)).

- **Surface solids:** closed, outward-oriented, face-tagged surface meshes with a BVH-accelerated signed-distance query.
- **STEP:** native import of parts and assemblies with rigid placements, and STEP export.
- **Modelling:** holes, bosses and fillets on supported geometry. A shape the modeller cannot edit is refused with a stated reason; the capability matrix is in [`docs/issue-46-phase-166.md`](docs/issue-46-phase-166.md).
- **Volume meshing:** tetrahedra through `fea-core`, with feature-size control and an element cap.
- **Assembly feature recognition:** holes, coaxial bores across bodies, fastener sites (including sites bridged by a third body) and overlapping planar interfaces, each reported with an explained recommendation. The plan and evidence are in [`docs/epic-assembly-feature-advice.md`](docs/epic-assembly-feature-advice.md).
- **Failure guidance:** when a model will not mesh or solve, the FEA Workbench diagnoses the cause and offers fixes that preview themselves on the model ([ADR-020](docs/adr/ADR-020-failure-guidance-diagnosis-remedies-questions.md)).

> [!IMPORTANT]
> Native import reads 32 of the 33 NIST PMI test files, and some spline-based parts are still unreadable. The limits are recorded in [`docs/issue-46-phase-182.md`](docs/issue-46-phase-182.md) and ADR-023.

---

## Engineering solvers

Pure calculation crates with no UI and no file access. Each has a fixed entry point and its own test suite, and each workbench only bridges one into window state.

| Crate | Purpose |
|---|---|
| `bushing-solver` | Press-fit bushing calculation, tolerance stacks, margins, reamer catalog, stackup arithmetic |
| `fastened-joint-solver` | Threaded-fastener preload, compliance, service-load and slip checks, uncertainty |
| `pressure-vessel-solver` | Lamé stresses, failure margins, minimum thickness, buckling, UG-27 heads, shell-to-head junction, line run |
| `edge-check` | Edge-distance cross-checks behind one `EdgeModel` trait |
| `lug-solver` | Pin-loaded lug finite-element models |
| `eccentric-bushing` | Offset-bore spin capacity and limits |
| `mechanics-core` | Material data and shared mechanics (Lamé relations, tables) |
| `engineering-math` | Shared numerical helpers |

**Numerical discipline.** A solver change is accepted on numerical proof: analytical or limiting cases, dimensional checks and differential tests, plus a measurement when the goal is speed or convergence. Full precision is kept inside calculations and rounded only at display. Tolerances are never loosened to hide a discrepancy.

---

## Architecture

```text
 ┌──────────────────────────── app-tui ────────────────────────────┐
 │  native dashboard (wgpu + egui)      terminal toolboxes (ratatui)│
 └───────┬───────────────┬──────────────┬──────────────┬───────────┘
         │               │              │              │
   search-core     fea-problem    domain solvers   solid-kernel
   (+ native-search,   │          (bushing, lug,       │
    search-ml)      fea-core       preload, vessel,    │
         │               └─────── eccentric, edge) ────┘
        cli                          │
   (search-cli)              mechanics-core, engineering-math
```

| Crate | Role |
|---|---|
| `app-tui` | The only front end. A native `wgpu` / `egui` dashboard window and a `ratatui` terminal interface over the same state. |
| `search-core` | All search logic: settings, matching, extraction, OCR, caching, reports, orchestration, semantic retrieval. **Zero GUI dependency.** |
| `native-search` | The Tantivy-backed index engine. It never reads files itself. |
| `search-ml` | Bundled offline embedding model for semantic search. |
| `cli` | `search-cli`, a second consumer of `search-core`. |
| `fea-core`, `fea-problem`, `solid-kernel` | The [FEA stack](#the-fea-stack). |
| solver crates | See [engineering solvers](#engineering-solvers). |
| `third_party/ocrs` | The OCR engine, vendored with a one-field patch for per-character confidence; excluded from the workspace. |

Design rules that hold across the project:

- **Logic lives in libraries, not in heads.** The reducer in `app-tui` returns effects as data and never touches the terminal, the filesystem or the OS, so the whole decision layer is testable without a terminal.
- **Reuse before writing.** Materials, Lamé and press-fit mathematics, unit conversion and search extraction each have one authoritative implementation.
- **New workbenches open in native windows** from the dashboard.
- **Live progress is a hard requirement** for anything that can run long.

Each crate has an `AGENTS.md` with its contracts, code map and pitfalls, and [`CLAUDE.md`](CLAUDE.md) holds the project map. Decisions are recorded as ADRs in [`docs/adr/`](docs/adr/README.md).

---

## Build and test

```sh
# Release builds
cargo build --release -p app-tui          # target/release/app-tui(.exe)
cargo build --release -p search-cli       # target/release/search-cli(.exe)

# Tests
cargo test --workspace                    # everything
cargo test -p search-core                 # no GUI toolchain needed
cargo test -p fea-core                    # the FE kernel
cargo test -p app-tui --lib               # the decision layer and window models

# The complete local qualification (same checks as CI)
python scripts/qualify.py
python scripts/qualify.py --gpu           # adds GPU rendering tests; needs a graphics adapter
```

`scripts/qualify.py` writes a JSON report with the tested commit, the commands and their exit codes.

Cargo features of `app-tui`:

| Feature | Default | Effect |
|---|---|---|
| `gpu-viewer` | yes | Native `wgpu` / `egui` window |
| `neural-search` | yes | Bundled embedding model |
| `semantic-search` | yes | Hybrid semantic retrieval in Search |
| `nl-find` | yes | Natural-language Find (rule-based, no model) |

> [!NOTE]
> A successful compile does not prove a rendered window works. Native interaction and clean-machine startup need a platform smoke check, and layout changes must be run, not only built.

### Disk use

Each worktree carries its own multi-gigabyte `target/` directory. Delete it when a branch is merged (`git worktree remove --force <path>`), and watch free space before long builds.

---

## Continuous integration

| Workflow | When | What it runs |
|---|---|---|
| `qualification.yml` | Pull requests touching code, or by hand | `scripts/qualify.py` on Linux, macOS and Windows |
| `windows-step-runtime.yml` | Pull requests touching the STEP crates | Native STEP import, meshing and solve tests, and benchmark rows, on Windows |
| `app-tui-build.yml` | By hand or on a tag | Builds the Windows `app-tui` executable |
| `search-bench-windows.yml` | By hand | Search benchmarks on Windows |

Hosted Windows runners are the only Windows evidence for a change. A numerical failure that appears only on some runs is first a hardware question, since runners are not one machine.

---

## Target environment and packaging

- **Platforms:** Windows 10 1809+ and Windows 11 (`win-x64`) are the target; macOS and Linux (X11) are supported for development and use.
- **Self-contained:** the built application needs no installed runtime, no internet access and no administrator rights. Build time uses crates.io; the running application never does.
- **Release folder:** ship `THIRD-PARTY-NOTICES.md` and the `licenses/` directory with the executable.
- **Diagnostics:** the app writes `toolbench-debug.log` into the folder it was launched from. Attach it to any report that does not reproduce elsewhere.

Build, publish and clean-machine verification steps are in [`docs/deployment-rust.md`](docs/deployment-rust.md).

---

## Working in this repository

- Work happens on `main`; short-lived branches are used for risky or long-running changes and are deleted once merged.
- History is linear and forward-only: no rewrites, no force pushes.
- Search retrieval and ranking changes are measured before adoption (see [Evidence, not claims](#evidence-not-claims)).
- Architecture decisions go in `docs/adr/`; per-crate facts go in that crate's `AGENTS.md`; one-off epic narratives go in `docs/issue-N-*.md`.

---

## Documentation map

| Topic | Where |
|---|---|
| Project map and rules | [`CLAUDE.md`](CLAUDE.md), [`AGENTS.md`](AGENTS.md) |
| Docs index and ADRs | [`docs/AGENTS.md`](docs/AGENTS.md), [`docs/adr/`](docs/adr/README.md) |
| Search matching contract | [`docs/search-semantics.md`](docs/search-semantics.md) |
| Search benchmarks | [`docs/search-benchmarks.md`](docs/search-benchmarks.md), [`docs/benchmarking.md`](docs/benchmarking.md) |
| Search viewer | [`docs/search-viewer.md`](docs/search-viewer.md) |
| FE kernel | [`docs/fea-core.md`](docs/fea-core.md), [`docs/derivative-engine.md`](docs/derivative-engine.md) |
| Lug analysis | [`docs/lug-analysis.md`](docs/lug-analysis.md) |
| Eccentric bushing | [`docs/eccentric-bushing.md`](docs/eccentric-bushing.md) |
| Bushing stackup | [`docs/bushing-stackup.md`](docs/bushing-stackup.md) |
| Edge-distance cross-checks | [`docs/edge-distance-crosscheck.md`](docs/edge-distance-crosscheck.md) |
| Pressure vessel | [`docs/pressure-vessel-heads.md`](docs/pressure-vessel-heads.md), [`docs/pressure-vessel-line-run.md`](docs/pressure-vessel-line-run.md), [`docs/pressure-vessel-thickness.md`](docs/pressure-vessel-thickness.md) |
| Assembly feature advice | [`docs/epic-assembly-feature-advice.md`](docs/epic-assembly-feature-advice.md) |
| Native CAD and meshing | [`docs/issue-46-resume.md`](docs/issue-46-resume.md), [`docs/issue-46-completion-checklist.md`](docs/issue-46-completion-checklist.md) |
| Deployment | [`docs/deployment-rust.md`](docs/deployment-rust.md) |
