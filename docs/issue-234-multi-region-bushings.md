# Issue #234: bushings, advice and refinement zones for every region

Backlog item "FEA Workbench: Bushings / advice / refinement zones for every region, not only the first" (2026-11-14).

## Scope (what the code already did)
- Refinement zones (`MeshSpec::refine`, `advice::refinement_regions`, `build::size_function`) and the advice rules already saw every region of a sketch (`advice::sketch_regions`); hole zones were labelled per region but a pin load or bushing marked only the first region's holes "loaded".
- Pin loads (`Load::Bearing { hole }`) and bushings (`Bushing::hole`) indexed `Geometry::Sketch::holes` only (region 1). `Problem::validate_geometry` refused any bushing in a multi-region sketch. `fit_setup` assumed the bushing blocks directly follow the plate block (block 0).

## Design
- **Global hole numbering** (no schema change, old files unchanged): hole `n` counts through region 1's holes, then region 2's, and so on. `Problem::hole_at(n)` gives shape, region and mesh surface (`hole{n}` for region 1, `r{k}/hole{m}` after); `Problem::hole_number(surface)` inverts it. `Load::Bearing::hole` and `Bushing::hole` use this number. One region: identical to before.
- The bushing machinery (`build::add_bushings`, `solve::fit_setup`, the bearing load, `interface_result`, the mesh estimate) resolves holes through `hole_at`. The plate's hole surface is kept as `plate_<surface>` and `<surface>` is re-pointed at the bore, as before. Bushing blocks are the *last* blocks of the mesh.
- Advice (`add_boundary` Bearing, `edge_of_load`, refinement zones' `loaded` flag), conversion (load kind to pin load) and diagnose use the same helpers.
- Not changed: contact pairs with bushings (still refused; the contact + plasticity work in `fea-core`/`fea-problem` is another session's), `Edit::RemoveHole`/`WidenHole` (edit region 1 only; removing a region-1 hole renumbers the later holes, which the existing bushing / load renumbering already handles).
- Regions are separate bodies unless `mesh.bond_regions`; a bushed region needs its own support (or bonding), exactly like any other region.
- Out of scope, stated: "different materials / thicknesses along the bore" is not representable in the plane model (a bushing is one annulus, one material; plane thickness is per problem). It would need a stacked 3D bushing; not started.

## Evidence
`fea-problem/tests/multi_region_bushing.rs`: numbering round trip; a bushed lug as region 2 against the same lug alone (mean fit pressure, torque capacity, normal force, open arc equal to 1e-4; reaction = pin load); Lame fit pressure in a region-2 bushing (4 %); resultant and equilibrium (1e-8) of a pin load on a later region's hole; validation messages; advice and refinement zones per region.
