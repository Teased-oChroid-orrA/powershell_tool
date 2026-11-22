//! The dashboard's workbench glyphs, painted with the egui painter (no image assets, so they follow the palette and
//! scale). Each is drawn in a 48 x 48 design space: dim strokes for the part, accent strokes for what the workbench
//! computes. Ported from `.audit/dashboard-modern-mockup.html`.
use super::theme::Palette;
use egui::{epaint::PathStroke, Color32, Painter, Pos2, Rect, Shape, Stroke};

/// What a shape is drawn with.
#[derive(Clone, Copy)]
enum Ink {
    Dim,
    Accent,
}

struct Pen<'a> {
    painter: &'a Painter,
    origin: Pos2,
    scale: f32,
    p: &'a Palette,
    /// The surface a "punched out" shape is filled with (the glyph box, which tints on hover).
    surface: Color32,
}

impl Pen<'_> {
    fn pt(&self, x: f32, y: f32) -> Pos2 {
        self.origin + egui::vec2(x, y) * self.scale
    }
    fn stroke(&self, ink: Ink) -> Stroke {
        let c = match ink {
            Ink::Dim => self.p.dim,
            Ink::Accent => self.p.accent,
        };
        Stroke::new((1.6 * self.scale).max(1.2), c)
    }
    fn tint(&self) -> Color32 {
        self.p.accent.gamma_multiply(0.3)
    }
    fn rect(&self, (x, y, w, h): (f32, f32, f32, f32), r: f32, ink: Ink, fill: Option<Color32>) {
        let rect = Rect::from_min_max(self.pt(x, y), self.pt(x + w, y + h));
        let r = egui::CornerRadius::same((r * self.scale).round().clamp(0.0, 255.0) as u8);
        if let Some(f) = fill {
            self.painter.rect_filled(rect, r, f);
        }
        self.painter.rect_stroke(rect, r, self.stroke(ink), egui::StrokeKind::Middle);
    }
    fn circle(&self, (x, y): (f32, f32), r: f32, ink: Ink, fill: Option<Color32>) {
        let c = self.pt(x, y);
        if let Some(f) = fill {
            self.painter.circle_filled(c, r * self.scale, f);
        }
        self.painter.circle_stroke(c, r * self.scale, self.stroke(ink));
    }
    fn line(&self, pts: &[(f32, f32)], ink: Ink) {
        self.painter.add(Shape::line(pts.iter().map(|&(x, y)| self.pt(x, y)).collect(), self.stroke(ink)));
    }
    fn closed(&self, pts: &[(f32, f32)], ink: Ink) {
        self.painter.add(Shape::closed_line(pts.iter().map(|&(x, y)| self.pt(x, y)).collect(), self.stroke(ink)));
    }
    fn dashed(&self, pts: &[(f32, f32)], ink: Ink) {
        let pts: Vec<Pos2> = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.painter.extend(Shape::dashed_line(&pts, self.stroke(ink), 2.0 * self.scale, 2.5 * self.scale));
    }
    /// A filled area with no outline (the "force" shading of the FE and stackup glyphs).
    fn shade(&self, pts: &[(f32, f32)]) {
        let pts = pts.iter().map(|&(x, y)| self.pt(x, y)).collect();
        self.painter.add(Shape::convex_polygon(pts, self.tint(), PathStroke::NONE));
    }
    fn ellipse(&self, (cx, cy): (f32, f32), (rx, ry): (f32, f32), ink: Ink) {
        let pts: Vec<(f32, f32)> = (0..48).map(|i| {
            let a = i as f32 / 48.0 * std::f32::consts::TAU;
            (cx + rx * a.cos(), cy + ry * a.sin())
        }).collect();
        self.closed(&pts, ink);
    }
}

fn cubic(p0: (f32, f32), p1: (f32, f32), p2: (f32, f32), p3: (f32, f32), n: usize) -> Vec<(f32, f32)> {
    (0..=n).map(|i| {
        let t = i as f32 / n as f32;
        let u = 1.0 - t;
        let f = |a: f32, b: f32, c: f32, d: f32| u * u * u * a + 3.0 * u * u * t * b + 3.0 * u * t * t * c + t * t * t * d;
        (f(p0.0, p1.0, p2.0, p3.0), f(p0.1, p1.1, p2.1, p3.1))
    }).collect()
}

