# Issue 46: native solid editor browser visual qualification plan

Status: execution plan, 2026-10-23. Phase 135 qualified the terminal browser in a real PTY and covered native editor state with focused tests. It did not inspect the native editor's visible macOS window.

## Goal and scope

Prove that File > Open and the keyboard Open action show a usable browser in the actual native solid editor window, that navigation and selection render correctly, and that opening a file uses the established validation path. This is a rendering qualification of Phase 135, not a new file workflow or an OCCT retirement gate.

The relevant path is `app-tui --gpu-solid` -> `SolidSession::begin_open_file` -> `browse_dir` / `browse_parent` / `browse_entry` -> existing `open_path`. The browser lists directories plus `.json`, `.msh`, `.inp`, `.step`, `.stp`; a file click clears the prompt/browser and opens through existing validation. `open_prompt_browses_and_keeps_typed_path` covers state transitions, not actual pixels. Follow root and `app-tui/AGENTS.md` before any code change.

## Steps

1. On current `main`, record the commit and working-tree state. Preserve `.claude/settings.json` and both untracked recovery JSON files. Build `app-tui` with locked offline dependencies in this checkout's own Cargo target, serializing Cargo work: `cargo build -p app-tui --locked --offline -j1`. Do not count `cargo check` or a headless egui test as visual proof.
2. Launch the rebuilt executable with `--gpu-solid` in an interactive macOS desktop session. If the application cannot appear because this environment has no WindowServer, GPU surface, or permission to observe its window, record the exact block and perform the remaining read-only test review; leave the visual gate open. Do not invent a screenshot or treat a terminal PTY as the native window.
3. Capture actual window images through the available desktop UI/screenshot mechanism. Apple's [screenshot guidance](https://support.apple.com/en-ie/102646) documents window capture on macOS. Prefer a window capture over a full-screen capture to avoid unrelated desktop content. Keep evidence local if it includes private paths or model names; a sanitized image or precise observation log may be committed if appropriate.
4. Inspect at normal and narrow window sizes: File > Open and Ctrl/Cmd+O present the browser without stale prompt borders; current directory and entries are readable; folder entry and `..` navigation render correctly; scrolling reaches off-screen entries; Esc cancels; typed path remains available; selecting a valid saved `.fea.json` opens and updates the model/status; selecting an invalid JSON reports the error and keeps the current model; a generated local STEP cube opens through the same path. Avoid browsing private user directories for evidence; use a temporary fixture directory with a few nested folders and files.
5. Record OS/display/session, source commit, executable build command, screenshots or observations, every exercised interaction, and any failure in a short addendum to the append-only Phase 135 record or a new phase note. Run the focused `cargo test -p app-tui --locked --offline -j1 open_prompt_browses_and_keeps_typed_path` only if code changes or a mismatch needs diagnosis. If a visual defect is found, trace its UI owner, fix it there, add a focused regression where practical, rebuild, rerun the real window, and report the corrected image.

## Acceptance and limits

Pass only with a visible native window showing the browser and a completed valid/invalid open flow. State tests and the terminal PTY result remain useful separate evidence. A macOS visual pass does not prove Windows rendering. This plan does not change `occt_retirement_approved`, remove OCCT, or claim mesh/solve parity.
