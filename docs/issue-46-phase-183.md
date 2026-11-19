# Phase 183: OpenCascade retired

Date 2026-11-19. Decision: `docs/adr/ADR-023-retire-opencascade.md` (owner approval given in chat). Evidence for the gate: Phase 182 (native import 32 of 33 NIST files), Phases 165, 166, 170.

## Removed

- Features `occt` and `occt-source` of `solid-kernel`, `fea-problem` and `app-tui`; the `cadrum` dependency (263 lines of `Cargo.lock`).
- `solid-kernel/src/occt.rs`; the OCCT adapter (`Brep`, `Capabilities`, `capabilities()`) in `solid-kernel/src/cad.rs`, which keeps the sketch types `Loop` and `Profile`.
- Every `cfg(feature = "occt")` item and test (28 sites plus the `cfg(not(...))` forms, now unconditional): the native paths in `fea-problem/src/solid.rs`, the `app-tui` solid editor tests and `window.rs` test.
- Tests and examples that only compared against OCCT: `solid-kernel/tests/{occt,parity_occt,grade_occt,native_brep_occt}.rs`, `fea-problem/tests/occt_vs_native.rs`, `solid-kernel/examples/{assembly-probe,shell-angle-probe,write-shapes}.rs` (the fixtures `write-shapes` produced stay committed).
- CI: the `--cad` flag of `scripts/qualify.py` and the OCCT matrix flags of `qualification.yml`; the `occt` input and LGPL file checks of `app-tui-build.yml`; the "OCCT is not in the feature set" step of `windows-step-runtime.yml` (it now records the feature list).
- `licenses/OpenCASCADE-*.txt` and the OCCT section of `THIRD-PARTY-NOTICES.md`; README and AGENTS text.

## Also fixed

`set_card_open` (test-only helper) is `cfg(test)`; a never-read assignment in the new tessellated-solid test.

## Limits (stated, not hidden)

- ctc_02 ap203 still refuses natively (Phase 182).
- Packaged offline clean-machine Windows `app-tui.exe` acceptance was not run (no Windows hardware here). Hosted Windows CI is the only Windows evidence; run `app-tui-build.yml` by hand for the executable.
- User-visible native refusals still begin "without OpenCASCADE" (asserted by tests); the wording is stale but accurate in meaning.
- No OCCT oracle remains for new parity work; recorded tables and committed OCCT-written fixtures are the reference.
- Volume meshing stays slow on real CAD (10 to 30 min per ~70k tets, both paths before); not profiled in this phase.

## Proof

`cargo test --workspace --release --offline --locked --no-fail-fast -j4` on macOS arm64: 3,569 passed, 0 failed, 131 ignored (`.audit/retire-tests.log`). `cargo check --workspace --tests --examples` has no `occt` feature left. Hosted CI (`qualification.yml`, `windows-step-runtime.yml`) is the Windows and Linux evidence.
