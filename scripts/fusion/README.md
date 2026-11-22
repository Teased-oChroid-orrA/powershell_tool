# Fusion 360 Gallery check of the assembly detector

Scores `solid_kernel::features::detect` against the **designed Hole features** of real public Fusion 360 assemblies, and surveys the
near-closed bores. The data is never committed or redistributed; the two scripts and `solid-kernel/examples/assembly-score.rs`
reproduce the numbers recorded in `docs/epic-assembly-feature-advice.md` ("Fusion 360 Gallery check").

Source: Fusion 360 Gallery Assembly Dataset a1.0.0 (Autodesk AI Lab), https://github.com/AutodeskAILab/Fusion360GalleryDataset. **Licence:
non-commercial research use only, no redistribution of the dataset** (`LICENSE.md` of that repository). This app is for personal,
non-commercial use (owner decision, 2026-11-22). Eleven 7z archives of about 750 assemblies each (1.1 to 2.3 GB; a 7z cannot be read by
byte ranges, so a whole archive is downloaded). Each assembly has `assembly.json` (occurrence tree and transforms, joints, contacts and
`holes`: the designer's Hole features with type, diameter, origin and direction, centimetres) and a STEP file per body.

```sh
mkdir DATA && cd DATA && bsdtar -xf /path/to/a1.0.0_04.7z --include='*.json' --include='*.step'   # macOS bsdtar reads 7z
python3 scripts/fusion/prepare.py DATA WORK_DIR 80      # one placed STEP per body instance; the manifest carries the labels
cargo build --release -p solid-kernel --example assembly-score
python3 scripts/fusion/score.py WORK_DIR target/release/examples/assembly-score
```

Archive `a1.0.0_04` alone is 751 assemblies, 1.2 GB packed, 1.9 GB of JSON and STEP. The scorer runs one detector process per assembly
(180 s limit) and takes tens of minutes. Assemblies with more than 80 body instances are skipped. Each assembly's own `assembly.step`
does not read through the multi-body path (a mapped item without a representation), so bodies are placed from `assembly.json`.

Labels (`score.py`): a **designed hole** is found when a detected hole of the same body has the same axis (1 degree, origin within 0.1 mm)
and diameter (0.05 mm). A **designed coaxial pair** is two designed holes of different bodies on one axis with one diameter (1 %); it is
found when a detected site names both bodies on that line. Partial bores come from `find_near_closed` at half a turn; one on a
designed hole's axis and diameter is a designed full hole the reader split short of a turn (a false candidate).
