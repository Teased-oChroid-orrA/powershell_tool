# ADR-019: OpenCascade (through the `opencascade` crate) is the B-rep kernel; a volume mesher is the missing piece

Status: Accepted (evidence from the spike below; the user chose STEP import/export and picking by face, which need a B-rep).
Closes the kernel question left open in ADR-017 decision 6.

## Evidence (spike, 2026-10-11, macOS arm64, scratch crates outside the repo)

Scenario: a 12 x 4 x 0.25 plate, round holes, bosses, a pocket, fillets, STEP.

| | truck 0.6 (pure Rust; last release 2024-09) | OpenCascade 7.8.1 via `opencascade` 0.3 (released 2026-08) |
|---|---|---|
| hole at 40 round-number positions | 39 ok, **1 failed** (x = 5.00: `and` returned `None`) | 40 ok, 98 ms for all 40 |
| boss standing on the face (coplanar contact, the common case) | **failed** (`or` returned `None`) | ok (8 faces) |
| boss overlapping / through the plate, pocket | ok | ok |
| hole tangent to both long edges | returned a "closed" 9-face solid (wrong: the part splits in two) | 13 faces (two pieces, as it should) |
| fillet every edge | no fillet in the crate family | ok (26 faces) |
| STEP | export only (7 KB plate) | write and read back: same face and edge counts |
| build | seconds | about 25 minutes first time (OCCT compiled from vendored source with CMake, `-j6`); a 22 MB statically linked test binary |

truck is rejected: a boolean that silently returns a wrong solid or nothing on the most ordinary alignments (round numbers,
coplanar contact) cannot sit under a modeller. A hand-written mesh or implicit boolean was rejected earlier (ADR-017) because
STEP and face picking need a B-rep.

## Decisions

1. **Kernel: OpenCascade 7.8.1 through `opencascade` / `opencascade-sys` 0.3**, in a new workspace crate `solid-kernel`
   behind a cargo feature `occt` (off by default so day-to-day builds and `cargo test` stay fast; CI builds it with the feature
   and caches the OCCT build). Without the feature the crate exposes only the data types and an `Unavailable` error, so the
   rest of the repo never needs a C++ toolchain. CMake 4 needs `CMAKE_POLICY_VERSION_MINIMUM=3.5` for the vendored OCCT
   (found in the spike); the Windows target needs MSVC and CMake in CI.
2. **License**: OCCT is LGPL 2.1 with the Open CASCADE exception (`OCCT_LGPL_EXCEPTION.txt`). Static linking into the shipped
   exe is the intended use of that exception, but ship the license text and have it confirmed before the first release that
   contains it.
3. **The kernel does not mesh volumes.** OCCT triangulates surfaces only (`BRepMesh`). The finite element mesh of a solid needs
   a tetrahedral volume mesher, which `fea-core` does not have (it meshes 2D regions, sweeps them, and imports Gmsh/Abaqus
   files). Decision: build the mesher in `fea-core` as a pure function of a **signed-distance / classification closure plus a
   size field** (BCC-lattice isosurface stuffing on an adaptive octree). That is testable on analytic solids with no OCCT, it
   gives the 3D size field that adaptive refinement needs (the same size field drives the ZZ-error passes), and the OCCT side
   only has to answer "is this point inside" and "which face is nearest" (`BRepClass3d_SolidClassifier`, `BRepExtrema`).
   Faces become named surfaces (`faceN`) so loads and supports bind to them as they do to sketch edges today.
4. **Order of work**: (a) the mesher in `fea-core` with analytic tests; (b) `solid-kernel` with the `occt` feature (extrude /
   revolve a sketch profile, union / cut / intersect, fillet, faces with ids and normals, ray pick, STEP in and out, surface
   triangulation for display); (c) `fea-problem` geometry from a kernel solid and the boss / cut / fillet features in the
   history; (d) the window: picking faces, STEP in the Open card. Each lands with its own tests; (b) and (c) run in the `occt`
   CI job.

## Consequences

- A second, slow native build exists; it is opt-in and cached.
- The mesher is the long pole for real 3D analysis; until it lands, 3D solids still come from extruding or revolving a 2D
  profile (the existing path).
- Face-naming stability after a boolean is the kernel's history-tracking problem (OCCT's `BRepTools_History`); it is not solved
  by this ADR (the "stable topological naming" item stays open).

## Addendum (2026-10-11): the Rust binding is `cadrum`, not `opencascade` 0.3

Decision 1 chose OpenCascade; this addendum replaces only the **binding crate** (the kernel is still OCCT, now 8.0.1).
The question was whether `opencascade` 0.3 (compiles OCCT 7.8.1 from vendored source) is the best way in, given the target
(Windows x64, offline, self-contained, no admin rights) and the 25 minute cold build.

