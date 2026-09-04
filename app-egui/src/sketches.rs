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

use bushing_solver::geometry::BushingType;

use crate::theme::Tokens;

/// Everything `bushing_side_view` needs to draw the REAL configuration
/// being analyzed - the solved head geometry (`cs_dia`/`cs_depth`, from
/// `BushingOutput::cs_solved_od` when the head is countersunk, so the
/// member's counterbore always matches the head by construction, not a
/// separately hand-picked shape) plus the drafting-only fields
/// (`bushing_length`, both edge chamfers) `BushingTool::inputs()` never
/// feeds into `bushing_solver` itself.
pub struct BushingSketchCtx {
    pub head_type: BushingType,
    pub housing_len: f64,
    pub bushing_length: f64,
    pub flange_od: f64,
    pub flange_thk: f64,
    pub cs_dia: f64,
    pub cs_depth: f64,
    pub lower_chamfer_min: f64,
    pub lower_chamfer_max: f64,
    pub lower_chamfer_angle_deg: f64,
    pub head_chamfer_min: f64,
    pub head_chamfer_max: f64,
    pub head_chamfer_angle_deg: f64,
    pub od_installed: f64,
}

/// Which of the two hatch treatments `hatch_rect`/`hatch_ring` uses -
/// matches the mockup's `g1`/`h1` (steel/housing) vs. `g2`/`h2` (bronze/
/// bushing) SVG `<pattern>`s exactly: crossed hatch angle, warm-vs-cool
/// line tint, and a distinct base fill color.
#[derive(Clone, Copy)]
pub enum Material {
    Steel,
    Bronze,
}

