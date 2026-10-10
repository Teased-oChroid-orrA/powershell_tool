# Issue #12 — Phase 16 aerospace mechanics scope

2026-10-10 UTC. The user explicitly chose **aircraft frame and truss members**.
Foundation verification precedes further aerospace formulations; no material tables or certification
claims belong in fea-core. The first scope is linear member load paths, deformation and section
actions, supported by the Phase 14 formulations. This scope is defined before adding any further
mechanics. No additional empirical aerospace formulation is needed for these capabilities.

Independent benchmark definitions (`tests/aircraft_members.rs`):

- Symmetric three-member spatial truss supporting a vertical apex load. Length
  `l=sqrt(r²+h²)`, apex deflection `-P l³/(3 E A h²)`, member compression
  `-P l/(3 h)`, support resultant `P`, and `2U=P |u|`. This checks load sharing,
  axial stress, equilibrium and work through geometry different from a single bar.
- A representative longeron under combined axial, transverse and torsional end loads.
  Transverse deformation is `P L³/(3 E Iz)+P L/(G As)`, twist `T L/(G J)`,
  axial stress `N/A`, and root moment `P L`. Section inputs are explicit.

These are representative independent benchmarks, not actual aircraft geometry/load/allowables
or a certified aircraft design. Local/overall buckling, open-section warping, joints/releases,
composite layups, fatigue/damage, plasticity and aeroelastic dynamics need their own actual-use-case
inputs, scope and independent references before implementation. They are not claimed by this phase.
User-selected family is implemented within the verified small-displacement member scope.
