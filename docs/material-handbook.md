# MIL-HDBK-5J material library

All 1,375 usable conditions of the MIL-HDBK-5J design mechanical and physical property tables are in the
material pickers of the Bushing Workbench and the Pressure Vessel Analyzer, after the curated built-ins
(`mechanics_core::materials::builtin_catalog()` = `MATERIALS` + `handbook::HANDBOOK`, then user-added
materials). Filter with several words (`7075 t651 plate`); the panel under the list shows every handbook
value for the highlighted condition.

## Source and licence

* MIL-HDBK-5J is a US-government handbook (public domain). The digitised tables were taken from
  KasperCalc's published dataset (<https://kaspercalc.com/MaterialPropertyLookup.html>,
  `js/data/design-allowables.json`, retrieved 2026-10-03), which states that every value is transcribed
  from the handbook. The raw file is kept as `mechanics-core/data/mil-hdbk-5j.json` (page/anchor fields
  dropped).
* `tools/gen_handbook.py` regenerates `mechanics-core/src/handbook.rs` from it (no build-time
  dependencies; run it again if the JSON is replaced). **Do not edit `handbook.rs` by hand.**
* Spot checks in `materials.rs` tests (AISI 1025 annealed; counts; Fbru(e/D 1.5) <= Fbru(e/D 2.0)).
  The extraction itself is the website's; check a value against the handbook before relying on it for
  substantiation. MIL-HDBK-5J is superseded by MMPDS, whose numbers differ for some alloys (7075-T6
  sheet Fbru e/D 2.0 is 156 ksi in MIL-HDBK-5J, 146 ksi from MMPDS-05).

## What each entry carries (`Material::extra`)

Ftu, Fty, Fcy, Fsu and elongation by grain direction (L/LT/ST); Fbru and Fbry at e/D 2.0 and 1.5; E, Ec, G,
Poisson's ratio, density; alloy, group, form, temper, thickness range, spec, A/B/S basis, clad flag, source
table and caption. The solver `Material` fields come from it as: lowest of L/LT for Ftu/Fty (ST only if
nothing else), lowest given value for Fsu, `fbru_ksi` = Fbru at e/D 2.0, `fbru_e15_ksi` = Fbru at e/D 1.5
(feeds the edge-check tabulated allowables), E and G in ksi (the handbook prints Msi).

## Not handbook data (flagged in each record's `estimated` field)

* **Thermal expansion is never in these tables**: every entry gets a group-typical value (Al 12.8,
  Mg 14.5, Ti 4.8-4.9, steels 6.0-6.5, austenitic stainless 9.2, Ni 7.0, ...). It only affects service-
  temperature interference changes in the Bushing Workbench; set it from the material's own data if
  that matters.
* E or Poisson's ratio missing from a table (about 13% / 8% of conditions) are filled with group-typical
  values and marked `*` in the panel.
* Fty missing -> Ftu, Ftu missing -> Fty (marked).
* 4 conditions with neither Ftu nor Fty in the source (5Cr-Mo-V steel, 250 maraging steel plate
  >0.250 in) are omitted from the library; they remain in the JSON.

## Not included

KasperCalc's supplementary datasets (third-party supplier curves, labelled "NOT MIL-HDBK-5") and the
702 digitised handbook figures (effect-of-temperature knockdown curves, S-N, stress-strain) were not
copied. Temperature derating of the allowables would be the natural next feature.
