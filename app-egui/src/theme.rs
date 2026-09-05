//! Color tokens ported 1:1 from `app/src/main.rs`'s CSS custom properties
//! (the `:root { --bg: ...; }` block) - same dark palette, no glass/blur
//! (egui has no compositor blur by default; this deliberately does not
//! try to fake the CSS `--glass`/backdrop-filter look, matching the
//! flatter aesthetic already called out in the approved mockup).

use eframe::egui::{Color32, Rounding, Visuals};

use crate::design::radii;

pub struct Tokens {
    /// Selects `Visuals::dark()` vs `Visuals::light()` as `visuals()`'s
    /// base - egui's own base visuals differ in more than color (e.g.
    /// default shadow tint, text-selection contrast handling), so picking
    /// the wrong base would leave those un-overridden details backwards
    /// even with every color field above set correctly for the theme.
    pub dark_mode: bool,
    pub bg: Color32,
    pub bg_raised: Color32,
    pub bg_sunken: Color32,
    pub fg: Color32,
    pub fg_muted: Color32,
    pub fg_subtle: Color32,
    pub border: Color32,
    pub border_strong: Color32,
    pub accent: Color32,
    pub accent_strong: Color32,
    pub accent_fg: Color32,
    pub good: Color32,
    pub good_bg: Color32,
    pub warning: Color32,
    pub warning_bg: Color32,
    pub danger: Color32,
    pub danger_bg: Color32,
}

impl Tokens {
    pub const DARK: Tokens = Tokens {
        dark_mode: true,
        bg: Color32::from_rgb(0x14, 0x16, 0x1b),
        bg_raised: Color32::from_rgb(0x1b, 0x1e, 0x25),
        bg_sunken: Color32::from_rgb(0x0e, 0x10, 0x13),
        fg: Color32::from_rgb(0xee, 0xf0, 0xf4),
        fg_muted: Color32::from_rgb(0x8d, 0x96, 0xa3),
        fg_subtle: Color32::from_rgb(0x62, 0x6a, 0x76),
        border: Color32::from_rgb(0x2b, 0x30, 0x3a),
        border_strong: Color32::from_rgb(0x3c, 0x42, 0x4e),
        accent: Color32::from_rgb(0x3f, 0xbf, 0xe8),
        accent_strong: Color32::from_rgb(0x6a, 0xd2, 0xf2),
        accent_fg: Color32::from_rgb(0x05, 0x22, 0x2c),
        good: Color32::from_rgb(0x52, 0xc9, 0x8a),
        good_bg: Color32::from_rgb(0x17, 0x3a, 0x2a),
        warning: Color32::from_rgb(0xe0, 0xb3, 0x55),
        warning_bg: Color32::from_rgb(0x3a, 0x2f, 0x16), // `--warning-bg` in the approved artifact's CSS
        danger: Color32::from_rgb(0xe2, 0x65, 0x7a),
        danger_bg: Color32::from_rgb(0x3a, 0x1f, 0x24),
    };

