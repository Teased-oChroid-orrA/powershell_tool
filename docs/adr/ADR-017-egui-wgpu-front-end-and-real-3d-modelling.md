# ADR-017: egui on wgpu 30 for the native window, and a B-rep path for real 3D modelling

Status: Accepted in direction (user decisions 2026-10-11 after reviewing the redesign options artifact). Kernel
choice: closed by ADR-019 (OpenCascade; truck rejected on spike evidence).

## Context

The native window (`app-tui/src/gpu_viewer`) draws its menus, ribbon, tree and property manager with a hand-written
8x8 bitmap font on a custom wgpu renderer. The user finds it convoluted and dated and wants a modern interface. They
also want a full 2D and 3D sketcher, modeller and analyzer, including real solid booleans, STEP import/export and
picking by face.

## Decisions

1. **egui (egui-wgpu + egui-winit) replaces the bitmap chrome** in the native window only. `app-tui` stays the sole
   GUI head and the terminal UI is untouched. This reverses the earlier deletion of `app-egui/` for the native window;
   nothing of that head is revived, the new front end is written against the `Authoring` seam.
2. **Path 2: upgrade wgpu and winit together to the versions the egui release needs.** Verified on crates.io
   (2026-10-11): egui / egui-wgpu / egui-winit 0.36.2 require `wgpu ^30.0` and `winit ^0.30.13`
   (0.35.0 requires wgpu ^29). The repo is on wgpu 0.19.4 and winit 0.30.13, so winit already matches and the work is
   the wgpu 0.19 -> 30 port of `render.rs`, `window.rs` and the shaders (about 220 `wgpu::` uses in `render.rs`).
   The terminal-only build (`--no-default-features`) must keep compiling without any of them.
3. **Layout: design B (Canvas) with a thin step bar from design C.** The model fills the window; tools float and
   follow the selection; Ctrl+K is the single way to reach every command, ranked by what is enabled; the step bar
   (Sketch, Model, Mesh, Solve, Results) is a thin strip derived from the editor's existing availability rules,
   never a second source of truth, and never blocks jumping to another step.
4. **A typed panel model beside `DockItem`.** `Authoring::properties()` returns preformatted text rows, which egui
   cannot turn into number fields, choices or toggles. A structured model (label, value, kind, action) is added; the
   bitmap renderer keeps consuming `DockItem` until it is removed. Editor cores and their tests are not changed to suit
   the front end.
5. **Refinement zones shrink and follow the ZZ error.** Defaults: hole core 1.25 r blending to 2 r; edge supports and
   loads become a band along the edge (about 3% of the diagonal) instead of a disc; each zone shows its element count and
   the Mesh panel shows a total budget. After every solve the ZZ error field is folded in automatically: zones are
   re-derived from it, adaptive passes start from it, and the panel shows error before and after. 3D needs the 3D
   size field from decision 6 before this applies to solids.
6. **Real 3D uses a B-rep kernel.** The user needs STEP and face picking, which rules out mesh-only booleans.
   Candidates: a pure-Rust B-rep (truck) or OpenCascade bindings. Selection is by spike (boolean robustness on the
   plate / lug / bushing parts, fillets, STEP round trip, Windows build and offline packaging cost), recorded in a
   follow-up ADR. Whichever wins, the kernel sits behind a small trait in `fea-problem` and feeds a tetrahedral
   mesher; `fea-core` gains a 3D size field and remeshing for adaptive solids.

## Consequences

- The wgpu port lands first and alone: viewport behaviour and all 1200 tests must be unchanged before egui appears.
- Windows-only and real-input behaviour cannot be verified on macOS; each increment states that limit.
- Rejected: keeping the bitmap chrome (user rejection), egui software rendering on a separate surface (second
  renderer to maintain), a mesh-only boolean path (no STEP or face picking).
