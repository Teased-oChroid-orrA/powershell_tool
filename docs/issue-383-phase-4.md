# Issue 383, Phase 4: public specimen search disposition

2026-11-27. Item 1 is closed under its **no complete public match** fallback. This is a search disposition, not physical qualification. The tapped-hole contact pilot remains barred from changing runtime pull-out capacity.

## Digitized Zhang force–displacement plot

The four experimental machine-screw traces in [Zhang et al. 2026, Figure 5(a)](https://www.mdpi.com/1996-1944/19/14/3070) are visually digitized as 65 marker read-offs in `app-tui/src/gpu_viewer/editor/thread_evidence/zhang-figure5.json`. Calibration used plot pixel x=224..1667 for 0..3 mm and y=1245..47 for 0..8 kN, then 25.4 mm/in and 4.4482216152605 N/lbf. Expected visual read-off uncertainty is about ±0.001 in and ±25 lbf. The editable Evidence page plots the curves, shows each immutable source read-off beside its working value, saves edits in app data, and exports both values to CSV. Green numerical-model circles and Figure 5(b)'s self-drilling tests are excluded. Figure 5(a) presents representative plotted curves, not released raw records tied to individual Table 1 specimens; plotted peaks need not match Table 1 group means. The Table 1 values and source conflicts remain documented in Phase 3.

## Public-source admission screen

| Source | Direct-tapped pull-out evidence | Missing admission fields |
|---|---|---|
| [NASA TM-86539, Table VI](https://ntrs.nasa.gov/api/citations/19860012479/downloads/19860012479.pdf) | Six 1/4-28 UNJF pull-out loads and prior-condition notes | Per-hole engagement, measured profile, wall and fixture geometry, force–displacement histories, same-condition material and fracture curves |
| [Tipton and Daily, Tulsa tests](https://www.machinedesign.com/news/article/21829573/internal-thread-strip-out-tests-yield-surprising-results) | Formed 5/8-18 tap-hole diameter and rounded load trend | Specimen-level engagement/profile, measured material and fracture curves, full traces |
| [Wu et al. 2021](https://pure.ulster.ac.uk/files/90086877/Authors_copy_Thin_Walled_Structures_Wu_et_al.pdf) | Conventionally drilled/cut tapped control in a broader comparison | Matched specimen profiles, same-condition curves, independent fracture calibration and raw pull-out histories |
| [Zhang et al. 2026](https://www.mdpi.com/1996-1944/19/14/3070) | Five hand-tapped machine-screw group means and four approximate plotted traces in a thin 6063-T6 tube | Individual thread profiles and engagement, raw specimen traces, same-heat coupon provenance, independent fracture calibration; reported wall and stiffness conflict |
| [Bull et al. 2015](https://eprints.whiterose.ac.uk/id/eprint/89369/1/2_pdfsam_Tensile.pdf) | Bolt-and-nut assembly curves and material tests | No tapped parent; cannot calibrate direct-tapped capacity |
| [Ullah et al. 2020](https://doi.org/10.1007/s00170-020-05243-8) | Cut-tapped M5/M8 in 5 mm printed AlSi10Mg plate and wrought 6082 comparison; fixture and stripping plots | Same-build stress–plastic-strain and fracture curves, raw force–displacement histories, specimen-level numerical thread profiles |
| [Swissi et al. 2019](https://doi.org/10.1007/s00170-019-04519-y) | Two-turn cut/form tapped thread tests describe visual profiles and load–displacement | Full public article/data not located; no matched material and fracture record established |
| [Unbrako Engineering Guide, pp. 64–67](https://unbrako.com/images/downloads/engguide.pdf) | Typical tapped-hole stripping charts, thread-depth and surrounding-metal conditions | Specimen-level profiles/loads/traces and same-heat material/fracture curves |
| [Grimsmo et al. 2016](https://doi.org/10.1016/j.jcsr.2016.06.023) | Bolt-and-nut tension and bolt material tests | Nut is not the tapped parent; its material cannot be assigned to another source's specimens |
| [SIMLab 2009 report](https://www.ntnu.edu/documents/139845/32416528/SIMLab%2BAnnual%2Breport%2B2009.pdf/20c07ac2-2b4d-4b5d-85ea-2c754a3b2a52) | High-rate M5 tapped thread stripping | Matched specimen geometry, material/fracture curves, and numeric pull-out histories |

Zenodo, Figshare, Dataverse, and ETH repository searches found no linked dataset that fills the remaining fields. Sources are retained as separate comparisons; no cross-study pairing is treated as one specimen. No source from this screen supplies all of measured major/minor/pitch/root geometry, engagement, parent wall/fixture, bolt condition, specimen force–displacement, observed failure mode, same-condition true stress versus plastic strain for bolt and parent, and independently measured fracture calibration. Item 1's acquisition criteria remain unmet. Items 2–5 and physical capacity validation remain open.
