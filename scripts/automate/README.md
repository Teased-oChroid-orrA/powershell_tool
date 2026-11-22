# AutoMate check of the assembly detector

Scores `solid_kernel::features::detect` against the user-authored mates of real public assemblies. The models are never
committed; the three scripts and `solid-kernel/examples/assembly-score.rs` reproduce the numbers recorded in
`docs/epic-assembly-feature-advice.md` ("AutoMate check").

Source: AutoMate (Jones, Hildreth, Chen, Baran, Kim, Schulz; ACM TOG 40(6), 2021), Zenodo 7776208, DOI 10.5061/dryad.2547d7wvw,
CC0 1.0. Public Onshape assemblies, parts as STEP, one JSON per assembly with occurrences (4x4 transform, metres) and mates
(type, two occurrences, two mate frames in the occurrences' local coordinates).

```sh
python3 scripts/automate/fetch.py DATA_DIR            # byte-range reads: the two zip directories (130 MB) plus the chosen files
python3 scripts/automate/prepare.py DATA_DIR WORK_DIR # one placed STEP per occurrence, a manifest with world mate frames
cargo build --release -p solid-kernel --example assembly-score
python3 scripts/automate/score.py WORK_DIR target/release/examples/assembly-score
```

`fetch.py` keeps to a few requests at a time; Zenodo rate-limits (HTTP 429) and the two parts it refused are skipped, with their
assemblies. Label rule (`score.py`): a bolted joint is one body whose mates on a single axis line join two other bodies that each
have a bore on that line, the fastener body carrying a convex cylinder that fits the bores and overlaps both. Mates whose frame
merely lies on two bores' axis without a shank are not counted.
