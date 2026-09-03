//! Labeled engineering cross-section sketches - ported directly from the
//! approved mockup artifact's hand-authored SVG (conversation record),
//! translated into `egui::Painter` calls rather than rasterized through
//! resvg. Simpler and crisper than an SVG round-trip, and every
//! coordinate below is the same coordinate math already tuned across
//! several real rounds of "labels overlapping"/"labels off-canvas"
//! feedback on the mockup itself - reused, not re-derived from scratch.
//!
//! Same drafting conventions the mockup settled on: ANSI-style diagonal
//! hatch for cut material (ring/annulus regions approximated by clipping
//! the hatch to the shape's bounding rect - a real, disclosed
//! approximation, not full path-clipping, since `egui::Painter` has no
//! arbitrary clip-path primitive), dash-dot centerlines, a small cross
//! center-mark, and dimension lines kept outside the part with extension
//! lines - exactly the layout discipline that fixed the mockup's
//! overlap bugs. Only the dimension group relevant to the current step
//! renders at full color; every other group is fully absent (not drawn
//! at all) rather than faded, per the same "hidden not translucent"
//! fix the mockup went through.

use eframe::egui::{self, Color32, FontId, Pos2, Rect, Sense, Stroke, Vec2};

use crate::theme::Tokens;

pub struct Sketch<'a> {
    painter: egui::Painter,
    tokens: &'a Tokens,
    to_screen: egui::emath::RectTransform,
    scale: f32,
}

impl<'a> Sketch<'a> {
    /// `view_size` is the SVG viewBox size this sketch's coordinates were
    /// authored in (matches the mockup's own `viewBox` values verbatim).
    pub fn new(ui: &mut egui::Ui, tokens: &'a Tokens, view_size: Vec2, height: f32) -> Self {
        let width = ui.available_width();
        let (rect, _) = ui.allocate_exact_size(egui::vec2(width, height), Sense::hover());
        let scale = (rect.width() / view_size.x).min(rect.height() / view_size.y);
        let drawn_size = view_size * scale;
        let offset = rect.center() - drawn_size / 2.0;
        let view_rect = Rect::from_min_size(Pos2::ZERO, view_size);
        let screen_rect = Rect::from_min_size(offset, drawn_size);
        Sketch { painter: ui.painter().with_clip_rect(rect), tokens, to_screen: egui::emath::RectTransform::from_to(view_rect, screen_rect), scale }
    }

    fn p(&self, x: f32, y: f32) -> Pos2 {
        self.to_screen.transform_pos(egui::pos2(x, y))
    }
    fn s(&self, v: f32) -> f32 {
        v * self.scale
    }

    // Not called by any of the 4 sketches shipped so far (all their
    // circular geometry goes through `hatch_ring`) - kept as reusable
    // primitives for whatever view a future tool's sketch needs, same
    // shape as `outline_rect`/`fill_rect` right below.
    #[allow(dead_code)]
    pub fn outline_circle(&self, cx: f32, cy: f32, r: f32, stroke: Color32) {
        self.painter.circle_stroke(self.p(cx, cy), self.s(r), Stroke::new(1.5, stroke));
    }
    #[allow(dead_code)]
    pub fn fill_circle(&self, cx: f32, cy: f32, r: f32, fill: Color32) {
        self.painter.circle_filled(self.p(cx, cy), self.s(r), fill);
    }
    pub fn outline_rect(&self, x: f32, y: f32, w: f32, h: f32, rounding: f32, stroke: Color32) {
        self.painter.rect_stroke(Rect::from_min_size(self.p(x, y), egui::vec2(self.s(w), self.s(h))), self.s(rounding), Stroke::new(1.5, stroke));
    }
    pub fn fill_rect(&self, x: f32, y: f32, w: f32, h: f32, rounding: f32, fill: Color32) {
        self.painter.rect_filled(Rect::from_min_size(self.p(x, y), egui::vec2(self.s(w), self.s(h))), self.s(rounding), fill);
    }

