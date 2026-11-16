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
- Cost: see "Plastic-increment step control (adopted)" below. Originally one plastic bushing solve was 70 to 215 s in release (8 fixed steps of the fit, 8 of the loaded stage), against about 10 s elastic; the solving tests stay `#[ignore]`d.

## Step-count sweep (2026-11-15): fewer steps is not a lever

`PLASTIC_STEPS` (the cap `dl_max = 1/steps` of the kernel's own step control, which already cuts on a Newton failure and grows 1.5x on a low Newton effort) was swept over 8 / 6 / 4 on the four solving tests (release, one run each, run one after another; M-series Mac). Accuracy is flat; time is not monotonic.

| test | 8 steps | 6 steps | 4 steps |
|------|---------|---------|---------|
| Hill plate yield: p (Hill 6816 at c = 1.502) | 6504.2, 88 s | 6504.3, 71 s | 6504.6, 24 s |
| Fully plastic: p (limit 8004) | 7997.0, 164 s | 7997.0, 120 s | 7997.3, 81 s |
| Lame, yield 1e12: p (Lame 8179.79) | 8179.83, 82 s | 8179.71, 27 s | 8179.38, 37 s |
| Soft bushing, time only | 250 s | 120 s | 158 s |

Reading: every proof holds at 4 steps (the Lame check drifts 8179.83 -> 8179.38, still inside its 1e-4 bound), so 8 is more than these cases need; but the time does not follow the step count (4 steps is slower than 6 on two tests: the cap only bounds the step, the contact passes and cuts decide the cost). The number was therefore not lowered: these four cases are axisymmetric and mild, and the sweep gives no evidence for a harder case (pin load through a plastic fit, 3D). A real saving needs a step control driven by the plastic increment per step (`StepInfo.max_ep`, `plastic_fraction`) rather than by a fixed count; not built.

### Plastic-increment step control (tried 2026-11-15, not adopted)

Built as an opt-in `NlOptions::ep_step` in `run_load_control`: a step that raised the peak equivalent plastic strain by more than `ep_step` was rejected and retaken shorter (down to 1/64 of the cap `1 / steps`), and the step did not grow while the increment was above half of it; `steps = 2` was only the cap. Measured on the Hill yielding fit of `fea-core/tests/contact_plastic.rs` (release, one run each):

| run | pressure | steps | time |
|-----|----------|-------|------|
| fixed 8 steps | 13093.0 | 8 | 18.1 s |
| `ep_step` 2e-3 | 13092.4 | 12 | 47.8 s |
| `ep_step` 5e-4 | 13092.4 | 12 | 23.2 s |

Same answer (5e-5), but more steps and no time saved: a fit yields from the first load step, so the controller finds no elastic stretch to cross in one long step, and each rejected step pays a full contact solve. The code was removed. Together with the sweep above, step count is not where the cost sits; the cost is contact passes and factorisations per step (`docs/fea-core.md` Phase 3 and the contact notes). A real saving must reduce factorisations per step, not steps.

### Plastic-increment step control (adopted for plastic contact, 2026-11-15)

The earlier plane-case trial (above) judged it on a fit that yields from step one. Profiling then showed where plasticity really costs: the 3D solid (2 in x 2 in x 2 layers, 0.25 in hole, fit pressure 16.1 ksi), fit only:

| plastic path, yield 1e12 (never yields) | time | factorisations | p |
|-----------------------------------------|------|----------------|---|
| 8 fixed steps | 1258 s | 293 | 16116 |
| 4 fixed steps | 308 s | 152 | 16117 |
| 2 fixed steps | 103 s | 72 | 16117 |
| `ep_step` 1e-3, cap 1 step | 62 s | 24 | 16116 |
| elastic fit (no plastic law) | 46 s | 21 | 16116 |

So a model that is flagged plastic but never yields paid 27x the elastic cost for nothing. `NlOptions::ep_step` (off by default) fixes that: `steps` becomes only the cap of the load step; a step that raises the peak equivalent plastic strain by more than `ep_step` is rejected and retaken shorter (floor 1/64 of the cap), and the step does not grow while the increment is above half of it. `fea-problem` sets cap 4 and `ep_step` 1e-3 (about one steel yield strain) on its contact solves (`CONTACT_PLASTIC_STEPS`, `CONTACT_EP_STEP`); the non-contact plastic path keeps 8 fixed steps.

Plane tests (release, single runs, fixed 8 -> cap 4 + `ep_step`): Lame elastic-limit 82 -> 9 s, soft bushing 250 -> 99 s, fully plastic 164 -> 72 s, Hill plate yield 88 -> 116 s (slower: the rejected steps at first yield cost a contact solve each). Every proof pressure is unchanged (Hill 6504.2, fully plastic 7997.0, Lame within 5e-5). All six `bushing_plasticity` tests run in 197 s together. A cap of 1 step fails the tight Lame bound (the elastic fit's own contact tolerance), so the cap is 4.

Profiling also showed where a plane plastic solve's time goes: GMRES refinement of the consistent friction tangent (`refine_consistent`: 224 calls, 2325 iterations in the Hill fit) is 46 s of 82 s, more than the 106 factorisations (18 s). A looser GMRES tolerance was not tried (that code carries the friction-convergence history in `fea-core/AGENTS.md`).

**Open:** the same 3D fit with yield 20000 (the plate does yield) ends in `NoConvergence("no convergence in 25 iterations (residual 5.468e2)")` after 325 factorisations at 8 steps, 4 steps and under `ep_step` alike, so it is not a step-size problem; not diagnosed (a perfectly plastic limit state, or 3D plastic contact convergence).
