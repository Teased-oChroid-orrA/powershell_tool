# lug-solver

> TL;DR: Pin-loaded lug FE: mapped Q9 mesh, rigid analytic pin, Gauss-point augmented-Lagrangian contact + Coulomb friction; optional bushing with a solved interference-fit interface; plane-strain elastic-perfectly-plastic / finite-strain collapse. **Two solvers on one mesh: the general kernel (`fea-core`, via `FeaLug`) is the main one the toolbox runs; the condensed solver (the lug condensed onto its contact dofs once per mesh) and `FiniteLug` are the legacy comparison.** Pure function of inputs. Design record + validation: `docs/lug-analysis.md`, `docs/fea-core.md` Phase 7.

## Purpose
Owns: lug outline/mesh, assembly + condensation, rigid-pin contact, stress recovery, the load solve.
Does not own: any UI (`app-tui/src/toolboxes/lug_analysis/`), material data (`mechanics-core`), Q9 shape/stiffness kernels and `BandedSpd` (`edge-check`, made `pub` for this crate).

## Code Map
| Looking for... | Go to |
|---|---|
| Outline (rounded rectangle), ray distance, critical angles | `src/geometry.rs` |
| Mapped star O-grid, per-ray grading, angular `Refinement` | `src/mesh.rs` |
| Assembly, folded ordering, bore Green's matrix `G` and Schur `S` | `src/fe.rs` |
| Rigid-pin Gauss-point contact, AL, friction, Newton + Woodbury | `src/contact.rs` |
| Nodal stress recovery (per material), polar components | `src/stress.rs` |
| Elastic-perfectly-plastic (J2) by initial strain, Anderson, `PlaneMode` | `src/plastic.rs` |
| `LugModel` (`build_bushed`, `solve`, `solve_second_order`, `solve_sweep`, `limit_load[_with]`, `collapse_sweep`), load root solve, `Verification`, `PinSpec` (`Thermal`, `PinBody`), `auto_refinement[_for]`, `BushingSpec` | `src/solve.rs` |
| Finite-strain collapse (total-Lagrangian Q9, Hencky J2, true stress-strain, sparse Cholesky + L-BFGS), `Hardening::true_curve` (the law itself lives in `mechanics-core/src/hardening.rs`, re-exported here as `lug_solver::Hardening`) | `src/finite.rs`, `src/sparse.rs`, `tests/finite_strain.rs`, `docs/finite-strain-collapse.md` |
| Elastic pin disc: surface compliance added to `G` (`G + A`) | `src/pin.rs` |
| Pin bending through the thickness (double shear), slices on the lug's own load-travel curve | `src/thickness.rs` |
| Patch recovery (SPR), ZZ error estimate, strain / geometric energy | `src/stress.rs` |
| Upgrade pass: self-checks, SPR/ZZ, elastic pin, hardening, thermal, thickness, P-delta | `tests/verification.rs`, `tests/elastic_pin.rs`, `tests/thermal.rs`, `tests/thickness.rs`, `tests/second_order.rs`, `tests/angle_sweep.rs` |
| Exact-solution and consistency tests | `tests/lug_fem.rs`, unit tests per module |
| Plasticity: exact rings, NACA TN 1503 (12 published points) | `tests/plastic_validation.rs`, `tests/validation_naca_tn1503.rs` |
| Bushing: exact two-cylinder Lame press fit, fit/separation, half-vs-full | `tests/bushing_validation.rs` |
| Against the `edge-check` bushing contact FE (ignored, slow) | `tests/compare_edge_check.rs` |
| **The lug on the general kernel (main solver)**: `FeaLug` (mesh -> kernel, bushing + interface, `Tuning` presets, `run`), `analyze` -> `LugSolution`, sweeps, `limit_load` / `finite_collapse` -> `LimitLoad` / `FsResult`, meshed elastic pin, 3D double-shear lug | `src/fea.rs`, `src/fea_solve.rs`, `src/fea_limit.rs` |
| Kernel vs condensed, Lame press fit, thermal, second order, angle sweeps, finite strain, NACA TN 1503 on the kernel | `tests/kernel_analysis.rs`, `tests/kernel_features.rs`, `tests/kernel_naca.rs`, `tests/fea_bridge.rs` (also ignored speed probes: `kernel_tuning_sweep`, `collapse_speed_probe`, ...) |
| faer (`fea-core`) vs this crate's own sparse Cholesky, timings (ignored) | `tests/sparse_speed_gate.rs` |

