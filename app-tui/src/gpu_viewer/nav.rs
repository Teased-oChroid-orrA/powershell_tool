//! Navigation inside the Toolbench dashboard window (ADR-024): what the nav strip shows and what it can ask for.
//! Pure data; `window/nav.rs` holds the parked workbenches and carries the actions out, `gui/nav_bar.rs` draws it.

/// What the person asked the nav strip (or a dashboard tile) for. Ids are `dashboard::Workbench::id`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum NavAct {
    /// Back to the dashboard; the open workbench stays parked.
    Home,
    /// Show the workbench: its tab when it is open, a new one when it is not.
    Open(&'static str),
    /// Close the tab; asks first while it holds unsaved edits or a running solve.
    Close(&'static str),
    /// Close the tab and drop its unsaved edits (the answer to the question `Close` raised).
    Discard(&'static str),
    /// The question `Close` raised was answered "keep open".
    KeepOpen,
    /// Move the shown workbench into a window of its own.
    PopOut(&'static str),
}

/// One open workbench as the strip shows it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NavTab {
    pub id: &'static str,
    pub name: &'static str,
    pub dirty: bool,
    /// A solve or other background work is running.
    pub busy: bool,
}

/// What the strip draws this frame.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NavStrip {
    /// Open workbenches in tab order.
    pub tabs: Vec<NavTab>,
    /// The tab shown; `None` is the dashboard.
    pub active: Option<&'static str>,
    /// The tab whose close is waiting for "Keep open" / "Discard and close".
    pub ask_close: Option<&'static str>,
}
