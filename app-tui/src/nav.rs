//! Toolbox registry and focus tracking. A plain enum + match, not a trait
//! object registry - matches how both existing GUI heads (`app/`,
//! `app-egui/`) still switch tools years into their own toolbox counts. A
//! `Toolbox` trait is worth adding only once a second real toolbox is
//! actually being migrated and the hardcoded match starts hurting.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ToolId {
    Search,
    FastenerHole,
    Bushing,
    LugAnalysis,
    EccentricBushing,
    FeaWorkbench,
    PressureVessel,
    PreloadAnalysis,
    MaterialLookup,
    Dupes,
    Rename,
    Logs,
}

impl ToolId {
    /// Deliberately excludes `app-egui`'s `StressSolver` - a documented,
    /// permanent exclusion (its PINN/AMR dependency would force the same
    /// `windows`-crate workspace-exclusion problem `app-egui` has, see
    /// `app-tui/AGENTS.md`), not an oversight like the other three were.
    /// `FastenerHole`/`PreloadAnalysis` have no equivalent in either
    /// existing GUI head - both are toolboxes unique to this crate, not
    /// ported placeholders.
    pub const ALL: [ToolId; 12] = [
        ToolId::Search,
        ToolId::FastenerHole,
        ToolId::Bushing,
        ToolId::EccentricBushing,
        ToolId::LugAnalysis,
        ToolId::FeaWorkbench,
        ToolId::PressureVessel,
        ToolId::PreloadAnalysis,
        ToolId::MaterialLookup,
        ToolId::Dupes,
        ToolId::Rename,
        ToolId::Logs,
    ];

    pub fn title(self) -> &'static str {
        match self {
            ToolId::Search => "Search Files",
            ToolId::FastenerHole => "Fastener Holes",
            ToolId::Bushing => "Bushing Workbench",
            ToolId::EccentricBushing => "Eccentric Bushing",
            ToolId::LugAnalysis => "Lug Analysis",
            ToolId::FeaWorkbench => "FEA Workbench",
            ToolId::PressureVessel => "Pressure Vessel Analyzer",
            ToolId::PreloadAnalysis => "Preload Analysis",
            ToolId::MaterialLookup => "Material Lookup",
            ToolId::Dupes => "Duplicate Finder",
            ToolId::Rename => "Batch Rename",
            ToolId::Logs => "Log Analyzer",
        }
    }

    /// Search Files, Fastener Holes, Bushing Workbench, Pressure Vessel
    /// Analyzer, and Preload Analysis are migrated - Dupes/Rename/Logs
    /// still render as dimmed "Soon" rail entries, mirroring both existing
    /// heads' own inert-placeholder convention for their own not-yet-built
    /// tools.
    pub fn enabled(self) -> bool {
        !matches!(self, ToolId::Dupes | ToolId::Rename | ToolId::Logs)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationState {
    pub active_tool: ToolId,
}

impl Default for NavigationState {
    fn default() -> Self {
        Self { active_tool: ToolId::Search }
    }
}

impl NavigationState {
    pub fn activate(&mut self, tool: ToolId) {
        if tool.enabled() {
            self.active_tool = tool;
        }
    }
}

/// Which pane currently has keyboard focus. Toolbox-specific meaning is
/// assigned by the toolbox itself (Search Files decides what pane 0 vs.
/// pane 1 means) - kept as a small generic cursor rather than a per-toolbox
/// enum so the shell doesn't need to know every toolbox's internal layout.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FocusArea {
    Rail,
    Workspace(u8),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FocusState {
    pub area: FocusArea,
}

impl Default for FocusState {
    fn default() -> Self {
        // Start on the rail, not inside the active toolbox's first pane -
        // several toolboxes (Search Files included) put a text-entry field
        // at workspace pane 0, and defaulting keyboard focus straight into
        // a text field would silently swallow the very first keypress
        // (e.g. "q" typed to quit) as a character instead of a global
        // shortcut. Starting on the rail keeps every global binding live
        // immediately on launch; Tab moves into the workspace from there.
        Self { area: FocusArea::Rail }
    }
}

impl FocusState {
    pub fn cycle_forward(&mut self, workspace_pane_count: u8) {
        self.area = match self.area {
            FocusArea::Rail => FocusArea::Workspace(0),
            FocusArea::Workspace(n) if n + 1 < workspace_pane_count => FocusArea::Workspace(n + 1),
            FocusArea::Workspace(_) => FocusArea::Rail,
        };
    }

    pub fn cycle_backward(&mut self, workspace_pane_count: u8) {
        self.area = match self.area {
            FocusArea::Rail => FocusArea::Workspace(workspace_pane_count.saturating_sub(1)),
            FocusArea::Workspace(0) => FocusArea::Rail,
            FocusArea::Workspace(n) => FocusArea::Workspace(n - 1),
        };
    }
}

/// Responsive layout breakpoint, chosen purely from terminal width - a pure
/// function, unit-testable without a real terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Breakpoint {
    Narrow,
    Medium,
    Wide,
}