## The kernel path (`src/fea.rs`, `fea_solve.rs`, `fea_limit.rs`) and its legacy twin
`FeaLug` hands the lug's own mesh (and bushing, both bodies as blocks) to `fea-core` and returns the condensed solver's result types, so the toolbox runs either. Everything the toolbox offers runs on it: bushing + interference + friction, thermal change of the fit, second order (`set_geometric_nonlinearity`: finite-strain elastic blocks), meshed elastic pin, oblique / hardening / finite-strain collapse, sweeps. Measured against the condensed solver in `docs/fea-core.md` Phase 7. Rules learnt:
- **`Tuning` lives in `fea-core/src/fit.rs`** (re-exported from `lug_solver::fea`, shared with `eccentric-bushing`) together with `interference_contacts` (the bushing/lug interface set-up `FeaLug::contacts` calls) and `start_after_fit`; change them there and rerun `kernel_analysis` / `kernel_features` (the Lame press-fit test is the guard).
- **`Tuning` presets are measured, not guessed**: soft penalty `10 E/a` (5-13x faster, same answer), `30 E/a` with friction (a soft one leaves the pressure peak ~10 % low), Newton tolerance 1e-4, one multiplier pass per collapse step. Pressure peak is the least converged output (Gauss-point scatter, 1-3 %).
- **Friction needs >= 2-3 load steps** (`analyze` uses 3): one step skips the history and lands ~8 % low on the hoop stress. The kernel is the friction reference; the condensed solver's friction is path / penalty dependent (~5-8 %) and says so in the notes.
- **The pin is positioned touching the loaded side** (`shift`); the bushed case first solves the interference fit alone (`fit_only`) and continues from it (`Start`, `fit_start`), else the first Newton step throws the pin through the fitted bore. The pin's force-free position is the centre (`bearing_deflection` = centre travel along the load, clearance included).
- **Bushing / interface**: two-pass `Reduced` contact with `with_overlap` (interference / 2) and a 6-face-length margin (the interface slides under collapse loads); the bushing is held by weak grounding springs (`Loads::ground`, `1e-9 E t`), so a frictionless interface works (the condensed solver cannot).
- **Oblique collapse rotates the lug** so the load is global `-x`, with the pin's `y` translation free and force-free. The collapse of an oblique load is mesh dependent in both solvers (45 deg: 5322 -> 4380 lbf from 24 to 120 around the bore, still falling); axial is converged.
- **Elastic pin**: full model without friction only (friction torques need the clevis); with friction on a full model `analyze` returns an error and the toolbox falls back to the legacy solver. A full ring is held by weak springs.
- A smooth-branch friction evaluation can see a tiny negative pressure: the capacity is clamped at zero (`fea-core/src/contact.rs`). Without it the tangent was NaN and frictional runs near full load failed.
- NACA TN 1503 on the kernel (`tests/kernel_naca.rs`): `(Ftu + Fty)/2` 6-25 % low, `Ftu` from 13 % low to 2 % **high**, `Ftu (1 + elong)` -2..+14 %; the legacy numbers (9-28 % low, 1-17 % low never high, -5..+10 %) stay in `validation_naca_tn1503.rs`. Both are quoted by the toolbox for the solver in use.
- `through_thickness` (beam on the lug springs) is the legacy mean-field estimate; the 3D face bearing (1.12-1.17 x mean) is a singularity that grows with refinement (`double_shear_through_thickness_load_matches_the_beam_model`, ignored). Single shear stays unoffered.
- Dependencies are optimised in dev (`[profile.dev.package."*"]` in the root `Cargo.toml`): unoptimised faer made an analysis take minutes. `target/` grows by it (watch disk).

