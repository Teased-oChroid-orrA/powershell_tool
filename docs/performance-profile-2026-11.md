# Performance profile (2026-11)

Evidence record of a profiling pass over four workloads, release builds, one machine. The audit that preceded it found the workspace already tuned (workspace clippy with perf lints shows only cold-path allocation lints; `lto` / `codegen-units` was measured and rejected in `docs/index-audit-2026-10.md`), so remaining speed can only come from profiling real workloads.

## Method and limits

- Machine: MacBook Air (M1, 8 cores), macOS 15. Other sessions compiled on it the whole time: load average ran from 130 down to 5. Wall times taken under load were up to 8 times too long (a single-thread run showed 13 % CPU), so **user CPU time and alternating A/B runs** are the evidence, and absolute milliseconds do not carry over to Windows.
- Builds: `CARGO_PROFILE_RELEASE_DEBUG=line-tables-only` in the environment (no `Cargo.toml` change), own worktree target.
- Profiler: `/usr/bin/sample` (no `samply` or `cargo flamegraph` installed). It must be started first and wait for the program: `sample <process name> <seconds> -wait -file out.txt`, then the program. Attaching by pid to an already running process returned an empty call graph for every Rust binary here. Self time = the "Sort by top of stack" section; inclusive time = the call graph. The sampler itself slows a multi-threaded run (index build 8.3 s sampled, 2.4 s unsampled).
- Runs were one after another, never in parallel; `uptime` was read before each run.
- Not measured: Windows, any run with `occt`, real NIST STEP files (see target 2).

## 1. Search: extraction and index build

### Workloads

| Workload | Harness |
|---|---|
| PDF extraction, 5.9 MB, 23,364 lines out | `search-core/examples/pdf_extract_bench` on `search-core/benches/data/xlarge.pdf` |
| All formats, parse only and full pipeline | `search-core/benches/discovery_and_extraction` |
| Index build, 3,000 files x 150 lines (36 MB of text), 8 threads | `search-core/examples/exact_bench 3000` ("index build" row) |
| Index build and commit, 5,000 tiny documents | `native-search/benches/indexing_and_search` |

### Measured

- `native-search` bench: 5,000 documents index in 0.058 s, commit 0.77 s, searches 137 to 562 us median. The whole run is 3.4 s wall, 0.5 s CPU: too small to profile and nothing in it to fix.
- `discovery_and_extraction` (parse only, in-memory, loaded machine): `.txt` 66,043 files/s (108.9 MB/s); docx, pptx, xlsx 0.1 to 3 ms median; rtf large 16 ms; **PDF 0.38 s (272 KB), 0.22 s (1 MB), 2.1 s (5.9 MB)**. Office, zip and text extraction cost next to nothing compared with PDF.
- `exact_bench 3000`, three runs, unsampled: index build 2.47 s and 2.43 s (8.25 s under the sampler and load 6); cold full scan 0.37 and 0.42 s; narrowing 20 ms.

### Hotspots, PDF extraction (baseline, `sample`, 1,143 samples of the xlarge file)

About 70 % of `extract_pdf_lines` was `line_quality::clean_lines`, and about half of all samples were `malloc` / `free`:

| Share | Where | Cause |
|---|---|---|
| ~40 % | `line_quality::assess` -> `classify` | three allocations per token (`Vec<char>`, `Vec<char>`, `String`) |
| ~30 % | `dehyphenate` -> `word_sets` | a `String` allocated and SipHash-inserted for every word occurrence, not every distinct word |
| rest | inflate (flate2), regex scan of content streams, `unescape_pdf_string` | spread, none above 12 % |

### Fix (a measured win)

`search-core/src/line_quality.rs`: `classify` takes a per-line `Scratch` (two `Vec<char>` reused for every token) and builds the lowercase copy only in the two-letter branch that uses it; `clean_lines` creates one `Scratch` per call; `word_sets` builds its lowercase key in one reused buffer and clones it only for a word not yet in the set. Public `assess` / `is_noise` are unchanged.

A/B on `xlarge.pdf`, seven alternating runs of old and new binary, load about 10:

| | old | new |
|---|---|---|
| wall, median | 0.60 s | 0.50 s (-17 %) |
| user CPU, median | 0.56 s | 0.47 s (-16 %) |

