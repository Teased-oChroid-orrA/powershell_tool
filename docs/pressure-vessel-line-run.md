# Pressure line run: stress check of a supported tube (#234)

`pressure-vessel-solver/src/line_run.rs` (+ `line_beam.rs`): a long aircraft pressure line (hydraulic / fuel / pneumatic tube)
supported at `x` positions, checked with the standard failure criterion of its material class. The native **Line Run** window
(dashboard tile, `--gpu-line-run`) drives it; see "Window" below.

## Pipeline
1. **Member actions per load case.** Two orthogonal bending planes (y, z) plus an axial bar on one station list (support, load and
   end points, each segment subdivided; `LineRun::subdivisions`). Hermite-cubic beam and linear bar elements are nodally exact for point
   loads at nodes and element-wise uniform loads; in-element moment, shear and axial force follow from statics, and the interior moment
   extremum of each element is added as its own station, so peak moments are exact for any station count. Banded Cholesky; a
   non-positive pivot (a mechanism) is refused as `Unstable`. Each case self-checks backward error and force / moment balance
   (`BALANCE_LIMIT = 1e-9`, relative) and returns `Rejected` otherwise.
2. **Loads.** Distributed weight (`weight_density * A` + contents + insulation, per length) times the case load-factor vector
   (`F = W n`; one g with z up is `n = [0, 0, -1]`); point loads (`force`, lumped `weight` scaled by `n`, explicit `moment`, and an
   `offset` arm giving `offset x F`); flow thrust at bends / tees from `flow_thrust`; thermal growth `alpha dT` of the bar between
   `axial_anchor` supports (free expansion with one anchor is stress-free; two anchors give `-E alpha dT` exactly); internal / external
   pressure through the existing exact Lame state of `stress.rs` (`pressure_end_load = true` is `EndCondition::Closed`: `PD/4t`
   axial; `false` is `Open`, for lines whose thrust is applied explicitly at the bends: never both for the same thrust: a `PointLoad` with `pressure_thrust: true` (a `flow_thrust` result) in a case with `pressure_end_load = true` is refused).
3. **Stress tensor** at every station, on the inner and outer surface, around the circumference: `sigma_x` = Lame axial +
   `N/A` - `(M_y y + M_z z)/I`; hoop and radial = exact Lame; `tau_x,theta = (-V_y sin t + V_z cos t) / (pi r_m t)` (thin-wall
   shear flow; integrates back to `V` and peaks at `2V/A` at the neutral axis, both tested). The radial normal stress is a principal
   stress on the surfaces; the `x`-`theta` pair is reduced by its Mohr circle. The worst angle per criterion is a 1-degree scan
   followed by golden-section refinement, so the maximum is not limited by the scan grid.
4. **Criteria by material class** (allowables and factors of safety are inputs; nothing is defaulted):
   - `Ductile`: von Mises and Tresca vs yield, maximum |principal| vs ultimate and von Mises vs ultimate (both reported; the governing one wins) (the crate's existing `failure.rs` functions, run on the
     principal triple carried in `StressState`); `MS = F / (FS * demand) - 1`.
   - `Brittle`: Rankine (tension against `Ftu`, compression against `Fcu`) and Coulomb-Mohr (`s1/Ft - s3/Fc = 1` when the
     principals straddle zero, `s1 = Ft` when both are tensile, `s3 = -Fc` when both are compressive); both linear in the stresses,
     so `MS = 1/u - 1` with `u` the (FS-scaled) utilization.
   - `Composite` / `Other`: refused with the reason (composites need ply-level criteria and layup data).
   Each margin carries the governing location (case, `x`, surface, `theta`), the stress components there, applied, allowable, FS.

## Why not the `fea-problem` frame solver
`fea-problem::members` fits a tube (`SectionSpec` takes `A`, `I`), but it is a 3-D solver with nodal loads and no station-wise moment
envelope, and depending on `fea-core` would turn this pure calculation crate into a kernel consumer. The tube run needs two 1-D
solves, so `line_beam.rs` is a few hundred lines with exact interior actions. `tests/members_cross_check.rs` (dev-dependency only)
solves a three-span line with an overhang, loads in both planes, interior point loads and an anchor on both codes and requires the
reactions to agree to 1e-9.

