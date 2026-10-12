//! Colours and text sizes of the window chrome: a graphite neutral with one blue accent, in a light and a dark
//! palette that follow the system theme. Semantic colours (good, warning, bad) are separate from the accent.
use crate::gpu_viewer::editor::session::Tone;
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub line: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub on_accent: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
}

pub fn palette(dark: bool) -> Palette {
    let rgb = |r, g, b| Color32::from_rgb(r, g, b);
    if dark {
        Palette {
            bg: rgb(14, 19, 24),
            panel: rgb(26, 33, 40),
            raised: rgb(34, 42, 51),
            line: rgb(44, 54, 64),
            text: rgb(225, 232, 238),
            dim: rgb(134, 151, 166),
            accent: rgb(110, 162, 255),
            on_accent: rgb(11, 17, 23),
            good: rgb(68, 201, 143),
            warn: rgb(240, 180, 58),
            bad: rgb(255, 106, 82),
        }
    } else {
        Palette {
            bg: rgb(223, 228, 233),
            panel: rgb(247, 248, 249),
            raised: rgb(238, 240, 242),
            line: rgb(211, 217, 223),
            text: rgb(28, 37, 48),
            dim: rgb(106, 120, 134),
            accent: rgb(47, 111, 235),
            on_accent: rgb(255, 255, 255),
            good: rgb(27, 127, 75),
            warn: rgb(180, 84, 10),
            bad: rgb(200, 52, 30),
        }
    }
}

pub fn tone_color(p: &Palette, tone: Tone) -> Color32 {
    match tone {
        Tone::Title => p.accent,
        Tone::Normal => p.text,
        Tone::Dim => p.dim,
        Tone::On => p.good,
        Tone::Off => p.dim,
        Tone::Warn => p.warn,
    }
}

fn visuals(dark: bool) -> egui::Visuals {
    let p = palette(dark);
    let mut v = if dark { egui::Visuals::dark() } else { egui::Visuals::light() };
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.raised;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = egui::Shadow { offset: [0, 6], blur: 22, spread: 0, color: Color32::from_black_alpha(if dark { 110 } else { 46 }) };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = p.accent.gamma_multiply(0.30);
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.accent;
    v.override_text_color = Some(p.text);
    let r = CornerRadius::same(6);
    let w = &mut v.widgets;
    w.noninteractive.bg_fill = p.panel;
    w.noninteractive.bg_stroke = Stroke::new(1.0, p.line);
    w.noninteractive.fg_stroke = Stroke::new(1.0, p.text);
    w.inactive.weak_bg_fill = Color32::TRANSPARENT;
    w.inactive.bg_fill = p.raised;
    w.inactive.bg_stroke = Stroke::NONE;
    w.inactive.fg_stroke = Stroke::new(1.0, p.text);
    w.hovered.weak_bg_fill = p.raised;
    w.hovered.bg_fill = p.raised;
    w.hovered.bg_stroke = Stroke::new(1.0, p.line);
    w.hovered.fg_stroke = Stroke::new(1.0, p.text);
    w.active.weak_bg_fill = p.accent.gamma_multiply(0.28);
    w.active.bg_fill = p.accent.gamma_multiply(0.28);
    w.active.bg_stroke = Stroke::new(1.0, p.accent);
    w.active.fg_stroke = Stroke::new(1.0, p.text);
    w.open.weak_bg_fill = p.raised;
    w.open.bg_fill = p.raised;
    w.open.bg_stroke = Stroke::new(1.0, p.line);
    for s in [&mut w.noninteractive, &mut w.inactive, &mut w.hovered, &mut w.active, &mut w.open] {
        s.corner_radius = r;
        s.expansion = 0.0;
    }
    v
}

/// Install both palettes and the text sizes; the theme preference stays the system one.
pub fn install(ctx: &egui::Context) {
    ctx.style_mut_of(egui::Theme::Dark, |s| s.visuals = visuals(true));
    ctx.style_mut_of(egui::Theme::Light, |s| s.visuals = visuals(false));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (TextStyle::Small, FontId::new(11.0, FontFamily::Proportional)),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Button, FontId::new(13.0, FontFamily::Proportional)),
            (TextStyle::Heading, FontId::new(16.0, FontFamily::Proportional)),
            (TextStyle::Monospace, FontId::new(12.0, FontFamily::Monospace)),
        ]
        .into();
        s.spacing.item_spacing = egui::vec2(8.0, 5.0);
        s.spacing.button_padding = egui::vec2(9.0, 4.0);
        s.spacing.menu_margin = egui::Margin::same(6);
        s.spacing.interact_size.y = 24.0;
    });
}

/// The palette of the theme in use this frame.
pub fn current(ctx: &egui::Context) -> Palette {
    palette(ctx.theme() == egui::Theme::Dark)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Relative luminance contrast ratio of two colours.
    fn contrast(a: Color32, b: Color32) -> f32 {
        let l = |c: Color32| {
            let f = |v: u8| {
                let v = v as f32 / 255.0;
                if v <= 0.03928 { v / 12.92 } else { ((v + 0.055) / 1.055).powf(2.4) }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        };
        let (x, y) = (l(a), l(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    #[test]
    fn text_stays_readable_on_every_surface_in_both_themes() {
        for dark in [false, true] {
            let p = palette(dark);
            for surface in [p.bg, p.panel, p.raised] {
                assert!(contrast(p.text, surface) >= 7.0, "text on a surface, dark={dark}");
                assert!(contrast(p.dim, surface) >= 3.4, "secondary text, dark={dark}: {}", contrast(p.dim, surface));
                assert!(contrast(p.accent, surface) >= 3.0, "accent, dark={dark}");
                assert!(contrast(p.warn, surface) >= 3.0 && contrast(p.good, surface) >= 3.0, "semantic colours, dark={dark}");
            }
            assert!(contrast(p.on_accent, p.accent) >= 3.0, "text on a filled accent, dark={dark}");
        }
    }
}
