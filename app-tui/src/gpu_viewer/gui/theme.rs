//! Colours and text sizes of the window chrome. The default theme is Circadian: colour temperature and light or dark follow
//! the time of day (warm dark brown and amber at night, warm paper by day, never a cool charcoal). `theme_library.rs` adds
//! six fixed palettes from published colour schemes; picking one replaces Circadian until Circadian is picked again. The
//! choice is remembered. Semantic colours (good, warning, bad) are separate from the accent.
use crate::gpu_viewer::editor::session::Tone;
use super::theme_library::LIBRARY;
use egui::{Color32, CornerRadius, FontFamily, FontId, Stroke, TextStyle};
use std::sync::atomic::{AtomicU32, Ordering};

#[derive(Clone, Copy, Debug)]
pub struct Palette {
    /// Whether this is a dark palette (it picks egui's dark or light base styles).
    pub dark: bool,
    pub bg: Color32,
    pub panel: Color32,
    pub raised: Color32,
    pub line: Color32,
    /// Sidebar and top bars: the deepest surface.
    pub side: Color32,
    pub text: Color32,
    /// Secondary text.
    pub dim: Color32,
    /// Muted text: captions, hints, separators.
    pub muted: Color32,
    pub accent: Color32,
    pub link: Color32,
    /// Selected text and the selected row.
    pub select: Color32,
    pub on_accent: Color32,
    pub good: Color32,
    pub warn: Color32,
    pub bad: Color32,
}

/// Warmth runs 0 (daylight, 6500 K) to 1 (deep amber, 1800 K); 0.5 is the designed dark palette.
pub const DEFAULT_WARMTH: f32 = 0.5;
const K_DAY: f32 = 6500.0;
const K_NIGHT: f32 = 1800.0;
/// Local hours of sunrise and sunset. There is no location in the app, so this is a fixed mid-latitude day; the preview
/// slider and the Light/Dark button cover everything else.
const SUNRISE: f32 = 6.5;
const SUNSET: f32 = 18.5;
/// Half-width of a dawn or dusk transition, in hours (the logistic scale: 90 % of the change takes about 2.6 h).
const TWILIGHT: f32 = 0.6;
/// Warmth above this is "evening": the dark palette. Below it, the light one.
const DAY_BELOW: f32 = 0.4;

/// A preview hour set by the slider; NaN follows the clock.
static MANUAL_HOUR: AtomicU32 = AtomicU32::new(f32::NAN.to_bits());

/// The chosen library theme as an index into `LIBRARY` plus one; 0 is Circadian.
static CHOICE: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);

/// The library theme in use, or `None` for Circadian.
pub fn chosen() -> Option<usize> {
    CHOICE.load(Ordering::Relaxed).checked_sub(1).filter(|i| *i < LIBRARY.len())
}

/// Use library theme `i`, or Circadian for `None`.
pub fn choose(i: Option<usize>) {
    CHOICE.store(i.filter(|i| *i < LIBRARY.len()).map_or(0, |i| i + 1), Ordering::Relaxed);
}

/// Whether the Circadian theme (not a fixed library palette) is in use.
pub fn is_circadian() -> bool {
    chosen().is_none()
}

/// Where the choice is remembered: `<app data>/fea-editor/theme` (the library id; absent means Circadian).
#[cfg(not(test))]
fn choice_path() -> Option<std::path::PathBuf> {
    crate::paths::app_data_dir().map(|d| d.join("fea-editor").join("theme"))
}

/// Read the remembered theme (nothing in tests, so a developer's own choice never changes a result).
pub fn load_saved_choice() {
    #[cfg(not(test))]
    if let Some(id) = choice_path().and_then(|p| std::fs::read_to_string(p).ok()) {
        choose(LIBRARY.iter().position(|t| t.id == id.trim()));
    }
}

