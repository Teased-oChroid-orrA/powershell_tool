# native_search offline build (issue #2 Section 15)

Scope: this document covers `native-search/` (the Rust crate) only. The
existing .NET/WinUI offline-build story is `docs/deployment.md` and is
unaffected by any of this — the two are verified independently.

## What "offline" means here

Section 15 distinguishes build-time internet access (fine, expected — NuGet
restore, crates.io) from *runtime* internet access (not fine, must be zero).
This document is about a narrower, stricter question than that: **can
`native-search/` be built at all without touching the network**, for a
scenario where CI/build-time internet access itself isn't available (an
air-gapped build machine, a vendored-dependency compliance requirement,
etc.)? That's a real, verifiable claim, not the same thing as "the published
app doesn't phone home at runtime" (which is separately true and unrelated
to this document).

## Verified directly, not assumed (2026-08-24)

```
cd native-search
cargo vendor .cargo-vendor-check
# produces a .cargo/config.toml stanza; applied it, then:
cargo build --offline
# → Finished `dev` profile [unoptimized + debuginfo] target(s) in 41.61s
```

This succeeded with the network disabled for the `cargo build --offline`
step (that's what `--offline` enforces — it hard-fails rather than silently
reaching the network if anything's missing from the vendor directory). Both
the vendor directory and the `.cargo/config.toml` used were deleted
afterward — this repo does not commit a vendored copy of the dependency
tree; the point of this exercise was to prove the *capability* works, not
to ship 100+ MB of vendored source by default.

## The one real caveat: this needs a C compiler, not just Rust

`cargo vendor` pulled in `zstd-sys` (Tantivy's compression backend), which
bundles zstd's actual C source and compiles it via the `cc` crate at build
time. **A Rust toolchain alone is not sufficient to build `native-search/`
— a C compiler must also be present.** This was incorrectly implied to be a
"pure-Rust build" in ADR-002's first pass; see that ADR's "Follow-up
verification" section for the correction.

In practice this is already satisfied everywhere this project actually
builds:

- **`windows-latest` GitHub Actions runner** (`.github/workflows/build.yml`):
  confirmed working — the 2026-08-24 CI run built `native-search` via MSVC
  successfully. The runner ships a full Visual Studio install (needed
  anyway for the WinUI/MSBuild steps), which provides `cl.exe`.
- **This development machine** (macOS): confirmed working — `cargo build`
  throughout this session used the system's preinstalled `clang`.

If `native-search/` is ever built on a machine provisioned with *only* a
Rust toolchain and nothing else (a minimal CI image, an air-gapped build
box built from scratch), it will fail at the `zstd-sys` compile step until
a C compiler is added — `cl.exe` (MSVC Build Tools) on Windows, `clang`/
`gcc` elsewhere. This is a real requirement to write into any future
from-scratch build-environment setup instructions, not a hypothetical edge
case.

## What this document does not claim

- It does not claim vendoring is wired into `build.yml` today — CI restores
  from crates.io over the network (fine, per Section 15's own build-time/
  runtime distinction) via the registry cache step already in the
  workflow. Vendoring would only matter for a genuinely air-gapped build
  environment, which isn't this project's current requirement.
- It does not re-verify vendoring on Windows specifically — the `cargo
  vendor`/`--offline` run above was done on macOS. The *mechanism*
  (`cargo vendor` + `.cargo/config.toml` source replacement) is
  platform-independent and part of Cargo itself, so there's no reason to
  expect Windows-specific behavior here, but this is stated as reasoning
  from Cargo's own cross-platform design, not as a claim that now includes
  fresh empirical Windows evidence beyond what ADR-002 item 12 already
  covers (the CI run itself, which used the network-restored registry, not
  a vendored one).