Output is identical (23,364 lines, 828,507 UTF-8 bytes) and a new profile puts `clean_lines` at 14 % of extraction. Differential tests keep a verbatim copy of the old `classify` and `word_sets` in the test module and compare them with the new ones: `classify_with_reused_buffers_matches_the_allocating_original` (200,000 pseudo-random tokens over an alphabet that reaches every verdict, one scratch reused throughout so stale contents would show; asserts each of the four verdicts is hit more than 500 times) and `word_sets_with_a_reused_key_matches_the_allocating_original` (3,000 lines of repeated words, compounds, line-end hyphens, accents). `cargo test --release -p search-core --lib line_quality`: 10 passed.

### Hotspots, index build (`exact_bench 3000`, 29,022 non-idle samples)

| Share | Where |
|---|---|
| 24 % | `tantivy PostingsWriter::index_text` |
| 11 % | `NgramTokenStream::advance` (trigram field) |
| 5 % | `LowerCaserTokenStream::advance` |
| 4 % | `tantivy_stacker` slice compare |
| 11 % | file system (`open`, `fcntl`, `write`) |
| 15 % | `textfold::fold_lower` and the char tables under it |

Verdict: the build is tantivy's own tokenising and posting work, which is the cost of a trigram index and not ours to change. The 15 % in `fold_lower` is this benchmark's worst case: every generated document holds a non-ASCII word, so `folded_trigram_text` folds the whole body, lowercases it a second time to compare, and the folded text is then indexed again (`docs/search-benchmarks.md` records the 1.75 to 1.85 times build cost for such a corpus). An ASCII document returns `None` before any of this. Dropping the redundant `to_lowercase()` would save a third of that 15 % (about 5 % of a build that exists only in this corpus): below the bar, not changed.

## 2. Tet volume mesher

### Workload and what was not available

The NIST PMI corpus (33 STEP files, `docs/issue-46-phase-175.md`) is not in the repository and not on this Mac outside `~/Downloads`, which the shell sandbox cannot read, so no NIST file was meshed here. The workload is the committed real CAD part `nema17-motor-bracket.step` through the production path (`Problem::from_step`, `solid_mesh`, `solve`), the NEMA17 row of `fea-problem/tests/native_step_benchmark.rs`:

`CASE=NEMA17 cargo test -p fea-problem --no-default-features --release --test native_step_benchmark -- --ignored --nocapture`

It produces 206,101 Tet10 tetrahedra (959,811 dofs), element size 1.667. Earlier mesher profiles on NIST `ctc_01 rd` (`docs/issue-46-phase-184.md`: face-state memo, exact-point cache, a sorted-`Vec` negative result) are not repeated.

### Measured

Import 0.03 s. Alternating old and new binary, two rounds, load 3 to 9 (the first run under load 17 to 70 took 62 s to mesh and 133 to 309 s to solve, so none of that is quoted as a result):

| | mesh | `solve()` call |
|---|---|---|
| before | 32.82 s, 32.76 s | 77.5 s, 78.4 s |
| after | 29.11 s, 28.42 s | 79.7 s, 78.5 s |

Mesh median 32.8 s -> 28.8 s (-12 %). Every printed result is identical before and after: tets 206,101, dofs 959,811, minimum scaled Jacobian 0.047, edge ratio 11.6, size jump 2.1, maximum displacement 6.69176e-3, strain energy 4.62377, reaction -2400.000, equilibrium error 6.0e-13.

### Hotspots (baseline `sample`, 58,567 non-idle samples over two meshing runs)

| Share | Where | Owner |
|---|---|---|
| 44 % | `solid_kernel::bvh::Bvh::nearest` 14,964 + `closest_on_triangle` 8,299 + `Bvh::within` 2,268, reached through `SurfaceSolid::signed_distance` | `solid-kernel/src/bvh.rs` (not editable in this pass) |
| 26 % | the `signed_distance` calls of `Refiner::needs` (four corner queries and one centroid query for every tetrahedron examined) | `fea-core/src/tetmesh.rs` |
| 8 % | `mesh_solid_reported` itself (inlined refinement and snap loops) | `fea-core/src/tetmesh.rs` |
| 14 % | malloc / free, SipHash (`HashMap` of edges), `simplex_validity`, `quality`, `Refiner::add_tet` | spread |

The two BVH rows and the `needs` row overlap: the BVH time is the cost of the `signed_distance` queries, and `needs` made 26 points of them.

### Fix (a measured win)