fn save_choice() {
    #[cfg(not(test))]
    if let Some(path) = choice_path() {
        let _ = path.parent().map(std::fs::create_dir_all);
        let _ = crate::persistence::write(&path, format!("{}\n", chosen().map_or("circadian", |i| LIBRARY[i].id)));
    }
}

fn sigmoid(x: f32) -> f32 {
    1.0 / (1.0 + (-x).exp())
}

/// Colour temperature (kelvin) of the interface at local `hour` (0..24): daylight after sunrise, easing to amber after
/// sunset, along smooth logistic curves like the sun, not a switch.
pub fn kelvin_at(hour: f32) -> f32 {
    let sun = sigmoid((hour - SUNRISE) / TWILIGHT) - sigmoid((hour - SUNSET) / TWILIGHT);
    K_NIGHT + (K_DAY - K_NIGHT) * sun.clamp(0.0, 1.0)
}

/// Warmth (0 daylight .. 1 deep amber) at local `hour`.
pub fn warmth_at(hour: f32) -> f32 {
    ((K_DAY - kelvin_at(hour)) / (K_DAY - K_NIGHT)).clamp(0.0, 1.0)
}

/// Whether warmth `w` is daytime (the light palette).
pub fn is_day(w: f32) -> bool {
    w < DAY_BELOW
}

fn clock_hour() -> Option<f32> {
    // Tests never read the clock, so a result cannot depend on the time of day.
    #[cfg(not(test))]
    {
        use chrono::Timelike;
        let t = chrono::Local::now();
        Some(t.hour() as f32 + t.minute() as f32 / 60.0)
    }
    #[cfg(test)]
    None
}

/// The hour in use: the preview, else the clock.
pub fn hour() -> Option<f32> {
    let m = f32::from_bits(MANUAL_HOUR.load(Ordering::Relaxed));
    if m.is_nan() {
        clock_hour()
    } else {
        Some(m)
    }
}

/// Preview local `hour` (clamped to 0..24), or `None` to follow the clock again.
pub fn set_hour(hour: Option<f32>) {
    let h = hour.filter(|h| h.is_finite()).map_or(f32::NAN, |h| h.clamp(0.0, 24.0));
    MANUAL_HOUR.store(h.to_bits(), Ordering::Relaxed);
}

/// The warmth in use (the designed one when there is no clock, as in tests).
pub fn warmth() -> f32 {
    hour().map_or(DEFAULT_WARMTH, warmth_at)
}

/// `c` shifted towards amber by `s` in -1..=1: positive lowers blue and lifts red; negative lets blue back in, but never past
/// a brown-grey, so a dark palette cannot turn into a cool charcoal.
fn tint(c: Color32, s: f32) -> Color32 {
    let (r, g, b) = if s >= 0.0 { (1.0 + 0.04 * s, 1.0 - 0.10 * s, 1.0 - 0.45 * s) } else { (1.0 + 0.03 * s, 1.0, 1.0 - 0.12 * s) };
    let ch = |v: u8, gain: f32| (v as f32 * gain).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(ch(c.r(), r), ch(c.g(), g), ch(c.b(), b))
}

/// `c` with its brightness scaled by `f` (night text is dimmed to spare dark-adapted eyes).
fn dimmed(c: Color32, f: f32) -> Color32 {
    let ch = |v: u8| (v as f32 * f).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(ch(c.r()), ch(c.g()), ch(c.b()))
}

/// The evening and night palette at warmth `w` (0.5 is the exact designed hex values; deeper night is more amber and
/// its foreground colours are dimmer; towards day it stays a warm brown).
fn warm_dark(w: f32) -> Palette {
    let s = (w - DEFAULT_WARMTH) * 2.0;
    let dim = 1.0 - 0.14 * s.max(0.0);
    let hex = |h: u32| tint(Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8), s);
    let fg = |h: u32| dimmed(hex(h), dim);
    Palette {
        dark: true,
        bg: hex(0x211E19),
        panel: hex(0x302A21),
        raised: hex(0x393126),
        side: hex(0x191712),
        line: hex(0x494032),
        text: fg(0xF0DFC2),
        dim: fg(0xC4B397),
        muted: fg(0xA18D70),
        accent: fg(0xD6A85F),
        link: fg(0xD6B477),
        select: hex(0x493A28),
        on_accent: hex(0x211E19),
        good: fg(0xB5B88A),
        warn: fg(0xD5A45F),
        bad: fg(0xD18B78),
    }
}

