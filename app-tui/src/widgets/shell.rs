//! Top-level layout composition: chrome (topbar/rail/status bar) plus the
//! active toolbox's workspace, then overlays drawn last. This is the one
//! place that knows the full z-order; individual widgets don't know about
//! each other.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph, Wrap};

use crate::app::AppState;
use crate::command_palette;
use crate::modal::{ConfirmDialog, ModalState};
use crate::mouse::MouseRegions;
use crate::nav::{Breakpoint, FocusArea, ToolId, pick_breakpoint};
use crate::theme::StatusTone;
use crate::widgets::{empty_state, help, hint_panel, spinner};

pub fn draw(frame: &mut Frame, state: &AppState, tick: u64, regions: &mut MouseRegions) {
    regions.clear();
    let area = frame.area();
    let breakpoint = pick_breakpoint(area.width);

    let root = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(1), Constraint::Min(1), Constraint::Length(1)])
        .split(area);

    draw_topbar(frame, root[0], state);
    draw_body(frame, root[1], state, breakpoint, tick, regions);
    draw_status_bar(frame, root[2], state, tick);

    match &state.modal {
        ModalState::Palette(palette) => command_palette::render(frame, area, &state.theme, palette, state.nav.active_tool, regions),
        ModalState::Help => draw_help(frame, area, state, regions),
        ModalState::Confirm(dialog) => draw_confirm(frame, area, state, dialog, regions),
        ModalState::None => {}
    }

    // Toolbox-local overlays (not a global `ModalState`) - drawn on top of
    // everything but the toasts.
    if state.search.extension_picker.open {
        crate::toolboxes::search::extension_picker::render(frame, area, &state.theme, &state.search.extension_picker, regions);
    }
    if state.pressure_vessel.material_picker.open {
        crate::toolboxes::pressure_vessel::material_picker::render(frame, area, &state.theme, &state.pressure_vessel.material_picker, &state.pressure_vessel.model, regions);
    }

    draw_toasts(frame, area, state);
}

fn draw_topbar(frame: &mut Frame, area: Rect, state: &AppState) {
    let title = format!("GS Toolbench › {}", state.nav.active_tool.title());
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(18)])
        .split(area);
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(title, state.theme.title_style(true)))),
        layout[0],
    );
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled("Ctrl+P Commands", state.theme.disabled_style())))
            .alignment(Alignment::Right),
        layout[1],
    );
}

fn draw_body(frame: &mut Frame, area: Rect, state: &AppState, breakpoint: Breakpoint, tick: u64, regions: &mut MouseRegions) {
    // Narrow terminals drop the persistent rail entirely - the command
    // palette becomes the only way to switch toolboxes (see the plan's
    // responsive-breakpoint design).
    let rail_width = match breakpoint {
        Breakpoint::Wide => 18,
        Breakpoint::Medium => 4,
        Breakpoint::Narrow => 0,
    };

    if rail_width == 0 {
        draw_workspace(frame, area, state, tick, regions);
        return;
    }

    let cols = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Length(rail_width), Constraint::Min(1)])
        .split(area);
    draw_rail(frame, cols[0], state, breakpoint, regions);
    draw_workspace(frame, cols[1], state, tick, regions);
}

fn draw_rail(frame: &mut Frame, area: Rect, state: &AppState, breakpoint: Breakpoint, regions: &mut MouseRegions) {
    let focused = state.focus.area == FocusArea::Rail;
    let block = Block::default().borders(Borders::RIGHT).border_style(state.theme.border_style(focused));
    let inner = block.inner(area);
    frame.render_widget(block, area);

    if inner.height == 0 {
        return;
    }

    let row_count = ToolId::ALL.len().min(inner.height as usize);
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints(vec![Constraint::Length(1); row_count])
        .split(inner);

    for (i, tool) in ToolId::ALL.iter().take(row_count).enumerate() {
        let active = *tool == state.nav.active_tool;
        let style = if !tool.enabled() {
            state.theme.disabled_style()
        } else if active {
            Style::default().fg(state.theme.accent).add_modifier(Modifier::BOLD)
        } else {
            Style::default().fg(state.theme.fg)
        };
        // Medium breakpoint has no room for full labels - fall back to a
        // single initial letter rather than truncating mid-word. A real
        // icon glyph per tool is a natural later upgrade, not required for
        // this phase.
        let text = if breakpoint == Breakpoint::Medium {
            tool.title().chars().next().map(String::from).unwrap_or_default()
        } else {
            let marker = if active { "> " } else { "  " };
            format!("{marker}{}", tool.title())
        };
        frame.render_widget(Paragraph::new(Line::from(Span::styled(text, style))), rows[i]);
        regions.rail.push((rows[i], *tool));
    }
}

