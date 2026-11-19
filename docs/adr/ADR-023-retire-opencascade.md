# ADR-023: Retire OpenCascade; the native kernel is the only CAD backend

Status: Accepted (2026-11-19), owner direction in chat ("keep working until occt is retired ... make the decisions that have to be made, you have my full approval"). Supersedes the OpenCascade parts of ADR-019 and its static-CRT addendum. Evidence: `docs/issue-46-phase-182.md`, `docs/issue-46-phase-183.md`, the comparison tables of Phases 165, 166 and 170.

## Context

ADR-019 put OpenCascade (OCCT 8.0.1 through `cadrum`) behind the off-by-default `occt` / `occt-source` features for STEP, booleans, fillets and tessellation, and set a retirement gate: the native path must import and mesh every corpus file OCCT reads; retirement means OCCT removed from the build. OCCT is LGPL 2.1 with exception, needs a 30 MB prebuilt or a 30-minute CMake build, and a static-CRT Windows build needs `occt-source`.

## Evidence at decision time

- Native STEP import reads 32 of 33 NIST PMI files (Phase 182); the 35-file corpus comparison of Phase 165 showed OCCT meshes only 3 files at its own default size and refuses the rest, so no file OCCT meshes is refused natively (Phase 170).
- Same-size meshes of the compared files match (Phase 182 table); every refusal observed is shared by both paths.
- Native hole, boss and fillet exist with narrower coverage (Phase 166).

## Decision

Remove OCCT from the build: the `occt` and `occt-source` features of `solid-kernel`, `fea-problem` and `app-tui`, the `cadrum` dependency, `solid-kernel/src/occt.rs`, the OCCT adapter in `solid-kernel/src/cad.rs` (the sketch types `Loop` and `Profile` stay), every `cfg(feature = "occt")` site, the OCCT-only tests and examples, the `--cad` qualification flag, the OCCT build option and LGPL checks of the Windows release workflow, and the OCCT notice and license texts. `production_backend` becomes `native` and `occt_retirement_approved` true in `docs/issue-46-current.json`. Evidence and history stay in `docs/` (append-only).

## Accepted capability losses

- ctc_02 ap203 (one NIST file, a spline band closed in v whose boundary winds a full period) is refused by the native import; OCCT read it. A seam-cut band chart for closed spline surfaces would admit it.
- Native holes and bosses act on flat faces only and fillets on convex polyhedra only; solids with spline or ellipse edge curves cannot be edited (Phase 166). STEP written by the editor is native `NativeBrep` text.
- No in-repo oracle remains for a new parity comparison with OCCT; the recorded Phase 124-182 tables and the committed OCCT-written fixtures are the reference.
- A packaged offline clean-machine Windows `app-tui.exe` acceptance run was not performed (it cannot run on the development Mac); hosted Windows CI (`qualification.yml`, `windows-step-runtime.yml`) is the Windows evidence, and `app-tui-build.yml` builds the executable by hand.

## Consequences

Builds need no CMake, prebuilt download or `OCCT_ROOT`; the release carries no LGPL obligations; one code path is tested everywhere. Reintroducing OCCT is possible from git history.