/// The daytime palette at warmth `w`: warm paper and ink, neutral at full daylight and a little creamier towards dusk.
fn paper(w: f32) -> Palette {
    let s = w * 0.5;
    let hex = |h: u32| tint(Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8), s);
    Palette {
        dark: false,
        bg: hex(0xE9E5DC),
        panel: hex(0xF7F4ED),
        raised: hex(0xEFEBE2),
        side: hex(0xE0DBD0),
        line: hex(0xD2CCBF),
        text: hex(0x231F18),
        dim: hex(0x5F5648),
        muted: hex(0x7B715F),
        accent: hex(0x9A5B00),
        link: hex(0x8A5200),
        select: hex(0xF0DDB4),
        on_accent: Color32::WHITE,
        good: hex(0x4F6B22),
        warn: hex(0xA35A00),
        bad: hex(0xB23A24),
    }
}

/// The palette of the light (`dark` false) or dark theme at the warmth in use.
pub fn palette(dark: bool) -> Palette {
    palette_for(chosen(), dark, warmth())
}

/// The palette of library theme `choice` (`None` is Circadian, which uses `dark` and the warmth `w`).
fn palette_for(choice: Option<usize>, dark: bool, w: f32) -> Palette {
    match choice {
        Some(i) => (LIBRARY[i].palette)(),
        None => palette_at(dark, w),
    }
}

