# Issue #11 Phase 3: materials audit for failure-mode data needs

Per `docs/issue-11-status.md`'s plan, this phase checks whether
`mechanics_core::materials::Material`'s existing fields (`e_ksi`,
`sy_ksi`, `fbru_ksi`, `fsu_ksi`, `ftu_ksi`, `nu`, `alpha_u_f`) cover what
v1's four failure modes need, before Phase 4 writes any failure-mode code.
**Confirmed sufficient - no new fields added.**

| Failure mode | Data needed | Existing field | Notes |
|---|---|---|---|
| Yield | Yield strength | `sy_ksi` | Direct. |
| Von Mises | Yield strength | `sy_ksi` | The equivalent stress itself is computed from the stress tensor components already available from `pressure_vessel_solver::stress::StressState` - no material property needed to *compute* it, only to judge it against. |
| Tresca (max shear) | Yield strength | `sy_ksi` | Tresca's criterion is standardly stated in terms of the *tensile* yield strength directly (`sigma_max - sigma_min >= sigma_y`, from the uniaxial-tension derivation `tau_yield = sigma_y / 2`) - no separate shear-yield field is needed, and none exists in `Material` to be confused with. |
| Ultimate | Ultimate tensile strength | `ftu_ksi` | Direct - and already a genuinely separate field from `sy_ksi`, so a yield-vs-ultimate mixup (the epic explicitly warns against exactly this: "The tool must distinguish: Yield criterion / Ultimate criterion. These are not interchangeable") isn't possible by construction. |

`e_ksi`/`nu` are consumed by `mechanics_core::lame` to compute the stress
state itself (already wired in Phase 2), not by the margin-of-safety
comparisons Phase 4 adds. `fbru_ksi`/`fsu_ksi` (bearing/shear ultimate)
and `alpha_u_f` (thermal expansion) have no v1 consumer - `fbru_ksi`/
`fsu_ksi` are bearing/shear-specific properties this crate's v1 failure
modes don't need (they're used by `bushing-solver`'s own edge-distance
checks, a different problem); `alpha_u_f` is relevant only to the
explicitly-deferred thermal-stress failure mode.

**No code changes this phase.**
