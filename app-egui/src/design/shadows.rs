//! Elevation (Design System Epic Phase 1) - `egui::Frame` already has a
//! real `shadow: Shadow` field (used today for windows/popups via
//! `style.visuals.window_shadow`/`popup_shadow`); this module gives cards
//! the same treatment via named elevation presets instead of a bespoke
//! shadow value invented per call site. Two levels only (`RAISED`/
//! `OVERLAY`) - this app has no deeper stacking (no dialogs/toasts yet;
//! those land with Phase 2's component set and can reuse `OVERLAY`).

#![allow(dead_code)]

use eframe::egui::{epaint::Shadow, vec2, Color32};

/// Cards sitting flat on the page background (`widgets::card`) - a subtle
/// downward shadow, matching the approved mockup's own
/// `box-shadow: 0 1px 2px rgba(0,0,0,.3)` on `.card`.
pub fn raised() -> Shadow {
    Shadow { offset: vec2(0.0, 2.0), blur: 6.0, spread: 0.0, color: Color32::from_black_alpha(60) }
}

/// Content that floats above the page (menus, the command palette, a
/// future dialog/toast) - stronger and less directional than `raised`.
pub fn overlay() -> Shadow {
    Shadow { offset: vec2(0.0, 4.0), blur: 16.0, spread: 0.0, color: Color32::from_black_alpha(90) }
}
