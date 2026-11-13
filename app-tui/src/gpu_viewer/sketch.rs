//! Dimensioned 2D drawings (a section, a view down an axis) as plain data, and the layout that turns one into screen
//! geometry: the drawing fitted to its panel at one scale, then every dimension, callout and note placed where its text
//! clashes with nothing (no other text, no drawn line or filled region, nothing outside the panel).
//!
//! The dimensions use the drafting forms that keep text clear: the text between inside arrows when it fits, the arrows
//! outside with the text beside them when it does not, stacked tiers on one side (a baseline set), and leaders from a
//! feature to text set off where there is room. `gui` only paints the result, so the no-clash rule is tested here,
//! without a window.

/// A point of the drawing, in model units (y up).
pub type P = [f64; 2];
/// A point on the screen, in points (y down).
pub type Q = [f32; 2];

/// How a line is drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Ink {
    /// A visible edge.
    Outline,
    /// Hatching, extension and dimension lines.
    Thin,
    /// A centre line (long dash, short dash).
    Center,
    /// Where something would have to be (dashed, in the warning colour), with its dimension and note.
    Ghost,
    /// A load arrow and its label.
    Load,
}

/// What a closed shape is filled with.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Fill {
    Housing,
    Bushing,
    /// An opening (drawn in the panel's own colour, over what is under it).
    Hole,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Shape {
    pub points: Vec<P>,
    pub closed: bool,
    pub ink: Ink,
    pub fill: Option<Fill>,
    /// Section hatching inside the shape (it must be convex).
    pub hatch: bool,
}
impl Shape {
    /// An outlined rectangle.
    pub fn rect(x0: f64, y0: f64, x1: f64, y1: f64) -> Self {
        Self { points: vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], closed: true, ink: Ink::Outline, fill: None, hatch: false }
    }
    /// An outlined circle (a 96-gon).
    pub fn circle(c: P, r: f64) -> Self {
        let points = (0..96).map(|k| {
            let a = std::f64::consts::TAU * k as f64 / 96.0;
            [c[0] + r * a.cos(), c[1] + r * a.sin()]
        });
        Self { points: points.collect(), closed: true, ink: Ink::Outline, fill: None, hatch: false }
    }
    /// An open line through `points`.
    pub fn line(points: Vec<P>, ink: Ink) -> Self {
        Self { points, closed: false, ink, fill: None, hatch: false }
    }
    pub fn filled(mut self, fill: Fill) -> Self {
        self.fill = Some(fill);
        self
    }
    pub fn hatched(mut self) -> Self {
        self.hatch = true;
        self
    }
    pub fn ink(mut self, ink: Ink) -> Self {
        self.ink = ink;
        self
    }
}

/// A dimension, callout or note of a drawing.
#[derive(Clone, Debug, PartialEq)]
pub enum Dim {
    /// The distance between `a` and `b` along x (`horizontal`) or y, drawn outside the drawing on `side` (+1: above or
    /// right, -1: below or left), `tier` steps out (tiers on one side stack, a baseline set).
    Linear { a: P, b: P, horizontal: bool, side: i8, tier: u8, text: String, ink: Ink },
    /// A leader from `at` (a point on the feature) to the text, set off wherever it clashes with nothing.
    Leader { at: P, text: String, ink: Ink },
    /// Text beside `at` (a leader is drawn only if it has to sit far away).
    Note { at: P, text: String, ink: Ink },
}

/// A load: tail at `from`, head at `to`.
#[derive(Clone, Debug, PartialEq)]
pub struct Arrow {
    pub from: P,
    pub to: P,
}

