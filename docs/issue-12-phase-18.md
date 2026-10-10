# Phase 18 — direct wgpu deformation viewport

2026-10-10. User-authorized extension to issue #12: native Rust `wgpu` directly, without a game
engine or higher-level 3D wrapper. Ratatui remains the Toolbench controller; `winit` owns a companion
native window, launched as the same executable's `--gpu-viewer <scene.json>` mode before terminal
initialization. Closing that window leaves the terminal running. Window/process/file effects are
executed in `main.rs`; the reducer emits effects and receives job completion events.

## Supported workflow

In FEA Workbench (outside text editing/pickers), press `g` / `G` after solving the current inputs:
static result with selected scalar contour, harmonic animation of the selected natural mode, or
static buckling shape. Modal timestamps use the computed frequency; modal amplitude is arbitrary
mass-normalized shape, not a prediction of excitation response. Press `t` / `T` for **actual linear
free-vibration frames**: solve the static mechanical load, release it instantaneously, integrate
undamped average-acceleration Newmark for two fundamental periods / 240 steps, and retain every
DOF. Positive density and homogeneous supports are required; contact bushings and thermal loading
are refused. Initial-state residual/equilibrium and every transient residual are checked; mechanical
energy must remain conserved to 1e-6. The scalar contour is explicitly initial displacement magnitude,
not a time-varying stress claim. Edited inputs discard stale animation results.

Window controls: Space play/pause; Left/Right step frames and pause; Home reset; Up/Down deformation
amplification; +/- playback rate; left drag orbit; wheel zoom; Escape close. The title reports source,
physical time (seconds for modal/transient), deformation amplification and playback rate. Display
speed defaults to four seconds per history/period and is distinct from physical time.

## Rendering and bounds

Direct WGSL vertex shader reads two frames from an immutable storage buffer and interpolates
displacements. Base geometry, indices and all frame samples upload once; playback updates a 48-byte
uniform buffer. No CPU remeshing or framebuffer readback occurs during interactive rendering.
Depth testing handles full 3D boundary surfaces, rather than the terminal raster's front face only.
Tri3/Tri6/Quad4/Quad8/Quad9 faces are tessellated through their supplied nodes, including midsides
and Quad9 centers. Piecewise triangular interpolation is a visualization approximation; curved
higher-order shapes are not evaluated continuously inside triangles. Scalar range is fixed across
playback. Coordinates are centered/scaled in f64 before conversion to f32 to retain small models
at large coordinate offsets. Solver data remain f64 and are never altered by rendering.

Scene validation rejects nonfinite coordinates/fields/times, invalid topology, mismatched frame
lengths and nonmonotonic timestamps before upload. Bounds: 500,000 vertices, 1,001 frames, 4,000,000
vertex-frame samples (64 MiB storage), 3,000,000 triangle indices, 256 MiB serialized scene file;
adapter storage limits are checked too. Transient full-history allocation is bounded before solving.
Temporary scene files live until the child closes and are removed by RAII. Viewer startup/exit
errors return to the terminal notification queue. No independent hardware speed claim is made.

`gpu-viewer` is a default app-tui feature; `--no-default-features` keeps the terminal build available
without GPU/window dependencies. There is no Bevy/three-d/glam rendering layer. Linux uses X11;
Windows/macOS use winit's native backend. A functioning desktop/display and driver are runtime
requirements. Software headless rendering verifies the real pipeline where available; it does not
verify Windows native-window interaction or hardware throughput.

## Validation

Commands, results, failures and limitations are appended after current-source validation. Relevant
checks: scoped app-tui bins/lib/tests check, lib and rendering suites, terminal-only compile, explicit
headless deformation pixel comparison, and the root CI workspace suite. Scene interpolation,
nonfinite/index/time rejection, midside/center retention, full solid boundary rendering, coordinate
normalization, full physical history and stale job handling have regression coverage.

### Targeted validation / diagnosed failures

`cargo check -p app-tui --bins --lib --tests --locked -j4` passed. GPU-module regressions passed:
9 passed, 0 failed, 2 adapter-dependent tests ignored by default (`gpu-regressions-final.log`).
Both adapter tests were then explicitly run **serially**: 2 passed, 0 failed
(`gpu-native-serial.log/.exit`). The real WGSL pipeline paints colored surfaces and produces different
pixel buffers at both endpoint frames and their interpolated midpoint. Software adapter:
Mesa llvmpipe (LLVM 19.1.7, 256 bits), OpenGL backend. Illustrative playback profile: 2,145 vertices,
4,096 triangles, 241 frames, 8,271,120 sample bytes uploaded once, 48 uniform bytes per frame;
upload/pipeline setup 85.428 ms, submitted playback plus GPU completion 370.717 ms at 512x256.
This batch measurement ran alongside mechanics tests and is not a hardware/interactive FPS claim.

First app-lib run exposed a fixture issue: the success test chose the large hole-refined template,
which correctly exceeded the explicit full-history budget. It now uses the bounded cantilever;
a separate test retains the oversized-history refusal. No memory limit or numerical tolerance was
relaxed. First explicit GPU run attempted both independent EGL contexts concurrently: the pixel
comparison passed, but Mesa teardown in the other test reported `BadDisplay`/SIGABRT. Serial execution
with workspace-local XDG runtime/cache directories passed both; use `--test-threads=1` for these
adapter tests. Interactive viewer runs one GPU context per companion process.

Final workspace/terminal-only/rendering results are recorded in the Claude Code handoff.
