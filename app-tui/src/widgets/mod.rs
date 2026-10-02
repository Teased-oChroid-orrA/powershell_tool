//! Global, toolbox-agnostic chrome widgets. Every function here takes only
//! plain parameters plus `&crate::theme::Theme` - never `AppState`, never a
//! toolbox type - so any future toolbox can reuse them.

pub mod empty_state;
pub mod progress_bar;
pub mod help;
pub mod hint_panel;
pub mod input_line;
pub mod number_edit;
pub mod scroll_list;
pub mod scroll_paragraph;
pub mod shell;
pub mod spinner;
