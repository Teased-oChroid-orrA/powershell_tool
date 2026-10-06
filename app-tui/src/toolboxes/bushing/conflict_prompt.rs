//! Import-conflict prompt shared by the reamer, material and Bushing ID
//! pickers: they differ only in the title and the two description lines.

use crate::app::Effect;
use crate::library::{ConflictQueue, ConflictResolution, LibraryItem};
use crate::theme::Theme;
use crossterm::event::{KeyCode, KeyEvent};
use ratatui::layout::Rect;
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Borders, Paragraph, Wrap};
use ratatui::Frame;

/// `k`/`o` resolve the current conflict, `z`/`a` the current and all
/// remaining. Same action either case on every letter (Windows Caps-Lock can
/// report an uppercase letter with no Shift held - see `app-tui/AGENTS.md`'s
/// Pitfalls).
pub fn handle_key<T: Clone>(queue: &mut ConflictQueue<T>, library: &mut [LibraryItem<T>], key: KeyEvent) -> (bool, Vec<Effect>) {
    let resolved = match key.code {
        KeyCode::Char('k' | 'K') => Some((ConflictResolution::KeepExisting, false)),
        KeyCode::Char('o' | 'O') => Some((ConflictResolution::Overwrite, false)),
        KeyCode::Char('z' | 'Z') => Some((ConflictResolution::KeepExisting, true)),
        KeyCode::Char('a' | 'A') => Some((ConflictResolution::Overwrite, true)),
        _ => None,
    };
    let Some((resolution, apply_all)) = resolved else {
        return (false, Vec::new());
    };
    queue.resolve(library, resolution, apply_all);
    (true, Vec::new())
}

/// `describe` returns the (headline, "Imported: ..." detail) lines for the
/// incoming item of the current conflict.
pub fn render<T: Clone>(frame: &mut Frame, area: Rect, theme: &Theme, title: &str, queue: &ConflictQueue<T>, describe: impl Fn(&T) -> (String, String)) {
    let block = Block::default().borders(Borders::ALL).border_type(BorderType::Rounded).border_style(theme.border_style(true)).title(format!(" {title} "));
    let inner = block.inner(area);
    frame.render_widget(block, area);
    if inner.width == 0 || inner.height == 0 {
        return;
    }
    let Some((_, incoming)) = queue.current() else { return };
    let (headline, detail) = describe(&incoming.item);
    let lines = vec![
        Line::from(headline),
        Line::from(""),
        Line::from(detail),
        Line::from(""),
        Line::from(Span::styled("k: keep existing    o: overwrite    z: keep all remaining    a: overwrite all remaining", theme.disabled_style())),
    ];
    frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: true }), inner);
}
