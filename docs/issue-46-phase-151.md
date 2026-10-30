# Phase 151: Windows executable opens the Toolbench dashboard

2026-10-28. A bare `app-tui.exe` launch now selects the native Toolbench dashboard on Windows. Explicit `--tui` and `--toolbox` launches retain the terminal route. The `app-tui-build.yml` workflow enables `gpu-viewer` for both its default and OCCT builds, so the published executable includes the dashboard path.

The focused `startup_tests` case passed locally in the prior session, and `git diff --check` passed on this worktree. Source review confirmed that explicit editor flags still route before terminal startup. An existing macOS binary opened its dashboard for a three-second smoke check; that check did not exercise the new Windows default.

Hosted Windows build and launch remain unverified. `gh api rate_limit` could not connect to `api.github.com` in this session, so the build workflow could not be dispatched. The issue's broader Windows mesh and solve gates remain open.

## 2026-10-30 addendum

The default Windows workflow [run 40780539693](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/40780539693) and OCCT-enabled [run 41044530973](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/41044530973) built and uploaded their release executables. Neither run launched the app. The workflow now starts the bare executable on the hosted runner and waits for the `GS Engineering Toolbench` window title, then closes the process. This check must pass before claiming hosted launch evidence. The broader Windows mesh and solve gates remain open.

The updated default [run 41047938782](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/41047938782) passed the release build, bare-executable dashboard window check, and artifact upload. The check establishes that a native dashboard window opened on hosted Windows; it does not inspect the rendered tiles. The broader Windows mesh and solve gates remain open.
