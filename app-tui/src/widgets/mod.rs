//! Global, toolbox-agnostic chrome widgets. Every function here takes only
//! plain parameters plus `&crate::theme::Theme` - never `AppState`, never a
//! toolbox type - so any future toolbox can reuse them.

pub mod empty_state;
pub mod gauge_row;
pub mod help;
pub mod hint_panel;
pub mod scroll_list;
pub mod shell;
pub mod spinner;
