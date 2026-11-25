# Thread Workbench: Unified axial strength and damage

The native Thread Workbench compares one changed characteristic with an undamaged baseline. It supports direct tapped threads, helical-coil STI tapped holes, and solid key-locking inserts. Inputs use inches, TPI, ksi and lbf. The saved case is JSON; the text report records both capacities and missing evidence.

## Sources and equations

- [ASME B1.1-2003](https://law.resource.org/pub/us/cfr/ibr/inc/asme/asme.b01.01.2003.pdf), Unified 60-degree profile: basic pitch diameter `d - 0.649519P`, internal minor diameter `d - 1.082532P`, and external basic root diameter `d - 1.226869P`. These are basic reference dimensions, not class 2A/2B or 3A/3B limits.
- [NASA NSTS 08307A, Appendix A](https://files.engineering.com/files/1e29284f-4130-4184-9997-17ed698ba73b/NSTS-08307.pdf), Unified tensile stress area `π/4(d - 0.9743P)^2`; 1/4-28 evaluates to approximately `0.0364 in²`. Bolt tensile capability is area times user-supplied tensile strength. A measured root reduction scales area by the ratio of damaged to basic root-section areas.
- [FED-STD-H28/2A](https://quicksearch.dla.mil/Transient/D0A44A8FD8844E7C9515CE8F1B4F0443.pdf), section 70.2: external stripping area `πnL D₁[1/(2n) + 0.57735(d₂ - D₁)]` and internal stripping area `πnL d[1/(2n) + 0.57735(d - D₂)]`. The Workbench uses the entered mating diameters and multiplies each area by the corresponding material shear strength. Its basic-dimension screen does not apply thread-class allowance or tolerance limits.
- [Stanley Heli-Coil catalog HC-2000, Table VII](https://usgage.com/wp-content/uploads/2023/08/Heli-Coil_Catalog_HC2000_Rev12.pdf): STI tapped-hole pitch and minor diameter ranges for the supported UNC/UNF sizes. The bolt's nominal TPI does **not** describe the larger STI tapped thread. Coil wire transfer, tang, parent pull-out, and installation effects require a manufacturer rating for the matched size, insert length, parent material and installation.
- [KEENSERTS Series KN catalog](https://www.hfsindustrial.com/pub/media/resources/images/image/4/-/4-e-inserts_9-2022.pdf): the default thin-wall 1/4-28 internal / 3/8-16 external insert is a catalog pairing. Other pairings may be entered. The external thread can have a modified root. Key retention, insert body strength, and full assembly pull-out require a sourced manufacturer rating.

## Scope and evidence gates

For a directly tapped thread, the lowest of bolt tension, bolt external stripping and parent internal stripping is shown. For a helical coil, bolt tension and a matched assembly rating are shown only when STI hole measurements fall inside a sourced catalog range. For a Keensert, the screen also models solid insert internal and external thread shear and parent thread shear; the key/body/assembly path remains unrated until the user enters a manufacturer rating and source. If any required path lacks evidence, the Workbench shows no governing assembly capacity. Every geometry or material edit invalidates an insert rating for the proposed case.

The default strengths and 3,000 lbf load are illustrative inputs, not material allowables. The analysis is axial and static. It does not model preload, torsion, combined stress, transverse load, fatigue, temperature, corrosion, tolerance stack or safety factors. A measured diameter is a full-circumference effective diameter; localized flank damage needs inspection or a separate qualified analysis. A manufacturer rating must match the actual insert, parent material, length and installation.

## Verification

On 2026-11-24, `cargo test -p fastened-joint-solver` passed 121 tests and `cargo test -p app-tui --features gpu-viewer --lib` passed 1,391 tests (46 ignored). The no-default-features check also passed. The macOS native window opened from `--gpu-thread`; a screen capture confirmed three-pane layout, the baseline/changed comparison, and parent and bolt sections visible under the surface-less display style. Hosted Windows qualification remains the Windows evidence.