fn draw_workspace(frame: &mut Frame, area: Rect, state: &AppState, tick: u64, regions: &mut MouseRegions) {
    match state.nav.active_tool {
        ToolId::Search => {
            let focused_pane = match state.focus.area {
                FocusArea::Workspace(n) => Some(n),
                FocusArea::Rail => None,
            };
            let search = &state.search;
            match search.screen {
                crate::toolboxes::search::ToolboxScreen::Run => {
                    crate::toolboxes::search::run_view::draw(frame, area, &state.theme, search, focused_pane, tick, regions);
                }
                crate::toolboxes::search::ToolboxScreen::Settings => {
                    crate::toolboxes::search::settings_view::draw(
                        frame,
                        area,
                        &state.theme,
                        &search.config,
                        &search.recent_searches,
                        &search.saved_presets,
                        &search.settings,
                        regions,
                    );
                }
            }
        }
        ToolId::FastenerHole => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::fastener_hole::view::draw(frame, area, &state.theme, &state.fastener_hole, focused, regions);
        }
        ToolId::PressureVessel => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::pressure_vessel::view::draw(frame, area, &state.theme, &state.pressure_vessel, focused, regions);
        }
        ToolId::Bushing => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::bushing::view::draw(frame, area, &state.theme, &state.bushing, focused, regions);
        }
        ToolId::PreloadAnalysis => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::preload_analysis::view::draw(frame, area, &state.theme, &state.preload_analysis, focused, regions);
        }
        ToolId::EccentricBushing => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::eccentric_bushing::view::draw(frame, area, &state.theme, &state.eccentric, &state.bushing.model, focused, regions);
        }
        ToolId::LugAnalysis => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::lug_analysis::view::draw(frame, area, &state.theme, &state.lug_analysis, focused, regions);
        }
        ToolId::FeaWorkbench => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::fea_workbench::view::draw(frame, area, &state.theme, &state.fea_workbench, focused, regions);
        }
        ToolId::MaterialLookup => {
            let focused = matches!(state.focus.area, FocusArea::Workspace(_));
            crate::toolboxes::material_lookup::view::draw(frame, area, &state.theme, &state.material_lookup, focused, regions);
        }
        _ => empty_state::render(frame, area, &state.theme, "Coming soon", None),
    }
}

fn draw_status_bar(frame: &mut Frame, area: Rect, state: &AppState, tick: u64) {
    let mut hints = vec![
        help::KeyHint { key: "Ctrl+P", label: "Commands" },
        help::KeyHint { key: "Tab", label: "Focus" },
    ];
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(10)])
        .split(area);
    let mut contextual = contextual_hint(state);
    // Narrow terminals drop the toolbox's own hints from the end (whole hints, never half a label);
    // the global ones, including Help and Quit, stay.
    let tail = [help::KeyHint { key: "?", label: "Help" }, help::KeyHint { key: "q", label: "Quit" }];
    let width = |hs: &[&help::KeyHint]| hs.iter().map(|h| h.key.chars().count() + 1 + h.label.chars().count()).sum::<usize>() + 2 * hs.len().saturating_sub(1);
    while !contextual.is_empty() && width(&hints.iter().chain(contextual.iter()).chain(tail.iter()).collect::<Vec<_>>()) > layout[0].width as usize {
        contextual.pop();
    }
    hints.extend(contextual);
    hints.extend(tail);
    help::render_status_hints(frame, layout[0], &state.theme, &hints);

    let status = if state.is_busy() {
        Line::from(vec![spinner::spinner_span(tick, &state.theme, StatusTone::Info), Span::raw(" Running")])
    } else {
        Line::from(Span::styled("Ready", state.theme.status_style(StatusTone::Success)))
    };
    frame.render_widget(Paragraph::new(status).alignment(Alignment::Right), layout[1]);
}

