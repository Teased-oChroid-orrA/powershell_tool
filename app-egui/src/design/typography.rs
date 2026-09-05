//! Type scale + bundled fonts (Design System Epic Phase 1).
//!
//! Before this module, `app-egui` had no custom `FontDefinitions` at all -
//! every label rendered in egui's bundled default fonts (`Ubuntu-Light`
//! proportional, `Hack-Regular` monospace). Those are kept installed as
//! FALLBACK entries, never removed: `CLAUDE.md`'s bug-class notes record
//! that several Unicode symbols used throughout this UI (✔ ⚙ 🖊 🌙) only
//! exist in those bundled default fonts, not in Inter or JetBrains Mono
//! (confirmed directly via `fontTools` cmap inspection before adopting
//! these faces - the same verification discipline that note itself
//! demands). Inter/JetBrains Mono are inserted as the FIRST (highest-
//! priority) font in each family's fallback chain, so Latin text/digits
//! render in the new faces while those four symbols still resolve through
//! to the old bundled fonts exactly as before.
//!
//! Static weights (not the variable-font originals) are bundled: `ab_glyph`
//! (egui/epaint's rasterizer) has no variable-font axis support, so a
//! variable font would render every weight at its single default instance.
//! The four Inter weights and two JetBrains Mono weights here were
//! instanced from the Google Fonts variable sources via `fonttools
//! varLib.instancer` (OFL-1.1 licensed - `assets/fonts/*.LICENSE.txt`).

// Most of this module's scale/family helpers are consumed by Phase 2+
// components, not yet built - kept as the reusable primitives those
// phases should read from rather than reinventing sizes ad hoc, same
// treatment already established for `widgets::text_field`.
#![allow(dead_code)]

use eframe::egui::{self, FontData, FontDefinitions, FontFamily, FontId};
use std::sync::Arc;

const INTER_REGULAR: &[u8] = include_bytes!("../../assets/fonts/Inter-Regular.ttf");
const INTER_MEDIUM: &[u8] = include_bytes!("../../assets/fonts/Inter-Medium.ttf");
const INTER_SEMIBOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-SemiBold.ttf");
const INTER_BOLD: &[u8] = include_bytes!("../../assets/fonts/Inter-Bold.ttf");
const MONO_REGULAR: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Regular.ttf");
const MONO_BOLD: &[u8] = include_bytes!("../../assets/fonts/JetBrainsMono-Bold.ttf");

/// Named `FontFamily` keys for weights beyond what egui's built-in
/// `Proportional`/`Monospace` two-family model covers. `FontFamily::Name`
/// takes an `Arc<str>` used for hashmap lookup - allocated once here as
/// consts-via-function since `Arc::from` isn't const-evaluable.
pub fn family_medium() -> FontFamily {
    FontFamily::Name(Arc::from("Inter-Medium"))
}
pub fn family_semibold() -> FontFamily {
    FontFamily::Name(Arc::from("Inter-SemiBold"))
}
pub fn family_bold() -> FontFamily {
    FontFamily::Name(Arc::from("Inter-Bold"))
}
pub fn family_mono_bold() -> FontFamily {
    FontFamily::Name(Arc::from("JetBrainsMono-Bold"))
}

