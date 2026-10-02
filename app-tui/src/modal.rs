//! Overlay state shared by the whole shell. Only one modal is ever shown
//! at a time - opening a new one replaces whatever was open, mirroring how
//! neither existing GUI head stacks its own dialogs/palettes either.

use crate::command_palette::CommandPalette;

#[derive(Debug, Clone, Default)]
pub enum ModalState {
    #[default]
    None,
    Palette(CommandPalette),
    Help,
    Confirm(ConfirmDialog),
}

impl ModalState {
    pub fn is_open(&self) -> bool {
        !matches!(self, ModalState::None)
    }

    pub fn close(&mut self) {
        *self = ModalState::None;
    }
}

/// Generic yes/no confirmation. Built now even though Search Files doesn't
/// need one yet in this phase - the plan calls for the modal *plumbing* to
/// exist ahead of the first real use (e.g. a future "cancel a running
/// search?" guard, or a later toolbox's own destructive action), rather
/// than inventing bespoke per-toolbox dialog code each time one is needed.
#[derive(Debug, Clone)]
pub struct ConfirmDialog {
    pub title: String,
    pub message: String,
    pub on_confirm: ConfirmAction,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfirmAction {
    Quit,
    /// Incrementally update the fast re-search index (a search found it stale).
    UpdateIndex,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_modal_state_is_closed() {
        assert!(!ModalState::default().is_open());
    }

    #[test]
    fn close_always_returns_to_none() {
        let mut modal = ModalState::Help;
        modal.close();
        assert!(!modal.is_open());
    }
}