/// Extra, always-visible status-bar hints for whatever non-obvious keys
/// actually do something right now - e.g. `s` opening Settings was
/// previously discoverable only via the `?` help overlay (a separate modal
/// you had to already know to open), with no hint on the always-visible
/// chrome that the binding existed at all. Toolbox-specific and
/// pane/screen-aware, so a hint never claims a key does something it won't
/// actually do given the current focus/screen. Returns zero, one, or two
/// hints - `draw_status_bar` appends whatever comes back to its own
/// always-shown hints, so there's no single-hint constraint to work around.
fn contextual_hint(state: &AppState) -> Vec<help::KeyHint> {
    match state.nav.active_tool {
        ToolId::Search => match state.search.screen {
            crate::toolboxes::search::ToolboxScreen::Run => {
                let results_focused =
                    matches!(state.focus.area, FocusArea::Workspace(n) if n == crate::toolboxes::search::PANE_RESULTS);
                if !results_focused {
                    return Vec::new();
                }
                let mut hints = vec![help::KeyHint { key: "s", label: "Settings" }];
                if !state.search.run.results.is_empty() {
                    hints.push(help::KeyHint { key: "e", label: "Export hits" });
                }
                hints
            }
            crate::toolboxes::search::ToolboxScreen::Settings => vec![help::KeyHint { key: "Esc", label: "Back" }],
        },
        // `e` (export report) and `d` (Numbers panel) are the two bindings
        // on these toolboxes with no on-screen affordance elsewhere -
        // discoverable only via `?` otherwise, same reasoning as Search's
        // own `s Settings` hint above. Only shown while the workspace
        // itself has focus and no overlay is covering it, so a hint never
        // claims a key does something it won't actually do right now.
        ToolId::FastenerHole if matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "e", label: "Export" }]
        }
        ToolId::PressureVessel if !state.pressure_vessel.material_picker.open && matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "d", label: "Numbers" }, help::KeyHint { key: "e", label: "Export" }]
        }
        ToolId::Bushing
            if !state.bushing.material_picker.open && !state.bushing.reamer_picker.open && matches!(state.focus.area, FocusArea::Workspace(_)) =>
        {
            vec![help::KeyHint { key: "d", label: "Numbers" }, help::KeyHint { key: "e", label: "Export" }]
        }
        ToolId::PreloadAnalysis if matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "d", label: "Numbers" }, help::KeyHint { key: "e", label: "Export" }]
        }
        ToolId::EccentricBushing if matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "r", label: "Analyse" }, help::KeyHint { key: "m", label: "Max offset" }, help::KeyHint { key: "l", label: "Max load" }, help::KeyHint { key: "e", label: "Export" }]
        }
        ToolId::LugAnalysis if state.lug_analysis.material_browser.is_none() && matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "d", label: "Bore profile" }, help::KeyHint { key: "e", label: "Export" }]
        }
        ToolId::FeaWorkbench if state.fea_workbench.material_browser.is_none() && matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "r", label: "Solve" }, help::KeyHint { key: "v", label: "Field" }, help::KeyHint { key: "g", label: "GPU view" }, help::KeyHint { key: "t", label: "Animate" }, help::KeyHint { key: "j", label: "Save" }]
        }
        ToolId::MaterialLookup if matches!(state.focus.area, FocusArea::Workspace(_)) => {
            vec![help::KeyHint { key: "Enter", label: "Mark" }, help::KeyHint { key: "F2", label: "Compare" }, help::KeyHint { key: "F3", label: "Export" }]
        }
        _ => Vec::new(),
    }
}