/// The palette of a theme at warmth `w`.
pub fn palette_at(dark: bool, w: f32) -> Palette {
    if dark {
        warm_dark(w)
    } else {
        paper(w)
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
    let dark = p.dark;
    let mut v = if dark {
        egui::Visuals::dark()
    } else {
        egui::Visuals::light()
    };
    v.panel_fill = p.panel;
    v.window_fill = p.panel;
    v.extreme_bg_color = p.bg;
    v.faint_bg_color = p.raised;
    v.window_stroke = Stroke::new(1.0, p.line);
    v.window_corner_radius = CornerRadius::same(10);
    v.menu_corner_radius = CornerRadius::same(8);
    v.window_shadow = egui::Shadow {
        offset: [0, 6],
        blur: 22,
        spread: 0,
        color: Color32::from_black_alpha(if dark { 110 } else { 46 }),
    };
    v.popup_shadow = v.window_shadow;
    v.selection.bg_fill = p.select;
    v.selection.stroke = Stroke::new(1.0, p.accent);
    v.hyperlink_color = p.link;
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
    for s in [
        &mut w.noninteractive,
        &mut w.inactive,
        &mut w.hovered,
        &mut w.active,
        &mut w.open,
    ] {
        s.corner_radius = r;
        s.expansion = 0.0;
    }
    v
}


/// The theme picker: Circadian, then the library. Returns whether the choice changed.
pub fn theme_picker(ui: &mut egui::Ui) -> bool {
    let current = chosen();
    let label = current.map_or("Circadian", |i| LIBRARY[i].name);
    let mut next = current;
    egui::ComboBox::from_id_salt("theme_picker")
        .selected_text(egui::RichText::new(label).small())
        .width(150.0)
        .show_ui(ui, |ui| {
            ui.selectable_value(&mut next, None, "Circadian").on_hover_text("Follows the time of day: warm and dim at night, bright paper by day");
            ui.separator();
            for (i, t) in LIBRARY.iter().enumerate() {
                ui.selectable_value(&mut next, Some(i), t.name).on_hover_text(format!("Colours from {}", t.source));
            }
        })
        .response
        .on_hover_text("Colour theme");
    if next == current {
        return false;
    }
    choose(next);
    save_choice();
    ui.ctx().request_repaint();
    true
}

/// The circadian control: the colour temperature now and a slider to preview any hour of the day. Auto follows the clock;
/// double-click the slider to return to it. Returns whether the preview changed.
pub fn circadian_control(ui: &mut egui::Ui, p: &Palette) -> bool {
    if !is_circadian() {
        return false;
    }
    let preview = f32::from_bits(MANUAL_HOUR.load(Ordering::Relaxed));
    let manual = !preview.is_nan();
    let h = hour().unwrap_or(19.0);
    let (hh, mm) = (h.floor() as u32 % 24, ((h.fract() * 60.0) as u32).min(59));
    let label = format!("{} {hh:02}:{mm:02} \u{b7} {:.0}K", if manual { "Preview" } else { "Auto" }, (kelvin_at(h) / 10.0).round() * 10.0);
    ui.label(egui::RichText::new(label).small().color(p.dim));
    let mut v = h;
    let r = ui
        .add(egui::Slider::new(&mut v, 0.0..=24.0).show_value(false).trailing_fill(true))
        .on_hover_text("Circadian colour: cooler and brighter by day, warmer and dimmer at night. Drag to preview an hour; double-click to follow the clock.");
    if r.double_clicked() {
        set_hour(None);
        ui.ctx().request_repaint();
        return true;
    }
    if r.changed() {
        set_hour(Some(v));
        ui.ctx().request_repaint();
    }
    r.changed()
}

/// Per-frame circadian update: reinstall the styles when the warmth moved, and follow the sun between light and dark
/// (only when daylight changes, so a Light/Dark click holds until the next sunrise or sunset). Repaints each minute.
pub fn tick(ctx: &egui::Context) {
    // A fixed library theme never moves; the key only tells a change of theme from none.
    let (key, day) = match chosen() {
        Some(i) => (10_000 + i as i32, !(LIBRARY[i].palette)().dark),
        None => {
            let Some(h) = hour() else { return };
            let w = warmth_at(h);
            ((w * 200.0).round() as i32, is_day(w))
        }
    };
    let id = egui::Id::new("circadian");
    let last: Option<(i32, bool)> = ctx.data(|d| d.get_temp(id));
    if last.map(|l| l.0) != Some(key) {
        install(ctx);
    }
    if last.map(|l| l.1) != Some(day) {
        ctx.set_theme(if day { egui::Theme::Light } else { egui::Theme::Dark });
    }
    ctx.data_mut(|d| d.insert_temp(id, (key, day)));
    if is_circadian() && f32::from_bits(MANUAL_HOUR.load(Ordering::Relaxed)).is_nan() {
        ctx.request_repaint_after(std::time::Duration::from_secs(60));
    }
}

/// Install both palettes and the text sizes; the theme preference stays the system one.
pub fn install(ctx: &egui::Context) {
    ctx.style_mut_of(egui::Theme::Dark, |s| s.visuals = visuals(true));
    ctx.style_mut_of(egui::Theme::Light, |s| s.visuals = visuals(false));
    ctx.all_styles_mut(|s| {
        s.text_styles = [
            (
                TextStyle::Small,
                FontId::new(11.0, FontFamily::Proportional),
            ),
            (TextStyle::Body, FontId::new(13.0, FontFamily::Proportional)),
            (
                TextStyle::Button,
                FontId::new(13.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Heading,
                FontId::new(16.0, FontFamily::Proportional),
            ),
            (
                TextStyle::Monospace,
                FontId::new(12.0, FontFamily::Monospace),
            ),
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
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c.r()) + 0.7152 * f(c.g()) + 0.0722 * f(c.b())
        };
        let (x, y) = (l(a), l(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// Every quarter hour of a day, and every warmth a user can pin with the preview slider.
    fn day() -> impl Iterator<Item = f32> {
        (0..96).map(|i| i as f32 / 4.0)
    }
    fn lum(c: Color32) -> f32 {
        0.2126 * c.r() as f32 + 0.7152 * c.g() as f32 + 0.0722 * c.b() as f32
    }

    #[test]
    fn default_dark_palette_is_the_designed_hex_values() {
        let hex = |h: u32| Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8);
        let p = palette(true);
        let want = [
            (p.bg, 0x211E19),
            (p.side, 0x191712),
            (p.panel, 0x302A21),
            (p.raised, 0x393126),
            (p.text, 0xF0DFC2),
            (p.dim, 0xC4B397),
            (p.muted, 0xA18D70),
            (p.line, 0x494032),
            (p.accent, 0xD6A85F),
            (p.link, 0xD6B477),
            (p.good, 0xB5B88A),
            (p.warn, 0xD5A45F),
            (p.bad, 0xD18B78),
            (p.select, 0x493A28),
        ];
        for (got, h) in want {
            assert_eq!(got, hex(h), "{h:06X}");
        }
    }

    #[test]
    fn colour_temperature_follows_the_sun_smoothly() {
        assert!(kelvin_at(3.0) < 1900.0, "deep night is amber: {}", kelvin_at(3.0));
        assert!(kelvin_at(12.5) > 6400.0, "noon is daylight: {}", kelvin_at(12.5));
        assert!(kelvin_at(23.75) < 2000.0);
        assert!(kelvin_at(6.0) < kelvin_at(7.0) && kelvin_at(7.0) < kelvin_at(9.0), "rises after sunrise");
        assert!(kelvin_at(17.0) > kelvin_at(19.0) && kelvin_at(19.0) > kelvin_at(21.0), "falls after sunset");
        let h: Vec<f32> = day().collect();
        for w in h.windows(2) {
            let step = (kelvin_at(w[1]) - kelvin_at(w[0])).abs();
            assert!(step < 1000.0, "a quarter hour moves {step} K at {}", w[0]);
        }
        assert_eq!(warmth_at(12.5).round(), 0.0);
        assert_eq!(warmth_at(3.0).round(), 1.0);
    }

    #[test]
    fn the_theme_follows_daylight_and_both_ends_are_readable_all_day() {
        let (mut light, mut dark) = (0, 0);
        for h in day() {
            let w = warmth_at(h);
            let day = is_day(w);
            if day { light += 1 } else { dark += 1 }
            let p = palette_at(!day, w);
            for surface in [p.bg, p.panel, p.raised] {
                assert!(contrast(p.text, surface) >= 6.0, "text at {h}h: {}", contrast(p.text, surface));
                assert!(contrast(p.dim, surface) >= 3.4, "dim at {h}h");
                assert!(contrast(p.accent, surface) >= 3.0, "accent at {h}h");
            }
            assert!(contrast(p.on_accent, p.accent) >= 3.0, "on accent at {h}h");
        }
        assert!(light >= 30 && dark >= 30, "a day has both: {light} light, {dark} dark quarter hours");
    }

    #[test]
    fn a_pinned_theme_is_readable_at_every_warmth() {
        for i in 0..=20 {
            let w = i as f32 / 20.0;
            for dark in [false, true] {
                let p = palette_at(dark, w);
                for surface in [p.bg, p.panel, p.raised] {
                    assert!(contrast(p.text, surface) >= 6.0, "text dark={dark} warmth {w}: {}", contrast(p.text, surface));
                    assert!(contrast(p.dim, surface) >= 3.4, "dim dark={dark} warmth {w}");
                    assert!(contrast(p.accent, surface) >= 3.0, "accent dark={dark} warmth {w}");
                    assert!(contrast(p.warn, surface) >= 3.0 && contrast(p.good, surface) >= 3.0, "semantic dark={dark} warmth {w}");
                }
            }
        }
    }

    #[test]
    fn every_surface_stays_warm_never_a_cool_charcoal() {
        for i in 0..=20 {
            let w = i as f32 / 20.0;
            for dark in [false, true] {
                let p = palette_at(dark, w);
                for (name, c) in [("bg", p.bg), ("side", p.side), ("panel", p.panel), ("raised", p.raised), ("line", p.line)] {
                    assert!(c.b() <= c.r(), "{name} turned cool at warmth {w}, dark={dark}: {c:?}");
                }
            }
        }
    }

    #[test]
    fn night_is_dimmer_than_evening_and_day_is_brighter_than_both() {
        let text = |w| lum(palette_at(true, w).text);
        assert!(text(1.0) < text(0.5), "deep-night text is dimmed");
        assert!(lum(palette_at(false, 0.0).bg) > 2.0 * lum(palette_at(true, 0.5).bg), "daytime is brighter");
    }

    #[test]
    fn every_library_theme_is_readable_and_consistent() {
        let mut ids = std::collections::HashSet::new();
        let mut bad = Vec::new();
        for t in LIBRARY.iter() {
            assert!(ids.insert(t.id), "duplicate id {}", t.id);
            let p = (t.palette)();
            if p.dark != (lum(p.bg) < 128.0) {
                bad.push(format!("{}: dark flag does not match the background", t.name));
            }
            let mut check = |what: &str, fg: Color32, bg: Color32, min: f32| {
                let c = contrast(fg, bg);
                if c < min {
                    bad.push(format!("{}: {what} {c:.2} < {min}", t.name));
                }
            };
            for (sn, surface) in [("bg", p.bg), ("panel", p.panel), ("raised", p.raised)] {
                check(&format!("text on {sn}"), p.text, surface, 7.0);
                check(&format!("dim on {sn}"), p.dim, surface, 3.4);
                check(&format!("accent on {sn}"), p.accent, surface, 3.0);
                check(&format!("link on {sn}"), p.link, surface, 3.0);
                check(&format!("good on {sn}"), p.good, surface, 3.0);
                check(&format!("warn on {sn}"), p.warn, surface, 3.0);
                check(&format!("bad on {sn}"), p.bad, surface, 3.0);
            }
            check("text on select", p.text, p.select, 4.5);
            check("on_accent on accent", p.on_accent, p.accent, 3.0);
            if lum(p.side) == lum(p.bg) {
                bad.push(format!("{}: the sidebar is not its own surface", t.name));
            }
        }
        assert!(bad.is_empty(), "{}", bad.join("\n"));
    }

    #[test]
    fn a_library_theme_replaces_circadian_in_both_slots_and_an_unknown_choice_is_circadian() {
        let fixed = palette_for(Some(2), true, 0.5);
        assert_eq!(fixed.bg, palette_for(Some(2), false, 0.5).bg, "a fixed theme ignores the light/dark slot");
        assert_eq!(fixed.bg, (LIBRARY[2].palette)().bg);
        assert_eq!(palette_for(None, true, DEFAULT_WARMTH).bg, palette_at(true, DEFAULT_WARMTH).bg);
        assert!(palette_for(None, false, 0.0).bg != palette_for(None, true, 0.0).bg);
    }

    #[test]
    fn text_stays_readable_on_every_surface_in_both_themes() {
        for dark in [false, true] {
            let p = palette(dark);
            for surface in [p.bg, p.panel, p.raised] {
                assert!(
                    contrast(p.text, surface) >= 7.0,
                    "text on a surface, dark={dark}"
                );
                assert!(
                    contrast(p.dim, surface) >= 3.4,
                    "secondary text, dark={dark}: {}",
                    contrast(p.dim, surface)
                );
                assert!(contrast(p.accent, surface) >= 3.0, "accent, dark={dark}");
                assert!(
                    contrast(p.warn, surface) >= 3.0 && contrast(p.good, surface) >= 3.0,
                    "semantic colours, dark={dark}"
                );
            }
            assert!(
                contrast(p.on_accent, p.accent) >= 3.0,
                "text on a filled accent, dark={dark}"
            );
        }
    }
}
