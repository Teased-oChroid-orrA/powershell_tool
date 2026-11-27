# Issue 383, Phase 3: bounded public comparison assembled

2026-11-27. The [Zhang et al. 2026 article](https://www.mdpi.com/1996-1944/19/14/3070) is the closest public direct-tapped series found in this search. Its hand-tapped machine-screw groups share one reported 6063-T6 rectangular tube, test method, and fixture. Five machine-screw rows from published Table 1 were transcribed, checked against the rendered PDF (p. 5), converted with exactly 25.4 mm/in and 4.4482216152605 N/lbf, and added to the editable inch/lbf Evidence tables page. The published converted values remain separate from user edits. Its chart plots mean pull-out load. Self-drilling screw rows were excluded because they are not cut-tapped holes.

## Candidate comparison, not calibrated capacity

The two MS-D5 groups have published peak load, peak travel, and secant stiffness consistent to displayed rounding. They offer a bounded **group-level observation** for future 3D thin-wall comparison. Each row averages three tests. Individual specimen profiles, engaged length, raw force–displacement traces, coupon IDs, and independently calibrated fracture properties are unavailable. The article's material curves are inputs to its numerical model; it does not establish same-heat coupon provenance. Its strain at modeled screw ultimate strength is not independent fracture calibration. Neither these curves nor curves from another heat are adopted as measured material data.

Source conflicts affect the other groups and the fixture:

- Text reports a 2.6 mm tube wall; the published Figure 2 schematic labels it 2.5 mm. Treat wall thickness as unresolved, not as a measured 2.6 mm value.
- For MS-D6P1.3, Table 1 gives 6.69 kN, 2.48 mm and 5155 N/mm. The first two imply about 2698 N/mm, a factor of 1.91 difference. MS-D6P1.1 gives 6.32 kN, 1.32 mm and 5247 N/mm; the first two imply about 4788 N/mm, a factor of 1.10 difference. Source entries were converted without correcting them, and the conflicts are flagged.
- Table 1 calls the final self-drilling row SS-D4P1.3; its plotted legend and Table 2 call it SS-D4P1.4. That row is excluded from the direct-tapped subset regardless.

The thin tube wall bends and tears at the hole. The present axisymmetric ring model omits that global 3D deformation, so a numerical match to these peaks would not validate its pull-out capacity. The selected group averages remain secondary comparison data. Item 1's independent matched-specimen gate stays open; the Workbench's analytical capacity remains governing.

## Companion-source screen and qualification

[Grimsmo et al. 2016](https://doi.org/10.1016/j.jcsr.2016.06.023) reports M16 bolt-and-nut tension tests, bolt material tests, and FE failure comparisons. Its nut is not the tapped parent in Zhang's tube, and its material condition cannot be assigned to Zhang's specimens. The [SIMLab 2009 annual report](https://www.ntnu.edu/documents/139845/32416528/SIMLab%2BAnnual%2Breport%2B2009.pdf/20c07ac2-2b4d-4b5d-85ea-2c754a3b2a52) describes M5 fastener thread stripping and necking inside a tapped hole at high strain rate, but does not provide a matched specimen-level geometry, material curve, and pull-out record. Neither source closes Item 1's physical admission gate.

Commit `758ea49` passed the focused `thread_evidence::tests` Rust test, 40 Table 1 unit conversions, JavaScript syntax check, and browser edit/chart/save/reload/reset check. [Windows run 46065884952](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/46065884952) passed release build, package, and bare-dashboard smoke check. The Windows run did not inspect the Thread Workbench window or validate pull-out physics.
