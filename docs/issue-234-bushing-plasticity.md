# Issue #234: plasticity with bushings (2026-11-14)

`Problem::validate_plasticity` no longer refuses bushings. What the lift needed (more than removing the refusal):
- `attach_plasticity` gave every block the *plate region's* law; the bushing blocks (the last `bushings.len()` blocks) now take the `Bushing::material` law (`plastic`, `yield_stress`, `hardening` of its `MaterialSpec`), validated like a region's.
- The fit solve used one load step; a plastic fit is path dependent, so a plastic problem uses `PLASTIC_STEPS` (8) for the fit and for the loaded stage (4 before). The loaded stage continues from the fit with `start_from`, which carries the Gauss-point state.
- `solve_mesh_contact`'s plastic early return dropped `Summary::interfaces`; it now fills them.
- A bushed plastic problem adapts through the generic `refine` loop (`solve_plastic_adaptive` has no contact).
- Correction to an earlier note: the kernel *does* have a proof for interference-overlap fits that yield (`fea-core/tests/contact_plastic.rs`, Hill); there was no test through the problem layer.

## Evidence (release, M1, `tests/bushing_plasticity.rs`, `--ignored`)
Plane strain, round plate `a = 1 .. b = 2`, E = 10e6, nu = 0.3, bore 0.1, diametral interference 0.004 unless stated.
- Yield stress 1e12: plastic path 8179.83 against Lame 8179.79 (5e-6); the elastic fit path reads 8177.56 (-2.8e-4, inside its 2e-3 contact tolerance, `Tuning::friction`), no plastic strain.
- Plate yield chosen so the elastic Lame pressure is twice the first-yield one: plastic radius c = 1.502, p = 6504 against Hill 6816 at c (inside the one-element band) and 20 % under the elastic 8180.
- Fully plastic (diametral 0.1, sigma_y 1e4): p = 7997 against `Y' ln(b/a)` = 8004 (0.1 %).
- Soft bushing (yield 2000) in an elastic plate: bushing peak plastic strain 7.5e-2, plate exactly 0; pressure below the elastic fit's.
- Cost: one plastic bushing solve is 70 to 215 s in release (8 steps of the fit, 8 of the loaded stage), against about 10 s elastic. Never automatic; the solving tests are `#[ignore]`d. Not measured: 3D solids with plastic bushings (27 s elastic fit; expect several times that).

## Step-count sweep (2026-11-15): fewer steps is not a lever

`PLASTIC_STEPS` (the cap `dl_max = 1/steps` of the kernel's own step control, which already cuts on a Newton failure and grows 1.5x on a low Newton effort) was swept over 8 / 6 / 4 on the four solving tests (release, one run each, run one after another; M-series Mac). Accuracy is flat; time is not monotonic.

| test | 8 steps | 6 steps | 4 steps |
|------|---------|---------|---------|
| Hill plate yield: p (Hill 6816 at c = 1.502) | 6504.2, 88 s | 6504.3, 71 s | 6504.6, 24 s |
| Fully plastic: p (limit 8004) | 7997.0, 164 s | 7997.0, 120 s | 7997.3, 81 s |
| Lame, yield 1e12: p (Lame 8179.79) | 8179.83, 82 s | 8179.71, 27 s | 8179.38, 37 s |
| Soft bushing, time only | 250 s | 120 s | 158 s |

Reading: every proof holds at 4 steps (the Lame check drifts 8179.83 -> 8179.38, still inside its 1e-4 bound), so 8 is more than these cases need; but the time does not follow the step count (4 steps is slower than 6 on two tests: the cap only bounds the step, the contact passes and cuts decide the cost). The number was therefore not lowered: these four cases are axisymmetric and mild, and the sweep gives no evidence for a harder case (pin load through a plastic fit, 3D). A real saving needs a step control driven by the plastic increment per step (`StepInfo.max_ep`, `plastic_fraction`) rather than by a fixed count; not built.