pub fn pick_breakpoint(width: u16) -> Breakpoint {
    if width >= 100 {
        Breakpoint::Wide
    } else if width >= 70 {
        Breakpoint::Medium
    } else {
        Breakpoint::Narrow
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_tool_has_a_distinct_title_and_the_migrated_toolboxes_are_enabled() {
        // Regression guard for the rail once genuinely missing three
        // entries (`Dupes`/`Rename`/`Logs`) that both `app/` and
        // `app-egui/` show as inert placeholders - `ALL` must list every
        // variant, `enabled()` must gate them correctly.
        assert_eq!(ToolId::ALL.len(), 12);
        let titles: Vec<&str> = ToolId::ALL.iter().map(|t| t.title()).collect();
        let mut distinct = titles.clone();
        distinct.sort_unstable();
        distinct.dedup();
        assert_eq!(distinct.len(), titles.len(), "every ToolId must have a distinct title: {titles:?}");
        for tool in ToolId::ALL {
            let expected = !matches!(tool, ToolId::Dupes | ToolId::Rename | ToolId::Logs);
            assert_eq!(tool.enabled(), expected, "{tool:?} enabled state");
        }
    }

    #[test]
    fn breakpoints_match_documented_thresholds() {
        assert_eq!(pick_breakpoint(50), Breakpoint::Narrow);
        assert_eq!(pick_breakpoint(69), Breakpoint::Narrow);
        assert_eq!(pick_breakpoint(70), Breakpoint::Medium);
        assert_eq!(pick_breakpoint(99), Breakpoint::Medium);
        assert_eq!(pick_breakpoint(100), Breakpoint::Wide);
        assert_eq!(pick_breakpoint(200), Breakpoint::Wide);
    }

    #[test]
    fn navigation_ignores_disabled_tools() {
        let mut nav = NavigationState::default();
        nav.activate(ToolId::Dupes);
        assert_eq!(nav.active_tool, ToolId::Search);
    }

    #[test]
    fn focus_cycles_forward_and_wraps_through_rail() {
        let mut focus = FocusState { area: FocusArea::Rail };
        focus.cycle_forward(3);
        assert_eq!(focus.area, FocusArea::Workspace(0));
        focus.cycle_forward(3);
        assert_eq!(focus.area, FocusArea::Workspace(1));
        focus.cycle_forward(3);
        assert_eq!(focus.area, FocusArea::Workspace(2));
        focus.cycle_forward(3);
        assert_eq!(focus.area, FocusArea::Rail);
    }

    #[test]
    fn focus_cycles_backward_symmetrically() {
        let mut focus = FocusState { area: FocusArea::Rail };
        focus.cycle_backward(3);
        assert_eq!(focus.area, FocusArea::Workspace(2));
        focus.cycle_backward(3);
        assert_eq!(focus.area, FocusArea::Workspace(1));
    }
}
