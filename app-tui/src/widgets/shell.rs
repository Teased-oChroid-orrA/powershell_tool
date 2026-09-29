//! Top-level layout composition: chrome (topbar/rail/status bar) plus the
//! active toolbox's workspace, then overlays drawn last. This is the one
//! place that knows the full z-order; individual widgets don't know about
//! each other.

use ratatui::Frame;
use ratatui::layout::{Alignment, Constraint, Direction, Layout, Rect};
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Clear, Paragraph};

use crate::app::AppState;
use crate::command_palette;
use crate::modal::{ConfirmDialog, ModalState};
use crate::mouse::MouseRegions;
use crate::nav::{Breakpoint, FocusArea, ToolId, pick_breakpoint};
use crate::theme::StatusTone;
use crate::widgets::{empty_state, help, spinner};

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
        ModalState::Palette(palette) => command_palette::render(frame, area, &state.theme, palette, regions),
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
        _ => empty_state::render(frame, area, &state.theme, "Coming soon", None),
    }
}

fn draw_status_bar(frame: &mut Frame, area: Rect, state: &AppState, tick: u64) {
    let mut hints = vec![
        help::KeyHint { key: "Ctrl+P", label: "Commands" },
        help::KeyHint { key: "Tab", label: "Focus" },
    ];
    if let Some(hint) = contextual_hint(state) {
        hints.push(hint);
    }
    hints.push(help::KeyHint { key: "?", label: "Help" });
    hints.push(help::KeyHint { key: "q", label: "Quit" });
    let layout = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Min(1), Constraint::Length(10)])
        .split(area);
    help::render_status_hints(frame, layout[0], &state.theme, &hints);

    let status = if state.is_busy() {
        Line::from(vec![spinner::spinner_span(tick, &state.theme, StatusTone::Info), Span::raw(" Running")])
    } else {
        Line::from(Span::styled("Ready", state.theme.status_style(StatusTone::Success)))
    };
    frame.render_widget(Paragraph::new(status).alignment(Alignment::Right), layout[1]);
}

/// One extra, always-visible status-bar hint for whatever single
/// non-obvious key actually does something right now - e.g. `s` opening
/// Settings was previously discoverable only via the `?` help overlay (a
/// separate modal you had to already know to open), with no hint on the
/// always-visible chrome that the binding existed at all. Toolbox-specific
/// (only `Search` has any of these bindings today) and pane/screen-aware,
/// so it never claims a key does something it won't actually do given the
/// current focus/screen.
fn contextual_hint(state: &AppState) -> Option<help::KeyHint> {
    match state.nav.active_tool {
        ToolId::Search => match state.search.screen {
            crate::toolboxes::search::ToolboxScreen::Run => {
                let results_focused =
                    matches!(state.focus.area, FocusArea::Workspace(n) if n == crate::toolboxes::search::PANE_RESULTS);
                results_focused.then_some(help::KeyHint { key: "s", label: "Settings" })
            }
            crate::toolboxes::search::ToolboxScreen::Settings => Some(help::KeyHint { key: "Esc", label: "Back" }),
        },
        // `e` (export report) is the one binding on this toolbox with no
        // on-screen affordance elsewhere - discoverable only via `?`
        // otherwise, same reasoning as Search's own `s Settings` hint above.
        // Only shown while the workspace itself has focus and no overlay
        // is covering it, so the hint never claims a key does something it
        // won't actually do right now.
        ToolId::PressureVessel if !state.pressure_vessel.material_picker.open && matches!(state.focus.area, FocusArea::Workspace(_)) => {
            Some(help::KeyHint { key: "e", label: "Export" })
        }
        ToolId::Bushing
            if !state.bushing.material_picker.open && !state.bushing.reamer_picker.open && matches!(state.focus.area, FocusArea::Workspace(_)) =>
        {
            Some(help::KeyHint { key: "e", label: "Export" })
        }
        ToolId::PreloadAnalysis if matches!(state.focus.area, FocusArea::Workspace(_)) => Some(help::KeyHint { key: "e", label: "Export" }),
        _ => None,
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
    let sections: [(&str, &[help::KeyHint]); 4] =
        [("Global", global), ("Search Files", search), ("Fastener Holes", fastener_hole), ("Pressure Vessel Analyzer", pressure_vessel)];
    help::render_overlay(frame, area, &state.theme, &sections);
}

fn draw_confirm(frame: &mut Frame, area: Rect, state: &AppState, dialog: &ConfirmDialog, regions: &mut MouseRegions) {
    let popup = command_palette::centered_rect(40, 20, area);
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
    let text = Paragraph::new(vec![
        Line::from(dialog.message.as_str()),
        Line::from(""),
        Line::from(Span::styled(BUTTON_LINE, state.theme.disabled_style())),
    ]);
    frame.render_widget(text, inner);

    // Button rects derived from the literal button line rather than
    // hardcoded column numbers, so they can never silently drift from what
    // was actually painted if that string is ever edited.
    if inner.height >= 3 && inner.width > 0 {
        let button_row = inner.y + 2;
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
    let height = (toasts.len() as u16).min(area.height);
    let width = 40u16.min(area.width);
    if width == 0 {
        return;
    }
    let toast_area = Rect {
        x: area.width.saturating_sub(width),
        y: area.height.saturating_sub(height),
        width,
        height,
    };
    let lines: Vec<Line> = toasts
        .iter()
        .rev()
        .take(height as usize)
        .map(|t| Line::from(Span::styled(t.message.clone(), state.theme.status_style(t.tone))))
        .collect();
    frame.render_widget(Clear, toast_area);
    frame.render_widget(Paragraph::new(lines), toast_area);
}