impl Material {
    fn base(self) -> Color32 {
        match self {
            Material::Steel => Color32::from_rgb(0x2c, 0x33, 0x3b),
            Material::Bronze => Color32::from_rgb(0x36, 0x2c, 0x20),
        }
    }
    fn line_color(self) -> Color32 {
        match self {
            Material::Steel => Color32::from_white_alpha(13),
            Material::Bronze => Color32::from_rgba_unmultiplied(0xff, 0xd6, 0xaa, 18),
        }
    }
    fn reversed(self) -> bool {
        matches!(self, Material::Bronze)
    }
}

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
    ///
    /// `Material::Steel`/`Material::Bronze` cross the hatch angle (45°
    /// vs. -45°, matching the mockup's own `h1`/`h2` SVG `<pattern>`s -
    /// `patternTransform="rotate(45)"` vs. `"rotate(-45)"`) and tint the
    /// line color (cool near-white for steel vs. warm peach for bronze,
    /// matching `rgba(255,255,255,.05)` vs. `rgba(255,214,170,.07)`) - a
    /// real ANSI drafting convention (different hatch angle = different
    /// material call-out) the first port dropped, using the same angle
    /// and pure-white lines for both. A real screenshot showed exactly
    /// what that costs: the housing and the bushing read as one
    /// undifferentiated hatched blob instead of an assembly of two
    /// distinct parts.
    pub fn hatch_rect(&self, x: f32, y: f32, w: f32, h: f32, material: Material) {
        let rect = Rect::from_min_size(self.p(x, y), egui::vec2(self.s(w), self.s(h)));
        self.painter.rect_filled(rect, 0.0, material.base());
        let step = self.s(9.0).max(4.0);
        let line_color = material.line_color();
        let mut off = -rect.height();
        while off < rect.width() {
            let x0 = rect.left() + off;
            let (a, b) = if material.reversed() {
                (egui::pos2(x0, rect.top()), egui::pos2(x0 + rect.height(), rect.bottom()))
            } else {
                (egui::pos2(x0, rect.bottom()), egui::pos2(x0 + rect.height(), rect.top()))
            };
            self.painter.with_clip_rect(rect).line_segment([a, b], Stroke::new(1.0, line_color));
            off += step;
        }
    }
    pub fn hatch_ring(&self, cx: f32, cy: f32, r_outer: f32, r_inner: f32, material: Material) {
        self.hatch_rect(cx - r_outer, cy - r_outer, r_outer * 2.0, r_outer * 2.0, material);
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

    /// Bold downward "applied force" arrow with an `F` label above it -
    /// matches the reference press-fit installation drawing (a real
    /// user-provided image, not a guess) that shows every bushing
    /// install step with a heavy downward `F` arrow driving the part.
    pub fn force_arrow(&self, x: f32, y1: f32, y2: f32, color: Color32) {
        let a = self.p(x, y1);
        let b = self.p(x, y2);
        self.painter.line_segment([a, b], Stroke::new(2.5, color));
        self.arrow(b, b - a, color);
        self.painter.text(egui::pos2(a.x, a.y - self.s(4.0)), egui::Align2::CENTER_BOTTOM, "F", FontId::proportional(self.s(15.0).max(12.0)), color);
    }

    /// Cuts a right-triangle wedge (filled with `fill`, typically the
    /// background color) out of whatever's already painted at `corner` -
    /// the real 45° lead-in chamfer confirmed at both bore openings of an
    /// actual reference part (`CONICAL_SURFACE` half-angle exactly
    /// π/4 in the user-provided STEP file) that this sketch's earlier,
    /// sharp-square-cornered bore never showed at all. `dx`/`dy` are
    /// signed offsets along each edge from the corner - equal magnitude
    /// gives the real 45° angle; sign picks which of the four corners.
    pub fn chamfer_notch(&self, x: f32, y: f32, dx: f32, dy: f32, fill: Color32) {
        let p0 = self.p(x, y);
        let p1 = self.p(x + dx, y);
        let p2 = self.p(x, y + dy);
        self.painter.add(egui::Shape::convex_polygon(vec![p0, p1, p2], fill, Stroke::NONE));
    }

    /// Arbitrary filled polygon in view-space coordinates - the general
    /// case `chamfer_notch`'s fixed right-triangle shape doesn't cover
    /// (the countersunk counterbore's trapezoid, the flange's rect).
    pub fn poly_filled(&self, pts: &[(f32, f32)], fill: Color32) {
        let poly: Vec<Pos2> = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(egui::Shape::convex_polygon(poly, fill, Stroke::NONE));
    }
    pub fn poly_stroke(&self, pts: &[(f32, f32)], stroke: Color32) {
        let poly: Vec<Pos2> = pts.iter().map(|&(x, y)| self.p(x, y)).collect();
        self.painter.add(egui::Shape::line(poly, Stroke::new(1.3, stroke)));
    }

    /// Edge break at the head's outermost corner. `angle_deg` near `0`
    /// draws a real square/vertical relief step (NOT a degenerate
    /// zero-width wedge - a genuinely different shape, per the user's own
    /// confirmed "normal to the head surface" default); any larger angle
    /// draws an angled wedge sized relative to a 45° reference (matching
    /// `chamfer_notch`'s equal-leg convention at 45°). `dx_sign`/`dy_sign`
    /// pick which of the four corner orientations this is.
    pub fn edge_relief(&self, x: f32, y: f32, size: f32, angle_deg: f64, dx_sign: f32, dy_sign: f32, fill: Color32) {
        if angle_deg.abs() < 0.5 {
            let w = size * 0.55;
            self.painter.rect_filled(Rect::from_two_pos(self.p(x, y), self.p(x + dx_sign * w, y + dy_sign * size)), 0.0, fill);
        } else {
            let dx = dx_sign * size * (angle_deg.to_radians().tan() as f32).max(0.05);
            let dy = dy_sign * size;
            self.chamfer_notch(x, y, dx, dy, fill);
        }
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

    /// ANSI section-cut marker: a bold tick + inward-pointing arrow and a
    /// letter at each end of a cutting plane - the "A     A" convention
    /// every reference drafting sheet in this pass uses to tie a plan/
    /// head-on view to the section view it produces. Drawn ALONG an
    /// existing centerline, not replacing it.
    pub fn section_cut(&self, x1: f32, x2: f32, y: f32, letter: &str, color: Color32) {
        for &x in &[x1, x2] {
            let a = self.p(x, y - 11.0);
            let b = self.p(x, y + 11.0);
            self.painter.line_segment([a, b], Stroke::new(2.2, color));
            self.arrow(b, egui::vec2(0.0, 1.0), color);
            self.painter.text(self.p(x, y - 18.0), egui::Align2::CENTER_BOTTOM, letter, FontId::proportional(self.s(12.0).max(10.0)), color);
        }
    }

    /// "SECTION A-A"-style caption under a section view, tying it back to
    /// the marker `section_cut` drew on the companion plan view.
    pub fn section_caption(&self, x: f32, y: f32, label: &str, color: Color32) {
        self.painter.text(self.p(x, y), egui::Align2::CENTER_TOP, label, FontId::proportional(self.s(11.5).max(10.0)), color);
    }
}

fn ellipse_pts(cx: f32, cy: f32, rx: f32, ry: f32, n: usize) -> Vec<(f32, f32)> {
    (0..=n)
        .map(|i| {
            let t = i as f32 / n as f32 * std::f32::consts::TAU;
            (cx + rx * t.cos(), cy + ry * t.sin())
        })
        .collect()
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
    sk.hatch_ring(170.0, 130.0, 68.0, 38.0, Material::Steel);
    sk.centerline_h(60.0, 280.0, 130.0);
    sk.centerline_v(40.0, 220.0, 170.0);
    sk.center_mark(170.0, 130.0, 8.0);
    sk.section_cut(60.0, 280.0, 130.0, "A", Color32::from_rgb(0xc9, 0x76, 0x5f));

    let accent = tokens.accent_strong;
    if emph == "od" || emph == "wall" {
        sk.dim_h(102.0, 238.0, 32.0, 130.0, "\u{d8} 6.00 in — Outer diameter", accent);
    }
    if emph == "wall" {
        sk.dim_h(208.0, 238.0, 130.0, 130.0, "", accent);
        sk.text_at(248.0, 126.0, egui::Align2::LEFT_CENTER, "t = 1.00 in", accent);
        sk.text_at(248.0, 140.0, egui::Align2::LEFT_CENTER, "\u{d8} 4.00 in bore", accent);
    }
    if emph == "material" {
        sk.leader_label(195.0, 160.0, 290.0, 215.0, "ANSI31 section hatch = material", egui::Align2::RIGHT_CENTER, accent);
    }
}

/// Pressure Vessel - longitudinal side view. `emph`: `"pressure"`,
/// `"end"`, `"buckling"`.
pub fn pv_side_view(ui: &mut egui::Ui, tokens: &Tokens, emph: &str) {
    let sk = Sketch::new(ui, tokens, egui::vec2(420.0, 200.0), 190.0);
    sk.hatch_rect(70.0, 60.0, 280.0, 70.0, Material::Steel);
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
    sk.section_caption(225.0, 172.0, "SECTION A-A", Color32::from_rgb(0xc9, 0x76, 0x5f));
}

/// Small shaded pseudo-isometric preview, same "not a real 3D render,
/// fixed-angle ellipse-and-wall silhouette" approach and scope as
/// `bushing_isometric` - see that function's doc comment for why. A
/// plain cylinder (this file's PV sketches have always been illustrative
/// fixed proportions, not wired to `PvContext`'s real geometry - a
/// pre-existing convention, not something this pass changed).
pub fn pv_isometric(ui: &mut egui::Ui, tokens: &Tokens) {
    let sk = Sketch::new(ui, tokens, egui::vec2(420.0, 190.0), 170.0);
    let cx = 210.0_f32;
    let squash = 0.4_f32;
    let r = 62.0_f32;
    let top_y = 34.0_f32;
    let bottom_y = 150.0_f32;

    let base = Material::Steel.base();
    let top_face = base.gamma_multiply(1.7);
    let side_face = base.gamma_multiply(0.95);
    let stroke = Color32::from_rgb(0xaa, 0xb4, 0xbd);

    sk.fill_rect(cx - r, top_y, r * 2.0, bottom_y - top_y, 0.0, side_face);
    let bottom = ellipse_pts(cx, bottom_y, r, r * squash, 32);
    sk.poly_filled(&bottom, side_face.gamma_multiply(0.85));
    sk.poly_stroke(&bottom, stroke);
    let top = ellipse_pts(cx, top_y, r, r * squash, 32);
    sk.poly_filled(&top, top_face);
    sk.poly_stroke(&top, stroke);
    let bore = ellipse_pts(cx, top_y, r * 0.68, r * squash * 0.68, 28);
    sk.poly_filled(&bore, tokens.bg_sunken);
    sk.poly_stroke(&bore, stroke);

    sk.text_at(cx, bottom_y + 18.0, egui::Align2::CENTER_TOP, "ISOMETRIC \u{2014} SCHEMATIC, NOT TO SCALE", tokens.fg_subtle);
}

/// Bushing - head-on (top view of housing + bushing). `emph`: `"repair"`
/// (housing length/width/edge distance), `"geom"` (bushing ID/OD).
pub fn bushing_head_on(ui: &mut egui::Ui, tokens: &Tokens, emph: &str) {
    let sk = Sketch::new(ui, tokens, egui::vec2(460.0, 250.0), 220.0);
    sk.hatch_rect(40.0, 35.0, 280.0, 170.0, Material::Steel);
    sk.outline_rect(40.0, 35.0, 280.0, 170.0, 8.0, Color32::from_rgb(0xaa, 0xb4, 0xbd));
    sk.hatch_ring(140.0, 120.0, 46.0, 26.0, Material::Bronze);
    sk.centerline_h(70.0, 220.0, 120.0);
    sk.centerline_v(60.0, 180.0, 140.0);
    sk.center_mark(140.0, 120.0, 8.0);
    // Ties this plan view to `bushing_side_view`'s own "SECTION A-A"
    // caption - every reference sheet in this pass uses exactly this
    // convention to show which cut produced the section view alongside
    // it, and this sketch never had one before.
    sk.section_cut(40.0, 320.0, 120.0, "A", Color32::from_rgb(0xc9, 0x76, 0x5f));

    let accent = tokens.accent_strong;
    if emph == "repair" {
        sk.dim_h(40.0, 320.0, 18.0, 35.0, "Housing length — 1.25 in", accent);
        sk.dim_v(35.0, 205.0, 20.0, 40.0, "Width — 2.00 in", accent);
        sk.dim_h(40.0, 94.0, 178.0, 120.0, "edge — 0.375 in", accent);
    }
    if emph == "geom" {
        sk.dim_h(114.0, 166.0, 55.0, 94.0, "\u{d8} 0.500 in — Bushing ID", accent);
        sk.leader_label(140.0, 166.0, 255.0, 222.0, "\u{d8} 0.875 in — Bushing OD", egui::Align2::LEFT_CENTER, accent);
    }
}

/// Bushing - longitudinal section through housing + bushing. `emph`:
/// `"geom"` (bushing length/head), `"material"` (housing vs. bushing
/// material), `"fit"` (interference at the OD/bore interface).
///
/// Draws the REAL configuration being analyzed, not a generic reference
/// shape (a real user correction: an earlier pass modeled this after a
/// general reference photo and drew two countersinks on opposite ends of
/// the member - that isn't any of the three real repair-bushing types).
/// Three distinct head types, reusing `bushing_solver::geometry::
/// BushingType` directly (`Straight` = slug/no head, `Flanged`,
/// `Countersink`), each with its own real member/bushing flush
/// relationship:
/// - Slug: no head feature; the bushing's head-end face sits flush with
///   the member's own upper (head-end) face.
/// - Countersunk: the head's actual solved cone (`ctx.cs_dia`/
///   `ctx.cs_depth`, sourced from `BushingOutput::cs_solved_od` - the
///   SAME geometry the solver validates, not a separate hand-picked
///   shape) sits recessed into a matching counterbore cut into the
///   member, head flush with the member's upper face.
/// - Flanged: the flange's lower face sits flush with the member's upper
///   face; the flange itself protrudes above/outside the member.
///
/// The opposite (lower) end's position is derived from `bushing_length`
/// vs. `housing_len` (both real inputs, not hardcoded): equal → flush
/// with the member's lower face too; unequal → a visible protrusion or
/// recess with a real leader-label annotation, never silently drawn as
/// flush. One continuous through-bore for all three types ("center
/// drilled to the inner diameter specs" applies uniformly - the
/// countersunk head's cone is an OD-side/external feature layered on top
/// of the same straight bore, not a separate internal-ID feature).
/// `bushing_length` and the two edge chamfers are drafting-only - see
/// `BushingTool::inputs()`'s doc comment for why they aren't fed into
/// `bushing_solver`.
pub fn bushing_side_view(ui: &mut egui::Ui, tokens: &Tokens, emph: &str, ctx: &BushingSketchCtx) {
    let sk = Sketch::new(ui, tokens, egui::vec2(460.0, 220.0), 200.0);
    let outline = Color32::from_rgb(0xaa, 0xb4, 0xbd);
    let bushing_outline = Color32::from_rgb(0xc9, 0xa0, 0x6a);
    let bg = tokens.bg_raised;

    let member_x0 = 60.0_f32;
    let member_x1 = 320.0_f32;
    let member_y0 = 40.0_f32;
    let member_y1 = 150.0_f32;
    let cy = 95.0_f32;
    let sleeve_half_h = 30.0_f32;

    // Member is always drawn at a fixed 260px width representing the
    // current `housing_len` - other real lengths (bushing_length) are
    // scaled relative to it, not to an absolute physical unit (this
    // sketch was never to-scale - bore/OD bands are fixed pixel bands
    // too, dimensioned via labels instead).
    let px_per_in = 260.0 / ctx.housing_len.max(0.05) as f32;
    let sleeve_len_px = (ctx.bushing_length.max(0.0) as f32 * px_per_in).clamp(20.0, 420.0);
    let sleeve_x0 = member_x0;
    let sleeve_x1 = sleeve_x0 + sleeve_len_px;

    sk.hatch_rect(member_x0, member_y0, member_x1 - member_x0, member_y1 - member_y0, Material::Steel);
    sk.fill_rect(member_x0, cy - sleeve_half_h - 5.0, member_x1 - member_x0, (sleeve_half_h + 5.0) * 2.0, 0.0, bg);

    // Head feature at the member's upper (head-end) face, x = member_x0.
    // `sleeve_body_x0` is where the PLAIN-diameter sleeve begins - for a
    // countersunk head that's after the cone's own depth, not at
    // member_x0 too (drawing both at the same x used to let the sleeve
    // rect paint straight over the cone, hiding it almost entirely - a
    // real, screenshot-confirmed rendering bug, not a subtlety).
    let (head_outer_x, head_half_h, sleeve_body_x0) = match ctx.head_type {
        BushingType::Flanged => {
            let flange_px_thk = (ctx.flange_thk.max(0.0) as f32 * px_per_in).clamp(8.0, 60.0);
            let half_h = (sleeve_half_h * (ctx.flange_od / ctx.od_installed.max(0.01)) as f32).clamp(sleeve_half_h + 6.0, sleeve_half_h * 1.9);
            let x0 = member_x0 - flange_px_thk;
            sk.hatch_rect(x0, cy - half_h, flange_px_thk, half_h * 2.0, Material::Bronze);
            sk.outline_rect(x0, cy - half_h, flange_px_thk, half_h * 2.0, 0.0, bushing_outline);
            (x0, half_h, member_x0)
        }
        BushingType::Countersink => {
            let depth_px = (ctx.cs_depth.max(0.0) as f32 * px_per_in).clamp(10.0, 70.0);
            let half_h = (sleeve_half_h * (ctx.cs_dia / ctx.od_installed.max(0.01)) as f32).clamp(sleeve_half_h + 8.0, sleeve_half_h * 1.7);
            // Counterbore cut into the member - matches the head's own
            // solved cone by construction (same `cs_dia`/`cs_depth` the
            // head itself is sized from).
            sk.poly_filled(
                &[(member_x0, cy - half_h), (member_x0 + depth_px, cy - sleeve_half_h), (member_x0 + depth_px, cy + sleeve_half_h), (member_x0, cy + half_h)],
                Material::Bronze.base(),
            );
            sk.poly_stroke(&[(member_x0, cy - half_h), (member_x0 + depth_px, cy - sleeve_half_h)], bushing_outline);
            sk.poly_stroke(&[(member_x0, cy + half_h), (member_x0 + depth_px, cy + sleeve_half_h)], bushing_outline);
            (member_x0, half_h, member_x0 + depth_px)
        }
        BushingType::Straight => (member_x0, sleeve_half_h, member_x0),
    };

    // Sleeve - the through-going plain-diameter tube, common to all
    // three types (starts after the cone's own depth for a countersunk
    // head, so the cone stays visible instead of being painted over).
    let sleeve_body_x0 = sleeve_body_x0.min(sleeve_x1);
    sk.hatch_rect(sleeve_body_x0, cy - sleeve_half_h, sleeve_x1 - sleeve_body_x0, sleeve_half_h * 2.0, Material::Bronze);
    sk.outline_rect(sleeve_body_x0, cy - sleeve_half_h, sleeve_x1 - sleeve_body_x0, sleeve_half_h * 2.0, 0.0, bushing_outline);

    // Bore: one continuous hole through the head and sleeve. If the
    // sleeve doesn't reach the member's far end, the bore keeps going
    // through the remaining (unlined) member thickness - a real
    // structural difference, not just visual filler.
    let bore_half_h = sleeve_half_h * 0.55;
    let sleeve_end_clamped = sleeve_x1.min(member_x1).max(sleeve_x0 + 4.0);
    sk.fill_rect(head_outer_x, cy - bore_half_h, sleeve_end_clamped - head_outer_x, bore_half_h * 2.0, 0.0, bg);
    if sleeve_x1 < member_x1 - 1.0 {
        sk.fill_rect(sleeve_x1, cy - bore_half_h, member_x1 - sleeve_x1, bore_half_h * 2.0, 0.0, bg);
    }

    // Lower-end chamfer - always at the end opposite the head, per the
    // repair-bushing convention confirmed by the user.
    let lower_x = sleeve_x1.min(member_x1);
    let lower_dim_px = (((ctx.lower_chamfer_min + ctx.lower_chamfer_max) / 2.0) as f32 * px_per_in).clamp(5.0, 14.0);
    sk.edge_relief(lower_x, cy - bore_half_h, lower_dim_px, ctx.lower_chamfer_angle_deg, -1.0, 1.0, bg);
    sk.edge_relief(lower_x, cy + bore_half_h, lower_dim_px, ctx.lower_chamfer_angle_deg, -1.0, -1.0, bg);

    // Head-top-edge chamfer, at the head's own outermost (largest-
    // radius) corner - the flange's OD edge, the countersink cone's rim,
    // or (slug) the sleeve's own OD corner.
    let head_dim_px = (((ctx.head_chamfer_min + ctx.head_chamfer_max) / 2.0) as f32 * px_per_in).clamp(5.0, 14.0);
    sk.edge_relief(head_outer_x, cy - head_half_h, head_dim_px, ctx.head_chamfer_angle_deg, 1.0, 1.0, bg);
    sk.edge_relief(head_outer_x, cy + head_half_h, head_dim_px, ctx.head_chamfer_angle_deg, 1.0, -1.0, bg);

    sk.outline_rect(member_x0, member_y0, member_x1 - member_x0, member_y1 - member_y0, 4.0, outline);
    if (sleeve_x1 - member_x1).abs() > 1.0 {
        // The bushing's own lower face differs from the member's true
        // lower face - draw both, don't silently merge them.
        sk.centerline_v(cy - 55.0, cy + 55.0, sleeve_x1.min(member_x1 + 40.0));
    }
    sk.centerline_h(30.0, 350.0, cy);
    sk.force_arrow((head_outer_x + sleeve_end_clamped) / 2.0, 12.0, 38.0, outline);
    sk.section_caption(190.0, 205.0, "SECTION A-A", Color32::from_rgb(0xc9, 0x76, 0x5f));

    // Fillet callouts at the head-to-sleeve transition - every reference
    // sheet in this pass calls these out explicitly. Drawn as a leader
    // label only, not a literal rounded corner (this sketch's polygon
    // primitives don't do per-corner rounding) - a disclosed
    // simplification, same spirit as the hatch-ring bounding-box
    // approximation this file's own module doc already calls out. A slug
    // has no shoulder feature to fillet, so it gets none.
    // Each type's fillet callout gets its OWN clear lane rather than one
    // shared y - a real collision found by screenshot: Countersink's
    // other label sits near y=30, but Flanged's own "Flange \u{d8}.../t..."
    // label (wide, LEFT_CENTER from the far left edge) is centered at
    // y=cy=95, so the same y=85 that was clear for one type ran straight
    // through the other's text.
    match ctx.head_type {
        BushingType::Flanged => sk.leader_label(member_x0, cy - head_half_h + 4.0, member_x0 + 34.0, 45.0, "R.06 typ. fillet", egui::Align2::LEFT_CENTER, outline),
        BushingType::Countersink => {
            let depth_px = (ctx.cs_depth.max(0.0) as f32 * px_per_in).clamp(10.0, 70.0);
            sk.leader_label(member_x0 + depth_px, cy - sleeve_half_h, member_x0 + depth_px + 20.0, 85.0, "R.03 typ. fillet", egui::Align2::LEFT_CENTER, outline)
        }
        BushingType::Straight => {}
    }

    let accent = tokens.accent_strong;
    if emph == "geom" {
        sk.dim_h(sleeve_x0 + 4.0, (sleeve_x1 - 4.0).max(sleeve_x0 + 8.0), 178.0, 158.0, &format!("Bushing length \u{2014} {:.3} in", ctx.bushing_length), accent);
        let len_delta = ctx.bushing_length - ctx.housing_len;
        if len_delta.abs() > 0.0005 {
            let word = if len_delta > 0.0 { "protrudes" } else { "recessed" };
            let anchor_x = sleeve_x1.min(member_x1);
            // Anchored above the part (not the bottom-right corner the
            // lower-edge-break/length labels already occupy) - a real
            // screenshot showed these two collide, same row, overlapping
            // text spans, when both lived at the bottom.
            sk.leader_label(anchor_x, cy - sleeve_half_h - 4.0, anchor_x + 15.0, 20.0, &format!("{} {:.3} in vs. member", word, len_delta.abs()), egui::Align2::LEFT_CENTER, accent);
        }
        match ctx.head_type {
            BushingType::Flanged => sk.dim_v(cy - head_half_h, cy + head_half_h, 14.0, head_outer_x - 6.0, &format!("Flange \u{d8}{:.3} in / t {:.3} in", ctx.flange_od, ctx.flange_thk), accent),
            BushingType::Countersink => {
                sk.leader_label(member_x0 + 10.0, cy - head_half_h + 4.0, member_x0 + 40.0, 30.0, &format!("Countersunk head, \u{d8}{:.3} in", ctx.cs_dia), egui::Align2::LEFT_CENTER, accent)
            }
            BushingType::Straight => sk.leader_label(sleeve_x0, cy - sleeve_half_h, sleeve_x0 - 6.0, 30.0, "Slug (no head)", egui::Align2::RIGHT_CENTER, accent),
        }
        sk.leader_label(lower_x, cy + bore_half_h, lower_x - 10.0, 190.0, &format!("{:.0}\u{b0} lower edge break", ctx.lower_chamfer_angle_deg), egui::Align2::RIGHT_CENTER, accent);
    }
    if emph == "material" {
        sk.leader_label(290.0, 60.0, 350.0, 40.0, "Housing: Al 2024-T3 (typical)", egui::Align2::LEFT_CENTER, accent);
        sk.leader_label(100.0, 95.0, 40.0, 175.0, "Bushing: Cres 15-5PH", egui::Align2::RIGHT_CENTER, accent);
    }
    if emph == "fit" {
        sk.dim_v(65.0, 125.0, sleeve_x0, sleeve_x0, "", accent);
        sk.dim_v(65.0, 125.0, sleeve_end_clamped, sleeve_end_clamped, "", accent);
        sk.leader_label(sleeve_x0, 150.0, 160.0, 178.0, "\u{394} interference (both sides)", egui::Align2::LEFT_CENTER, accent);
    }
}

/// Small shaded pseudo-isometric preview - NOT a real 3D render (egui
/// has no 3D pipeline outside `PaintCallback` for external engine
/// integration, out of scope for this crate); a fixed-angle axonometric
/// silhouette (ellipses for the round top/bottom faces + straight side
/// walls, flat 2-tone shading, no real lighting model) built from the
/// same head-type logic `bushing_side_view` uses, so it always matches
/// the configuration being analyzed rather than being a static
/// illustration. Every reference sheet this pass was grounded in pairs
/// its orthographic views with exactly this kind of shaded preview -
/// labeled "(SCHEMATIC)" in the caller so it's never mistaken for a real
/// rendered solid.
pub fn bushing_isometric(ui: &mut egui::Ui, tokens: &Tokens, ctx: &BushingSketchCtx) {
    let sk = Sketch::new(ui, tokens, egui::vec2(460.0, 210.0), 190.0);
    let cx = 230.0_f32;
    let squash = 0.42_f32;
    let sleeve_r = 30.0_f32;
    let bore_r = sleeve_r * 0.45;
    let axis_len = 100.0_f32;

    let base = Material::Bronze.base();
    let top_face = base.gamma_multiply(1.7);
    let side_face = base.gamma_multiply(0.9);
    let bore_color = tokens.bg_sunken;
    let stroke = Color32::from_rgb(0xc9, 0xa0, 0x6a);

    let head_top_y = match ctx.head_type {
        BushingType::Flanged => 44.0,
        BushingType::Countersink => 34.0,
        BushingType::Straight => 24.0,
    };
    let sleeve_top_y = match ctx.head_type {
        BushingType::Flanged => head_top_y + 16.0,
        BushingType::Countersink => head_top_y + 20.0,
        BushingType::Straight => head_top_y,
    };
    let bottom_y = sleeve_top_y + axis_len;

    // Main sleeve wall + bottom cap (drawn first, so head features layer
    // on top of it).
    sk.fill_rect(cx - sleeve_r, sleeve_top_y, sleeve_r * 2.0, bottom_y - sleeve_top_y, 0.0, side_face);
    let bottom_ellipse = ellipse_pts(cx, bottom_y, sleeve_r, sleeve_r * squash, 28);
    sk.poly_filled(&bottom_ellipse, side_face.gamma_multiply(0.85));
    sk.poly_stroke(&bottom_ellipse, stroke);

    match ctx.head_type {
        BushingType::Straight => {
            let top = ellipse_pts(cx, sleeve_top_y, sleeve_r, sleeve_r * squash, 28);
            sk.poly_filled(&top, top_face);
            sk.poly_stroke(&top, stroke);
        }
        BushingType::Flanged => {
            let flange_r = sleeve_r * 1.65;
            sk.fill_rect(cx - flange_r, head_top_y, flange_r * 2.0, sleeve_top_y - head_top_y, 0.0, side_face.gamma_multiply(1.1));
            let flange_bottom = ellipse_pts(cx, sleeve_top_y, flange_r, flange_r * squash, 28);
            sk.poly_filled(&flange_bottom, side_face.gamma_multiply(0.85));
            sk.poly_stroke(&flange_bottom, stroke);
            let flange_top = ellipse_pts(cx, head_top_y, flange_r, flange_r * squash, 28);
            sk.poly_filled(&flange_top, top_face);
            sk.poly_stroke(&flange_top, stroke);
        }
        BushingType::Countersink => {
            let rim_r = sleeve_r * 1.35;
            // Frustum side (approximated with a filled quad rather than a
            // true swept surface - a flat-shaded stand-in, not a curved
            // shade).
            sk.poly_filled(
                &[(cx - rim_r, head_top_y), (cx - sleeve_r, sleeve_top_y), (cx + sleeve_r, sleeve_top_y), (cx + rim_r, head_top_y)],
                side_face.gamma_multiply(1.1),
            );
            let rim = ellipse_pts(cx, head_top_y, rim_r, rim_r * squash, 28);
            sk.poly_filled(&rim, top_face);
            sk.poly_stroke(&rim, stroke);
        }
    }

    // Bore opening in whichever face is now on top.
    let bore_y = head_top_y;
    let bore = ellipse_pts(cx, bore_y, bore_r, bore_r * squash, 20);
    sk.poly_filled(&bore, bore_color);
    sk.poly_stroke(&bore, stroke);

    sk.text_at(cx, bottom_y + 18.0, egui::Align2::CENTER_TOP, "ISOMETRIC \u{2014} SCHEMATIC, NOT TO SCALE", tokens.fg_subtle);
}