| | `opencascade` 0.3 (bschwind) | `cadrum` 0.9 (lzpel) | `occt-wasm` 4 |
|---|---|---|---|
| OCCT | 7.8.1, built from source with CMake | 8.0.1, **prebuilt static tarball per target** (aarch64/x86_64 macOS, x86_64/aarch64 Linux, x86_64 Windows MSVC and GNU; about 30 MB macOS) | OCCT compiled to wasm, run through wasmtime |
| First build | about 25 min, CMake and a C++ toolchain, `CMAKE_POLICY_VERSION_MINIMUM=3.5` | about 1.5 s once the tarball is there; no C++ toolchain | no C++ toolchain, but a wasm runtime in the exe and about 21 MB loaded |
| Faces | iterator, no ids, `normal_at_center` aborts on curved faces | `iter_face`, `Face::id/area/center/surface`, per-triangle `face_ids` in the mesh, `iter_history` (old id, new id) | facade methods, handle based |
| Modelling | extrude, booleans, fillet (all edges), STEP | extrude with hole loops, revolve, sweep, loft, shell, offset, booleans as an expression tree (pieces returned), fillet and chamfer of chosen edges, STEP read and write, `clean` | primitives, booleans, sweeps, tessellation, STEP |
| Offline | needs the OCCT source in the crate (it is) | `OCCT_ROOT=<extracted tarball>` or `CADRUM_PREBUILT_URL=<mirror>`; vendor the tarball for an offline build machine | self-contained |
| Licence | MIT/Apache wrapper, LGPL 2.1 + exception OCCT | MIT wrapper; the tarball ships `LICENSE_LGPL_21.txt` and `OCCT_LGPL_EXCEPTION.txt` | MIT/Apache wrapper, OCCT inside |
| Maturity | 0.3.0, 2026-08, about 5.6k downloads | 0.9.0 published 2026-10-10, frequent releases (0.8.20 on 2026-09-13), about 7k downloads; API still moves between 0.x | 4.0.0, about 355 downloads, MSRV 1.95 |

Decision: **switch to `cadrum`** (pin the minor, `=0.9`-style review on every upgrade). Reasons, in order: the prebuilt
OCCT removes the 25 minute build and the C++ toolchain from every developer and CI machine; the Windows MSVC target is a
prebuilt, which matches the offline / self-contained constraint; face ids per triangle remove the connected-component
recovery the old wrapper needed; `iter_history` is the hook for stable topological naming; fillet and chamfer take chosen
edges. `occt-wasm` is rejected (a wasmtime runtime in a native app for no gain, tiny user base). The change was local to
`solid-kernel/src/occt.rs`; the public `Brep` API is unchanged and all 5 `tests/occt.rs` tests plus the 6 surface tests pass
(plate with hole through OCCT -> Tet10 -> SPR: 0.8% from Peterson).

Evidence and caveats (macOS arm64 only; Windows not run):
- In the build script of `cadrum` 0.9.0 the tarball download failed in this sandbox ("unexpected end of file") although
  `curl` fetched the same file; extracting it and setting `OCCT_ROOT` works. Treat the in-build download as best effort; CI
  and any offline machine should provide `OCCT_ROOT`.
- `Face::id` is not unique: the two caps of `Solid::extrude` share one id, so the mesh's `face_ids` merge them.
  `Brep::tessellate` splits such an id into connected pieces and assigns each to the face with the nearest centre.
  Face ids are also session-local (a TShape address): they do not survive STEP or a restart, so persistent face names still
  need the history map (open item).
- The wrapper's default features include a PNG renderer; it is turned off here.
- The release-blocking licence check stays open: confirm the LGPL 2.1 + exception notice ships with the exe.

## Addendum 2026-10-12: Windows linking and the license notice

- **Linking.** The prebuilt MSVC OpenCASCADE that `cadrum` downloads is `/MD` (dynamic CRT); the workspace forces
  `-C target-feature=+crt-static` so the shipped exe needs no Visual C++ runtime. On Windows the two cannot link
  (`unresolved external symbol __imp_*`, hosted run 38236448142). Hosted CI therefore checks the code against the prebuilt with
  `-crt-static` (`windows-prebuilt-dynamic-crt`, not a shipping configuration) and has a manual job that builds OCCT from source
  under the static CRT (`occt-windows-static-crt`, cargo feature `occt-source`). An `occt` Windows exe is shippable only once
  that job is green.
- **License.** An `occt` build carries OCCT under LGPL 2.1 with the Open CASCADE exception. `THIRD-PARTY-NOTICES.md` and the two
  license texts in `licenses/` ship with the executable (`app-tui-build.yml` stages `dist/` and refuses to publish without them;
  its `occt` input builds with `occt-source`). The default release build has no OCCT in it. The legal sufficiency of static
  linking for the way it is distributed is **not** decided here: it needs sign-off from the distributor.