    /// Diagonal-line hatch (ANSI31-style) clipped to a rect - the
    /// bounding-box approximation for ring/annulus regions the module
    /// doc explains.
    pub fn hatch_rect(&self, x: f32, y: f32, w: f32, h: f32, base: Color32, line_alpha: u8) {
        let rect = Rect::from_min_size(self.p(x, y), egui::vec2(self.s(w), self.s(h)));
        self.painter.rect_filled(rect, 0.0, base);
        let step = self.s(9.0).max(4.0);
        let line_color = Color32::from_white_alpha(line_alpha);
        let mut off = -rect.height();
        while off < rect.width() {
            let x0 = rect.left() + off;
            self.painter.with_clip_rect(rect).line_segment([egui::pos2(x0, rect.bottom()), egui::pos2(x0 + rect.height(), rect.top())], Stroke::new(1.0, line_color));
            off += step;
        }
    }
    pub fn hatch_ring(&self, cx: f32, cy: f32, r_outer: f32, r_inner: f32, base: Color32, line_alpha: u8) {
        self.hatch_rect(cx - r_outer, cy - r_outer, r_outer * 2.0, r_outer * 2.0, base, line_alpha);
        self.painter.circle_filled(self.p(cx, cy), self.s(r_inner), self.tokens.bg_raised);
        self.painter.circle_stroke(self.p(cx, cy), self.s(r_outer), Stroke::new(1.5, Color32::from_rgb(0xaa, 0xb4, 0xbd)));
        self.painter.circle_stroke(self.p(cx, cy), self.s(r_inner), Stroke::new(1.3, Color32::from_rgb(0xaa, 0xb4, 0xbd)));
    }

    pub fn center_mark(&self, cx: f32, cy: f32, s: f32) {
        let c = self.p(cx, cy);
        let stroke = Stroke::new(1.0, Color32::from_rgb(0xc9, 0x76, 0x5f));
        self.painter.line_segment([c - egui::vec2(self.s(s), 0.0), c + egui::vec2(self.s(s), 0.0)], stroke);
        self.painter.line_segment([c - egui::vec2(0.0, self.s(s)), c + egui::vec2(0.0, self.s(s))], stroke);
    }
    pub fn centerline_h(&self, x1: f32, x2: f32, y: f32) {
        self.dashed(self.p(x1, y), self.p(x2, y), Color32::from_rgb(0xc9, 0x76, 0x5f));
    }
    pub fn centerline_v(&self, y1: f32, y2: f32, x: f32) {
        self.dashed(self.p(x, y1), self.p(x, y2), Color32::from_rgb(0xc9, 0x76, 0x5f));
    }
    fn dashed(&self, a: Pos2, b: Pos2, color: Color32) {
        let dir = (b - a).normalized();
        let len = a.distance(b);
        let mut d = 0.0;
        while d < len {
            let seg_end = (d + 8.0).min(len);
            self.painter.line_segment([a + dir * d, a + dir * seg_end], Stroke::new(1.0, color));
            d += 12.0;
        }
    }

    fn arrow(&self, at: Pos2, dir: Vec2, color: Color32) {
        let dir = dir.normalized();
        let perp = egui::vec2(-dir.y, dir.x);
        let tip = at;
        let back = at - dir * self.s(7.0);
        let p1 = back + perp * self.s(3.0);
        let p2 = back - perp * self.s(3.0);
        self.painter.add(egui::Shape::convex_polygon(vec![tip, p1, p2], color, Stroke::NONE));
    }

    /// Horizontal dimension: extension lines up to the part, a dimension
    /// line with arrows at both ends, label centered above.
    pub fn dim_h(&self, x1: f32, x2: f32, y: f32, ext_to: f32, label: &str, color: Color32) {
        let a = self.p(x1, y);
        let b = self.p(x2, y);
        let ext = self.p(x1, ext_to);
        let ext2 = self.p(x2, ext_to);
        self.painter.line_segment([a, ext], Stroke::new(1.0, color.gamma_multiply(0.5)));
        self.painter.line_segment([b, ext2], Stroke::new(1.0, color.gamma_multiply(0.5)));
        self.painter.line_segment([a, b], Stroke::new(1.0, color));
        self.arrow(a, egui::vec2(-1.0, 0.0), color);
        self.arrow(b, egui::vec2(1.0, 0.0), color);
        self.painter.text(egui::pos2((a.x + b.x) / 2.0, a.y - self.s(9.0)), egui::Align2::CENTER_CENTER, label, FontId::monospace(self.s(11.0).max(9.0)), color);
    }

    pub fn dim_v(&self, y1: f32, y2: f32, x: f32, ext_to: f32, label: &str, color: Color32) {
        let a = self.p(x, y1);
        let b = self.p(x, y2);
        let ext = self.p(ext_to, y1);
        let ext2 = self.p(ext_to, y2);
        self.painter.line_segment([a, ext], Stroke::new(1.0, color.gamma_multiply(0.5)));
        self.painter.line_segment([b, ext2], Stroke::new(1.0, color.gamma_multiply(0.5)));
        self.painter.line_segment([a, b], Stroke::new(1.0, color));
        self.arrow(a, egui::vec2(0.0, -1.0), color);
        self.arrow(b, egui::vec2(0.0, 1.0), color);
        self.painter.text(egui::pos2(a.x + self.s(9.0), (a.y + b.y) / 2.0), egui::Align2::LEFT_CENTER, label, FontId::monospace(self.s(11.0).max(9.0)), color);
    }