fn draw_help(frame: &mut Frame, area: Rect, state: &AppState, regions: &mut MouseRegions) {
    regions.help_overlay = Some(area);
    let global: &[help::KeyHint] = &[
        help::KeyHint { key: "Ctrl+P", label: "Command palette" },
        help::KeyHint { key: "?", label: "Toggle this help" },
        help::KeyHint { key: "Tab / Shift+Tab", label: "Cycle focus" },
        help::KeyHint { key: "Left/Right", label: "Switch toolbox (rail focused)" },
        help::KeyHint { key: "q", label: "Quit" },
        help::KeyHint { key: "Esc", label: "Close overlay" },
    ];
    let search: &[help::KeyHint] = &[
        help::KeyHint { key: "Enter", label: "Run search (on Path/Filters) / Open (on Results)" },
        help::KeyHint { key: "c", label: "Cancel a running search (Results pane)" },
        help::KeyHint { key: "y", label: "Copy full path (Results pane)" },
        help::KeyHint { key: "r", label: "Reveal containing folder (Results pane)" },
        help::KeyHint { key: "e", label: "Export this file's hits to .txt (Results pane)" },
        help::KeyHint { key: "s", label: "Open Settings (Results pane) / Esc to go back" },
        help::KeyHint { key: "Tab", label: "In Settings: cycle Fields / Recents / Presets" },
        help::KeyHint { key: "Space/Enter", label: "Toggle/edit a field, or apply a recent search or preset" },
        help::KeyHint { key: "Up/Down", label: "Move selection (Results pane)" },
        help::KeyHint { key: "Backspace", label: "Edit Path/Filters field" },
    ];
    let fastener_hole: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Space/Enter", label: "Toggle Hole Type/Tolerance Input/Solve For/Method, or edit a value" },
        help::KeyHint { key: "Enter/Esc", label: "While editing: commit / cancel" },
        help::KeyHint { key: "e", label: "Export a plain-text report and open it" },
        help::KeyHint { key: "Hole Type", label: "Regular: two toleranced diameters and their fit. Countersunk: D/d/h/angle geometry" },
        help::KeyHint { key: "Fit sign", label: "Fit = Hole 2 - Hole 1: positive = clearance, negative = interference" },
        help::KeyHint { key: "Classification", label: "Min>0 Clearance; Max<0 Interference; otherwise Transition (boundary zero = Transition)" },
        help::KeyHint { key: "Solve For", label: "Countersink: pick which of Outer/Hole/Depth/Angle is solved from the other three" },
        help::KeyHint { key: "Preserve Depth", label: "Secondary countersink keeps the same depth+angle; outer diameter is solved" },
        help::KeyHint { key: "Preserve Area", label: "Secondary countersink keeps the same angle+lateral area; outer diameter and depth are solved" },
        help::KeyHint { key: "Lateral Area", label: "Always calculated, always read-only - never a direct input, even in Preserve Area mode" },
        help::KeyHint { key: "[TAG]s", label: "INPUT/CALCULATED/TRANSFERRED/PRESERVED/DERIVED/INVALID mark every value's origin" },
    ];
    let pressure_vessel: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Space/Enter", label: "Toggle End Condition, open the Material picker, or edit a value" },
        help::KeyHint { key: "Enter/Esc", label: "While editing: commit / cancel" },
        help::KeyHint { key: "d", label: "Toggle the Numbers panel (Lame constants + per-surface stress breakdown)" },
        help::KeyHint { key: "e", label: "Export a plain-text report and open it" },
        help::KeyHint { key: "Governing", label: "The failure mode with the lowest margin, at whichever surface is worse" },
        help::KeyHint { key: "Classification", label: "Thin-wall vs. thick-wall is engineering interpretation only - both use the full Lame solution" },
        help::KeyHint { key: "Buckling", label: "Only evaluated with external pressure and a nonzero unsupported length, within OD/t >= 40" },
        help::KeyHint { key: "Thermal", label: "Only evaluated with a nonzero temperature differential - folded into the four checks, not a separate row" },
        help::KeyHint { key: "Material picker", label: "/ filters by name, n opens a blank template to add a custom material, Enter selects" },
    ];
    let bushing: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Space/Enter", label: "Toggle OD/ID Geometry etc., open a material/reamer picker, or edit a value" },
        help::KeyHint { key: "Enter/Esc", label: "While editing: commit / cancel" },
        help::KeyHint { key: "d", label: "Toggle the Numbers panel (per-radius hoop/radial/axial stress breakdown)" },
        help::KeyHint { key: "e", label: "Export a plain-text report and open it" },
        help::KeyHint { key: "Bore Diameter", label: "Enter opens the aircraft reamer catalog; m inside it types an exact value instead" },
        help::KeyHint { key: "Reamer/Material picker", label: "/ filters the list, Enter selects, Esc closes without changing anything" },
        help::KeyHint { key: "Reamer picker", label: "i: import a library file, x: export current + built-in catalog, n: tag the highlighted size Preferred" },
        help::KeyHint { key: "Friction/Fit Type", label: "Enter on Friction opens typical values with usage notes ('m' types an exact value); Fit Type is a label - Shrink also enables Install Thermal Assist" },
        help::KeyHint { key: "Bushing ID/Material pickers", label: "n saves the current value/opens the add-material form, i: import a library file, x: export it" },
    ];
    let preload_analysis: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Space/Enter", label: "Toggle Mode/Tightening From etc., open the bolt picker, or edit a value" },
        help::KeyHint { key: "Enter/Esc", label: "While editing: commit / cancel" },
        help::KeyHint { key: "d", label: "Toggle the Numbers panel (torque/deformation/rotation/stress breakdown)" },
        help::KeyHint { key: "e", label: "Export a plain-text report and open it" },
        help::KeyHint { key: "Bolt (AN Standard)", label: "Enter opens the AN/NAS/MS/Hi-Lok catalog and auto-fills thread geometry" },
        help::KeyHint { key: "Uncertainty Analysis", label: "Worst-case corner search and/or a seeded Monte Carlo sampler, toggled together" },
    ];
    let eccentric: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Enter/Space", label: "Edit a value, toggle the axial condition, or run the selected action" },
        help::KeyHint { key: "r", label: "Analyse the entered offset: pressure map, friction torque capacity against F e sin(angle), margin" },
        help::KeyHint { key: "m / l", label: "Find the largest offset that still holds / the largest pin load that is held" },
        help::KeyHint { key: "d / e", label: "Toggle the interface pressure profile / export a text report" },
        help::KeyHint { key: "Inputs", label: "Bore, ID, interference, friction, length, load and materials come from the Bushing Workbench" },
    ];
    let lug_analysis: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Space/Enter", label: "Toggle Head Shape / Mesh Density, open the material browser, or edit a value" },
        help::KeyHint { key: "Enter/Esc", label: "While editing: commit / cancel" },
        help::KeyHint { key: "d", label: "Toggle the bore profile (contact pressure, friction and hoop stress every 15 degrees)" },
        help::KeyHint { key: "e", label: "Export a plain-text report and open it" },
        help::KeyHint { key: "Re-analysis", label: "Automatic a moment after any input stops changing; the FE runs on a worker thread" },
        help::KeyHint { key: "Elastic FE", label: "Peak stresses are elastic indicators; local yielding at the hole edge is expected in a ductile lug" },
        help::KeyHint { key: "Load Angle", label: "0 is axial tension, 90 transverse, 180 compression; oblique loads use the full lug and a clamped far end" },
    ];
    let fea_workbench: &[help::KeyHint] = &[
        help::KeyHint { key: "Up/Down", label: "Move field selection" },
        help::KeyHint { key: "Space/Enter", label: "Cycle a choice, open the material browser, add or remove a hole / support / load, or edit a value" },
        help::KeyHint { key: "Left/Right", label: "Previous / next template, analysis, element type or edge name" },
        help::KeyHint { key: "r / a", label: "Solve now / toggle the automatic solve of a small model" },
        help::KeyHint { key: "v / m / x", label: "Cycle the contour field / toggle mesh lines / toggle the deformed shape" },
        help::KeyHint { key: "d", label: "Toggle the full report in the Results pane" },
        help::KeyHint { key: "j / o", label: "Save the problem as JSON / open a problem file" },
        help::KeyHint { key: "e / p", label: "Export the text report / the result as .vtu (ParaView)" },
        help::KeyHint { key: "Geometry From", label: "Imported: read a Gmsh .msh or Abaqus .inp mesh; its set and surface names become the edges" },
        help::KeyHint { key: "Support / Load edges", label: "Rectangle: bottom right top left; polygon: edge1..; holes: hole1..; 3D ends: start, end" },
    ];
    let material_lookup: &[help::KeyHint] = &[
        help::KeyHint { key: "Type", label: "Search by name, alloy, temper, spec or table; every word must match" },
        help::KeyHint { key: "Up/Down, PgUp/PgDn", label: "Move through the list; Home/End jump" },
        help::KeyHint { key: "Left/Right", label: "Change the material group filter" },
        help::KeyHint { key: "Ctrl+B / Ctrl+S / Ctrl+R", label: "Cycle the statistical basis / sort key / reverse the sort" },
        help::KeyHint { key: "Enter / Ctrl+O", label: "Mark the highlighted material for comparison (up to four); Ctrl+X clears the marks" },
        help::KeyHint { key: "F2 / Ctrl+V", label: "Show the side-by-side comparison" },
        help::KeyHint { key: "F3 / Ctrl+E", label: "Export the list and the comparison as a text report" },
        help::KeyHint { key: "Ctrl+PgUp/PgDn", label: "Scroll the property panel" },
    ];
    let sections: [(&str, &[help::KeyHint]); 10] = [
        ("Global", global),
        ("Search Files", search),
        ("Fastener Holes", fastener_hole),
        ("Bushing Workbench", bushing),
        ("Pressure Vessel Analyzer", pressure_vessel),
        ("Preload Analysis", preload_analysis),
        ("Eccentric Bushing", eccentric),
        ("Lug Analysis", lug_analysis),
        ("FEA Workbench", fea_workbench),
        ("Material Lookup", material_lookup),
    ];
    help::render_overlay(frame, area, &state.theme, &sections);
}