## Proof (`cargo test -p pressure-vessel-solver`)
Simply supported, fixed-fixed, cantilever and point-load closed forms; two- and three-span continuous beams against the three-moment
equation (reactions `1.25 / 0.375 wL` and `0.4 / 1.1 wL`, support moment `wL^2/8` and `wL^2/10`, end-span peak `0.08 wL^2` at `0.4 L` as
an interior extremum); reaction sums equal the total load with weights, g-factors, valve offsets, an overhang and point loads in all
three directions; pure couple reactions `M0/L`; offset equals the explicit moment; exact Lame state and the `PD/2t`, `PD/4t` limits
(tolerance `1.5 t/r_m`, the order of the membrane approximation, stated); restrained thermal stress `E alpha dT` and its reduction of
von Mises and Tresca to the uniaxial stress; free growth stress-free; bending margin and worst angle against `M c / I`; hand-computed
multiaxial von Mises / Tresca / Coulomb-Mohr values; bend, tee and straight-pipe thrust; refusals (composite, other, torque,
mechanism, missing anchor, bad allowables).

## Sources (what is and is not verified)
- Euler-Bernoulli beam theory and the three-moment equation (Clapeyron); Roark's *Formulas for Stress and Strain* 7th ed. beam tables
  contain the same closed forms. **The specific table and case numbers were not checked against the supplied PDF** (not downloaded);
  the tests assert the textbook results, not table entries.
- Von Mises, Tresca, Rankine, Coulomb-Mohr: standard (e.g. Budynas & Nisbett, *Shigley's Mechanical Engineering Design*, failure
  theories chapter); cited from memory, edition and page not checked. Von Mises / Tresca / max-principal implementations are the
  crate's own `failure.rs`.
- Thin-wall tube shear `tau_max = 2V/A`: classical mechanics of materials. Flow thrust: steady control-volume momentum balance
  (fluid mechanics texts); sign convention derived in the `flow_thrust` doc and tested on a bend, a tee and a straight pipe.
- Material allowables and factors of safety: owner input (MMPDS or the spec). None are stored or assumed.

## Limits (stated, not guessed)
Torque about the line axis is refused (needs a torsional-restraint model); no fatigue, vibration, buckling, bend / tee / clamp
stress intensification, partial (spring) axial anchors, or through-wall thermal gradient; thin-wall shear on thick tubes is an
approximation; supports are pinned in both transverse directions (optionally clamped in rotation) with no gaps or friction;
thermal restraint is uniaxial (no Poisson coupling). Ductile ultimate is checked both ways (maximum |principal| per the crate's
convention, and von Mises, owner request 2026-11-14).

## Window (shipped 2026-11-15; owner approved the mockup, a new dashboard tile, with export and unit switching)
`app-tui` `--gpu-line-run [case.json]`: model `app-tui/src/toolboxes/line_run/`, window `app-tui/src/gpu_viewer/editor/line_run/`
(each has an `AGENTS.md`). Inspector cards: Tube, Material (class Ductile / Brittle; E, alpha, density, Fty / Ftu / Fcu and the
factors of safety are owner inputs, shown "not set" and never defaulted: a run without them says which is missing), Supports (x,
axial anchor, clamp), one card per Point load (x, weight, force, offset, pressure-thrust flag, the case it acts in or every case),
Load cases (selector rows, name, pressures, temperature change, load factors, end-load flag). Results: verdict with the
equilibrium residual, a Stat and a utilization bar per criterion of the selected case (governing tagged, with its location),
two station plots (moment resultant; axial force dashed) with support and governing marks, the reactions. The viewport draws
the line along the vertical axis coloured by bending moment (axial force as the second field), a collar at each support; the
tube radius is enlarged for the picture only. Export writes `<case>.report.txt` and `<case>.stations.csv`; Units toggles
in, lbf, psi, degF / mm, N, MPa, degC (display only: values are stored and solved in the first set).
Deviations from the mockup, forced by the shared window chrome: moment and axial force are two plots (one axis, two units;
`Plot` has no fill) and the case pills are selector rows. Proof: `cargo test -p app-tui --lib line_run` (closed-form midspan
moment `wL^2/8` through the typed inputs, SI entry equals US solve, case-scoped loads, brittle class, refusals, file and export
round trip); GPU snapshot `line_run_snapshot` (`FEA_SNAPSHOT_LINE_RUN_OUT`). Not verified: real-mouse use on a desktop and Windows.
Not exposed: point-load moments, subdivisions, composite / other classes.
