# Pressure line run: stress check of a supported tube (#234)

`pressure-vessel-solver/src/line_run.rs` (+ `line_beam.rs`): a long aircraft pressure line (hydraulic / fuel / pneumatic tube)
supported at `x` positions, checked with the standard failure criterion of its material class. API only: no window yet (a window
needs an owner-approved mockup; see "Proposed window" below).

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

## Segment mode (a stretch between two supports)
`line_run/segment.rs` checks only the stretch between two supports instead of the whole run.
- **End conditions**: `EndFixity::{Free, Pinned, Spring(k), Clamped}` plus an axial-anchor flag per end; `analyze_segment` adds them as
  supports at `x_start` / `x_end` (interior supports stay in `LineRun::supports`; one on a segment end is refused). Free + tip load is
  the valve case, Clamped + Free the cantilever, Clamped + Pinned the propped beam.
- **Clip stiffness**: `Support::rotation_stiffness: Option<f64>` is a rotational spring to ground (moment per radian, both planes;
  `None` / `Some(0.0)` = pinned, `rotation_fixed` = the infinite case; negative, non-finite, or both set is refused). Proved against the
  closed form `M = k w L^3/(24 EI) / (1 + k L / 2EI)` for a uniformly loaded span with equal springs, the pinned and clamped limits, and
  monotone movement of moment from midspan to the ends. The stiffness itself is an owner input: nothing here estimates it.
- **Cut-end actions from a full-run solve**: no new API. `PointLoad::moment` (`[0, M, 0]` at the left end, `[0, -M, 0]` at the right
  end for a plane-z bending moment `M`; the slope-conjugate load is `-m_y`) and `PointLoad::force` (shear, axial) at the end station.
  Tested: the middle span of a three-span run with its two cut moments on pinned ends reproduces the full-run moments and shears to 1e-10.
- **Bounds** (`analyze_segment_bounds`, two held ends): runs pinned-pinned and clamped-clamped and returns both results plus, per
  criterion, the lowest margin over cases and bounds with the governing bound. Rationale: the true end restraint of a cut segment lies
  between none and rigid, pinned-pinned gives the largest midspan moment (`wL^2/8`) and clamped-clamped the largest restrained-end
  moment, so the lower margin is the conservative one. Tested to bracket the continuous two- and three-span margins.
- **Limits of the bound**: it brackets the stress margin of uniformly loaded spans, not every action. The clamped end moment
  (`wL^2/12`) is *below* the continuous three-span support moment (`wL^2/10`), because loaded neighbours rotate the joint the other way
  (tested); reactions and support moments of the segment must not be read from the bound, and non-uniform or pattern loading is not proven
  to be bracketed. A free end has no restraint to bracket (pinned-free is a mechanism): bounds refuse it; use a `Spring` end when the clip
  stiffness is known.

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
approximation; supports are pinned in both transverse directions (optionally clamped or given a rotational spring) with no gaps or friction;
thermal restraint is uniaxial (no Poisson coupling). Ductile ultimate is checked both ways (maximum |principal| per the crate's
convention, and von Mises, owner request 2026-11-14).

## Proposed window (needs owner approval before any code)
A "Line Run" page in the native Pressure Vessel family: left, tube inputs (OD, wall, material class and allowables, FS) and a
support / load table (x, kind, anchor, force, weight, offset); right, a station plot of moment resultant and axial force over `x` with
supports marked, a margin card per criterion (governing case, `x`, surface, angle), and the reactions table. Case selector for load
cases (1g, crash factors, thermal). No window is built here.

## Segment mode in the window (2026-11-16)
The Pressure Vessel window's line-run page takes the solver's segment mode (`line_run::segment`), per the owner's pressure-line workflow
(check only the stretch between two supports: clamp and valve, two clamps, one clamp, a pin).
- **Segment card** (Inputs): "Segment between two supports" switch; per end (left `x = 0`, right `x = run length`) a Condition cycle
  Free, Pinned, Spring clip, Clamped (`FixityIn` -> `EndFixity`), the clip stiffness in lbf-in per radian for Spring (an owner input,
  0 acts as pinned, never estimated), an axial-anchor switch on held ends, and "+ Valve or fitting at this end" (a point load at the
  end station; its weight and forces are typed in the Point loads card). Defaults: both ends pinned, the left one the axial anchor (as the
  whole run's first support).
- **Supports at the end stations are superseded** while the segment is on (kept in the list, shown "Not solved", never passed to the solver);
  interior supports stay. Switching the segment off restores the whole run exactly.
- **Bound toggle** runs `analyze_segment_bounds` (both ends pinned, both clamped; the stated conditions are not used). The page shows a
  "Segment bound" table per requirement row: pinned margin, clamped margin and the worse one with its bound, each with the row's factor;
  the Segment bound card repeats the worse margins. The margin curve is the worse of the two at every station; the picture, reactions
  and equilibrium follow the bound with the lower margin (the reactions card says not to read segment reactions from a bound).
  A free end with the bound on is refused with the solver's reason, never a stale result.
- **Minimum wall** of a bounded segment is the thicker of the wall each bound needs (`LineWallJob::runs`, one run per bound via
  `segment_run`, new in `pressure-vessel-solver`: the run `analyze_segment` solves, so a wall search can hold it). An infeasible bound
  makes the row infeasible.
- **Idealisation statement** appears on the page, in the card and in the report: a cut segment's real end restraint depends on the
  neighbouring spans and clips; pinned and clamped bracket the stress margin of the continuous run for uniform loading, not reactions or every
  load pattern (see Limits of the bound above).
- **Case file format 5** stores `line_run.segment` (`enabled`, `ends` with fixity, spring and anchor, `bound`); format 4 and older cases load
  with the segment off; a file from a newer build is refused. The report lists the ends and, with the bound, each row's pinned, clamped and
  worse margins.
- **Requirement rows (verified, no change needed)**: rows are seeded by the material's class (`MaterialKind::default_for`, ductile
  or brittle by catalog elongation, composite refused), the allowables are the catalog's yield and ultimate (`effective_material`, shown in the
  Allowables card and, per row, in the Requirements card), factor 1.0 and required MS 0.0 stay neutral with the visible hint, an edited row
  sticks through a material change and only unedited rows re-seed (tests in `toolboxes/pressure_vessel/run.rs`).