    /// A short arrow (pressure/load direction) plus, when `label` is
    /// given, text at the tail end.
    pub fn arrow_label(&self, x1: f32, y1: f32, x2: f32, y2: f32, color: Color32) {
        let a = self.p(x1, y1);
        let b = self.p(x2, y2);
        self.painter.line_segment([a, b], Stroke::new(1.6, color));
        self.arrow(b, b - a, color);
    }

    /// Leader line from a real point on the part out to a label, with a
    /// small dot at the anchor - the mockup's "callout" pattern, used
    /// wherever a label can't sit right next to the thing it describes.
    pub fn leader_label(&self, x1: f32, y1: f32, x2: f32, y2: f32, label: &str, align: egui::Align2, color: Color32) {
        let a = self.p(x1, y1);
        let b = self.p(x2, y2);
        self.painter.line_segment([a, b], Stroke::new(1.0, color.gamma_multiply(0.6)));
        self.painter.circle_filled(a, self.s(2.5), color);
        self.painter.text(b, align, label, FontId::proportional(self.s(11.0).max(9.5)), color);
    }
}

impl<'a> Sketch<'a> {
    pub fn text_at(&self, x: f32, y: f32, align: egui::Align2, label: &str, color: Color32) {
        self.painter.text(self.p(x, y), align, label, FontId::monospace(self.s(11.0).max(9.0)), color);
    }
}

/// Pressure Vessel - head-on radial cross-section. `emph` selects which
/// dimension group renders (`"od"`, `"wall"`, `"material"`) - matches
/// `PvStep`'s step name 1:1 in `pressure_vessel.rs`.
pub fn pv_head_on(ui: &mut egui::Ui, tokens: &Tokens, emph: &str) {
    let sk = Sketch::new(ui, tokens, egui::vec2(380.0, 240.0), 220.0);
    let steel = Color32::from_rgb(0x2c, 0x33, 0x3b);
    sk.hatch_ring(170.0, 130.0, 68.0, 38.0, steel, 13);
    sk.centerline_h(60.0, 280.0, 130.0);
    sk.centerline_v(40.0, 220.0, 170.0);
    sk.center_mark(170.0, 130.0, 8.0);

    let accent = tokens.accent_strong;
    if emph == "od" || emph == "wall" {
        sk.dim_h(102.0, 238.0, 32.0, 130.0, "\u{2300} 6.00 in — Outer diameter", accent);
    }
    if emph == "wall" {
        sk.dim_h(208.0, 238.0, 130.0, 130.0, "", accent);
        sk.text_at(248.0, 126.0, egui::Align2::LEFT_CENTER, "t = 1.00 in", accent);
        sk.text_at(248.0, 140.0, egui::Align2::LEFT_CENTER, "\u{2300} 4.00 in bore", accent);
    }
    if emph == "material" {
        sk.leader_label(195.0, 160.0, 290.0, 215.0, "ANSI31 section hatch = material", egui::Align2::RIGHT_CENTER, accent);
    }
}

/// Pressure Vessel - longitudinal side view. `emph`: `"pressure"`,
/// `"end"`, `"buckling"`.
pub fn pv_side_view(ui: &mut egui::Ui, tokens: &Tokens, emph: &str) {
    let sk = Sketch::new(ui, tokens, egui::vec2(420.0, 200.0), 190.0);
    let steel = Color32::from_rgb(0x2c, 0x33, 0x3b);
    sk.hatch_rect(70.0, 60.0, 280.0, 70.0, steel, 13);
    sk.fill_rect(88.0, 69.0, 244.0, 52.0, 24.0, tokens.bg_raised);
    sk.outline_rect(70.0, 60.0, 280.0, 70.0, 34.0, Color32::from_rgb(0xaa, 0xb4, 0xbd));
    sk.outline_rect(88.0, 69.0, 244.0, 52.0, 24.0, Color32::from_rgb(0xaa, 0xb4, 0xbd));
    sk.centerline_h(40.0, 380.0, 95.0);

    let accent = tokens.accent_strong;
    if emph == "pressure" {
        for x in [150.0, 190.0, 230.0] {
            sk.arrow_label(x, 69.0, x, 50.0, accent);
            sk.arrow_label(x, 121.0, x, 140.0, accent);
        }
        sk.text_at(190.0, 42.0, egui::Align2::CENTER_CENTER, "Pi = 5000 psi (acts outward on the wall)", accent);
    }
    if emph == "end" {
        sk.leader_label(78.0, 95.0, 30.0, 165.0, "Closed end (dome cap)", egui::Align2::LEFT_CENTER, accent);
        sk.leader_label(342.0, 95.0, 395.0, 165.0, "Po = 0 psi (external)", egui::Align2::RIGHT_CENTER, accent);
    }
    if emph == "buckling" {
        sk.dim_h(150.0, 270.0, 168.0, 130.0, "L = 0 in — no supports entered", accent);
    }
}

