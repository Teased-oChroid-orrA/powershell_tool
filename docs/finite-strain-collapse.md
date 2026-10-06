# Finite-strain collapse (`lug-solver/src/finite.rs`)

Status: shipped for axial loads without a bushing (toolbox: Ultimate Model = Finite strain). Full-model (oblique) runs
exist in the solver but are slower and travel-cap limited; the toolbox does not offer them.

## Why

A perfectly plastic collapse is proportional to its flow stress, so the NACA TN 1503 under-prediction was the rule, not the
solver (`docs/lug-analysis.md`, "Flow-stress rule"). A finite-element analysis gets it automatically from three things the
condensed small-strain solver cannot have: a true stress-strain curve, finite-strain kinematics, and a failure criterion.

## Method

* **Kinematics and mesh.** Total-Lagrangian Q9, the crate's star mesh. Plane strain by construction (`F_zz = 1`, the plastic
  `zz` strain is in the state), true `E`, `nu`.
* **Material.** Multiplicative J2 plasticity in the logarithmic-strain (Hencky) form: state = inverse plastic right
  Cauchy-Green tensor + equivalent plastic strain; return map = the small-strain radial return on the principal elastic log
  strains; the piecewise-linear true yield curve is solved exactly (`Hardening::plastic_increment`). The curve
  (`Hardening::true_curve`) is Ramberg-Osgood through Fty and Ftu to the uniform elongation, converted to true stress and
  strain, then Swift `sigma_u (eps/n)^n`, `n = ln(1 + e_u)`, past the necking point (value and slope continuous).
* **Tangent.** `dP/dF` by forward differences of the point update (4 extra updates per Gauss point, threaded over elements).
* **Contact.** Rigid analytic pin at its current position, Gauss points on the bore edges, penalty (`10 E/a`, only a convergence
  rate) + augmented-Lagrangian normal multiplier + Coulomb friction (tangential multiplier, incremental slip), geometric term
  of the contact tangent, sideways pin coordinate as a bordered scalar (full model).
* **Solver.** Direct nonlinear solve on the whole mesh with a **supernodal sparse Cholesky** (`sparse.rs`: geometric nested
  dissection, fundamental supernodes factored as dense blocks with contiguous kernels, large updates threaded; 4x fewer flops
  than the band, 1.4e8 vs 6e8 madds on the full ring, and ~2.7 GFLOP/s) and
  **L-BFGS** correction of a rarely refreshed factor: about two factorisations per pin-travel step. Secant predictor,
  trust region on the sideways coordinate, plain Newton while the contact establishes itself and as the fallback.
* **Collapse.** The load where the equivalent plastic strain anywhere reaches the failure strain (interpolated), or the peak
  of the load-travel curve, whichever comes first.

## Validation (`lug-solver/tests/finite_strain.rs`)

| Check | Result |
|---|---|
| Material point: objectivity (rigid rotation), ideal-plastic simple shear -> `sigma_y/sqrt 3`, tangent symmetric and equal to the elastic modulus below yield | unit tests in `finite.rs` |
| EPP collapse vs the small-strain solver, default lug | 9842 vs 9485 lbf (+3.8 %, the finite-strain geometry) |
| Mesh convergence (NACA 24S e/D 2) | 24 / 32 / 64 around: within 5 % / 2 % of the fine mesh (0.2 % between 32 and 96) |
| NACA TN 1503, true curve from Table I elongations, one failure strain per alloy (0.11 / 0.28 / 0.38) fitted to the same tests | all 12 points -1.7 to +2.5 % (mean 1.7 %), both edge distances and sizes |
| The inverse calibration (failure strain that reproduces each test) | 75S 0.10-0.14, 14S 0.23-0.33, 24S 0.33-0.43: a per-material constant, independent of e/D and size |
| Common failure strain 0.10 | never more than +2 % high, 0 to -20 % |
| Speed | default lug 0.8 s, NACA cases about 1 s; band -> sparse -> L-BFGS -> supernodal took the benchmark from 10.5 s to 0.8 s. One factorisation: half model 29 -> 15 ms, full ring 107 -> 37 ms |

The failure strains were fitted to the same twelve tests; what the table shows is that **one number per alloy explains both
edge distances and both sizes**, which no flow-stress rule does. Treat them as material constants to be confirmed by tests of
your own material.

## Not done / limits

* **Oblique loads.** The full model works (45 degrees: 13 s at 24-32 elements around) but the load keeps rising with pin
  travel until the cap, is mesh sensitive (10 %), and is a lower bound; the toolbox falls back to the small-strain collapse.
* **Bushing, elastic pin, thermal fit stresses** are not in the finite-strain model (the pin diameter includes the thermal change).
* **Failure** is a local equivalent-plastic-strain criterion (mesh-converged here, 0.2 %); no damage evolution, no triaxiality
  dependence, no crack growth.
* **Oblique loads** are 17 s now (was 34 s); the remaining cost is the number of factorisations (about 240), not their speed.
* **Failure strain** per material: `docs/failure-strain-library.md` (rule, evidence, what is and is not validated).
