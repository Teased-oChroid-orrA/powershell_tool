# Plate Edge workbench

A native window (`app-tui --gpu-plate-edge`, dashboard tile "Plate Edge", `EditorKind::PlateEdge`, case file `plate-edge-case.json`) for one question: a pin or
fastener in a hole near a free edge of a plate, loaded toward the edge. What load does each failure mode allow, and which
governs? Advisory only: the legacy `Fbru + 0.8 p` check in the Bushing workbench stays the governing check and nothing here is
promoted to it (owner rule, `docs/edge-distance-crosscheck.md`).

## What the person gives

* **Plate material**: a library material (the MIL-HDBK-5J catalogue plus the curated table, found by typing part of an alloy,
  temper or id) or custom values (E, nu, Fty, Ftu, Fsu, Fbru and Fbry at e/D 2.0 and 1.5).
* **Fastener or pin**: a nominal size (the NAS6200-series diameters of `fastened-joint-solver`) and a source: a specification
  class (tension and double-shear ratings only), a library material (adds yield and bearing), or custom. Shear planes (1 or 2)
  and the bending span.
* **Fit**: clearance, press-fit, interference (diametral values) or a custom contact pressure. The hole is the pin diameter less
  the interference (plus the clearance).
* Edge distance, thickness, pin-hole friction, and the FE mesh mode (graded, graded + converge, fixed fine).

## What it computes, and from where

| Row | Source |
|---|---|
| Bearing onset (yield) | `Fbry(e/D) D t` from the material table, the 2 % D offset definition (`edge_check::modes`) |
| Bearing ultimate | `Fbru(e/D) D t` (`mechanics_core::materials::fbru_at_edge_ratio`) |
| Shear-out | handbook two-plane rule `2 Fsu t (e - a cos 40 deg)` less the fit's `p D t`, and the elastic stress field's mean tangent-plane shear (the `analytic` model) |
| Tear-out | mean hoop tension across the ligament = `Ftu` (elastic field, mode `Splitting`) |
| First yield, collapse | elastic field; the 2D contact FE (plane strain, advisory) when run |
| Pin shear / bearing / bending | closed forms (planes x Fsu x pi D^2/4; Fbru of the pin x D t; Fty pi D^3/32 against `M = P L/8` double shear or `P L/2` single) |

A row that cannot be computed carries the reason instead of a number (a bolt specification gives no yield or bearing strength;
a material with no tabulated `Fbry`; e/D below the 1.5 where the handbook tables stop). The lowest finite row among the
governing ones is shown as "Governing". Each failure mode is asked of the shared models one at a time
(`edge_check::runner::mode_capacities`, the same bisection `capacity()` the report uses).

FE bearing response (`lug_solver::bracket`, the driver of the 3D bracket study moved out of the test): a rigid pin driven toward
the edge in a clearance or line-to-line hole, as 2D plane strain, 2D plane stress and a 3D Hex27 solid (quarter model), the
housing elastic-perfectly-plastic at `contact_model::flow_stress`. Each gives three loads from its load-travel curve: the
onset of yielding (secant stiffness 5 % below its initial value), the 2 % D offset load, and the collapse load at 0.1 D of
travel, plus the deformed model coloured by equivalent plastic strain.

## Mesh

The default is one mesh graded about the bore (36 elements around, 2 layers over half the thickness for 3D). "Graded + converge"
runs finer meshes (18 more elements around each step, and a layer from the third mesh in 3D) until the collapse load and the
yield load each change by less than the tolerance (default 0.5 %), at most 4 meshes in 2D and 3 in 3D, and reports the table.
This is a stop-on-measured-change rule, not remeshing: `fea-core::adapt` remeshes tetrahedra, the bracket is extruded hexes, and
contact AMR is 2D only and was deliberately not exposed (`docs/hybrid-meshing-amr-readiness.md`).

## Evidence

* 2D plane strain, NACA 75S-T 1x2 bar, e/D 1.5, t/D 0.5, 36 / 54 / 72 elements around: collapse 13728 / 13645 / 13624 lbf
  (changes -0.60 % and -0.16 %); departure 10329, 2 % offset 13595 / 13545 / 13529 (first mesh to third).
  A 0.5 % tolerance stops at the third mesh (`tests/edge_3d_bracket.rs`, `the_convergence_loop_stops_when_the_load_stops_moving`).
* 3D, same case, 2 layers: 12234 lbf in 148 s at 17205 dof (the recorded 12234; layers 2/3/4 differ by 0.08 %).
* The window's own default case with a 0.002 in clearance (0.25 in pin, handbook 7075 T651 plate 0.126-0.249 in [A], e = 0.5 in, e/D 1.98,
  t = 0.125 in, t/D 0.50, friction 0.12, the plate sized as the Bushing window's contact FE sizes it): collapse 3841 lbf plane strain,
  2304 lbf plane stress, 2754 lbf in 3D, so the 2D idealisations bracket the 3D solid (end to end through the window, 104 s,
  `a_three_d_run_adds_the_solid_below_plane_strain_and_draws_it`, ignored by default).
* With the perfectly plastic housing the curve is flat by about 0.05 D, so the 2 % offset load is within about 1 % of the
  collapse load. The housing law has no hardening: the FE cannot separate bearing yield from ultimate the way the tabulated
  `Fbry` and `Fbru` do, which is why the bearing-onset row is the handbook number and the FE shows onset, yield and collapse
  of the load-travel curve.

## Limits

* The FE and 3D model a rigid pin in a clearance or line-to-line hole. A press-fit or interference pin is in the 2D contact FE of
  the allowables (the pin as a thick cylinder with interference); the 3D contact stage with interference did not converge in
  trials (`docs/edge-distance-crosscheck.md`), so it is refused with the reason, never faked.
* Plane strain runs 12 to 36 % above the 3D model at e/D 1.5 to 2.0 and plane stress below it: read the 2D numbers as a bracket.
* The fastener classes are specification ratings (tension and double shear), not allowables of any program; verify them. Pin
  bending and bearing need a pin material or typed strengths.
* The custom contact pressure is charged against the elastic modes; the contact FE has no interference in that case.
