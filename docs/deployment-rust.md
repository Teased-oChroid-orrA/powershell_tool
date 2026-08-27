# Deployment (Rust/Dioxus app and CLI)

Scope: this document covers `app/` (the Dioxus desktop GUI) and `cli/`
(the headless `search-cli` binary) - the actively-developed Rust stack.
The existing .NET/WinUI deployment story is `docs/deployment.md` and is
unaffected by any of this - the two are verified independently, same
relationship `docs/offline-build.md` already has to `deployment.md`.

## Target requirements (unchanged from the original migration brief)

- No internet access on the target machine
- No administrator privileges required
- No pre-installed runtime of any kind (no .NET, no Windows App SDK, no
  WebView2 - see CLAUDE.md's "Why `dioxus-native`, not `dioxus-desktop`"
  for why WebView2 specifically is a hard constraint here, not an
  oversight)
- No installer requiring elevation

## Exact build commands

```sh
cargo build --release -p app --target x86_64-pc-windows-msvc
cargo build --release -p search-cli --target x86_64-pc-windows-msvc
```

This is exactly what `.github/workflows/rust-build.yml` runs on a
`windows-latest` GitHub Actions runner, producing
`target/x86_64-pc-windows-msvc/release/app.exe` and
`target/x86_64-pc-windows-msvc/release/search-cli.exe`. Unlike the C#
app, there is no separate "publish" step and no runtime-bundling
concern - a `cargo build --release` output for a pure-Rust binary with
no native runtime dependency (see "Why this is simpler than the C#
deployment story" below) already **is** the deployable artifact: a
single, self-contained `.exe`.

## What ends up in the build output

- `app.exe` / `search-cli.exe` - single files, nothing else needed
  alongside them. No companion DLLs, no resource files, no runtime
  folder.
- Windows' own bundled D3D12 (always present on any supported Windows
  version) is `app`'s only runtime graphics dependency -
  `dioxus-native`/Blitz has no WebView engine to bundle or depend on at
  all, unlike the wry/WebView2-based alternative this project
  deliberately avoided (see CLAUDE.md).
- `.github/workflows/rust-build.yml`'s "Verify published exe does not
  link WebView2/wry" step scans `app.exe`'s import table for
  `WebView2Loader.dll` and fails the build loudly if found - the
  regression guard for that constraint, run on every CI build rather
  than trusted to stay true.

## Why this is simpler than the C# deployment story

The C# deployment doc (`docs/deployment.md`) spends most of its length on
problems that are structural to a managed-runtime, XAML-based framework:
bundling `hostfxr.dll`/`coreclr.dll` so the target machine doesn't need
.NET installed, bundling `Microsoft.WindowsAppRuntime.dll` so it doesn't
need the Windows App SDK installed, choosing MSIX-vs-unpackaged, and a
real historical bug where a packaging-tooling flag silently dropped the
compiled-XAML resource index (`resources.pri`), breaking every XAML page
identically. None of that category of problem exists for a native,
statically-linked Rust binary - there is no separate runtime to bundle,
no XAML resource compilation step, no packaging-tooling flag that could
silently drop a resource index. `cargo build --release` for a target
with no native runtime dependency already produces the self-contained
artifact.

## Clean-machine test procedure

Same spirit as `docs/deployment.md`'s own procedure, adapted for a
single-file binary:

1. Download the `TextInFilesSearch-rust-app-win-x64` (and, if testing the
   CLI, `search-cli-win-x64`) artifact from the
   `.github/workflows/rust-build.yml` GitHub Actions run.
2. Copy `app.exe` (and/or `search-cli.exe`) to a Windows 10 (1809+) or
   Windows 11 machine that:
   - Has no internet connectivity
   - You do not have administrator rights on (or deliberately run as a
     standard/non-admin user)
   - Does not have any WebView2 Runtime installed (checking this
     specifically matters here, unlike a generic clean-machine test - a
     machine that happens to already have WebView2 installed for
     unrelated reasons could mask a real regression back to the
     `wry`/`dioxus-desktop` dependency this app deliberately avoids)