## Contracts
- Units: inches, psi, lbf. Per-unit-thickness inside (forces are `P / t`); stresses are plane stress.
- Axes: hole centre at the origin, lug axis `+x` toward the shank, head at `-x`, far end clamped at `x = length`. `LoadCase.angle_deg`: 0 = pin pulled away from the shank (contact on the head side), 90 = `+y`, 180 = pushed toward the shank.
- Axial loads use the **half model** (`y >= 0`): the target load there is `P/2` (`LugModel::solve_for_load` doubles the half-model force); `contact_arc_deg` is symmetrised. Oblique/transverse loads need the full model (`LugModel::build(.., symmetric = false)`; `solve` rejects the mismatch).
- The condensed model depends on geometry, material (E, nu), mesh, symmetry, plane mode and bushing only; pin diameter, friction and load are per-solve. Keep it that way (the UI caches it).
- `penalty_factor` only sets Newton speed: the augmented Lagrangian removes the penetration error. Do not "fix" accuracy by raising it.

## Pitfalls
- **Ultimate capacity needs plane strain.** Plane-stress EPP crushes the bore at ~1.15 sigma_f D t whatever the head length and hides every other mode. `PlaneMode::Strain` assembles with the effective constants and the plastic model must be given the TRUE `(E, nu)` (`PlasticModel::new` checks the pairing). Elastic stresses stay plane stress: use two `LugModel`s.
- **Contact radius is not the hole radius** once there is a bushing: clearance, pin-step caps and the penalty scale use `contact_radius()` / `mesh.bore_radius` (the bushing's inner surface). Using the hole radius threw the pin out of the bushing.
- A bushing is a separate body touching the lug only through the interface: ground it weakly (1e-6 E) or the factorisation is singular. A free body (lug without a clamp, a disc) can also rotate rigidly with no resistance in a small-displacement model: validate such cases on a symmetry half-model.
- **Do not cut the multiplier passes inside the plastic loop** (tried): one AL pass per plastic iteration needed 3-5x more iterations and drifted to different collapse loads.
- The tangent solve (`solve_tangent`) keeps `G R^T` transposed so every loop is contiguous: the strided version was 1.6x slower.
- A plateau is the collapse only when the curve then falls or flattens; `LimitLoad::plateau == false` means the travel cap ended the run (a lower bound), and the toolbox says so.
- Residual merit for Newton must be the **2-norm with Armijo**; an infinity-norm decrease test stalled on active-set changes (found the hard way).
- Cap the multiplier passes (`MAX_OUTER = 3`): more only chase Gauss-point pressure oscillations the stresses cannot see (and inflate the reported peak pressure). Report peak pressure from element averages when comparing with Hertz.
- A pin with an interference fit has a **net force at the centred position** (head and shank sides differ in stiffness); the load solve first settles the pin at zero net force. Never skip it for a zero load.
- Root solve: take the slope only from points in contact (clearance gives zero load), never step below the previous step before bracketing, cap a step at `0.1 a`. A trial that throws the pin out of the plate gives load 0 and breaks the bracket.
- Narrow (loose-pin) patches need angular refinement (`auto_refinement`); 72 uniform elements underestimate a 14 degree Hertz peak by 25 %.
- Midside nodes sit at the formula midpoints; the Jacobian is checked positive at build (`Mesh::measure`).
- A free body (no clamp) is singular: only `FarEnd::Soft` (1e-6 E grounding) is used, for self-equilibrated tests such as the Lame disc.
- **Load angle (guarded by `tests/angle_sweep.rs`).** Oblique loads once failed ("contact did not stiffen"; 45 deg first). Causes and the rules that fixed them: (1) the root solve's predictor must only interpolate between bracketing points - secant extrapolation overshoots and is rejected unless the load rises monotonically; (2) at first touch the sideways pin DOF is neutral, so the pivot is regularised (`1e-5 E`) and the geometric terms `c_geo/h_geo` are kept; (3) when plain stepping fails `advance_nested` re-solves in sub-steps (never delete it: it is the last resort for 60-90 deg); (4) the plastic loop has a runaway-strain guard, safeguarded Anderson and step halving on load collapse, because the zero-stress state is a spurious fixed point of the initial-strain iteration.
- **Friction is Alart-Curnier: a tangential multiplier per contact point, carried across the augmented-Lagrangian passes** (`lam[nn..]`, increments measured from the step start). A pure-penalty stick lets a sticking point creep by `t/kt` and made the peak hoop 4-8 % low at 45-90 deg. `STICK_RATIO` (0.005) is therefore only a convergence rate (0.005-0.03 agree within 0.3 %); friction runs `MAX_OUTER + FRICTION_EXTRA_PASSES` passes. Do not "stiffen" it. A frictionless bushing interface (`friction = 0`) in an oblique full model leaves the bushing floating and is not supported.
- **The bushing interface is nodal collocation** (`NODAL_INTERFACE`, lumped edge weights 1/6, 4/6, 1/6): Gauss-point constraints on quadratic edges gave a +-60 % pressure sawtooth and false separation. The pin contact keeps Gauss points (edge-point noise only, ~1 %).
- **Bushing grounding is 1e-9 E (`fe::BUSHING_GROUND`) and leaks `k u` of force to ground**: `Verification.ground_leak` measures it (0.016 %). At 1e-6 E it leaked 1.4 % of the load. Do not raise it.
- **An elastic pin does not change a limit load** (limit analysis ignores elastic compliance; NACA 18.0 % vs 18.2 %). It changes the elastic pressure peak (3-9 % lower for steel). `LugModel` caches one `G + A` inversion per (E, nu, radius).
- **`finite.rs` is a separate solver (not the condensed one).** It has no bushing, no elastic pin and is validated for axial loads; the failure strain is a per-material constant from `mechanics_core::fracture` (`m ln(1 + A)`, validated for 2014/2024/7075 aluminium on 20 NACA points, `docs/failure-strain-library.md`; regenerate with `tools/gen_fracture.py`). The multiplier passes use loose tolerances (1e-2) before the last (1e-6); a high penalty slows it (10 E/a is best). The first steps of a run use plain Newton (the sideways pin coordinate has no stiffness until contact): do not switch the quasi-Newton on earlier. Quasi-Newton failure retries as plain Newton before the step is halved.
- **Hardening (small-strain, `limit_load_with`) is plane strain only** and uses deformation theory (yield stress follows the equivalent plastic strain of the current state): exact for the monotonic travel-controlled collapse, not for unloading. The collapse is the load at `strain_limit`; the result depends on that choice.
- **Peak von Mises and the ZZ error ignore the clamped far end** (`clamp_zone_x`, `evaluation_zone_x`): its corners are singular and are not the lug.
- **Second order (`solve_second_order`) adds `K_sigma` of the converged stress to a rebuilt condensed model**, fixed point on the peak hoop; the mesh is not moved. Tension stiffens, compression softens, a non-positive-definite tangent is reported as a buckling error. Cost = one model build per pass.
- **Single-shear pin bending is excluded** (`Shear` has only `Double`): the load and its reaction act on different planes, so the pin carries a net couple only the members' own bending can balance (Newton diverged on it).
- **The load root solve** uses one multiplier pass (`advance_coarse`) for trials more than 5 % from the target, halves the step before the nested sideways-coordinate search, and forces no step growth near the target (it overshot into a stiff, slow contact).
- The lug is symmetric about its axis: `+angle` mirrors `-angle` (NOT `180 - angle`: 0 pulls the pin out of the head, 180 pushes it into the body).
- Build is optimised even in dev (`[profile.dev.package.lug-solver]` and all dependencies): the TUI runs it on a key press.
- **Legacy solvers**: `LugModel` / `FiniteLug` are retained as a user-selectable comparison and a fallback, not extended; new features go into `FeaLug`.

## Boundaries
### Always
- Add or extend an exact-solution or consistency test for every numerical change (`tests/lug_fem.rs`).
### Never
- Add UI, material tables or file I/O here.
- Compute a result by loosening tolerances; find the cause.

## Navigation
Consumer: `app-tui/src/toolboxes/lug_analysis/AGENTS.md`. Kernels: `edge-check/AGENTS.md`. Write-up: `docs/lug-analysis.md`.