/// Paint the glyph of workbench `id` filling `rect` (square). `surface` is the colour behind it. False: no glyph for `id`.
pub(super) fn paint(painter: &Painter, rect: Rect, id: &str, p: &Palette, surface: Color32) -> bool {
    let pen = Pen { painter, origin: rect.min, scale: rect.width() / 48.0, p, surface };
    let (d, a) = (Ink::Dim, Ink::Accent);
    let pf = Some(pen.surface);
    let af = Some(pen.tint());
    match id {
        "search" => {
            pen.rect((8.0, 6.0, 24.0, 33.0), 3.0, d, None);
            pen.line(&[(14.0, 14.0), (26.0, 14.0)], d);
            pen.line(&[(14.0, 20.0), (26.0, 20.0)], d);
            pen.line(&[(14.0, 26.0), (20.0, 26.0)], d);
            pen.circle((32.0, 31.0), 7.0, a, pf);
            pen.line(&[(37.0, 36.0), (42.0, 41.0)], a);
        }
        "fastener-hole" => {
            pen.rect((6.0, 10.0, 36.0, 24.0), 2.0, d, None);
            pen.circle((22.0, 22.0), 6.0, a, None);
            pen.dashed(&[(22.0, 13.0), (22.0, 31.0)], a);
            pen.dashed(&[(13.0, 22.0), (31.0, 22.0)], a);
            pen.line(&[(6.0, 40.0), (22.0, 40.0)], d);
            pen.line(&[(6.0, 37.0), (6.0, 43.0)], d);
            pen.line(&[(22.0, 37.0), (22.0, 43.0)], d);
        }
        "assembly" => {
            pen.rect((5.0, 8.0, 30.0, 10.0), 2.0, d, None);
            pen.rect((13.0, 28.0, 30.0, 10.0), 2.0, d, None);
            pen.circle((24.0, 13.0), 2.6, a, None);
            pen.circle((24.0, 33.0), 2.6, a, None);
            pen.dashed(&[(24.0, 20.0), (24.0, 26.0)], a);
        }
        "bushing" => {
            pen.circle((24.0, 24.0), 18.0, d, None);
            pen.circle((24.0, 24.0), 11.0, a, af);
            pen.circle((24.0, 24.0), 6.0, d, pf);
            for l in [[(24.0, 2.0), (24.0, 8.0)], [(24.0, 40.0), (24.0, 46.0)], [(2.0, 24.0), (8.0, 24.0)], [(40.0, 24.0), (46.0, 24.0)]] {
                pen.line(&l, d);
            }
        }
        "bushing-stackup" => {
            for y in [6.0, 19.0, 32.0] {
                pen.rect((6.0, y, 36.0, 9.0), 2.0, d, None);
            }
            pen.shade(&[(20.0, 6.0), (28.0, 6.0), (28.0, 41.0), (20.0, 41.0)]);
            pen.line(&[(20.0, 4.0), (20.0, 44.0)], a);
            pen.line(&[(28.0, 4.0), (28.0, 44.0)], a);
        }
        "pressure-vessel" => {
            pen.rect((5.0, 12.0, 38.0, 20.0), 10.0, d, None);
            pen.ellipse((24.0, 22.0), (4.0, 10.0), a);
            pen.line(&[(14.0, 32.0), (14.0, 37.0)], d);
            pen.line(&[(34.0, 32.0), (34.0, 37.0)], d);
            pen.line(&[(10.0, 37.0), (18.0, 37.0)], d);
            pen.line(&[(30.0, 37.0), (38.0, 37.0)], d);
        }
        "preload" => {
            pen.rect((7.0, 15.0, 34.0, 8.0), 1.5, d, None);
            pen.rect((7.0, 26.0, 34.0, 8.0), 1.5, d, None);
            pen.rect((20.5, 8.0, 7.0, 34.0), 1.0, a, pf);
            pen.rect((15.0, 4.0, 18.0, 5.0), 1.5, a, pf);
            pen.rect((15.0, 39.0, 18.0, 5.0), 1.5, a, pf);
        }
        "lug" => {
            let mut outline = vec![(4.0, 14.0), (24.0, 14.0)];
            outline.extend((1..16).map(|i| {
                let t = -std::f32::consts::FRAC_PI_2 + i as f32 / 16.0 * std::f32::consts::PI;
                (24.0 + 10.0 * t.cos(), 24.0 + 10.0 * t.sin())
            }));
            outline.extend([(24.0, 34.0), (4.0, 34.0)]);
            pen.closed(&outline, d);
            pen.circle((24.0, 24.0), 4.5, a, None);
            pen.line(&[(36.0, 24.0), (44.0, 24.0)], a);
            pen.line(&[(41.0, 21.0), (44.0, 24.0), (41.0, 27.0)], a);
        }
        "eccentric-bushing" => {
            pen.circle((22.0, 24.0), 17.0, d, None);
            pen.circle((27.0, 24.0), 8.0, a, af);
            pen.circle((22.0, 24.0), 1.2, d, None);
        }
        "fea" => {
            pen.rect((6.0, 8.0, 36.0, 32.0), 1.0, d, None);
            for l in [
                [(18.0, 8.0), (18.0, 40.0)],
                [(30.0, 8.0), (30.0, 40.0)],
                [(6.0, 24.0), (42.0, 24.0)],
                [(6.0, 8.0), (18.0, 24.0)],
                [(18.0, 8.0), (30.0, 24.0)],
                [(30.0, 8.0), (42.0, 24.0)],
                [(6.0, 24.0), (18.0, 40.0)],
                [(18.0, 24.0), (30.0, 40.0)],
                [(30.0, 24.0), (42.0, 40.0)],
            ] {
                pen.line(&l, d);
            }
            pen.shade(&[(30.0, 8.0), (30.0, 24.0), (42.0, 24.0)]);
            pen.shade(&[(30.0, 24.0), (42.0, 24.0), (42.0, 40.0)]);
        }
        "members" => {
            pen.line(&[(6.0, 36.0), (42.0, 36.0)], d);
            pen.line(&[(15.0, 20.0), (33.0, 20.0)], d);
            pen.line(&[(6.0, 36.0), (15.0, 20.0), (24.0, 36.0), (33.0, 20.0), (42.0, 36.0)], d);
            pen.closed(&[(6.0, 36.0), (3.0, 42.0), (9.0, 42.0)], d);
            pen.closed(&[(42.0, 36.0), (39.0, 42.0), (45.0, 42.0)], d);
            pen.line(&[(24.0, 5.0), (24.0, 18.0)], a);
            pen.line(&[(21.0, 15.0), (24.0, 18.0), (27.0, 15.0)], a);
            for c in [(6.0, 36.0), (24.0, 36.0), (42.0, 36.0), (15.0, 20.0), (33.0, 20.0)] {
                pen.circle(c, 2.0, a, pf);
            }
        }
        "material-lookup" => {
            pen.line(&[(9.0, 6.0), (9.0, 40.0), (41.0, 40.0)], d);
            let mut curve = cubic((9.0, 40.0), (13.0, 20.0), (17.0, 14.0), (24.0, 14.0), 16);
            curve.extend(cubic((24.0, 14.0), (31.0, 14.0), (35.0, 16.0), (41.0, 11.0), 16));
            pen.line(&curve, a);
            pen.circle((24.0, 14.0), 2.2, a, pf);
        }
        _ => return false,
    }
    true
}

