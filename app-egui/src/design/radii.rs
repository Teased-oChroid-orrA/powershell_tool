//! Radius scale (Design System Epic Phase 1) - the epic's 0/4/6/8/12/16/999
//! scale. `999` (epic's "fully round" token, e.g. pill buttons/avatars) is
//! represented as `PILL` using a concretely large value - `egui::Rounding`
//! is stored as `f32` corner radii, not a percentage, so "999" itself would
//! over-round anything wider than ~2000px; any value at least half an
//! ordinary widget's height fully rounds its ends the same way.
//!
//! Applied to the two existing hardcoded rounding values this pass could
//! change with zero visual difference (they already equal a scale step):
//! `theme.rs`'s global field rounding (was a bare `6.0.into()`) and
//! `widgets.rs`'s `nav_item` active-state rounding (was a bare `6.0`).
//! `card`'s `8.0` and `stepper`'s `13.0` (a deliberate half-height pill,
//! not a scale step) are left as-is - see each site's own comment.

#![allow(dead_code)]

use eframe::egui::Rounding;

pub const NONE: f32 = 0.0;
pub const SM: f32 = 4.0;
pub const MD: f32 = 6.0;
pub const LG: f32 = 8.0;
pub const XL: f32 = 12.0;
pub const XXL: f32 = 16.0;
pub const PILL: f32 = 999.0;

pub fn sm() -> Rounding {
    SM.into()
}
pub fn md() -> Rounding {
    MD.into()
}
pub fn lg() -> Rounding {
    LG.into()
}
pub fn xl() -> Rounding {
    XL.into()
}