#[derive(Clone, Debug, PartialEq, Default)]
pub struct Sketch {
    pub title: String,
    pub caption: String,
    pub shapes: Vec<Shape>,
    pub arrows: Vec<Arrow>,
    pub dims: Vec<Dim>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect {
    pub min: Q,
    pub max: Q,
}
impl Rect {
    pub fn new(x: f32, y: f32, w: f32, h: f32) -> Self {
        Self { min: [x, y], max: [x + w, y + h] }
    }
    pub fn width(&self) -> f32 {
        self.max[0] - self.min[0]
    }
    pub fn height(&self) -> f32 {
        self.max[1] - self.min[1]
    }
    pub fn intersects(&self, o: &Rect) -> bool {
        self.min[0] < o.max[0] && o.min[0] < self.max[0] && self.min[1] < o.max[1] && o.min[1] < self.max[1]
    }
    pub fn contains_rect(&self, o: &Rect) -> bool {
        o.min[0] >= self.min[0] && o.min[1] >= self.min[1] && o.max[0] <= self.max[0] && o.max[1] <= self.max[1]
    }
    fn expand(&self, d: f32) -> Rect {
        Rect { min: [self.min[0] - d, self.min[1] - d], max: [self.max[0] + d, self.max[1] + d] }
    }
    fn contains(&self, q: Q) -> bool {
        q[0] >= self.min[0] && q[0] <= self.max[0] && q[1] >= self.min[1] && q[1] <= self.max[1]
    }
    /// The segment `a`-`b` crosses or touches the rectangle (Liang-Barsky).
    pub fn hits_segment(&self, a: Q, b: Q) -> bool {
        if self.contains(a) || self.contains(b) {
            return true;
        }
        let d = [b[0] - a[0], b[1] - a[1]];
        let (mut t0, mut t1) = (0.0f32, 1.0f32);
        for (p, q) in [(-d[0], a[0] - self.min[0]), (d[0], self.max[0] - a[0]), (-d[1], a[1] - self.min[1]), (d[1], self.max[1] - a[1])] {
            if p == 0.0 {
                if q < 0.0 {
                    return false;
                }
            } else {
                let r = q / p;
                if p < 0.0 {
                    t0 = t0.max(r);
                } else {
                    t1 = t1.min(r);
                }
                if t0 > t1 {
                    return false;
                }
            }
        }
        true
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Stroke {
    pub points: Vec<Q>,
    pub ink: Ink,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub points: Vec<Q>,
    pub fill: Fill,
}
/// An arrowhead: its tip and the unit direction it points in.
#[derive(Clone, Debug, PartialEq)]
pub struct Head {
    pub tip: Q,
    pub dir: Q,
    pub ink: Ink,
}
#[derive(Clone, Debug, PartialEq)]
pub struct Label {
    pub rect: Rect,
    pub text: String,
    pub ink: Ink,
}

/// A drawing on the screen: fills first, then strokes, heads and labels.
#[derive(Clone, Debug, PartialEq, Default)]
pub struct Layout {
    pub regions: Vec<Region>,
    pub strokes: Vec<Stroke>,
    pub heads: Vec<Head>,
    pub labels: Vec<Label>,
    /// Screen points per model unit.
    pub scale: f32,
}

/// Arrowhead length.
pub const ARROW: f32 = 7.0;
/// From the drawing to the first tier of dimensions, and from one tier to the next.
const GAP: f32 = 16.0;
const STEP: f32 = 22.0;
/// Room kept around the drawing for callouts and notes.
const PAD: f32 = 26.0;
/// Hatch spacing.
const HATCH: f32 = 6.0;
/// Step of a label sliding along its row to a clear place.
const SLIDE: f32 = 6.0;

/// Where text may not go: lines (drawn strokes, dimension and leader lines), filled regions and placed labels.
#[derive(Default)]
struct Obstacles {
    segments: Vec<(Q, Q)>,
    rects: Vec<Rect>,
}
impl Obstacles {
    fn clashes(&self, r: &Rect) -> usize {
        let r = r.expand(1.5);
        self.segments.iter().filter(|(a, b)| r.hits_segment(*a, *b)).count() + self.rects.iter().filter(|o| o.intersects(&r)).count()
    }
    fn add_polyline(&mut self, pts: &[Q]) {
        self.segments.extend(pts.windows(2).map(|w| (w[0], w[1])));
    }
}

/// The drawing's share of the room it could fill, tried in turn until every label lands clear.
const SHRINK: [f32; 7] = [1.0, 0.86, 0.74, 0.64, 0.55, 0.47, 0.4];

/// Lay `s` out in `area` (screen points); `measure` gives a text's size. The drawing is fitted as large as its
/// dimensions allow, then made smaller step by step while any callout or note still has nowhere clear to go (callouts
/// need room beside the drawing); the first clear layout wins, else the one with the fewest clashes.
pub fn layout(s: &Sketch, area: Rect, measure: &dyn Fn(&str) -> Q) -> Layout {
    let mut best: Option<(usize, Layout)> = None;
    for k in SHRINK {
        let l = layout_at(s, area, measure, k);
        let n = clashes(&l, area).len();
        if n == 0 {
            return l;
        }
        if best.as_ref().is_none_or(|(b, _)| n < *b) {
            best = Some((n, l));
        }
    }
    best.map(|(_, l)| l).unwrap_or_default()
}

fn layout_at(s: &Sketch, area: Rect, measure: &dyn Fn(&str) -> Q, shrink: f32) -> Layout {
    let mut lo = [f64::INFINITY; 2];
    let mut hi = [f64::NEG_INFINITY; 2];
    let mut grow = |p: &P| {
        for k in 0..2 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    };
    s.shapes.iter().flat_map(|sh| &sh.points).for_each(&mut grow);
    s.arrows.iter().flat_map(|a| [&a.from, &a.to]).for_each(&mut grow);
    for d in &s.dims {
        match d {
            Dim::Linear { a, b, .. } => [a, b].into_iter().for_each(&mut grow),
            Dim::Leader { at, .. } | Dim::Note { at, .. } => grow(at),
        }
    }
    let mut out = Layout::default();
    if !(lo[0].is_finite() && hi[0] > lo[0] - 1e-12 && hi[1] >= lo[1]) {
        return out;
    }
    let th = measure("0")[1];
    // Room for the tiers of dimensions on each side: [left, right, top, bottom].
    let mut tiers = [0u8; 4];
    for d in &s.dims {
        if let Dim::Linear { horizontal, side, tier, .. } = d {
            let k = match (horizontal, *side > 0) {
                (true, true) => 2,
                (true, false) => 3,
                (false, true) => 1,
                (false, false) => 0,
            };
            tiers[k] = tiers[k].max(tier + 1);
        }
    }
    let room = |k: usize, along: f32| if tiers[k] == 0 { PAD } else { GAP + f32::from(tiers[k] - 1) * STEP + along + 6.0 };
    // A vertical dimension's text sits beside its line: allow for a typical width.
    let side_text = s.dims.iter().filter_map(|d| match d {
        Dim::Linear { horizontal: false, text, .. } => Some(measure(text)[0] + 6.0),
        _ => None,
    });
    let wide = side_text.fold(0.0f32, f32::max);
    let margin = [room(0, wide).max(PAD), room(1, wide).max(PAD), room(2, th).max(PAD), room(3, th).max(PAD)];
    let (bw, bh) = ((hi[0] - lo[0]).max(1e-9) as f32, (hi[1] - lo[1]).max(1e-9) as f32);
    let (iw, ih) = ((area.width() - margin[0] - margin[1]).max(10.0), (area.height() - margin[2] - margin[3]).max(10.0));
    let scale = (iw / bw).min(ih / bh) * shrink;
    let ox = area.min[0] + margin[0] + (iw - bw * scale) / 2.0;
    let oy = area.min[1] + margin[2] + (ih - bh * scale) / 2.0;
    let q = |p: &P| -> Q { [ox + (p[0] - lo[0]) as f32 * scale, oy + (hi[1] - p[1]) as f32 * scale] };
    out.scale = scale;
    let body = Rect { min: q(&[lo[0], hi[1]]), max: q(&[hi[0], lo[1]]) };

    let mut obstacles = Obstacles::default();
    for sh in &s.shapes {
        let mut pts: Vec<Q> = sh.points.iter().map(q).collect();
        if let Some(fill) = sh.fill {
            out.regions.push(Region { points: pts.clone(), fill });
            if fill != Fill::Hole {
                obstacles.rects.push(bounds(&pts));
            }
            if sh.hatch {
                for seg in hatch(&pts, if fill == Fill::Bushing { -1.0 } else { 1.0 }) {
                    out.strokes.push(Stroke { points: vec![seg.0, seg.1], ink: Ink::Thin });
                }
            }
        }
        if sh.closed {
            pts.push(pts[0]);
        }
        obstacles.add_polyline(&pts);
        out.strokes.push(Stroke { points: pts, ink: sh.ink });
    }
    for a in &s.arrows {
        let (f, t) = (q(&a.from), q(&a.to));
        let dir = unit([t[0] - f[0], t[1] - f[1]]);
        out.strokes.push(Stroke { points: vec![f, [t[0] - dir[0] * ARROW * 0.6, t[1] - dir[1] * ARROW * 0.6]], ink: Ink::Load });
        out.heads.push(Head { tip: t, dir, ink: Ink::Load });
        obstacles.segments.push((f, t));
    }

    let place = |out: &mut Layout, obstacles: &mut Obstacles, text: &str, ink: Ink, candidates: &[Rect]| -> Rect {
        let fits: Vec<&Rect> = candidates.iter().filter(|r| area.contains_rect(r)).collect();
        let best = fits.iter().find(|r| obstacles.clashes(r) == 0).copied().or_else(|| fits.iter().min_by_key(|r| obstacles.clashes(r)).copied()).or(candidates.first()).copied().unwrap_or(Rect::new(area.min[0], area.min[1], 0.0, 0.0));
        obstacles.rects.push(best.expand(1.0));
        out.labels.push(Label { rect: best, text: text.to_string(), ink });
        best
    };

    // Linear dimensions first (their places are fixed by drafting rules), then callouts and notes into what is left. Every
    // dimension's lines go in before any of their text, so a label never lands on a line drawn after it.
    let mut pending: Vec<(&str, Ink, Vec<Rect>)> = Vec::new();
    for d in &s.dims {
        let Dim::Linear { a, b, horizontal, side, tier, text, ink } = d else { continue };
        let (qa, qb) = (q(a), q(b));
        let [tw, th] = measure(text);
        let out_side = *side > 0;
        let dim_ink = if *ink == Ink::Ghost { Ink::Ghost } else { Ink::Thin };
        let offset = GAP + f32::from(*tier) * STEP;
        let mut lines: Vec<Vec<Q>> = Vec::new();
        let mut candidates: Vec<Rect>;
        // Past the ends of the extension lines (they stop 3 pt beyond the dimension line) the text is clear of them however
        // long it is. It slides along that row, nearest first, to the gap between other dimensions' extension lines (a
        // baseline set shares its first one).
        let slides = |at: f32| (1..=40).flat_map(move |k| [at + SLIDE * k as f32, at - SLIDE * k as f32]);
        if *horizontal {
            let y = if out_side { body.min[1] - offset } else { body.max[1] + offset };
            let (x0, x1) = (qa[0].min(qb[0]), qa[0].max(qb[0]));
            for p in [qa, qb] {
                let beyond = if out_side { -3.0 } else { 3.0 };
                let start = if out_side { p[1] - 3.0 } else { p[1] + 3.0 };
                lines.push(vec![[p[0], start], [p[0], y + beyond]]);
            }
            let mid = 0.5 * (x0 + x1);
            let above = Rect::new(mid - tw / 2.0, y - th - 2.0, tw, th);
            let below = Rect::new(mid - tw / 2.0, y + 2.0, tw, th);
            if tw + 2.0 * ARROW + 6.0 <= x1 - x0 {
                lines.push(vec![[x0, y], [x1, y]]);
                out.heads.push(Head { tip: [x0, y], dir: [-1.0, 0.0], ink: dim_ink });
                out.heads.push(Head { tip: [x1, y], dir: [1.0, 0.0], ink: dim_ink });
                candidates = if out_side { vec![above, below] } else { vec![below, above] };
            } else {
                // Too short for its text: the arrows come in from outside and the text sits beside them.
                lines.push(vec![[x0 - ARROW - 8.0, y], [x1 + ARROW + 8.0, y]]);
                out.heads.push(Head { tip: [x0, y], dir: [1.0, 0.0], ink: dim_ink });
                out.heads.push(Head { tip: [x1, y], dir: [-1.0, 0.0], ink: dim_ink });
                let right = Rect::new(x1 + ARROW + 11.0, y - th / 2.0, tw, th);
                let left = Rect::new(x0 - ARROW - 11.0 - tw, y - th / 2.0, tw, th);
                candidates = vec![right, left, above, below];
            }
            let yo = if out_side { y - 6.0 - th } else { y + 6.0 };
            candidates.extend(std::iter::once(mid - tw / 2.0).chain(slides(mid - tw / 2.0)).map(|x| Rect::new(x, yo, tw, th)));
        } else {
            let x = if out_side { body.max[0] + offset } else { body.min[0] - offset };
            let (y0, y1) = (qa[1].min(qb[1]), qa[1].max(qb[1]));
            for p in [qa, qb] {
                let start = if out_side { p[0] + 3.0 } else { p[0] - 3.0 };
                lines.push(vec![[start, p[1]], [x + if out_side { 3.0 } else { -3.0 }, p[1]]]);
            }
            let mid = 0.5 * (y0 + y1);
            let beside_out = if out_side { Rect::new(x + 5.0, mid - th / 2.0, tw, th) } else { Rect::new(x - 5.0 - tw, mid - th / 2.0, tw, th) };
            let beside_in = if out_side { Rect::new(x - 5.0 - tw, mid - th / 2.0, tw, th) } else { Rect::new(x + 5.0, mid - th / 2.0, tw, th) };
            if th + 2.0 * ARROW + 6.0 <= y1 - y0 {
                lines.push(vec![[x, y0], [x, y1]]);
                out.heads.push(Head { tip: [x, y0], dir: [0.0, -1.0], ink: dim_ink });
                out.heads.push(Head { tip: [x, y1], dir: [0.0, 1.0], ink: dim_ink });
                candidates = vec![beside_out, beside_in];
            } else {
                lines.push(vec![[x, y0 - ARROW - 8.0], [x, y1 + ARROW + 8.0]]);
                out.heads.push(Head { tip: [x, y0], dir: [0.0, 1.0], ink: dim_ink });
                out.heads.push(Head { tip: [x, y1], dir: [0.0, -1.0], ink: dim_ink });
                let top = Rect::new(x - tw / 2.0, y0 - ARROW - 10.0 - th, tw, th);
                let bottom = Rect::new(x - tw / 2.0, y1 + ARROW + 10.0, tw, th);
                candidates = vec![beside_out, top, bottom, beside_in];
            }
            let xo = if out_side { x + 5.0 } else { x - 5.0 - tw };
            candidates.extend(slides(mid - th / 2.0).map(|y| Rect::new(xo, y, tw, th)));
        }
        for l in &lines {
            obstacles.add_polyline(l);
        }
        out.strokes.extend(lines.into_iter().map(|points| Stroke { points, ink: dim_ink }));
        pending.push((text, *ink, candidates));
    }
    for (text, ink, candidates) in pending {
        place(&mut out, &mut obstacles, text, ink, &candidates);
    }
    for d in &s.dims {
        let (at, text, ink, note) = match d {
            Dim::Leader { at, text, ink } => (at, text, *ink, false),
            Dim::Note { at, text, ink } => (at, text, *ink, true),
            Dim::Linear { .. } => continue,
        };
        let anchor = q(at);
        let [tw, th] = measure(text);
        // Each candidate: the text's rectangle and the leader that reaches it (empty: the text sits by the anchor).
        let mut candidates: Vec<(Rect, Vec<Q>)> = Vec::new();
        if note {
            for r in [6.0f32, 14.0] {
                candidates.push((Rect::new(anchor[0] - tw / 2.0, anchor[1] + r, tw, th), Vec::new()));
                candidates.push((Rect::new(anchor[0] - tw / 2.0, anchor[1] - r - th, tw, th), Vec::new()));
                candidates.push((Rect::new(anchor[0] + r, anchor[1] - th / 2.0, tw, th), Vec::new()));
                candidates.push((Rect::new(anchor[0] - r - tw, anchor[1] - th / 2.0, tw, th), Vec::new()));
            }
        }
        for r in [26.0f32, 40.0, 58.0, 80.0, 106.0, 136.0, 170.0, 210.0] {
            for dir in [[1.0, -1.0], [-1.0, -1.0], [1.0, 1.0], [-1.0, 1.0], [1.0, 0.0], [-1.0, 0.0], [0.0, -1.0], [0.0, 1.0]] {
                let d = unit(dir);
                let elbow = [anchor[0] + d[0] * r, anchor[1] + d[1] * r];
                // The shoulder turns away from the anchor first, then back toward it (a long text in a narrow panel).
                let away = dir[0] >= 0.0;
                for right in [away, !away] {
                    let shoulder = [elbow[0] + if right { 10.0 } else { -10.0 }, elbow[1]];
                    let rect = if right { Rect::new(shoulder[0] + 3.0, elbow[1] - th / 2.0, tw, th) } else { Rect::new(shoulder[0] - 3.0 - tw, elbow[1] - th / 2.0, tw, th) };
                    candidates.push((rect, vec![anchor, elbow, shoulder]));
                }
            }
        }
        // The leader may cross the drawing (leaders do) but never a label: count the labels and regions it crosses (a region
        // it starts in is the feature it points at). With no clear place, the one with the fewest crossings wins.
        let leader_hits = |l: &[Q], obstacles: &Obstacles| l.windows(2).map(|w| obstacles.rects.iter().filter(|r| r.hits_segment(w[0], w[1]) && !r.contains(w[0])).count()).sum::<usize>();
        let score = |i: usize, obstacles: &Obstacles| obstacles.clashes(&candidates[i].0) + leader_hits(&candidates[i].1, obstacles);
        let inside: Vec<usize> = (0..candidates.len()).filter(|&i| area.contains_rect(&candidates[i].0)).collect();
        let pick = inside.iter().copied().find(|&i| score(i, &obstacles) == 0).or_else(|| inside.iter().copied().min_by_key(|&i| score(i, &obstacles))).unwrap_or(0);
        let (rect, leader) = candidates.swap_remove(pick);
        if !leader.is_empty() {
            let line_ink = if ink == Ink::Ghost { Ink::Ghost } else { Ink::Thin };
            if !note {
                let dir = unit([leader[0][0] - leader[1][0], leader[0][1] - leader[1][1]]);
                out.heads.push(Head { tip: anchor, dir, ink: line_ink });
            }
            obstacles.add_polyline(&leader);
            out.strokes.push(Stroke { points: leader, ink: line_ink });
        }
        place(&mut out, &mut obstacles, text, ink, &[rect]);
    }
    out
}

fn unit(v: Q) -> Q {
    let n = (v[0] * v[0] + v[1] * v[1]).sqrt().max(1e-6);
    [v[0] / n, v[1] / n]
}

fn bounds(pts: &[Q]) -> Rect {
    let mut r = Rect { min: [f32::INFINITY; 2], max: [f32::NEG_INFINITY; 2] };
    for p in pts {
        for k in 0..2 {
            r.min[k] = r.min[k].min(p[k]);
            r.max[k] = r.max[k].max(p[k]);
        }
    }
    r
}

/// 45 degree hatch lines (`slope` +1 or -1) clipped to the convex polygon `pts`.
fn hatch(pts: &[Q], slope: f32) -> Vec<(Q, Q)> {
    let r = bounds(pts);
    if !(r.width() > 1.0 && r.height() > 1.0) {
        return Vec::new();
    }
    let mut out = Vec::new();
    // Lines x - slope * y = c (screen y down), swept across the bounding box.
    let corners = [r.min, [r.max[0], r.min[1]], r.max, [r.min[0], r.max[1]]];
    let cs: Vec<f32> = corners.iter().map(|p| p[0] - slope * p[1]).collect();
    let (c0, c1) = (cs.iter().copied().fold(f32::INFINITY, f32::min), cs.iter().copied().fold(f32::NEG_INFINITY, f32::max));
    let mut c = c0 + HATCH / 2.0;
    while c < c1 {
        // The line through (c, 0) with direction (slope, 1), clipped against every edge (Cyrus-Beck).
        let (p0, d) = ([c, 0.0f32], [slope, 1.0f32]);
        let (mut t0, mut t1) = (f32::NEG_INFINITY, f32::INFINITY);
        let n = pts.len();
        let area: f32 = (0..n).map(|i| pts[i][0] * pts[(i + 1) % n][1] - pts[(i + 1) % n][0] * pts[i][1]).sum();
        let mut empty = false;
        for i in 0..n {
            let (a, b) = (pts[i], pts[(i + 1) % n]);
            let e = [b[0] - a[0], b[1] - a[1]];
            // Inward normal for either winding.
            let nrm = if area > 0.0 { [-e[1], e[0]] } else { [e[1], -e[0]] };
            let den = nrm[0] * d[0] + nrm[1] * d[1];
            let num = nrm[0] * (p0[0] - a[0]) + nrm[1] * (p0[1] - a[1]);
            if den.abs() < 1e-9 {
                if num < 0.0 {
                    empty = true;
                    break;
                }
            } else {
                let t = -num / den;
                if den > 0.0 {
                    t0 = t0.max(t);
                } else {
                    t1 = t1.min(t);
                }
            }
        }
        if !empty && t1 - t0 > 0.5 {
            out.push(([p0[0] + d[0] * t0, p0[1] + d[1] * t0], [p0[0] + d[0] * t1, p0[1] + d[1] * t1]));
        }
        c += HATCH * std::f32::consts::SQRT_2;
    }
    out
}

/// No two labels overlap, none crosses a drawn line or filled region it does not belong to, every one is inside `area`
/// (the rule `layout` keeps; the tests of every drawing call it).
pub fn clashes(l: &Layout, area: Rect) -> Vec<String> {
    let mut bad = Vec::new();
    for (i, a) in l.labels.iter().enumerate() {
        if !area.contains_rect(&a.rect) {
            bad.push(format!("{:?} is outside the panel", a.text));
        }
        for b in &l.labels[i + 1..] {
            if a.rect.intersects(&b.rect) {
                bad.push(format!("{:?} overlaps {:?}", a.text, b.text));
            }
        }
        for s in &l.strokes {
            if s.points.windows(2).any(|w| a.rect.hits_segment(w[0], w[1])) {
                bad.push(format!("{:?} crosses a {:?} line", a.text, s.ink));
            }
        }
        for r in l.regions.iter().filter(|r| r.fill != Fill::Hole) {
            if bounds(&r.points).intersects(&a.rect) {
                bad.push(format!("{:?} lies on a {:?} region", a.text, r.fill));
            }
        }
    }
    bad
}

/// About the window's 11 pt proportional font, for tests of drawings without a window.
#[cfg(test)]
pub(crate) fn test_measure(t: &str) -> Q {
    [6.2 * t.chars().count() as f32, 13.0]
}

#[cfg(test)]
mod tests {
    use super::*;
    use super::test_measure as measure;

    fn plate() -> Sketch {
        Sketch {
            shapes: vec![Shape::rect(0.0, 0.0, 2.0, 1.0).filled(Fill::Housing).hatched(), Shape::circle([0.6, 0.5], 0.2).filled(Fill::Hole)],
            dims: vec![
                Dim::Linear { a: [0.0, 1.0], b: [0.6, 1.0], horizontal: true, side: 1, tier: 0, text: "e 0.600".into(), ink: Ink::Thin },
                Dim::Linear { a: [0.0, 1.0], b: [2.0, 1.0], horizontal: true, side: 1, tier: 1, text: "2.000".into(), ink: Ink::Thin },
                Dim::Linear { a: [2.0, 0.0], b: [2.0, 1.0], horizontal: false, side: 1, tier: 0, text: "1.000".into(), ink: Ink::Thin },
                Dim::Linear { a: [0.55, 0.0], b: [0.65, 0.0], horizontal: true, side: -1, tier: 0, text: "0.100 slot".into(), ink: Ink::Thin },
                Dim::Leader { at: [0.6 + 0.2 * 0.7071, 0.5 + 0.2 * 0.7071], text: "dia 0.400 bore".into(), ink: Ink::Thin },
                Dim::Leader { at: [0.6, 0.3], text: "a second callout on the same feature".into(), ink: Ink::Thin },
                Dim::Note { at: [0.0, 0.5], text: "free edge".into(), ink: Ink::Thin },
            ],
            ..Default::default()
        }
    }

    #[test]
    fn every_label_lands_clear_of_the_others_the_lines_and_the_panel_edge() {
        for (w, h) in [(520.0, 320.0), (360.0, 240.0), (300.0, 420.0)] {
            let area = Rect::new(10.0, 20.0, w, h);
            let l = layout(&plate(), area, &measure);
            assert_eq!(l.labels.len(), 7);
            let bad = clashes(&l, area);
            assert!(bad.is_empty(), "{w} x {h}: {bad:?} {:?}", l.labels);
        }
    }

    #[test]
    fn a_dimension_too_short_for_its_text_puts_the_arrows_outside_and_the_text_beside_them() {
        let area = Rect::new(0.0, 0.0, 520.0, 320.0);
        let l = layout(&plate(), area, &measure);
        let slot = l.labels.iter().find(|x| x.text == "0.100 slot").unwrap();
        // Its arrows point inward (outside arrows) and the text is not between the extension lines.
        let inward = l.heads.iter().filter(|h| h.dir[1] == 0.0 && (h.tip[1] - slot.rect.min[1]).abs() < 30.0).count();
        assert!(inward >= 2);
        let e = layout(&plate(), area, &measure);
        let wide = e.labels.iter().find(|x| x.text == "2.000").unwrap();
        let first = e.labels.iter().find(|x| x.text == "e 0.600").unwrap();
        assert!(wide.rect.max[1] < first.rect.min[1], "the second tier stacks outside the first");
    }

    #[test]
    fn a_label_longer_than_its_dimension_finds_the_gap_in_a_baseline_set_and_clears_lines_drawn_after_it() {
        // Tier 0 is short and its text long; tier 1 shares its first extension line and is drawn after it, and a leader
        // placed later must not cross it either.
        let s = Sketch {
            shapes: vec![Shape::rect(0.0, 0.0, 3.0, 1.5).filled(Fill::Housing), Shape::circle([0.75, 0.75], 0.25).filled(Fill::Hole)],
            dims: vec![
                Dim::Linear { a: [0.0, 1.5], b: [0.75, 1.5], horizontal: true, side: 1, tier: 0, text: "e 0.750 \u{b7} e/D 1.500".into(), ink: Ink::Thin },
                Dim::Linear { a: [0.0, 1.5], b: [3.0, 1.5], horizontal: true, side: 1, tier: 1, text: "3.000".into(), ink: Ink::Thin },
                Dim::Linear { a: [-0.6, 0.0], b: [0.75, 0.0], horizontal: true, side: -1, tier: 0, text: "needs e/D 2.075 (e 1.037) for sequencing".into(), ink: Ink::Ghost },
                Dim::Leader { at: [1.0, 0.75], text: "\u{d8}0.5000 bore".into(), ink: Ink::Thin },
            ],
            ..Default::default()
        };
        for (w, h) in [(420.0, 270.0), (330.0, 240.0)] {
            let area = Rect::new(8.0, 30.0, w, h);
            let l = layout(&s, area, &measure);
            assert_eq!(l.labels.len(), 4);
            let bad = clashes(&l, area);
            assert!(bad.is_empty(), "{w} x {h}: {bad:?}");
        }
    }

    #[test]
    fn hatching_stays_inside_its_shape() {
        let pts = [[0.0, 0.0], [40.0, 0.0], [40.0, 20.0], [0.0, 20.0]];
        let lines = hatch(&pts, 1.0);
        assert!(lines.len() > 5);
        for (a, b) in lines {
            for p in [a, b] {
                assert!(p[0] >= -0.01 && p[0] <= 40.01 && p[1] >= -0.01 && p[1] <= 20.01, "{p:?}");
            }
        }
    }
}