/// The small icon of a dashboard section (`None`: all workbenches) in a 24-unit design space, one colour.
pub(super) fn section_icon(painter: &Painter, rect: Rect, group: Option<crate::gpu_viewer::dashboard::Group>, color: Color32) {
    use crate::gpu_viewer::dashboard::Group;
    let s = rect.width() / 24.0;
    let st = Stroke::new((1.7 * s).max(1.2), color);
    let pt = |x: f32, y: f32| rect.min + egui::vec2(x, y) * s;
    let line = |pts: &[(f32, f32)]| painter.add(Shape::line(pts.iter().map(|&(x, y)| pt(x, y)).collect(), st));
    match group {
        None => {
            for (x, y) in [(4.0, 4.0), (13.5, 4.0), (4.0, 13.5), (13.5, 13.5)] {
                painter.rect_stroke(Rect::from_min_size(pt(x, y), egui::vec2(6.5, 6.5) * s), egui::CornerRadius::same((1.5 * s) as u8), st, egui::StrokeKind::Middle);
            }
        }
        Some(Group::Joints) => {
            painter.circle_stroke(pt(12.0, 12.0), 7.0 * s, st);
            painter.circle_stroke(pt(12.0, 12.0), 2.6 * s, st);
        }
        Some(Group::Structures) => {
            line(&[(3.0, 19.0), (21.0, 19.0)]);
            line(&[(5.0, 19.0), (10.0, 7.0), (14.0, 19.0)]);
            line(&[(10.0, 7.0), (19.0, 7.0)]);
            line(&[(14.0, 19.0), (19.0, 7.0)]);
        }
        Some(Group::Data) => {
            line(&[(6.0, 3.0), (15.0, 3.0), (19.0, 7.0), (19.0, 21.0), (6.0, 21.0), (6.0, 3.0)]);
            line(&[(15.0, 3.0), (15.0, 7.0), (19.0, 7.0)]);
            line(&[(9.0, 12.0), (16.0, 12.0)]);
            line(&[(9.0, 16.0), (16.0, 16.0)]);
        }
    }
}

/// The Toolbench mark: a hexagon with a ring in it, on `color` (24-unit design space).
pub(super) fn brand_mark(painter: &Painter, rect: Rect, color: Color32) {
    let s = rect.width() / 24.0;
    let pt = |x: f32, y: f32| rect.min + egui::vec2(x, y) * s;
    let hex = [(12.0, 2.5), (20.2, 7.25), (20.2, 16.75), (12.0, 21.5), (3.8, 16.75), (3.8, 7.25)];
    painter.add(Shape::closed_line(hex.iter().map(|&(x, y)| pt(x, y)).collect(), Stroke::new((1.8 * s).max(1.2), color)));
    painter.circle_stroke(pt(12.0, 12.0), 3.3 * s, Stroke::new((1.8 * s).max(1.2), color));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_workbench_has_a_glyph_and_an_unknown_id_has_none() {
        let ctx = egui::Context::default();
        let p = super::super::theme::current(&ctx);
        let painter = Painter::new(ctx, egui::LayerId::background(), Rect::from_min_size(Pos2::ZERO, egui::vec2(100.0, 100.0)));
        let rect = Rect::from_min_size(Pos2::ZERO, egui::vec2(40.0, 40.0));
        for w in crate::gpu_viewer::dashboard::WORKBENCHES {
            assert!(paint(&painter, rect, w.id, &p, p.raised), "{} has a glyph", w.id);
        }
        assert!(!paint(&painter, rect, "no-such-workbench", &p, p.raised));
    }
}