fn draw_confirm(frame: &mut Frame, area: Rect, state: &AppState, dialog: &ConfirmDialog, regions: &mut MouseRegions) {
    // Sized from the wrapped message so a long message is never truncated.
    let width = area.width.min(64).max(area.width.min(20));
    let inner_width = width.saturating_sub(2).max(1);
    let message_rows = hint_panel::wrapped_height(&dialog.message, inner_width);
    let height = (message_rows + 2 + 2).min(area.height); // message + blank + buttons + borders
    let popup = Rect {
        x: area.x + area.width.saturating_sub(width) / 2,
        y: area.y + area.height.saturating_sub(height) / 2,
        width,
        height,
    };
    frame.render_widget(Clear, popup);
    let block = Block::default()
        .borders(Borders::ALL)
        .border_type(BorderType::Rounded)
        .border_style(state.theme.border_style(true))
        .title(format!(" {} ", dialog.title));
    let inner = block.inner(popup);
    frame.render_widget(block, popup);
    const BUTTON_LINE: &str = "y confirm   n cancel";
    const YES_LABEL: &str = "y confirm";
    const NO_LABEL: &str = "n cancel";
    let button_row = inner.y + message_rows + 1;
    let message_area = Rect { height: message_rows.min(inner.height), ..inner };
    frame.render_widget(Paragraph::new(dialog.message.as_str()).wrap(Wrap { trim: false }), message_area);
    if button_row < inner.y + inner.height {
        let buttons = Rect { y: button_row, height: 1, ..inner };
        frame.render_widget(Paragraph::new(Span::styled(BUTTON_LINE, state.theme.disabled_style())), buttons);
    }

    // Button rects derived from the literal button line rather than
    // hardcoded column numbers, so they can never silently drift from what
    // was actually painted if that string is ever edited.
    if button_row < inner.y + inner.height && inner.width > 0 {
        if let Some(yes_col) = BUTTON_LINE.find(YES_LABEL) {
            regions.confirm_yes =
                Some(Rect { x: inner.x + yes_col as u16, y: button_row, width: YES_LABEL.len() as u16, height: 1 });
        }
        if let Some(no_col) = BUTTON_LINE.find(NO_LABEL) {
            regions.confirm_no =
                Some(Rect { x: inner.x + no_col as u16, y: button_row, width: NO_LABEL.len() as u16, height: 1 });
        }
    }
}