    /// Design System Epic Phase 3: a real, distinct light palette - not
    /// the dark colors left as-is (that placeholder shipped for several
    /// phases; every color below is chosen for light-background
    /// contrast, not copied from `DARK`). `accent`/`accent_strong` are
    /// deliberately darker/more saturated than dark mode's bright cyan -
    /// `DARK`'s `0x3fbfe8` fails contrast against a near-white
    /// background (checked against WCAG AA's ~4.5:1 text-contrast
    /// guideline, not just eyeballed), so light mode uses a deeper teal
    /// that still reads as "the same brand accent," not a different hue.
    pub const LIGHT: Tokens = Tokens {
        dark_mode: false,
        bg: Color32::from_rgb(0xf6, 0xf8, 0xfa),
        bg_raised: Color32::from_rgb(0xff, 0xff, 0xff),
        bg_sunken: Color32::from_rgb(0xec, 0xf0, 0xf3),
        fg: Color32::from_rgb(0x16, 0x1a, 0x1f),
        fg_muted: Color32::from_rgb(0x51, 0x59, 0x62),
        fg_subtle: Color32::from_rgb(0x83, 0x8b, 0x94),
        border: Color32::from_rgb(0xd6, 0xdc, 0xe2),
        border_strong: Color32::from_rgb(0xb8, 0xc1, 0xca),
        accent: Color32::from_rgb(0x0b, 0x7d, 0xa1),
        accent_strong: Color32::from_rgb(0x08, 0x63, 0x82),
        accent_fg: Color32::from_rgb(0xff, 0xff, 0xff),
        good: Color32::from_rgb(0x1d, 0x82, 0x52),
        good_bg: Color32::from_rgb(0xdd, 0xf2, 0xe6),
        warning: Color32::from_rgb(0x8f, 0x64, 0x09),
        warning_bg: Color32::from_rgb(0xfa, 0xec, 0xcf),
        danger: Color32::from_rgb(0xb8, 0x2e, 0x49),
        danger_bg: Color32::from_rgb(0xf9, 0xdf, 0xe3),
    };

    pub fn visuals(&self) -> Visuals {
        let mut v = if self.dark_mode { Visuals::dark() } else { Visuals::light() };
        v.override_text_color = Some(self.fg);
        v.panel_fill = self.bg_raised;
        v.window_fill = self.bg_raised;
        v.extreme_bg_color = self.bg_sunken;
        v.faint_bg_color = self.bg_sunken;
        v.hyperlink_color = self.accent;
        v.selection.bg_fill = self.accent.gamma_multiply(0.35);
        v.selection.stroke.color = self.accent_strong;
        v.widgets.noninteractive.bg_fill = self.bg_raised;
        v.widgets.noninteractive.bg_stroke.color = self.border;
        v.widgets.noninteractive.fg_stroke.color = self.fg_muted;
        v.widgets.inactive.bg_fill = self.bg_sunken;
        v.widgets.inactive.bg_stroke.color = self.border_strong;
        v.widgets.inactive.fg_stroke.color = self.fg;
        v.widgets.hovered.bg_fill = self.border;
        v.widgets.hovered.bg_stroke.color = self.accent;
        v.widgets.hovered.fg_stroke.color = self.fg;
        v.widgets.active.bg_fill = self.accent.gamma_multiply(0.25);
        v.widgets.active.bg_stroke.color = self.accent;
        v.widgets.active.fg_stroke.color = self.accent_strong;
        v.window_stroke.color = self.border;
        v.window_rounding = 8.0.into();
        v.menu_rounding = 8.0.into();
        // The command palette (a `Window`) and every `ComboBox`/context
        // menu popup get this app's own named elevation instead of
        // egui's built-in default shadow - same `design::shadows` scale
        // `widgets::card` uses, so overlay content and card content read
        // as one consistent elevation system rather than two.
        v.window_shadow = crate::design::shadows::overlay();
        v.popup_shadow = crate::design::shadows::overlay();
        // `.field input, .field select { border-radius: 6px }` in the
        // approved mockup CSS - egui's own default widget rounding (2px)
        // read as flat/unstyled boxes next to this app's 6-8px card/
        // button rounding elsewhere, a real user-reported "ugly, doesn't
        // match" gap across every text/number input in every tool, not
        // just one. Applied globally here (not per-widget) so every
        // `DragValue`/`TextEdit`/`ComboBox`/checkbox in the app picks it
        // up automatically instead of needing a style override at each
        // of the (many) call sites.
        let field_rounding: Rounding = radii::md();
        v.widgets.inactive.rounding = field_rounding;
        v.widgets.hovered.rounding = field_rounding;
        v.widgets.active.rounding = field_rounding;
        v.widgets.open.rounding = field_rounding;
        v
    }
}
