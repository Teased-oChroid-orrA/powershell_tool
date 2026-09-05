//! Design System Epic Phase 3: icon system. Every nav-rail icon used to
//! be a single Unicode glyph rendered via `Painter::text` (`⚙`, `■`, `🖊`,
//! etc. - see `CLAUDE.md`'s glyph-coverage bug-class notes for why that
//! was already a fragile approach, not just a stylistic one). This
//! replaces the six tool-rail icons with real Lucide vector icons
//! (ISC-licensed, `assets/icons/LUCIDE-LICENSE.txt`), rasterized at
//! runtime via the `resvg`/`usvg`/`tiny-skia` dependencies `Cargo.toml`
//! already declared for this exact purpose but never actually used
//! anywhere in `app-egui` until now (confirmed: no other file in this
//! crate calls into any of the three crates).
//!
//! Scope note (disclosed, not silent): the theme-toggle and pin-rail
//! buttons keep their existing Unicode glyphs (🌙/☀/📌) rather than
//! joining this system - those are already confirmed-present in egui's
//! bundled fallback fonts (`CLAUDE.md`'s fix table), render at a size
//! where a raster/vector distinction is imperceptible, and converting
//! them would roughly double this module's scope for icons that aren't
//! the ones a user actually navigates by. The six tool-rail icons are the
//! ones rendered largest, most persistently, and most central to
//! wayfinding - the right place to spend this phase's icon-system budget.

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use std::collections::HashMap;

/// Every bundled icon is a Lucide 24x24 viewBox SVG using
/// `stroke="currentColor"` - rasterization substitutes that literal
/// string for a real hex color before parsing, since `usvg` has no CSS
/// `currentColor` concept on its own (there's no cascade to resolve it
/// against outside a browser).
fn svg_source(name: &str) -> &'static str {
    match name {
        "search" => include_str!("../../assets/icons/search.svg"),
        "settings" => include_str!("../../assets/icons/settings.svg"),
        "cylinder" => include_str!("../../assets/icons/cylinder.svg"),
        "copy-check" => include_str!("../../assets/icons/copy-check.svg"),
        "pencil-line" => include_str!("../../assets/icons/pencil-line.svg"),
        "chart-column" => include_str!("../../assets/icons/chart-column.svg"),
        _ => panic!("unknown bundled icon {name:?} - add it to design::icons::svg_source"),
    }
}

fn rasterize(svg: &str, color: Color32, px: u32) -> ColorImage {
    let hex = format!("#{:02x}{:02x}{:02x}", color.r(), color.g(), color.b());
    let colored = svg.replace("currentColor", &hex);
    let tree = usvg::Tree::from_str(&colored, &usvg::Options::default()).expect("bundled icon SVG must parse - it shipped with the app");
    let mut pixmap = tiny_skia::Pixmap::new(px, px).expect("icon raster size must be nonzero");
    // Every bundled Lucide icon shares the same `viewBox="0 0 24 24"` -
    // scaling by px/24 maps it to fill the target pixmap exactly, no
    // per-icon size bookkeeping needed.
    let scale = px as f32 / 24.0;
    resvg::render(&tree, tiny_skia::Transform::from_scale(scale, scale), &mut pixmap.as_mut());
    // tiny-skia's `Pixmap` stores premultiplied-alpha RGBA8, which is
    // exactly what `from_rgba_premultiplied` expects - using the
    // unmultiplied constructor here would double-darken every
    // partially-transparent edge pixel (anti-aliasing), a real, easy to
    // miss bug specific to combining these two crates.
    ColorImage::from_rgba_premultiplied([px as usize, px as usize], pixmap.data())
}

/// Caches one rasterized texture per (icon, color, size) combination -
/// nav icons need two colors (active/inactive), so this can't be a
/// single texture per icon name. Rasterizing is real CPU work (SVG parse
/// + path fill); caching means it happens once per combination actually
/// used, not once per frame.
#[derive(Default)]
pub struct IconCache {
    textures: HashMap<(&'static str, [u8; 3], u32), TextureHandle>,
}

impl IconCache {
    pub fn get(&mut self, ctx: &egui::Context, name: &'static str, color: Color32, px: u32) -> TextureHandle {
        let key = (name, [color.r(), color.g(), color.b()], px);
        self.textures
            .entry(key)
            .or_insert_with(|| {
                let image = rasterize(svg_source(name), color, px);
                ctx.load_texture(format!("icon-{name}-{:02x}{:02x}{:02x}-{px}", color.r(), color.g(), color.b()), image, TextureOptions::LINEAR)
            })
            .clone()
    }
}