`Refiner::needs` (`fea-core/src/tetmesh.rs`) now remembers on which side of the surface each vertex lies (`inside: Vec<Option<bool>>`, indexed by vertex; a vertex never moves while refining, and one vertex belongs to many tetrahedra), and asks for the centroid distance only when the four corners agree (the centroid only mattered in the `||` that a corner disagreement already satisfies). The distance function is pure, so no answer changes.

Differential test `tetmesh::tests::refinement_with_cached_vertex_sides_matches_the_uncached_original_and_asks_less` keeps a verbatim copy of the old `needs` and runs the refinement loop of `mesh_solid_reported` with each on a sphere, a box with a through hole and an offset ellipsoid: vertices, tetrahedra and alive flags are equal element for element, and the new loop makes less than half the distance queries (asserted). `cargo test --release -p fea-core --lib refinement_with_cached`: passed.

### Remaining and follow-ups (not done)

- After the fix the BVH rows are still the largest block (`Bvh::nearest` 6,066 + `closest_on_triangle` 5,186 + `within` 2,828 in the post-fix 140 s sample, which also contains the solve). Further gain needs a faster `nearest` or fewer queries from `boundary_repair` and the snap loops (`order.sort_by` in the snap pass asks the distance of both points on every comparison: about 2 % of samples, left). `solid-kernel/src/bvh.rs` is under edit by another session: follow-up with these numbers.
- `fea_problem::solve` always calls `solid_mesh` again (`fea-problem/src/solve.rs`, `solve_resolved`). A caller that meshed first pays the mesh twice: in this benchmark 29 of the 78 s of the `solve()` call are a second mesh of the same problem. Whether the editor does this in the app was not checked. A mesh cache keyed on the problem is a design decision (invalidation rules), not a local fix.
- Not a mesher cost but visible here: the 960k-dof solve is the iterative path (section 3).

## 3. Sparse solves

### Workloads

| Workload | Harness |
|---|---|
| 2D and 3D structured models (Quad4/9, Hex8/20/27, Tet10), assemble, symbolic, factor, solve | `cargo test -p fea-core --release --test bench -- --ignored --nocapture pipeline_2d_and_3d` |
| 3D bracket, 959,811 dofs (iterative path, PCG + AMG) | NEMA17 row of section 2 |
| Lug (kernel solver `FeaLug`, 2D contact, Newton) | `cargo test -p lug-solver --release --test kernel_analysis -- --nocapture --test-threads=1 matches` |

### Measured

`pipeline_2d_and_3d` (best of three per stage, load 5, ms):

| Model | dofs | nnz(L) | assemble | symbolic | factor | solve |
|---|---|---|---|---|---|---|
| Quad9 100x100 | 80,802 | 7.6 M | 10.1 | 60.0 | 144 | 9.7 |
| Hex8 10^3 | 27,783 | 25.6 M | 13.1 | 78.5 | 810 | 19.5 |
| Hex20 14^3 | 38,475 | 60.4 M | 50.1 | 198 | 2,895 | 43.9 |
| Hex8 14^3 | 73,167 | 127 M | 42.0 | 316 | 8,075 | 90.3 |
| Hex27 14^3 | 73,167 | 123 M | 75.4 | 407 | 6,508 | 69.1 |
| Tet10 10^3 | 27,783 | 27.4 M | 16.9 | 82.8 | 832 | 20.6 |

The 959,811-dof bracket solves in about 49 s (the 78 s `solve()` call less its second 29 s mesh) on the iterative path that `SolveMethod::Auto` takes above 60 M factor entries.

Lug, kernel against the legacy condensed solver (one run each): axial 145 ms against 12.9 ms, bushed axial 908 against 446, elastic pin 743 against 22, collapse oblique 60 degrees 1,770 against 158. The kernel is the app's default, so these are what a user waits for; they are milliseconds to a second.

### Hotspots

