# Issue 46 — Claude Code handoff

## Transition, 2026-10-16 UTC

Reliability PR [#45](https://github.com/Teased-oChroid-orrA/powershell_tool/pull/45) merged with explicit user authorization at **`8ba27a2fd21b88f7790d3ac2f530b498e3b031c2`**. Its head was `6f76dbb9eb2cd224a3a611b7e8bbeb69c5c5cab1`. The merged `codex/workbench-reliability` branch was removed locally and remotely. The new **`feature/native-cad-advanced-meshing`** branch starts at that exact merge commit. Do not merge this new epic without a subsequent decision.

Parent [#46](https://github.com/Teased-oChroid-orrA/powershell_tool/issues/46) has **18 native GitHub sub-issues (#47–64)** with blocked-by dependencies and acceptance checklists. Read [the execution index](issue-46-execution-plan.md) and [machine-readable initial tracking](issue-46-tracking.json). The original [epic](epic-native-cad-advanced-meshing-source.md) and [Pillar 7 addendum](epic-native-cad-autotuning-source.md) are preserved byte-for-byte. Their SHA-256 hashes respectively are `b28c41af3e0be55110e81b1564af5d92246c7c304b79d2d65e0b40ea9259d88c` and `8fac71c8ddbb512cd0ab2ee62b307aa152c4bbdc0e70b6930248e87730ba48de`.

## Current evidence and outstanding validation

The complete clean local application-source qualification passed: **2,569 tests, zero failures, 86 intentionally ignored**, terminal-only compilation, and **9 explicit software-Vulkan checks**. See [the immutable report](workbench-qualification-linux-2026-10-15.json) and [reliability handoff](workbench-reliability-2026-10-15.md) for commands, exact source revision, earlier failures and corrections. The only subsequent reliability changes were CI/docs. No application, dependency or test source changes occur in this new planning commit, so no redundant test/build run is needed.

[Hosted final-revision qualification](https://github.com/Teased-oChroid-orrA/powershell_tool/actions/runs/38850003423) was still running at merge; **it is not claimed as passed**. GitHub accepted the normal merge without a gate override. Read live checks before making cross-platform claims and investigate any actual failure. Earlier Linux/macOS default-feature checks passed; the obsolete earlier run was cancelled to avoid duplicate Windows usage. Do not rerun successful local qualification or dispatch new CI simply to create tracking. Hosted Windows CAD, macOS/Windows hardware graphics and clean-machine packaged startup remain unverified. Historical PR/issue comments may describe the prior no-merge authorization; this dated transition records the user's later explicit merge approval.

## Start here

1. Read root CLAUDE.md, relevant AGENTS.md, the execution index and issue #47. Preserve existing work; no new worktree is needed.
2. Finish the actual Phase 0 capability/license/dependency/risk audit. The source-linked orientation table is **not** a completed audit, fresh performance baseline or kernel selection.
3. Lead with CAD backend isolation (#48) and robust predicates/topology (#49). Start verified workload telemetry (#50) and the early compatibility registry (#64) alongside isolation, using only capabilities confirmed by the audit.
4. Native B-Rep import/validation/healing (#51), constrained 3D boundary recovery (#52) and sliver/mesh validation (#53) precede advanced high-order/hybrid/anisotropic work. Preserve the OCCT reference adapter.
5. Audit/reuse existing cost, hybrid, AMR timing/transfer and quality infrastructure. Defer learned predictions until representative verified benchmarks exist. AMR remains independent and authoritative; no hidden multi-solve exploration.
6. Record each milestone's commit/PR, exact tested revision, files/modules, applicable commands/exits, accuracy, quality, time/memory, failures, limits and next tasks on its issue and in a new append-only phase log.
7. OpenCascade retirement (#63) is final, with full committed-workflow parity and separate explicit approval. The reliability merge authorization does not authorize retirement or future feature merges.

## Local environment

Use the installed toolchain and caches, not a new install:

```sh
export CARGO_HOME=/workspace/toolchains/cargo
export RUSTUP_HOME=/workspace/toolchains/rustup
export PATH="$CARGO_HOME/bin:/workspace/toolchains/link-tools:$PATH"
export OCCT_ROOT=/workspace/toolchains/occt/occt-8_0_1_rev3-x86_64_unknown_linux_gnu
export CARGO_INCREMENTAL=0
cd /workspace/powershell_tool
```

For a justified full integration check: `python3 scripts/qualify.py --cad --jobs 1 --output /workspace/qualification.json`. Check disk space first. For targeted numerical changes run the relevant existing tests and analytical/dimensional acceptance cases; never weaken tolerances. Actual UI changes need a real native window/screenshot, not compilation alone. Software-Vulkan and Xvfb setup is in the reliability handoff/saved environment instructions. Keep setup scripts separate from authorized application edits.

There is no new epic implementation yet. No new PR/hosted run is needed for this documentation-only branch initialization. The user explicitly requests GitHub usage restraint. Future CI must be justified by a required check, meaningful cross-platform risk or release gate; use local focused checks and existing evidence first.
