# Phase 151: Windows executable opens the Toolbench dashboard

2026-10-28. A bare `app-tui.exe` launch now selects the native Toolbench dashboard on Windows. Explicit `--tui` and `--toolbox` launches retain the terminal route. The `app-tui-build.yml` workflow enables `gpu-viewer` for both its default and OCCT builds, so the published executable includes the dashboard path.

The focused `startup_tests` case passed locally in the prior session, and `git diff --check` passed on this worktree. Source review confirmed that explicit editor flags still route before terminal startup. An existing macOS binary opened its dashboard for a three-second smoke check; that check did not exercise the new Windows default.

Hosted Windows build and launch remain unverified. `gh api rate_limit` could not connect to `api.github.com` in this session, so the build workflow could not be dispatched. The issue's broader Windows mesh and solve gates remain open.
