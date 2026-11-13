//! The theme library: six fixed palettes taken from published website colour schemes, as alternatives to the default
//! Circadian theme (`theme.rs`). Each entry starts from the scheme's own hex values and derives the roles the interface
//! needs (surfaces, text steps, selection, semantic colours) by mixing them, so every theme keeps its character. A test
//! in `theme.rs` holds each one to the same contrast floors as the circadian palette.
//!
//! Sources: Hook Agency, "Website color schemes" (Deep Blue And Tan, Pink Green And Purple, Dark Horse) and August Ash,
//! "25 Amazing Website Color Schemes" (Monochromatic Ocean, Compelling Complimentary, Striking Elegance).
use super::theme::Palette;
use egui::Color32;

/// One entry of the library.
pub struct LibraryTheme {
    pub id: &'static str,
    pub name: &'static str,
    /// Where the scheme comes from.
    pub source: &'static str,
    pub palette: fn() -> Palette,
}

const HOOK: &str = "Hook Agency, Website color schemes";
const ASH: &str = "August Ash, 25 Amazing Website Color Schemes";

pub const LIBRARY: [LibraryTheme; 6] = [
    LibraryTheme { id: "deep-blue-tan", name: "Deep Blue and Tan", source: HOOK, palette: deep_blue_tan },
    LibraryTheme { id: "pink-green-purple", name: "Pink, Green and Purple", source: HOOK, palette: pink_green_purple },
    LibraryTheme { id: "dark-horse", name: "Dark Horse", source: HOOK, palette: dark_horse },
    LibraryTheme { id: "monochromatic-ocean", name: "Monochromatic Ocean", source: ASH, palette: monochromatic_ocean },
    LibraryTheme { id: "compelling-complimentary", name: "Compelling Complimentary", source: ASH, palette: compelling_complimentary },
    LibraryTheme { id: "striking-elegance", name: "Striking Elegance", source: ASH, palette: striking_elegance },
];

fn rgb(h: u32) -> Color32 {
    Color32::from_rgb((h >> 16) as u8, (h >> 8) as u8, h as u8)
}

/// `a` towards `b` by `t` (0 is `a`, 1 is `b`).
fn mix(a: u32, b: u32, t: f32) -> Color32 {
    let (a, b) = (rgb(a), rgb(b));
    let ch = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round().clamp(0.0, 255.0) as u8;
    Color32::from_rgb(ch(a.r(), b.r()), ch(a.g(), b.g()), ch(a.b(), b.b()))
}

const WHITE: u32 = 0xFFFFFF;
const BLACK: u32 = 0x000000;

/// Success, warning and error colours (the schemes have none): clean hues tuned for a dark or a light surface.
fn semantic(dark: bool) -> (Color32, Color32, Color32) {
    if dark {
        (rgb(0x86CB8A), rgb(0xE9B24C), rgb(0xF08C7C))
    } else {
        (rgb(0x2E7D32), rgb(0x9A5A00), rgb(0xB3261E))
    }
}

/// Night Blue and Sand Tan: `#2D545E`, `#12343B`, `#E1B382`, `#C89666`.
fn deep_blue_tan() -> Palette {
    let (good, warn, bad) = semantic(true);
    Palette {
        dark: true,
        bg: rgb(0x12343B),
        panel: mix(0x12343B, 0x2D545E, 0.45),
        raised: rgb(0x2D545E),
        side: mix(0x12343B, BLACK, 0.35),
        line: mix(0x2D545E, 0xE1B382, 0.28),
        text: mix(0xE1B382, WHITE, 0.82),
        dim: mix(0xE1B382, WHITE, 0.30),
        muted: rgb(0xC89666),
        accent: rgb(0xE1B382),
        link: mix(0xE1B382, WHITE, 0.30),
        select: mix(0x12343B, 0xC89666, 0.40),
        on_accent: rgb(0x12343B),
        good,
        warn,
        bad,
    }
}

/// Grassy Green `#9BC400`, Purple Mountains Majesty `#8076A3`, Misty Mountain Pink `#F9C5BD`, Factory Stone Purple `#7C677F`.
fn pink_green_purple() -> Palette {
    let (_, warn, bad) = semantic(false);
    Palette {
        dark: false,
        bg: mix(0xF9C5BD, WHITE, 0.62),
        panel: mix(0xF9C5BD, WHITE, 0.86),
        raised: mix(0xF9C5BD, WHITE, 0.74),
        side: mix(0xF9C5BD, WHITE, 0.36),
        line: mix(0xF9C5BD, 0x7C677F, 0.30),
        text: mix(0x7C677F, BLACK, 0.80),
        dim: mix(0x7C677F, BLACK, 0.42),
        muted: mix(0x7C677F, BLACK, 0.22),
        accent: mix(0x8076A3, BLACK, 0.16),
        link: mix(0x8076A3, BLACK, 0.38),
        select: mix(0x8076A3, WHITE, 0.62),
        on_accent: rgb(WHITE),
        good: mix(0x9BC400, BLACK, 0.52),
        warn,
        bad,
    }
}