- Direct factorisation: the self time is `gemm_f64::microkernel::neon::x4x4` (110,774 of 139,443 non-idle samples, 79 %), faer's supernodal dense update, plus its operand packing (6 %). Symbolic analysis and AMD ordering are 2 %, element assembly about 1 %. Nothing of ours to fix: it is faer calling `gemm`. The Hex8 14^3 factor (8 s) is the case that `AUTO_ITERATIVE_FACTOR_NNZ` sends to PCG + AMG.
- Iterative solve of the bracket (non-idle samples of the whole run, 145,160): `amg::gauss_seidel` 28 %, `Csr::spmv` 24 %, the rest is the two meshing runs. Both are memory-bandwidth-bound sparse sweeps; `gauss_seidel` is a sequential row sweep. A multicoloured or parallel smoother would change the iteration path and the converged digits, so it cannot carry an identical-output test and was not attempted; bounds-check removal in these loops was not tried because `fea-core` is being audited for `unsafe` by another session.
- Lug on the kernel (17,784 non-idle samples): memory zeroing (`__bzero`, `madvise`, `memset`) is 29 % of them; 1,293 samples are attributed to `kernel::Work::new`, the rest come through rayon closures with inlined frames and no named caller. `Work::new` zero-fills about 120 KB of scratch, and `try_for_each_init` in `assembly.rs` (lines 205, 257) and `nonlinear.rs` (line 299) builds one per rayon task, which for the small colour classes of a 2D lug is many times per assembly. Both files are being edited by another session, so this is a follow-up with the numbers: build the scratch once per thread (a `thread_local!` or `with_min_len` so a task covers many elements) instead of once per task. Worth fixing; likely the largest cheap win in this section for small models, not measured as a fix here.
- Lug on the kernel, the rest: `stiffness_fast` 6 %, faer simplicial factorisation 4 %.

## 4. egui redraw (CPU side)

