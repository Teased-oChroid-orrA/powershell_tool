//! Spacing scale (Design System Epic Phase 1) - a straight port of the
//! epic's 4/8/12/16/20/24/32/40/48/64 scale. Existing call sites already
//! use several of these values ad hoc (`card`'s 16px inner margin,
//! `side_by_side`'s 14px gutter, `num_field`'s 7px/3px gaps) - this module
//! names the scale for new Phase 2+ components to build against, without
//! retrofitting every existing literal (a mechanical, low-value rename
//! with real regression risk for zero visual change - not worth doing to
//! every call site that already matches a step in this scale by
//! coincidence rather than by reference).

#![allow(dead_code)]

pub const XS: f32 = 4.0;
pub const SM: f32 = 8.0;
pub const MD: f32 = 12.0;
pub const LG: f32 = 16.0;
pub const XL: f32 = 20.0;
pub const XXL: f32 = 24.0;
pub const XXXL: f32 = 32.0;
pub const HUGE: f32 = 40.0;
pub const XHUGE: f32 = 48.0;
pub const MASSIVE: f32 = 64.0;