/// Yellow `#FEDA6A`, Silver Fox `#D4D4DC`, Deep Matte Grey `#393F4D`, Dark Slate `#1D1E22`.
fn dark_horse() -> Palette {
    let (good, warn, bad) = semantic(true);
    Palette {
        dark: true,
        bg: rgb(0x1D1E22),
        panel: mix(0x1D1E22, 0x393F4D, 0.50),
        raised: rgb(0x393F4D),
        side: mix(0x1D1E22, BLACK, 0.40),
        line: mix(0x393F4D, 0xD4D4DC, 0.20),
        text: mix(0xD4D4DC, WHITE, 0.45),
        dim: rgb(0xD4D4DC),
        muted: mix(0xD4D4DC, 0x393F4D, 0.30),
        accent: rgb(0xFEDA6A),
        link: mix(0xFEDA6A, WHITE, 0.25),
        select: mix(0x1D1E22, 0xFEDA6A, 0.24),
        on_accent: rgb(0x1D1E22),
        good,
        warn,
        bad,
    }
}

/// Four blues: `#0B294D`, `#11335A`, `#4CA3E5` and white.
fn monochromatic_ocean() -> Palette {
    let (good, warn, bad) = semantic(true);
    Palette {
        dark: true,
        bg: rgb(0x0B294D),
        panel: rgb(0x11335A),
        raised: mix(0x11335A, 0x4CA3E5, 0.16),
        side: mix(0x0B294D, BLACK, 0.40),
        line: mix(0x11335A, 0x4CA3E5, 0.34),
        text: mix(WHITE, 0x4CA3E5, 0.08),
        dim: mix(WHITE, 0x4CA3E5, 0.38),
        muted: mix(0x4CA3E5, WHITE, 0.30),
        accent: rgb(0x4CA3E5),
        link: mix(0x4CA3E5, WHITE, 0.30),
        select: mix(0x11335A, 0x4CA3E5, 0.42),
        on_accent: rgb(0x0B294D),
        good,
        warn,
        bad,
    }
}

/// Orange `#DD6832`, light grey-blue `#E2E5EC` and `#CED4DE`, navy `#12356B`, green `#367A25`.
fn compelling_complimentary() -> Palette {
    let (_, warn, bad) = semantic(false);
    Palette {
        dark: false,
        bg: rgb(0xE2E5EC),
        panel: mix(0xE2E5EC, WHITE, 0.62),
        raised: mix(0xE2E5EC, WHITE, 0.34),
        side: rgb(0xCED4DE),
        line: mix(0xCED4DE, 0x12356B, 0.22),
        text: mix(0x12356B, BLACK, 0.45),
        dim: mix(0x12356B, 0xE2E5EC, 0.18),
        muted: mix(0x12356B, 0xE2E5EC, 0.40),
        accent: mix(0xDD6832, BLACK, 0.14),
        link: rgb(0x12356B),
        select: mix(0xDD6832, WHITE, 0.74),
        on_accent: rgb(WHITE),
        good: rgb(0x367A25),
        warn,
        bad,
    }
}

/// Midnight `#030432`, gold `#F2A249`, white, navy `#0E285C`, slate `#4F4F6E`.
fn striking_elegance() -> Palette {
    let (good, warn, bad) = semantic(true);
    Palette {
        dark: true,
        bg: rgb(0x030432),
        panel: mix(0x030432, 0x0E285C, 0.55),
        raised: rgb(0x0E285C),
        side: mix(0x030432, BLACK, 0.45),
        line: mix(0x0E285C, 0x4F4F6E, 0.60),
        text: rgb(WHITE),
        dim: mix(0x4F4F6E, WHITE, 0.62),
        muted: mix(0x4F4F6E, WHITE, 0.38),
        accent: rgb(0xF2A249),
        link: mix(0xF2A249, WHITE, 0.22),
        select: mix(0x0E285C, 0xF2A249, 0.28),
        on_accent: rgb(0x030432),
        good,
        warn,
        bad,
    }
}