Harness: `app-tui/src/gpu_viewer/gui/frame_cost.rs` (new, `#[ignore]`d, no GPU): per frame it runs `Gui::run` (or `run_dashboard`) and `egui::Context::tessellate`, the CPU work the window does on every redraw before the renderer paints. Five warm-up frames (scroll areas and floating cards take the previous pass's size), then 7 batches of 100 identical frames; the median batch is printed.

`cargo test -p app-tui --release --lib frame_cost -- --ignored --nocapture` (three runs, load 11 to 14; the three runs agree to within 3 %):

| Case | Size | `Gui::run` | tessellate | total | triangles |
|---|---|---|---|---|---|
| dashboard | 1360x860 | 0.057 ms | 0.043 ms | 0.10 ms | 5,961 |
| dashboard | 780x900 | 0.060 ms | 0.040 ms | 0.10 ms | 5,643 |
| Search window, empty | 1360x860 | 0.176 ms | 0.064 ms | 0.24 ms | 8,271 |
| Search window, empty | 780x900 | 0.170 ms | 0.058 ms | 0.23 ms | 7,435 |
| Bushing Stackup | 1360x860 | 0.320 ms | 0.071 ms | 0.39 ms | 6,889 |
| Bushing Stackup | 780x900 | 0.310 ms | 0.044 ms | 0.35 ms | 4,037 |

The GPU side, `render::performance_tests::headless_playback_profile` (ignored; Apple M1 Metal adapter; it only runs, `render.rs` is not edited): 241 playback frames of a 4,096-triangle contour in 31.4 to 32.9 ms (0.13 ms a frame), upload 16.9 to 17.9 ms.

### Hotspots and verdict

Every measured chrome frame costs 0.1 to 0.4 ms of CPU, about 2 % of a 60 Hz frame, and the time is spread over layout and widget code with no function that matters. The Search page cost known from PR #344 (document rebuild per redraw 0.14 ms with the mined-term cache, 2.4 ms without) is in the same range and was already fixed there. **Not worth fixing.** What this does not cover: a Search window with hits on screen, a large Solid editor tree, a window being resized or dragged (the frames above are identical, idle frames), and Windows (a different `wgpu` backend and `egui` text rasteriser). The harness is left in the tree so those cases can be added.

## Summary

| Target | Result | Change |
|---|---|---|
| 1. Search extraction | PDF extraction was 70 % `line_quality` allocation | fixed: `xlarge.pdf` -16 % CPU (0.56 -> 0.47 s), identical output |
| 1. Search index build | tantivy trigram indexing 47 %, folded field 15 % (worst-case corpus only) | none: not ours / below the bar |
| 2. Tet mesher | 44 % in `solid-kernel` BVH queries, 26 % of it from `Refiner::needs` | fixed: NEMA17 mesh -12 % (32.8 -> 28.8 s), identical output; BVH and a second mesh in `solve()` left as follow-ups |
| 3. Sparse solves | direct: faer GEMM 79 %; iterative: AMG smoother and SpMV 52 %; lug kernel: scratch zeroing 29 % | none in `fea-core` outside two files others are editing: follow-up for `assembly.rs` / `nonlinear.rs` |
| 4. egui redraw | 0.1 to 0.4 ms CPU per frame | none: not worth fixing |

### Follow-ups (with numbers above)

1. Done, section 6: `fea-core` builds the element scratch once per thread, not once per rayon task (section 3, lug).
2. `solid-kernel/src/bvh.rs`: `Bvh::nearest` and `closest_on_triangle` are the largest block left in meshing (section 2). Done, section 5: the NEMA17 mesh 28.3 -> 25.4 s, identical output.
3. `fea_problem::solve` re-meshes a solid problem that the caller already meshed; a mesh cache needs an invalidation rule (section 2).
4. Not measured here: the NIST corpus, Windows, the index build on real (mostly ASCII) corpora, a Search window with hits.

## 5. Follow-up: BVH surface-distance queries (`solid-kernel/src/bvh.rs`)

Follow-up 2 of the list above, done in a later session on the same machine (M1 Air, load 3 to 9 during the timed runs, read from `uptime` before each).

### Workload

A temporary hook in `SurfaceSolid` recorded every query of the NEMA17 mesh run: **about 19 million**, 12.2 M `signed_distance` (`Bvh::nearest`), 6.5 M `faces_near` (`Bvh::within`), 0.3 M `nearest_face`. The surface is small: 350 vertices, 740 triangles (the fixture `solid-kernel/tests/data/bvh-nema17-surface.txt` is that exact mesh), so the tree (369 nodes) lives in cache and the cost is arithmetic and branches, not memory. Per `nearest` call: about 41 nodes visited and 25 triangles tested, of which 5.2 improve the best distance; 66 % of the 25 tested triangles have a bounding box already farther than the best (so the closest-point test could not change the answer).

### Changes (all answers bit-identical)

- Triangle corners copied into leaf order (`corners`, `ids`): a leaf is read sequentially instead of `order` -> `triangles` -> three `vertices` loads. Nodes are 64 byte, `u32` indices. `nearest` / `within` no longer take `vertices` / `triangles`.
- `nearest` carries each node's box distance on the stack (computed once, when the node is pushed) instead of recomputing it when the node is popped.
- Per-triangle box skip: a leaf triangle whose box distance exceeds `(sqrt(best) + 1e-12 x magnitude)^2 x (1 + 1e-12)` cannot produce a computed squared distance below `best`, so its closest-point test is skipped. The margin covers the rounding of the computed closest point (absolute, scaled by the largest coordinate of mesh and query) and of the box distance (relative). A triangle that ties or nearly ties is never skipped, so the tie-break (first triangle met in the traversal) is untouched.
- Traversal order, tree shape (`LEAF = 4`, median split on the longest axis), the `<` / `>=` comparisons and the order `within` reports in are unchanged: that is what fixes which of several equidistant triangles is returned.
- `fea-core/src/tetmesh.rs`: the snap pass sorted `order` by `|sdf|` with `sort_by`, which asked the distance of both points on every comparison (about 2 n log n queries); it now uses `sort_by_cached_key` on the distance's bit pattern (n queries; same stable order).

### Proof

`bvh::tests` keep a verbatim copy of the old tree, traversal and `closest_on_triangle` (`reference`) and compare on bits (squared distance, triangle id, region, closest point; `within` also the order and values reported) on 47,831 query points per mesh: scattered inside and far around the part, on / beside the surface at offsets from 1e-12 to 0.3 of the part size, exactly on vertices, edge midpoints and centroids, one-ulp neighbours, and a lattice that lies on the planes of the box meshes (ties asserted: 1,200 to 1,900 tied queries per grid box). Meshes: the NEMA17 surface, two grid boxes, an icosphere, an icosahedron, a 12-triangle box, and the NEMA17 surface shifted to 1e6 / 1e9, scaled by 1e-6 and 1e4 (the margin scales with magnitude). `tetmesh::tests::cached_distance_sort_matches_the_comparator_sort_on_ties_and_special_values` does the same for the sort. End to end the mesh is identical: tets 206,101, dofs 959,811, and an FNV hash of the node coordinates' bits `d8c394c4d15c1c79` before and after every step.

### Measured

Micro-benchmark `bvh::tests::replay_recorded_queries` (ignored; 2.4 M recorded queries, old and new alternated in one process, seven rounds, medians): old 2.73 s; layout and stack only 2.38 s (0.87); with the box skip 1.90 s (**0.70**).

NEMA17 mesh, alternating mesh-only runs of the old and new binary (`Problem::from_step` + `solid_mesh`; the full `native_step_benchmark` also solves for 80 s):

| Build | mesh seconds | rounds |
|---|---|---|
| before | 28.34, 28.29, 28.36 | 3 |
| layout + stack | 27.55, 27.12, 27.18 | 3 |
| + triangle box skip | 25.81, 25.86, 25.84 | 3 |
| + cached-key snap sort | 25.37, 25.35, 25.37 | 3 |

Mesh median 28.3 s -> 25.4 s (-10.5 %); the BVH loop itself is 30 % faster, the rest of the mesher (about 60 % of the run) is unchanged.

### Not done, and why

- Visiting children in a different order, or seeding `best` from the previous query's triangle, would cut node visits (41 per query) but changes which of several equidistant triangles is returned when a box distance equals the best exactly (axis-aligned parts: every flat face). No proof of identical output exists, so it was not tried. A different leaf size or split rule changes the traversal the same way.
- A plane-distance lower bound per triangle (stored normal): 35 % of tests skippable against 66 % for the box, and the box already leaves more than half of the remaining tests as ones that improve the best; no measurement taken, judged below the bar.
- Not measured: the NIST corpus, Windows, a large surface mesh (cache behaviour of the new layout is untested at 100k+ triangles).

## 6. Update: element scratch per thread (follow-up 1, done)

`assembly.rs` (`assemble_stiffness`, `assemble_scalar_identity`) and `nonlinear.rs` (`assemble_nl`) took their element buffers (`Work`, element matrices, `xyz`; about 150 KB) from `try_for_each_init`, which builds them once per rayon task. They now come from one `thread_local!` (`assembly::with_scratch`, `Scratch`): allocated once per thread, a re-entrant call gets a fresh one. No kernel reads a buffer before writing it, and the per-task state was already reused across elements, so the results do not depend on which thread ran which element.

Measured with the `lug-solver` `kernel_analysis` tests (`-- --nocapture --test-threads=1 matches`, release), old and new binaries run alternately five times each on a machine shared with other builds (load average 5 to 47, so wall times are noisy; the CPU split is the stable number):

| | before | after |
|---|---|---|
| sys CPU of the whole test binary (page faults of the zero-fill) | 7.0 to 30.6 s | 4.1 to 5.0 s |
| user CPU | 21.5 to 29.3 s | 16.7 to 24.9 s |
| wall, two quiet runs | 11.2, 12.9 s | 10.9, 9.9 s |
| best-of-five kernel ms: axial / bushed axial / collapse axial / collapse bushed / collapse oblique 30 / 60 | 158 / 972 / 346 / 519 / 1050 / 1792 | 127 / 884 / 297 / 447 / 995 / 1609 |
| best-of-five kernel ms: elastic pin 0 / 0 (second case) / 45 / oblique frictional | 769 / 1366 / 1097 / 861 | 624 / 1144 / 888 / 601 |

The printed physics (hoop, pressure, travel, arc, von Mises, verification residuals) is byte-identical before and after (same md5 over the output without the timings), and the `fea-core` tests give the same counts. The gain is mostly CPU time, not wall time: on an idle 8-core machine the lug tests are 5 to 30 % faster, on a busy one the saved CPU shows up as wall time as well. What remains of the zero-fill is one allocation per thread; the per-task cost is gone.


Repeat on a quiet machine (load 4), the merged tree against the old binary, three runs each, alternating: wall 8.8 to 9.0 s -> 7.7 to 8.0 s, user CPU 18.2 to 18.5 s -> 13.3 s, sys CPU 5.5 to 6.8 s -> 3.5 s; kernel ms per test (old, new): axial 119 -> 109, bushed axial 775 -> 728, collapse axial 280 -> 245, collapse bushed 440 -> 405, collapse oblique 30 / 60 860 / 1364 -> 790 / 1180, elastic pin 0 / 0 / 45 605 / 1037 / 863 -> 482 / 850 / 700, oblique frictional 622 -> 475 (the run-to-run spread of each side is under 5 %, except the old binary's last three cases at up to 15 %). Output again byte-identical.