3. Double-click `app.exe` (or run `search-cli.exe --help` /
   `search-cli.exe` for the interactive CLI menu). It should launch
   without any dialog about missing dependencies, without requesting
   elevation, and without any network activity.
4. Run a search against a small local test folder containing at least
   one `.txt`, `.docx`, `.pptx`, `.pdf`, and `.rtf` file, with a filter
   you know appears in each. Confirm hits are found and the
   report/CSV/JSON/JSONL export (if enabled) is written to the chosen
   output folder.
5. Confirm the per-file in-flight progress list shows live status while
   a PDF is being processed (this app's own live-progress-reporting
   requirement - see CLAUDE.md).
6. Optionally, enable "Index this folder for fast re-search" and confirm
   a `.native-search-index` folder is created and a subsequent search
   is faster and still correct.
7. Close the app and confirm no background process was left running.

If any step fails, report exactly which step failed and the error text -
same principle as `docs/deployment.md`'s own procedure: a clean-machine
failure is genuinely new information, not something to guess around.

## Known limitations

- **PDF text extraction remains best-effort, not a full renderer.**
  CID-keyed/Type0-font PDFs (the common case for modern PDF generators)
  are handled via `/ToUnicode` CMap resolution (see `extraction.rs`'s
  `parse_tounicode_cmap`), and a scanned/image-only PDF (no text-showing
  operators at all - just a drawn page image) can optionally be OCR'd if
  the "OCR image-only/scanned PDFs" setting is enabled - see "OCR" below.
- **Architecture is win-x64 only for now**, matching the C# app's own
  scope - `x86_64-pc-windows-msvc` is the only target CI builds and
  verifies.
- **The `app` crate's actual rendered window has not been verified via
  this exact clean-machine procedure yet** - CI confirms the build
  succeeds and doesn't link WebView2, but a human running the numbered
  procedure above on a genuinely clean Windows machine is still the
  first real-world confirmation, the same gap `docs/deployment.md`
  documented (and then found a real bug through) for the C# app before
  its own first real-machine run.

## OCR ("OCR image-only/scanned PDFs" setting)

Bundled, not downloaded - satisfies "no internet access on the target
machine" the same as everything else in this doc. `ocrs`+`rten`'s two
`.rten` model files (~12MB total: `search-core/assets/ocr/text-detection.rten`,
`text-recognition.rten`) are embedded into `app.exe`/`search-cli.exe` via
`include_bytes!` at compile time (see `search-core/Cargo.toml`'s `ocrs`/
`rten` dependency comment for the full evaluation behind this specific
choice - pure-Rust ONNX-model execution, no system runtime, unlike
Tesseract bindings or `ort`). Nothing extra ships alongside the exe for
this feature; the binary itself is still the complete, self-contained
artifact "What ends up in the build output" above describes.

**Real-world measured cost** (this development machine, not win-x64 -
same "wrong hardware" caveat as `docs/benchmarking.md`): ~0.6-1s per
full-page image against a real scanned document
(`search-core/benches/data/xlarge-scanned.pdf`, 2479x3509 raw
`/DeviceRGB` pixels per page). A multi-page scanned PDF is bounded by the
same `overall_timeout_seconds` the rest of `extract_pdf_lines` respects -
checked before every page, not just once up front - so a large scanned
document produces partial (not zero, not unbounded-runtime) results if
OCR would otherwise run past the configured PDF timeout, the same
truncation behavior the normal text-extraction path already has for
large/complex files.

**Scope**: only `/DCTDecode` (JPEG) and raw, uncompressed `/FlateDecode`
`/DeviceRGB`/`/DeviceGray` 8-bit images are attempted - the two encodings
real-world scan-to-PDF tools were actually observed using (including the
committed `xlarge-scanned.pdf` fixture, which uses the raw-FlateDecode
form specifically - its ~26MB-per-page raw pixel data is why a dedicated,
dimension-bounded inflate path exists in `extraction.rs`
(`bounded_inflate_to_exact_len`) rather than reusing the shared 20MB
deflate-bomb cap the rest of PDF extraction uses, which a real full-page
scan at this resolution legitimately exceeds). `CCITTFaxDecode`/
`JPXDecode` and non-8-bit images are skipped, not guessed at.
