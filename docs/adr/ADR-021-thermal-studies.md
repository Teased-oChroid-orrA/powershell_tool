# ADR-021: Thermal studies on the problem schema

Status: Accepted (2026-10-12). Builds on `fea-core` phase 12-13 (`thermal.rs`, `coupling.rs`) and ADR-012.

## Context

`fea-core` conducts heat (steady and transient, flux / convection / source, temperature-dependent conductivity) and can load the
structure with a nodal temperature field, but `fea-problem` was structural only: a uniform `delta_t` was the only thermal input,
and `advice.rs` said no thermal advice existed. The question was whether a thermal problem is a separate problem type or a
different question asked of the same geometry, mesh and names.

## Decision

A thermal problem is a **study** of the same `Problem`, not a second schema:

- `Problem::study`: `Structural` (default, every old file), `Thermal` (steady temperature only), `ThermalStress` (the temperature
  field, then the stresses it causes, together with the structural supports and loads). `Problem::thermal: ThermalSpec`
  (conductivity, volumetric source, stress-free reference temperature, fixed temperatures `HeatSupport`, heat loads `HeatLoad`:
  flux, convection, point input) is written only when it is not the default.
- Same geometry, mesh and surface names: a heat boundary condition names `left` / `hole1` / `r2/outer` like a support does.
  Thermal BCs are their own lists, so a model carries both halves and a study picks what it uses.
- One solve path (`solve_mesh`): the heat solve runs on the model before the structural one; its field minus the reference is
  `Loads::temperature`. A thermal-only study returns a `Solved` with zero displacements and stresses and a temperature, so
  every consumer of a result keeps working; `Solved::fields()` says which fields exist and `default_field()` which opens first.
- Failure guidance is the same machinery (ADR-020): `NoThermalAnchor` (nothing fixes the temperature level) with remedies
  "hold a surface at a temperature" / "cool it by convection"; unknown surfaces in thermal BCs retarget or drop.
- Native window and terminal Workbench share the row model (`FieldRow`): a Study row, a Thermal section, fixed-temperature and
  heat-load lists, the tree and the surface popup offer the thermal kinds (`BcKind::FixedTemperature / HeatFlux / Convection`).

## Consequences

- Validated against closed forms (`fea-problem/tests/thermal.rs`): linear bar between two temperatures, flux plus convection
  (`T(L) = ambient + q/h`, slope `q/k`), uniform generation (`g L^2 / 8k`), and the constrained-bar thermal stress `-E alpha dT`.
- Steady only. Transient heat, temperature-dependent conductivity and heat capacity exist in the kernel but are not exposed in the
  schema; two-way coupling does not exist anywhere. Adaptive remeshing is driven by the stress error estimate, so it is off for a
  thermal-only study. Bushings (contact) do not combine with a thermal study (refused).
- The reported heat flow is the net over all fixed temperatures; a per-surface heat rate needs a flux integral the kernel does not
  expose yet.
- `Summary::equilibrium_error` is now judged against the total magnitude of the forces rather than the net force: a thermal load
  is self-equilibrated, so its net force is zero and the old yardstick divided noise by noise.