/// Bushing - head-on (top view of housing + bushing). `emph`: `"repair"`
/// (housing length/width/edge distance), `"geom"` (bushing ID/OD).
pub fn bushing_head_on(ui: &mut egui::Ui, tokens: &Tokens, emph: &str) {
    let sk = Sketch::new(ui, tokens, egui::vec2(460.0, 250.0), 220.0);
    let steel = Color32::from_rgb(0x2c, 0x33, 0x3b);
    let bronze = Color32::from_rgb(0x36, 0x2c, 0x20);
    sk.hatch_rect(40.0, 35.0, 280.0, 170.0, steel, 10);
    sk.outline_rect(40.0, 35.0, 280.0, 170.0, 8.0, Color32::from_rgb(0xaa, 0xb4, 0xbd));
    sk.hatch_ring(140.0, 120.0, 46.0, 26.0, bronze, 14);
    sk.centerline_h(70.0, 220.0, 120.0);
    sk.centerline_v(60.0, 180.0, 140.0);
    sk.center_mark(140.0, 120.0, 8.0);

    let accent = tokens.accent_strong;
    if emph == "repair" {
        sk.dim_h(40.0, 320.0, 18.0, 35.0, "Housing length — 1.25 in", accent);
        sk.dim_v(35.0, 205.0, 20.0, 40.0, "Width — 2.00 in", accent);
        sk.dim_h(40.0, 94.0, 178.0, 120.0, "edge — 0.375 in", accent);
    }
    if emph == "geom" {
        sk.dim_h(114.0, 166.0, 55.0, 94.0, "\u{2300} 0.500 in — Bushing ID", accent);
        sk.leader_label(140.0, 166.0, 255.0, 222.0, "\u{2300} 0.875 in — Bushing OD", egui::Align2::LEFT_CENTER, accent);
    }
}

/// Bushing - longitudinal section through housing + bushing. `emph`:
/// `"geom"` (bushing length/flange), `"material"` (housing vs. bushing
/// material), `"fit"` (interference at the OD/bore interface).
pub fn bushing_side_view(ui: &mut egui::Ui, tokens: &Tokens, emph: &str) {
    let sk = Sketch::new(ui, tokens, egui::vec2(460.0, 220.0), 200.0);
    let steel = Color32::from_rgb(0x2c, 0x33, 0x3b);
    let bronze = Color32::from_rgb(0x36, 0x2c, 0x20);
    sk.hatch_rect(60.0, 40.0, 260.0, 110.0, steel, 10);
    sk.fill_rect(60.0, 75.0, 260.0, 40.0, 0.0, tokens.bg_raised);
    sk.hatch_rect(48.0, 55.0, 26.0, 80.0, bronze, 14);
    sk.hatch_rect(74.0, 65.0, 200.0, 60.0, bronze, 14);
    sk.fill_rect(80.0, 78.0, 188.0, 34.0, 0.0, tokens.bg_raised);
    sk.outline_rect(60.0, 40.0, 260.0, 110.0, 4.0, Color32::from_rgb(0xaa, 0xb4, 0xbd));
    sk.outline_rect(48.0, 55.0, 26.0, 80.0, 0.0, Color32::from_rgb(0xc9, 0xa0, 0x6a));
    sk.outline_rect(74.0, 65.0, 200.0, 60.0, 0.0, Color32::from_rgb(0xc9, 0xa0, 0x6a));
    sk.centerline_h(30.0, 350.0, 95.0);

    let accent = tokens.accent_strong;
    if emph == "geom" {
        sk.dim_h(86.0, 270.0, 25.0, 55.0, "Bushing length — 1.25 in", accent);
        sk.dim_v(55.0, 135.0, 30.0, 55.0, "Flange \u{2300} 1.10 in", accent);
        sk.leader_label(61.0, 58.0, 20.0, 18.0, "Flange t — 0.062 in", egui::Align2::LEFT_CENTER, accent);
    }
    if emph == "material" {
        sk.leader_label(290.0, 60.0, 350.0, 40.0, "Housing: Al 2024-T3 (typical)", egui::Align2::LEFT_CENTER, accent);
        sk.leader_label(100.0, 95.0, 40.0, 175.0, "Bushing: Cres 15-5PH", egui::Align2::RIGHT_CENTER, accent);
    }
    if emph == "fit" {
        sk.dim_v(65.0, 125.0, 74.0, 74.0, "", accent);
        sk.dim_v(65.0, 125.0, 274.0, 274.0, "", accent);
        sk.leader_label(74.0, 150.0, 160.0, 178.0, "\u{394} 0.0015 in diametral interference (both sides)", egui::Align2::LEFT_CENTER, accent);
    }
}