fn draw_toasts(frame: &mut Frame, area: Rect, state: &AppState) {
    let toasts = state.notifications.visible();
    if toasts.is_empty() || area.height == 0 {
        return;
    }
    let width = 60u16.min(area.width);
    if width == 0 {
        return;
    }
    // Newest first, stacked up from the bottom-right corner; each toast is
    // wrapped to its full height instead of being cut to one 40-column row.
    let mut bottom = area.height;
    for toast in toasts.iter().rev() {
        let rows = hint_panel::wrapped_height(&toast.message, width).min(bottom);
        if rows == 0 {
            break;
        }
        let rect = Rect { x: area.width.saturating_sub(width), y: bottom - rows, width, height: rows };
        frame.render_widget(Clear, rect);
        frame.render_widget(
            Paragraph::new(Span::styled(toast.message.clone(), state.theme.status_style(toast.tone))).wrap(Wrap { trim: false }),
            rect,
        );
        bottom -= rows;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::Terminal;
    use ratatui::backend::TestBackend;

    fn screen_text(terminal: &Terminal<TestBackend>) -> String {
        let buf = terminal.backend().buffer();
        (0..buf.area.height)
            .map(|y| (0..buf.area.width).map(|x| buf[(x, y)].symbol().to_string()).collect::<String>())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn a_long_toast_is_wrapped_not_truncated() {
        let mut state = AppState::default();
        let message = "Index build: 3 file(s) failed - e.g. C:/some/deeply/nested/folder/report.pdf: IndexError: Failed to open file for write";
        state.notifications.push(message, StatusTone::Warning);
        let mut terminal = Terminal::new(TestBackend::new(100, 20)).unwrap();
        terminal.draw(|f| draw_toasts(f, f.area(), &state)).unwrap();
        let text = screen_text(&terminal);
        let squashed: String = text.split_whitespace().collect();
        let expected: String = message.split_whitespace().collect();
        assert!(squashed.contains(&expected), "full toast text must be visible:\n{text}");
    }

    #[test]
    fn a_long_confirm_message_is_wrapped_and_buttons_stay_visible() {
        let mut state = AppState::default();
        let dialog = ConfirmDialog {
            title: "Fast index is out of date".into(),
            message: "1234 new/changed, 56 removed file(s) since the index was built. Update the index now?".into(),
            on_confirm: crate::modal::ConfirmAction::UpdateIndex,
        };
        state.modal = ModalState::Confirm(dialog.clone());
        let mut regions = MouseRegions::default();
        let mut terminal = Terminal::new(TestBackend::new(50, 20)).unwrap();
        terminal.draw(|f| draw_confirm(f, f.area(), &state, &dialog, &mut regions)).unwrap();
        let text = screen_text(&terminal);
        assert!(text.contains("Update the index now?"), "{text}");
        assert!(text.contains("y confirm"), "{text}");
        assert!(regions.confirm_yes.is_some() && regions.confirm_no.is_some());
    }
}