/// Builds the app's full `FontDefinitions`: starts from egui's own
/// defaults (keeping every bundled fallback font already registered, for
/// the glyph-coverage reason above) and inserts the bundled Inter/
/// JetBrains Mono weights ahead of them. Call once via
/// `cc.egui_ctx.set_fonts(...)` at startup - font data (~1.6MB total) is
/// too expensive to rebuild every frame.
pub fn font_definitions() -> FontDefinitions {
    let mut fonts = FontDefinitions::default();

    fonts.font_data.insert("Inter-Regular".to_owned(), FontData::from_static(INTER_REGULAR));
    fonts.font_data.insert("Inter-Medium".to_owned(), FontData::from_static(INTER_MEDIUM));
    fonts.font_data.insert("Inter-SemiBold".to_owned(), FontData::from_static(INTER_SEMIBOLD));
    fonts.font_data.insert("Inter-Bold".to_owned(), FontData::from_static(INTER_BOLD));
    fonts.font_data.insert("JetBrainsMono-Regular".to_owned(), FontData::from_static(MONO_REGULAR));
    fonts.font_data.insert("JetBrainsMono-Bold".to_owned(), FontData::from_static(MONO_BOLD));

    // Default text (`FontFamily::Proportional`) and monospace digits/code
    // (`FontFamily::Monospace`) both get their real-weight Inter/
    // JetBrains Mono face first, existing bundled fallbacks kept after.
    let proportional = fonts.families.entry(FontFamily::Proportional).or_default();
    proportional.insert(0, "Inter-Regular".to_owned());

    let monospace = fonts.families.entry(FontFamily::Monospace).or_default();
    monospace.insert(0, "JetBrainsMono-Regular".to_owned());

    // Named families for weights the two built-in `FontFamily` variants
    // can't express. Each falls through to the same bundled-default
    // fallback chain as `Proportional`/`Monospace` respectively, so a
    // glyph missing from a specific Inter/JetBrains Mono weight (there
    // are none among the symbols this app uses, but this keeps the
    // guarantee general) still resolves rather than tofu-boxing.
    let with_fallback = |primary: &str, base: FontFamily| {
        let mut chain = vec![primary.to_owned()];
        chain.extend(fonts.families.get(&base).cloned().unwrap_or_default());
        chain
    };
    let medium_chain = with_fallback("Inter-Medium", FontFamily::Proportional);
    let semibold_chain = with_fallback("Inter-SemiBold", FontFamily::Proportional);
    let bold_chain = with_fallback("Inter-Bold", FontFamily::Proportional);
    let mono_bold_chain = with_fallback("JetBrainsMono-Bold", FontFamily::Monospace);
    fonts.families.insert(FontFamily::Name(Arc::from("Inter-Medium")), medium_chain);
    fonts.families.insert(FontFamily::Name(Arc::from("Inter-SemiBold")), semibold_chain);
    fonts.families.insert(FontFamily::Name(Arc::from("Inter-Bold")), bold_chain);
    fonts.families.insert(FontFamily::Name(Arc::from("JetBrainsMono-Bold")), mono_bold_chain);

    fonts
}

/// Type scale (px). Named per the epic's Display/H1-3/Body/BodySmall/
/// Caption/Label roles, values chosen to match sizes already established
/// ad hoc across this app (`13.5`/`12.5`/`11.5`/`11.0`/`10.5` appear
/// repeatedly in `widgets.rs`/`bushing.rs`/`search.rs` already) rather than
/// introducing a competing scale - this names the scale that was already
/// implicit, and gives later phases one place to read sizes from instead
/// of a new magic number per call site.
pub const DISPLAY: f32 = 26.0;
pub const H1: f32 = 20.0;
pub const H2: f32 = 16.0;
pub const H3: f32 = 15.0;
pub const BODY: f32 = 13.0;
pub const BODY_SMALL: f32 = 11.5;
pub const CAPTION: f32 = 10.5;
pub const LABEL: f32 = 11.5;
pub const MONOSPACE: f32 = 13.0;

pub fn display() -> FontId {
    FontId::new(DISPLAY, family_semibold())
}
pub fn h1() -> FontId {
    FontId::new(H1, family_semibold())
}
pub fn h2() -> FontId {
    FontId::new(H2, family_semibold())
}
pub fn h3() -> FontId {
    FontId::new(H3, family_medium())
}
pub fn body() -> FontId {
    FontId::new(BODY, FontFamily::Proportional)
}
pub fn body_small() -> FontId {
    FontId::new(BODY_SMALL, FontFamily::Proportional)
}
pub fn caption() -> FontId {
    FontId::new(CAPTION, FontFamily::Proportional)
}
pub fn label() -> FontId {
    FontId::new(LABEL, FontFamily::Proportional)
}
pub fn monospace() -> FontId {
    FontId::new(MONOSPACE, FontFamily::Monospace)
}

/// `RichText` helper for the common "card title" role (`H3`, `Inter-
/// SemiBold`) - `widgets::card_title` uses this instead of a locally
/// hardcoded size, so both stay in sync with the named scale.
pub fn card_title_text(text: &str) -> egui::RichText {
    egui::RichText::new(text).font(h3())
}
