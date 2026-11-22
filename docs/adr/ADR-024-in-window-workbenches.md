# ADR-024: Workbenches open inside the dashboard window

Status: Accepted (owner direction 2026-11-21: "launch workbenches on the same window", with a way back to the dashboard
and to other workbenches; amends ADR-014 decision 1)

## Problem

ADR-014 made every workbench a child process (`app-tui --gpu-solid|--gpu-search|... [file]`), so a tile click opened a
second OS window and the dashboard stayed behind it. Moving between workbenches meant window switching, and nothing
showed which workbenches were open or had unsaved edits.

## Decision

1. **The dashboard window is a shell.** A tile click (or a recent file) builds the workbench in-process
   (`window/launch.rs::build_workbench`, the factory every `run_*` entry point also uses) and shows it in the same
   window. The workbench that is not shown is **parked** with its editor, project and view
   (`window/nav.rs::Nav`, `Open`, `Parked`); parked workbenches keep polling their workers (`poll_parked`), so a solve
   finishes while another tab is shown.
2. **A nav strip** (`gui/nav_bar.rs`, data in `gpu_viewer/nav.rs`: `NavAct`, `NavStrip`, `NavTab`) is drawn above the
   dashboard and every workbench page: Toolbench (home), one tab per open workbench with an unsaved-edits dot and a busy
   spinner, `x` to close, **Open** (the switcher, Ctrl/Cmd+J; Ctrl+K stays the per-workbench palette) and **New window**.
   Strip state is set on `Gui` (`set_nav`); a standalone `--gpu-*` window has no `Nav` and no strip.
3. **Closing asks first** while a tab has unsaved edits or a running solve ("Keep open" / "Discard and close"); quitting
   brings the first dirty tab forward and asks about it, repeating until none is left. Esc on the dashboard no longer
   quits while workbenches are open. A case only one window may hold (the stackup lease) stays held while its tab is open.
4. **Separate processes remain** for Shift-click on a tile (`DashEvent::NewWindow`) and **New window** (`pop_out`: refused
   while the tab is dirty or solving, closes the tab, spawns the child, and reopens the tab here if the spawn fails).
   ADR-014's file-as-interface rule is unchanged for those children and for `handoff.rs`.
5. `debug_log::init` installs its banner and panic hook once per process; the Search session `init` moved from `main.rs`
   into `build_workbench`.

Rejected: tabs as child windows embedded by the OS (no portable API), and one `Viewer` per workbench with window
switching in winit (a second surface, device and egui context per workbench; the parked state is a few boxed values).

## Follow-up (2026-11-21, same day)

The first cut's gaps were closed:

- **Page state** (selected tab, folded cards, drawer, tree state, step tracking, a page's scroll and selection) is taken
  from `Gui` when a workbench is parked (`Gui::park_page`, `PageState`) and given back when it is shown (`restore_page`).
  Transient things (popups, menus, the switcher) are still cleared (`reset_page`, `DocState::settle`).
- **Hand-offs**: `editor::handoff::launch` is the one seam every "open the other workbench" path uses (Bushing and
  Eccentric to FEA and back, the stackup, the fastener transfer). The dashboard window calls `handoff::serve_shell()`;
  after that `launch` queues the request (`take_asked`) and `Viewer::serve_asked` opens it as a tab. A standalone
  `--gpu-*` window still starts a process. Thread-local, so it is the window thread's setting.
- **Window size**: the dashboard window starts at 1200 x 760 like the workbench windows and grows to that when a workbench
  opens (`nav::room_for_workbench`); it never shrinks a window the person sized.
- **`pop_out`** is tested: `Dashboard::launch` records instead of spawning under `cfg(test)` (`dashboard::SPAWNED`).

- **Dashboard left rail** (owner review of the mockup): brand, section filter buttons with counts (`Dashboard::group`),
  theme at the foot; chips above the tiles below 760 px.
- **Way back from "Send joint to FEM"**: a Model > Send group (To Bushing / To Eccentric) in the FEA workbench for a file linked
  to that case (`SolidSession::linked_sends`); the Bushing window's status says where it is. The Tools menu entries stay.

## Known gaps

- A tab asked for by a hand-off while its case is already open is not brought forward (the open window updates itself
  from the file, as before).
- The live native window was driven only through the window-level tests and snapshots, not by a pointer on a real display.

## Evidence

`cargo test -p app-tui --lib`: 11 shell tests in `window/tests/nav.rs` (open, park, switch, close guards, quit rules) and the
dashboard click tests. Rendered check: ignored snapshots `nav_strip_snapshot` (`FEA_SNAPSHOT_NAV_OUT`; dashboard, close
question, switcher, viewer page) and `search_snapshot` with `FEA_SNAPSHOT_NAV=1` (Search page under the strip). The first
render showed an empty box for the unsaved dot (the UI font lacks U+25CF; now painted) and a translucent close question
(fade-in; now off). The live native window was not driven by a pointer in this environment.
