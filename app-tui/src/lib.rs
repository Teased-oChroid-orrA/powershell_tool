//! Library crate for the ratatui terminal head. Splitting a `lib.rs` out
//! from `main.rs` (a thin binary that just does terminal lifecycle + calls
//! into here) lets every module's `#[cfg(test)]` unit tests run via
//! `cargo test -p app-tui --lib` without needing a live terminal - the
//! plan's "model layer must be fully headless-testable" rule applies to
//! the whole crate, not just the toolbox model modules.

pub mod app;
pub mod command_palette;
pub mod format;
pub mod modal;
pub mod nav;
pub mod notifications;
pub mod theme;
pub mod toolboxes;
pub mod widgets;
