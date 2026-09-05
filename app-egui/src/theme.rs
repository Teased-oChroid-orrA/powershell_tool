//! Color tokens ported 1:1 from `app/src/main.rs`'s CSS custom properties
//! (the `:root { --bg: ...; }` block) - same dark palette, no glass/blur
//! (egui has no compositor blur by default; this deliberately does not
//! try to fake the CSS `--glass`/backdrop-filter look, matching the
//! flatter aesthetic already called out in the approved mockup).

use eframe::egui::{Color32, Rounding, Visuals};

use crate::design::radii;

pub struct Tokens {
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

    /// Light variant is not yet ported (the real app's `--*-light` block
    /// in `main.rs` has its own full set) - flagged as a known gap rather
    /// than guessed. Dark is this app's default theme (`AppState`'s own
    /// `dark_theme` persisted default is `true`), so it's the only one
    /// that blocks Stage 2 from being useful to look at.
    pub const LIGHT: Tokens = Tokens::DARK;

    pub fn visuals(&self) -> Visuals {
        let mut v = Visuals::dark();
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
