//! Synthetic, hand-labelled corpus for assembly feature detection (epic exit gate, `docs/epic-assembly-feature-advice.md`).
//!
//! Not real CAD: every part is a `NativeBrep` (boxes, extruded outlines, drilled holes) composed into one STEP assembly through
//! the production product-structure route (`NEXT_ASSEMBLY_USAGE_OCCURRENCE` + `ITEM_DEFINED_TRANSFORMATION`, read by
//! `AnalyticModel::read_all`), so the detector sees placed bodies exactly as it does for a file. The ground truth is written as
//! data next to each case: the holes, fastener sites and contact interfaces a person would label, in the assembly frame.
//!
//! Every case is run under four global placements of the whole assembly: axis-aligned, and three rotations about oblique axes
//! (none maps a hole axis or a plane normal onto a coordinate axis; one makes the hole axes x-dominant, which is where
//! `features::stable_basis` switches its reference direction). Detected features are matched to the labels by position, not by
//! count, and scored per class as true positives, false positives and missed detections. A label marked `limit` is a documented
//! limit of the detector: it is expected to be missed, and the test fails if that stops being true so the document is updated.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cad::{Loop, Profile};
use solid_kernel::features::{detect, Closure, FaceGeometry, Features, Side};
use solid_kernel::native_brep::{Curve, Edge, Face, NativeBrep, Surface, Use};
use solid_kernel::step::{inventory, Limits};
use solid_kernel::V3;
use std::collections::BTreeMap;
use std::f64::consts::PI;

// ---------------------------------------------------------------------------------------------------------------------------
// Rigid placements and the assembly writer
// ---------------------------------------------------------------------------------------------------------------------------

/// `p' = R p + t` (rows of `r`).
#[derive(Clone, Copy, Debug)]
struct Pose {
    r: [[f64; 3]; 3],
    t: V3,
}

fn unit(v: V3) -> V3 {
    let n = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt();
    [v[0] / n, v[1] / n, v[2] / n]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

impl Pose {
    const ID: Pose = Pose { r: [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]], t: [0.; 3] };

    fn apply(&self, p: V3) -> V3 {
        let r = &self.r;
        [
            r[0][0] * p[0] + r[0][1] * p[1] + r[0][2] * p[2] + self.t[0],
            r[1][0] * p[0] + r[1][1] * p[1] + r[1][2] * p[2] + self.t[1],
            r[2][0] * p[0] + r[2][1] * p[1] + r[2][2] * p[2] + self.t[2],
        ]
    }
    fn dir(&self, d: V3) -> V3 {
        let o = self.apply([0.; 3]);
        let p = self.apply(d);
        [p[0] - o[0], p[1] - o[1], p[2] - o[2]]
    }
    /// `self` applied after `inner`.
    fn after(&self, inner: &Pose) -> Pose {
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                r[i][j] = (0..3).map(|k| self.r[i][k] * inner.r[k][j]).sum();
            }
        }
        Pose { r, t: self.apply(inner.t) }
    }
    /// Turn by `radians` about the unit `k` (Rodrigues), the axis passing through `point`.
    fn rotation(k: V3, radians: f64, point: V3) -> Pose {
        let (s, c) = radians.sin_cos();
        let mut r = [[0.0; 3]; 3];
        for i in 0..3 {
            for j in 0..3 {
                r[i][j] = (1. - c) * k[i] * k[j] + if i == j { c } else { 0. };
            }
        }
        r[0][1] -= s * k[2];
        r[0][2] += s * k[1];
        r[1][0] += s * k[2];
        r[1][2] -= s * k[0];
        r[2][0] -= s * k[1];
        r[2][1] += s * k[0];
        let mut p = Pose { r, t: [0.; 3] };
        let moved = p.apply(point);
        p.t = [point[0] - moved[0], point[1] - moved[1], point[2] - moved[2]];
        p
    }
    fn about_line(axis: V3, degrees: f64, point: V3) -> Pose {
        Pose::rotation(unit(axis), degrees.to_radians(), point)
    }
    /// The rotation that carries the unit vector `from` onto `to` about the axis perpendicular to both.
    fn taking(from: V3, to: V3) -> Pose {
        let c = cross(from, to);
        let s = (c[0] * c[0] + c[1] * c[1] + c[2] * c[2]).sqrt();
        let d = from[0] * to[0] + from[1] * to[1] + from[2] * to[2];
        Pose::rotation(unit(c), s.atan2(d), [0.; 3])
    }
    fn shifted(mut self, v: V3) -> Pose {
        for k in 0..3 {
            self.t[k] += v[k];
        }
        self
    }
    fn column(&self, j: usize) -> V3 {
        [self.r[0][j], self.r[1][j], self.r[2][j]]
    }
}

/// A STEP real: shortest round-trip decimal with a decimal point.
fn real(x: f64) -> String {
    let s = format!("{x}");
    if s.contains('.') {
        s
    } else {
        format!("{s}.")
    }
}
fn triple(v: V3) -> String {
    format!("({},{},{})", real(v[0]), real(v[1]), real(v[2]))
}

/// Shift every `#id` of a STEP data line by `off`.
fn shift_ids(line: &str, off: u64) -> String {
    let mut out = String::new();
    let mut rest = line;
    while let Some(i) = rest.find('#') {
        out.push_str(&rest[..=i]);
        let tail = &rest[i + 1..];
        let digits = tail.chars().take_while(|c| c.is_ascii_digit()).count();
        let n: u64 = tail[..digits].parse().unwrap();
        out.push_str(&(n + off).to_string());
        rest = &tail[digits..];
    }
    out.push_str(rest);
    out
}

/// One STEP file holding every part as a placed instance of its own product, in the order given: the part's own coordinates are
/// carried into the assembly frame by its pose, as `ITEM_DEFINED_TRANSFORMATION` does it (source frame: the part's origin
/// placement; target frame: the pose's axes and origin).
fn compose(parts: &[(NativeBrep, Pose)]) -> String {
    const ROOT: u64 = 9_000_000;
    let mut data = Vec::new();
    let mut root = Vec::new();
    let mut targets = Vec::new();
    let mut ctx0 = 0;
    for (k, (brep, pose)) in parts.iter().enumerate() {
        let off = 100_000 * k as u64;
        let text = brep.to_step_text();
        let body = text.split("DATA;\n").nth(1).unwrap().split("ENDSEC;").next().unwrap();
        let mut origin = 0;
        let mut ctx = 0;
        for line in body.lines().filter(|l| !l.is_empty()) {
            if line.starts_with("#10 = ") {
                // `#10 = ADVANCED_BREP_SHAPE_REPRESENTATION('',(#origin,#solid),#context);`
                origin = line.split("(#").nth(1).unwrap().split(',').next().unwrap().parse().unwrap();
                ctx = line.rsplit('#').next().unwrap().trim_end_matches(");").parse().unwrap();
            }
            data.push(shift_ids(line, off));
        }
        if k == 0 {
            ctx0 = ctx;
        }
        let b = ROOT + 100 * (k as u64 + 1);
        let (part_def, part_rep, source) = (5 + off, 10 + off, origin + off);
        root.push(format!("#{b} = NEXT_ASSEMBLY_USAGE_OCCURRENCE('{k}','','',#{},#{part_def},$);", ROOT + 3));
        root.push(format!("#{} = PRODUCT_DEFINITION_SHAPE('Placement','Placement of an item',#{b});", b + 1));
        root.push(format!("#{} = ITEM_DEFINED_TRANSFORMATION('','',#{source},#{});", b + 2, b + 3));
        root.push(format!("#{} = AXIS2_PLACEMENT_3D('',#{},#{},#{});", b + 3, b + 4, b + 5, b + 6));
        root.push(format!("#{} = CARTESIAN_POINT('',{});", b + 4, triple(pose.t)));
        root.push(format!("#{} = DIRECTION('',{});", b + 5, triple(pose.column(2))));
        root.push(format!("#{} = DIRECTION('',{});", b + 6, triple(pose.column(0))));
        root.push(format!(
            "#{} = ( REPRESENTATION_RELATIONSHIP('','',#{part_rep},#{}) REPRESENTATION_RELATIONSHIP_WITH_TRANSFORMATION(#{}) SHAPE_REPRESENTATION_RELATIONSHIP() );",
            b + 7,
            ROOT + 5,
            b + 2
        ));
        root.push(format!("#{} = CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#{},#{});", b + 8, b + 7, b + 1));
        targets.push(format!("#{}", b + 3));
    }
    let head = [
        format!("#{} = PRODUCT('ASSY','ASSY','',(#8));", ROOT + 1),
        format!("#{} = PRODUCT_DEFINITION_FORMATION('','',#{});", ROOT + 2, ROOT + 1),
        format!("#{} = PRODUCT_DEFINITION('design','',#{},#9);", ROOT + 3, ROOT + 2),
        format!("#{} = PRODUCT_DEFINITION_SHAPE('','',#{});", ROOT + 4, ROOT + 3),
        format!("#{} = SHAPE_REPRESENTATION('',(#{},{}),#{ctx0});", ROOT + 5, ROOT + 6, targets.join(",")),
        format!("#{} = AXIS2_PLACEMENT_3D('',#{},#{},#{});", ROOT + 6, ROOT + 7, ROOT + 8, ROOT + 9),
        format!("#{} = CARTESIAN_POINT('',(0.,0.,0.));", ROOT + 7),
        format!("#{} = DIRECTION('',(0.,0.,1.));", ROOT + 8),
        format!("#{} = DIRECTION('',(1.,0.,0.));", ROOT + 9),
        format!("#{} = SHAPE_DEFINITION_REPRESENTATION(#{},#{});", ROOT + 10, ROOT + 4, ROOT + 5),
    ];
    let mut out = String::from(
        "ISO-10303-21;\nHEADER;\nFILE_DESCRIPTION(('synthetic corpus'),'2;1');\nFILE_NAME('corpus','1979-01-01T00:00:00',('Author'),('solid-kernel'),'solid-kernel','solid-kernel','Unknown');\nFILE_SCHEMA(('AUTOMOTIVE_DESIGN { 1 0 10303 214 1 1 1 1 }'));\nENDSEC;\nDATA;\n",
    );
    for l in data.iter().chain(head.iter()).chain(root.iter()) {
        out.push_str(l);
        out.push('\n');
    }
    out.push_str("ENDSEC;\nEND-ISO-10303-21;\n");
    out
}

// ---------------------------------------------------------------------------------------------------------------------------
// Parts
// ---------------------------------------------------------------------------------------------------------------------------

/// A plate `[lo, hi]` with round holes drilled down through its top face (through, or `depth` deep).
fn plate(lo: V3, hi: V3, holes: &[([f64; 2], f64)]) -> NativeBrep {
    let mut b = NativeBrep::box_from_corners(lo, hi).unwrap();
    for (c, r) in holes {
        let top = b.planar_faces_through([lo[0], lo[1], hi[2]], [0., 0., 1.])[0];
        b = b.drill(top, [c[0], c[1], hi[2]], *r, None).unwrap();
    }
    b
}

/// A 40 x 30 plate between `z0` and `z1` with through holes.
fn slab(z0: f64, z1: f64, holes: &[([f64; 2], f64)]) -> NativeBrep {
    plate([0., 0., z0], [40., 30., z1], holes)
}

#[derive(Clone, Copy)]
enum Seg {
    Line,
    /// An arc about `c`, run counter-clockwise or not, ending at the next point.
    Arc { c: [f64; 2], ccw: bool },
}

/// A closed outline: points and the segment that runs from each to the next.
type Outline = (Vec<[f64; 2]>, Vec<Seg>);

fn reversed((pts, segs): &Outline) -> Outline {
    let n = pts.len();
    let q: Vec<[f64; 2]> = pts.iter().rev().copied().collect();
    let s = (0..n)
        .map(|k| match segs[(2 * n - 2 - k) % n] {
            Seg::Line => Seg::Line,
            Seg::Arc { c, ccw } => Seg::Arc { c, ccw: !ccw },
        })
        .collect();
    (q, s)
}

fn rect_outline(x0: f64, y0: f64, x1: f64, y1: f64) -> Outline {
    (vec![[x0, y0], [x1, y0], [x1, y1], [x0, y1]], vec![Seg::Line; 4])
}

/// Counter-clockwise obround slot: end-arc centres `(x1, y)` and `(x2, y)`, radius `rho`.
fn slot_outline(x1: f64, x2: f64, y: f64, rho: f64) -> Outline {
    (
        vec![[x1, y - rho], [x2, y - rho], [x2, y + rho], [x1, y + rho]],
        vec![Seg::Line, Seg::Arc { c: [x2, y], ccw: true }, Seg::Line, Seg::Arc { c: [x1, y], ccw: true }],
    )
}

/// Counter-clockwise rectangle with corner radius `rho`.
fn rounded_outline(x0: f64, y0: f64, x1: f64, y1: f64, rho: f64) -> Outline {
    (
        vec![[x0 + rho, y0], [x1 - rho, y0], [x1, y0 + rho], [x1, y1 - rho], [x1 - rho, y1], [x0 + rho, y1], [x0, y1 - rho], [x0, y0 + rho]],
        vec![
            Seg::Line,
            Seg::Arc { c: [x1 - rho, y0 + rho], ccw: true },
            Seg::Line,
            Seg::Arc { c: [x1 - rho, y1 - rho], ccw: true },
            Seg::Line,
            Seg::Arc { c: [x0 + rho, y1 - rho], ccw: true },
            Seg::Line,
            Seg::Arc { c: [x0 + rho, y0 + rho], ccw: true },
        ],
    )
}

/// Extrude an outer outline (counter-clockwise) with through openings (given counter-clockwise, reversed here) between `zb` and
/// `zt`: planes for line segments, cylinders for arcs. `NativeBrep::extrude` only does polygons and full circles.
fn extrude_outlines(outer: &Outline, openings: &[Outline], zb: f64, zt: f64) -> NativeBrep {
    let mut b = NativeBrep::default();
    let loops: Vec<Outline> = std::iter::once(outer.clone()).chain(openings.iter().map(reversed)).collect();
    let (mut bottom, mut top): (Vec<Vec<Use>>, Vec<Vec<Use>>) = (vec![], vec![]);
    let unit2 = |a: [f64; 2], c: [f64; 2]| {
        let l = ((c[0] - a[0]).powi(2) + (c[1] - a[1]).powi(2)).sqrt();
        [(c[0] - a[0]) / l, (c[1] - a[1]) / l]
    };
    for (pts, segs) in &loops {
        let n = pts.len();
        let vb: Vec<usize> = pts
            .iter()
            .map(|p| {
                b.vertices.push([p[0], p[1], zb]);
                b.vertices.len() - 1
            })
            .collect();
        let vt: Vec<usize> = pts
            .iter()
            .map(|p| {
                b.vertices.push([p[0], p[1], zt]);
                b.vertices.len() - 1
            })
            .collect();
        let edge = |b: &mut NativeBrep, curve: Curve, s: usize, e: usize| {
            b.edges.push(Edge { curve, start: s, end: e });
            b.edges.len() - 1
        };
        let ev: Vec<usize> = (0..n).map(|i| edge(&mut b, Curve::Line { origin: [pts[i][0], pts[i][1], zb], direction: [0., 0., 1.] }, vb[i], vt[i])).collect();
        let (mut lb, mut lt) = (vec![], vec![]);
        for i in 0..n {
            let j = (i + 1) % n;
            let (eb, et, fwd, surface, same_sense);
            match segs[i] {
                Seg::Line => {
                    let d = unit2(pts[i], pts[j]);
                    let line = |z: f64| Curve::Line { origin: [pts[i][0], pts[i][1], z], direction: [d[0], d[1], 0.] };
                    eb = edge(&mut b, line(zb), vb[i], vb[j]);
                    et = edge(&mut b, line(zt), vt[i], vt[j]);
                    fwd = true;
                    surface = Surface::Plane { origin: [pts[i][0], pts[i][1], zb], normal: [d[1], -d[0], 0.], xdir: [d[0], d[1], 0.] };
                    same_sense = true;
                }
                Seg::Arc { c, ccw } => {
                    let radius = ((pts[i][0] - c[0]).powi(2) + (pts[i][1] - c[1]).powi(2)).sqrt();
                    let circle = |z: f64| Curve::Circle { centre: [c[0], c[1], z], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius };
                    // a circle edge runs counter-clockwise: a clockwise segment is the edge from its end to its start, used reversed
                    let pick = |v: (usize, usize)| if ccw { v } else { (v.1, v.0) };
                    let (pb, pt) = (pick((vb[i], vb[j])), pick((vt[i], vt[j])));
                    eb = edge(&mut b, circle(zb), pb.0, pb.1);
                    et = edge(&mut b, circle(zt), pt.0, pt.1);
                    fwd = ccw;
                    surface = Surface::Cylinder { origin: [c[0], c[1], zb], axis: [0., 0., 1.], xdir: [1., 0., 0.], radius };
                    // the outward normal of the material points away from the centre for a counter-clockwise run
                    same_sense = ccw;
                }
            }
            let wall = vec![Use { edge: eb, forward: fwd }, Use { edge: ev[j], forward: true }, Use { edge: et, forward: !fwd }, Use { edge: ev[i], forward: false }];
            b.faces.push(Face { surface, same_sense, outer: wall, holes: vec![] });
            lb.push(Use { edge: eb, forward: fwd });
            lt.push(Use { edge: et, forward: fwd });
        }
        bottom.push(lb);
        top.push(lt);
    }
    // the bottom cap (normal -z) runs every loop the other way round; the top cap (normal +z) as the loops are
    let flip = |l: &Vec<Use>| l.iter().rev().map(|u| Use { edge: u.edge, forward: !u.forward }).collect::<Vec<_>>();
    let cap = |loops: &Vec<Vec<Use>>, flipped: bool| {
        let run: Vec<Vec<Use>> = loops.iter().map(|l| if flipped { flip(l) } else { l.clone() }).collect();
        (run[0].clone(), run[1..].to_vec())
    };
    let (bo, bh) = cap(&bottom, true);
    let (to, th) = cap(&top, false);
    b.faces.push(Face { surface: Surface::Plane { origin: [0., 0., zb], normal: [0., 0., -1.], xdir: [1., 0., 0.] }, same_sense: true, outer: bo, holes: bh });
    b.faces.push(Face { surface: Surface::Plane { origin: [0., 0., zt], normal: [0., 0., 1.], xdir: [1., 0., 0.] }, same_sense: true, outer: to, holes: th });
    b
}

// ---------------------------------------------------------------------------------------------------------------------------
// Ground truth
// ---------------------------------------------------------------------------------------------------------------------------

/// A bore section a person would label: a point on its axis (assembly frame), radius and closure.
struct ExpHole {
    body: usize,
    at: V3,
    r: f64,
    closure: Closure,
}

/// A place a fastener passes through, with the size the bores say.
struct ExpSite {
    at: V3,
    bodies: Vec<usize>,
    fastener: Option<&'static str>,
    mismatch: bool,
    /// Set when the detector is documented not to find it.
    limit: Option<&'static str>,
}

/// A contact between two bodies. `curved` is the total curved boundary length on the shared faces: the measured area reads
/// inside a curved boundary by at most that times the chord tolerance.
struct ExpIface {
    bodies: [usize; 2],
    area: f64,
    curved: f64,
    gap: f64,
}

struct Case {
    class: &'static str,
    name: &'static str,
    parts: Vec<(NativeBrep, Pose)>,
    holes: Vec<ExpHole>,
    sites: Vec<ExpSite>,
    ifaces: Vec<ExpIface>,
    /// Faces that must exist for the case to mean what it says: concave (hole-wall side) cylinder faces in the whole assembly.
    concave_cylinders: Option<usize>,
}

fn hole(body: usize, at: V3, r: f64, closure: Closure) -> ExpHole {
    ExpHole { body, at, r, closure }
}
fn site(at: V3, bodies: &[usize], fastener: Option<&'static str>, mismatch: bool) -> ExpSite {
    ExpSite { at, bodies: bodies.to_vec(), fastener, mismatch, limit: None }
}
fn iface(a: usize, b: usize, area: f64, curved: f64) -> ExpIface {
    ExpIface { bodies: [a, b], area, curved, gap: 0. }
}
fn disk(r: f64) -> f64 {
    PI * r * r
}
fn ring(r: f64) -> f64 {
    2. * PI * r
}
const PLATE: f64 = 1200.;
const R5: f64 = 2.5; // a 5.0 mm bore
/// The tolerance `Options::default()` gives every model.
const TOL: f64 = 1e-7;

fn pts(n: usize) -> Vec<[f64; 2]> {
    match n {
        1 => vec![[20., 15.]],
        2 => vec![[10., 15.], [30., 15.]],
        _ => vec![[10., 8.], [30., 8.], [10., 22.], [30., 22.]],
    }
}

/// `plates` 40 x 30 x 10 plates stacked in z, each with the same through holes of radius `r`.
fn stack_case(class: &'static str, name: &'static str, plates: usize, holes: &[[f64; 2]], r: f64) -> Case {
    let hs: Vec<([f64; 2], f64)> = holes.iter().map(|h| (*h, r)).collect();
    let n = holes.len() as f64;
    let bodies: Vec<usize> = (0..plates).collect();
    Case {
        class,
        name,
        parts: (0..plates).map(|k| (slab(10. * k as f64, 10. * (k + 1) as f64, &hs), Pose::ID)).collect(),
        holes: (0..plates).flat_map(|k| holes.iter().map(move |h| hole(k, [h[0], h[1], 10. * k as f64 + 5.], r, Closure::Through))).collect(),
        sites: holes.iter().map(|h| site([h[0], h[1], 0.], &bodies, Some("M5"), false)).collect(),
        ifaces: (0..plates - 1).map(|k| iface(k, k + 1, PLATE - n * disk(r), n * ring(r))).collect(),
        concave_cylinders: None,
    }
}

/// An L section (extruded along z, foot along x on y 0..10, leg up to y 30) bolted down onto a plate below y = 0.
fn l_case(name: &'static str, holes: &[[f64; 2]]) -> Case {
    let l = Profile { outer: Loop::Polygon(vec![[0., 0.], [40., 0.], [40., 10.], [10., 10.], [10., 30.], [0., 30.]]), holes: vec![] };
    let mut angle = NativeBrep::extrude(&l, 0., 20.).unwrap();
    let mut base = NativeBrep::box_from_corners([-5., -10., -5.], [45., 0., 25.]).unwrap();
    for h in holes {
        let foot = angle.planar_faces_through([25., 10., 10.], [0., 1., 0.])[0];
        angle = angle.drill(foot, [h[0], 10., h[1]], R5, None).unwrap();
        let top = base.planar_faces_through([0., 0., 0.], [0., 1., 0.])[0];
        base = base.drill(top, [h[0], 0., h[1]], R5, None).unwrap();
    }
    let n = holes.len() as f64;
    Case {
        class: "L-section on plate",
        name,
        parts: vec![(base, Pose::ID), (angle, Pose::ID)],
        holes: holes.iter().flat_map(|h| [hole(0, [h[0], -5., h[1]], R5, Closure::Through), hole(1, [h[0], 5., h[1]], R5, Closure::Through)]).collect(),
        sites: holes.iter().map(|h| site([h[0], 0., h[1]], &[0, 1], Some("M5"), false)).collect(),
        ifaces: vec![iface(0, 1, 800. - n * disk(R5), n * ring(R5))],
        concave_cylinders: None,
    }
}

/// Area of the part of a 40 x 30 face outside two circles of radius `r` whose centres are `d` apart.
fn outside_two_disks(r: f64, d: f64) -> f64 {
    let lens = if d >= 2. * r { 0. } else { 2. * r * r * (d / (2. * r)).acos() - 0.5 * d * (4. * r * r - d * d).sqrt() };
    PLATE - (2. * disk(r) - lens)
}

/// Two plates with one through hole each, the second's centre moved by `dx`; the site is labelled `truth`.
fn offset_case(name: &'static str, dx: f64, truth: bool, limit: Option<&'static str>) -> Case {
    let a = slab(0., 10., &[([20., 15.], R5)]);
    let b = slab(10., 20., &[([20. + dx, 15.], R5)]);
    let mut s = site([20., 15., 0.], &[0, 1], Some("M5"), false);
    s.limit = limit;
    Case {
        class: "offset vs tolerance",
        name,
        parts: vec![(a, Pose::ID), (b, Pose::ID)],
        holes: vec![hole(0, [20., 15., 5.], R5, Closure::Through), hole(1, [20. + dx, 15., 15.], R5, Closure::Through)],
        sites: if truth { vec![s] } else { vec![] },
        ifaces: vec![iface(0, 1, outside_two_disks(R5, dx.abs()), 2. * ring(R5))],
        concave_cylinders: None,
    }
}

/// Two plates `gap` apart in z (negative: overlapping), each with one coaxial through hole when `holed`.
fn gap_case(name: &'static str, gap: f64, holed: bool, touching: bool, site_truth: bool, limit: Option<&'static str>) -> Case {
    let hs: &[([f64; 2], f64)] = if holed { &[([20., 15.], R5)] } else { &[] };
    let n = holed as usize as f64;
    let mut s = site([20., 15., 0.], &[0, 1], Some("M5"), false);
    s.limit = limit;
    let mut c = Case {
        class: "touching vs gapped",
        name,
        parts: vec![(slab(0., 10., hs), Pose::ID), (slab(10. + gap, 20. + gap, hs), Pose::ID)],
        holes: if holed { vec![hole(0, [20., 15., 5.], R5, Closure::Through), hole(1, [20., 15., 15. + gap], R5, Closure::Through)] } else { vec![] },
        sites: if site_truth { vec![s] } else { vec![] },
        ifaces: if touching { vec![iface(0, 1, PLATE - n * disk(R5), n * ring(R5))] } else { vec![] },
        concave_cylinders: None,
    };
    if touching {
        c.ifaces[0].gap = gap;
    }
    c
}

/// A 1 mm shim between two plates, with a coaxial hole or solid.
fn shim_case(name: &'static str, shim_hole: bool) -> Case {
    let hole_a: &[([f64; 2], f64)] = &[([20., 15.], R5)];
    let shim: &[([f64; 2], f64)] = if shim_hole { hole_a } else { &[] };
    let mut holes = vec![hole(0, [20., 15., 5.], R5, Closure::Through), hole(2, [20., 15., 16.], R5, Closure::Through)];
    if shim_hole {
        holes.insert(1, hole(1, [20., 15., 10.5], R5, Closure::Through));
    }
    Case {
        class: "shim and air gap",
        name,
        parts: vec![(slab(0., 10., hole_a), Pose::ID), (slab(10., 11., shim), Pose::ID), (slab(11., 21., hole_a), Pose::ID)],
        holes,
        sites: if shim_hole { vec![site([20., 15., 0.], &[0, 1, 2], Some("M5"), false)] } else { vec![] },
        ifaces: vec![iface(0, 1, PLATE - disk(R5), ring(R5)), iface(1, 2, PLATE - disk(R5), ring(R5))],
        concave_cylinders: None,
    }
}

/// Two plates (`ra` in the lower, `rb` in the upper) and what a person reads from the pair.
fn fit_case(name: &'static str, radii: &[f64], fastener: Option<&'static str>, mismatch: bool) -> Case {
    let n = radii.len();
    let bodies: Vec<usize> = (0..n).collect();
    Case {
        class: "tapped vs clearance",
        name,
        parts: (0..n).map(|k| (slab(10. * k as f64, 10. * (k + 1) as f64, &[([20., 15.], radii[k])]), Pose::ID)).collect(),
        holes: (0..n).map(|k| hole(k, [20., 15., 10. * k as f64 + 5.], radii[k], Closure::Through)).collect(),
        sites: vec![site([20., 15., 0.], &bodies, fastener, mismatch)],
        ifaces: (0..n - 1).map(|k| iface(k, k + 1, PLATE - disk(radii[k].max(radii[k + 1])), ring(radii[k]) + ring(radii[k + 1]))).collect(),
        concave_cylinders: None,
    }
}

/// A round `r` hole in the top of a block `[0, 40] x [0, 30] x [z0, z1]`, blind from the top face (`depth`) or from the bottom.
fn blind_block(z0: f64, z1: f64, r: f64, depth: f64, from_bottom: bool) -> NativeBrep {
    let b = NativeBrep::box_from_corners([0., 0., z0], [40., 30., z1]).unwrap();
    if from_bottom {
        let f = b.planar_faces_through([0., 0., z0], [0., 0., -1.])[0];
        b.drill(f, [20., 15., z0], r, Some(depth)).unwrap()
    } else {
        let f = b.planar_faces_through([0., 0., z1], [0., 0., 1.])[0];
        b.drill(f, [20., 15., z1], r, Some(depth)).unwrap()
    }
}

fn counterbore_block(z0: f64, z1: f64) -> NativeBrep {
    let c = NativeBrep::box_from_corners([0., 0., z0], [40., 30., z1]).unwrap();
    let top = c.planar_faces_through([0., 0., z1], [0., 0., 1.])[0];
    let c = c.drill(top, [20., 15., z1], 3., Some(3.)).unwrap();
    let floor = c.planar_faces_through([20., 15., z1 - 3.], [0., 0., 1.])[0];
    c.drill(floor, [20., 15., z1 - 3.], 1.5, None).unwrap()
}

fn slot_area(x1: f64, x2: f64, rho: f64) -> f64 {
    2. * rho * (x2 - x1) + disk(rho)
}

fn cases() -> Vec<Case> {
    let mut v = Vec::new();
    // stacks of 2 and 3 plates with 1, 2 and 4 coaxial holes
    for (n, name2, name3) in [(1, "2 plates, 1 hole", "3 plates, 1 hole"), (2, "2 plates, 2 holes", "3 plates, 2 holes"), (4, "2 plates, 4 holes", "3 plates, 4 holes")] {
        v.push(stack_case("stack of plates", name2, 2, &pts(n), R5));
        v.push(stack_case("stack of plates", name3, 3, &pts(n), R5));
    }
    v.push(l_case("L on plate, 1 hole", &[[25., 10.]]));
    v.push(l_case("L on plate, 2 holes", &[[15., 5.], [30., 15.]]));

    // offsets against the tolerance
    v.push(offset_case("hole offset 0.5 tol", 0.5 * TOL, true, None));
    v.push(offset_case("hole offset 2 tol", 2. * TOL, false, None));
    v.push(offset_case("hole offset 0.2 mm", 0.2, true, Some("a coaxial pair is within the model's geometry tolerance; 0.2 mm is a real misalignment but not within it")));

    // touching, gapped, interpenetrating
    v.push(gap_case("touching, holed", 0., true, true, true, None));
    v.push(gap_case("gap 0.5 tol, holed", 0.5 * TOL, true, true, true, None));
    v.push(gap_case("overlap 0.5 tol, holed", -0.5 * TOL, true, true, true, None));
    v.push(gap_case("gap 2 tol, holed", 2. * TOL, true, false, false, None));
    v.push(gap_case("air gap 0.5 mm, blank", 0.5, false, false, false, None));
    v.push(gap_case(
        "air gap 0.5 mm, holed",
        0.5,
        true,
        false,
        true,
        Some("a gap larger than the tolerance (a washer, a spacer not modelled) breaks the axial range overlap that makes a site"),
    ));
    {
        let side = vec![
            (NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap(), Pose::ID),
            (NativeBrep::box_from_corners([40., 0., 0.], [80., 30., 10.]).unwrap(), Pose::ID),
        ];
        v.push(Case { class: "touching vs gapped", name: "side by side, face contact", parts: side, holes: vec![], sites: vec![], ifaces: vec![iface(0, 1, 300., 0.)], concave_cylinders: None });
        let edge = vec![
            (NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap(), Pose::ID),
            (NativeBrep::box_from_corners([40., 30., 0.], [80., 60., 10.]).unwrap(), Pose::ID),
        ];
        v.push(Case { class: "touching vs gapped", name: "edge contact only", parts: edge, holes: vec![], sites: vec![], ifaces: vec![], concave_cylinders: None });
        // faces that overlap in a strip half a tolerance wide are not a contact either
        let strip = vec![
            (NativeBrep::box_from_corners([0., 0., 0.], [40., 30., 10.]).unwrap(), Pose::ID),
            (NativeBrep::box_from_corners([40., 30. - 0.5 * TOL, 0.], [80., 60., 10.]).unwrap(), Pose::ID),
        ];
        v.push(Case { class: "touching vs gapped", name: "edge contact, strip 0.5 tol wide", parts: strip, holes: vec![], sites: vec![], ifaces: vec![], concave_cylinders: None });
    }
    v.push(stack_case("touching vs gapped", "stacked blank plates", 2, &[], R5));

    // shim
    v.push(shim_case("1 mm shim with hole", true));
    v.push(shim_case("1 mm solid shim", false));

    // counterbore and blind holes
    {
        let c = counterbore_block(10., 20.);
        v.push(Case {
            class: "counterbore and blind",
            name: "counterbore over through hole",
            parts: vec![(slab(0., 10., &[([20., 15.], 1.5)]), Pose::ID), (c, Pose::ID)],
            holes: vec![
                hole(0, [20., 15., 5.], 1.5, Closure::Through),
                hole(1, [20., 15., 13.5], 1.5, Closure::Through),
                hole(1, [20., 15., 18.5], 3., Closure::Through),
            ],
            sites: vec![site([20., 15., 0.], &[0, 1], Some("M3"), false)],
            ifaces: vec![iface(0, 1, PLATE - disk(1.5), ring(1.5) * 2.)],
            concave_cylinders: None,
        });
        v.push(Case {
            class: "counterbore and blind",
            name: "blind hole from the top, not reaching the joint",
            parts: vec![(slab(0., 10., &[([20., 15.], R5)]), Pose::ID), (blind_block(10., 20., R5, 4., false), Pose::ID)],
            holes: vec![hole(0, [20., 15., 5.], R5, Closure::Through), hole(1, [20., 15., 18.], R5, Closure::Blind)],
            sites: vec![],
            ifaces: vec![iface(0, 1, PLATE - disk(R5), ring(R5))],
            concave_cylinders: None,
        });
        v.push(Case {
            class: "counterbore and blind",
            name: "tapped blind hole from the joint face",
            parts: vec![(slab(0., 10., &[([20., 15.], 3.3)]), Pose::ID), (blind_block(10., 20., R5, 6., true), Pose::ID)],
            holes: vec![hole(0, [20., 15., 5.], 3.3, Closure::Through), hole(1, [20., 15., 13.], R5, Closure::Blind)],
            sites: vec![site([20., 15., 0.], &[0, 1], Some("M6"), true)],
            ifaces: vec![iface(0, 1, PLATE - disk(3.3), ring(3.3) + ring(R5))],
            concave_cylinders: None,
        });
    }

    // partial overlap: only the hole pair inside the common region is a site
    {
        let a = plate([0., 0., 0.], [40., 30., 10.], &[([10., 15.], R5), ([32., 15.], R5)]);
        let b = plate([25., 5., 10.], [65., 35., 20.], &[([32., 15.], R5), ([55., 20.], R5)]);
        v.push(Case {
            class: "partial overlap",
            name: "shifted plates, one hole pair in the overlap",
            parts: vec![(a, Pose::ID), (b, Pose::ID)],
            holes: vec![
                hole(0, [10., 15., 5.], R5, Closure::Through),
                hole(0, [32., 15., 5.], R5, Closure::Through),
                hole(1, [32., 15., 15.], R5, Closure::Through),
                hole(1, [55., 20., 15.], R5, Closure::Through),
            ],
            sites: vec![site([32., 15., 0.], &[0, 1], Some("M5"), false)],
            ifaces: vec![iface(0, 1, 15. * 25. - disk(R5), 2. * ring(R5))],
            concave_cylinders: None,
        });
    }

    // tapped (5.0) against clearance (6.6) and the other sizes
    v.push(fit_case("5.0 over 6.6 (M6 tap over clearance)", &[R5, 3.3], Some("M6"), true));
    v.push(fit_case("6.6 over 5.0 (clearance over M6 tap)", &[3.3, R5], Some("M6"), true));
    v.push(fit_case("6.6 over 6.6 (M6 clearance pair)", &[3.3, 3.3], Some("M6"), false));
    v.push(fit_case("5.0 over 7.8 (no common size)", &[R5, 3.9], None, true));
    v.push(fit_case("0.201 in over 0.250 in (1/4-20)", &[0.201 * 25.4 / 2., 0.250 * 25.4 / 2.], Some("1/4-20"), true));
    v.push(fit_case("6.6, 6.6, 5.0 (three plates)", &[3.3, 3.3, R5], Some("M6"), true));

    // a plate turned half a turn, and one flipped over so its blind hole opens on the joint face
    {
        let turn = Pose::about_line([0., 0., 1.], 180., [20., 15., 0.]);
        let a = slab(0., 10., &[([10., 8.], R5), ([25., 20.], R5)]);
        let b = slab(10., 20., &[([30., 22.], R5), ([15., 10.], R5)]);
        v.push(Case {
            class: "placed instances",
            name: "plate turned 180 degrees about z",
            parts: vec![(a, Pose::ID), (b, turn)],
            holes: vec![
                hole(0, [10., 8., 5.], R5, Closure::Through),
                hole(0, [25., 20., 5.], R5, Closure::Through),
                hole(1, [10., 8., 15.], R5, Closure::Through),
                hole(1, [25., 20., 15.], R5, Closure::Through),
            ],
            sites: vec![site([10., 8., 0.], &[0, 1], Some("M5"), false), site([25., 20., 0.], &[0, 1], Some("M5"), false)],
            ifaces: vec![iface(0, 1, PLATE - 2. * disk(R5), 2. * ring(R5))],
            concave_cylinders: None,
        });
        let flip = Pose::about_line([1., 0., 0.], 180., [0., 15., 15.]);
        let a = slab(0., 10., &[([20., 15.], R5)]);
        let b = blind_block(10., 20., R5, 4., false);
        v.push(Case {
            class: "placed instances",
            name: "plate flipped over, blind hole on the joint face",
            parts: vec![(a, Pose::ID), (b, flip)],
            holes: vec![hole(0, [20., 15., 5.], R5, Closure::Through), hole(1, [20., 15., 12.], R5, Closure::Blind)],
            sites: vec![site([20., 15., 0.], &[0, 1], Some("M5"), false)],
            ifaces: vec![iface(0, 1, PLATE - disk(R5), 2. * ring(R5))],
            concave_cylinders: None,
        });
    }

    // a slot and rounded pocket corners are concave cylinders and not holes
    {
        let base = slab(0., 10., &[]);
        let slot = extrude_outlines(&rect_outline(0., 0., 40., 30.), &[slot_outline(14., 26., 15., 3.)], 10., 20.);
        v.push(Case {
            class: "slot and corner fillets",
            name: "slotted plate on a blank plate",
            parts: vec![(base.clone(), Pose::ID), (slot.clone(), Pose::ID)],
            holes: vec![],
            sites: vec![],
            ifaces: vec![iface(0, 1, PLATE - slot_area(14., 26., 3.), ring(3.))],
            concave_cylinders: Some(2),
        });
        let window = extrude_outlines(&rect_outline(0., 0., 40., 30.), &[rounded_outline(10., 8., 30., 22., 2.5)], 10., 20.);
        v.push(Case {
            class: "slot and corner fillets",
            name: "rounded-corner window on a blank plate",
            parts: vec![(base, Pose::ID), (window, Pose::ID)],
            holes: vec![],
            sites: vec![],
            ifaces: vec![iface(0, 1, PLATE - (20. * 14. - (4. - PI) * 2.5 * 2.5), ring(2.5))],
            concave_cylinders: Some(4),
        });
        // the hole's disk lies wholly inside the slot (its right half in the straight part, its left half in the end cap)
        // a bolt through a slot end is a fastener a person would label, but a half turn is not a bore
        let under = slab(0., 10., &[([14., 15.], 3.)]);
        let mut s = site([14., 15., 0.], &[0, 1], None, false);
        s.limit = Some("a slot end wraps half a turn, so it is not a hole: slots are not recognised as fastener locations");
        v.push(Case {
            class: "slot and corner fillets",
            name: "slot end over a round hole of its radius",
            parts: vec![(under, Pose::ID), (slot.clone(), Pose::ID)],
            holes: vec![hole(0, [14., 15., 5.], 3., Closure::Through)],
            sites: vec![s],
            ifaces: vec![iface(0, 1, PLATE - slot_area(14., 26., 3.), 2. * ring(3.))],
            concave_cylinders: Some(3),
        });
        // a slot and a round hole in one plate: only the round pair is a site
        let both = {
            let b = slot.clone();
            let top = b.planar_faces_through([0., 0., 20.], [0., 0., 1.])[0];
            b.drill(top, [34., 15., 20.], R5, None).unwrap()
        };
        let under = slab(0., 10., &[([34., 15.], R5)]);
        v.push(Case {
            class: "slot and corner fillets",
            name: "slot and a round hole, one round pair",
            parts: vec![(under, Pose::ID), (both, Pose::ID)],
            holes: vec![hole(0, [34., 15., 5.], R5, Closure::Through), hole(1, [34., 15., 15.], R5, Closure::Through)],
            sites: vec![site([34., 15., 0.], &[0, 1], Some("M5"), false)],
            ifaces: vec![iface(0, 1, PLATE - slot_area(14., 26., 3.) - disk(R5), ring(3.) + 2. * ring(R5))],
            concave_cylinders: Some(4),
        });
    }
    v
}

// ---------------------------------------------------------------------------------------------------------------------------
// Scoring
// ---------------------------------------------------------------------------------------------------------------------------

#[derive(Default, Clone, Copy, Debug)]
struct Tally {
    tp: usize,
    fp: usize,
    /// Truth labels the detector is not documented to miss, and missed.
    missed: usize,
    /// Truth labels documented as beyond the detector, and (as documented) missed.
    limit: usize,
    /// A documented limit that was found after all: the document is stale.
    cleared: usize,
}

#[derive(Default, Clone, Copy, Debug)]
struct Score {
    holes: Tally,
    sites: Tally,
    ifaces: Tally,
    /// A site at the right place and with the right bodies but a different fastener size or mismatch flag.
    mislabelled: usize,
}

impl Score {
    fn add(&mut self, o: &Score) {
        for (a, b) in [(&mut self.holes, &o.holes), (&mut self.sites, &o.sites), (&mut self.ifaces, &o.ifaces)] {
            a.tp += b.tp;
            a.fp += b.fp;
            a.missed += b.missed;
            a.limit += b.limit;
            a.cleared += b.cleared;
        }
        self.mislabelled += o.mislabelled;
    }
    fn clean(&self) -> bool {
        [self.holes, self.sites, self.ifaces].iter().all(|t| t.fp == 0 && t.missed == 0 && t.cleared == 0) && self.mislabelled == 0
    }
}

fn dist_to_line(p: V3, o: V3, a: V3) -> f64 {
    let d = [p[0] - o[0], p[1] - o[1], p[2] - o[2]];
    let t = d[0] * a[0] + d[1] * a[1] + d[2] * a[2];
    ((d[0] - a[0] * t).powi(2) + (d[1] - a[1] * t).powi(2) + (d[2] - a[2] * t).powi(2)).sqrt()
}

/// Score detected features against a case's labels, the assembly placed by `global`.
fn score(case: &Case, global: &Pose, f: &Features, chord: f64) -> Score {
    let mut s = Score::default();
    // holes
    let mut taken = vec![false; f.holes.len()];
    for e in &case.holes {
        let at = global.apply(e.at);
        let found = f.holes.iter().enumerate().position(|(i, h)| {
            !taken[i] && h.body == e.body && (h.radius - e.r).abs() < 1e-6 && dist_to_line(at, h.axis_point, h.axis) < 1e-6 && h.closure() == e.closure
        });
        match found {
            Some(i) => {
                taken[i] = true;
                s.holes.tp += 1;
            }
            None => s.holes.missed += 1,
        }
    }
    s.holes.fp += taken.iter().filter(|t| !**t).count();
    // sites
    let mut taken = vec![false; f.sites.len()];
    for e in &case.sites {
        let at = global.apply(e.at);
        let found = f.sites.iter().enumerate().position(|(i, st)| !taken[i] && st.bodies == e.bodies && dist_to_line(at, st.axis_point, st.axis) < 1e-6);
        match (found, e.limit) {
            (Some(i), None) => {
                taken[i] = true;
                s.sites.tp += 1;
                if f.sites[i].fastener != e.fastener || f.sites[i].diameter_mismatch != e.mismatch {
                    s.mislabelled += 1;
                    eprintln!("  mislabelled site: {:?} mismatch {} (wanted {:?} {})", f.sites[i].fastener, f.sites[i].diameter_mismatch, e.fastener, e.mismatch);
                }
            }
            (Some(i), Some(_)) => {
                taken[i] = true;
                s.sites.cleared += 1;
            }
            (None, None) => s.sites.missed += 1,
            (None, Some(_)) => s.sites.limit += 1,
        }
    }
    s.sites.fp += taken.iter().filter(|t| !**t).count();
    // interfaces
    let mut taken = vec![false; f.interfaces.len()];
    for e in &case.ifaces {
        let bound = e.curved * chord + 1e-6;
        let found = f
            .interfaces
            .iter()
            .enumerate()
            .position(|(i, x)| !taken[i] && [x.a.body, x.b.body] == e.bodies && (x.overlap_area - e.area).abs() <= bound && (x.gap - e.gap).abs() < 1e-9);
        match found {
            Some(i) => {
                taken[i] = true;
                s.ifaces.tp += 1;
            }
            None => s.ifaces.missed += 1,
        }
    }
    s.ifaces.fp += taken.iter().filter(|t| !**t).count();
    s
}

/// Read the placed bodies of a case and check each is where its pose says (every vertex of the part, placed, is a sample point
/// of the matching body's planar boundaries), so a body-order or placement fault in the writer cannot pass as a detection.
fn read_placed(case: &Case, global: &Pose) -> Vec<AnalyticModel> {
    let placed: Vec<(NativeBrep, Pose)> = case.parts.iter().map(|(b, p)| (b.clone(), global.after(p))).collect();
    let text = compose(&placed);
    let data = inventory(&text, Limits::default()).unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
    let models = AnalyticModel::read_all(&data, Options::default()).unwrap_or_else(|e| panic!("{}: {e:?}", case.name));
    assert_eq!(models.len(), placed.len(), "{}: bodies", case.name);
    let faces = solid_kernel::features::classify_faces(&models).unwrap();
    for (k, (brep, pose)) in placed.iter().enumerate() {
        let samples: Vec<V3> = faces
            .iter()
            .filter(|f| f.key.body == k)
            .flat_map(|f| match &f.geometry {
                FaceGeometry::Plane { outer, holes, .. } => outer.iter().chain(holes.iter().flatten()).copied().collect::<Vec<_>>(),
                _ => vec![],
            })
            .collect();
        for v in &brep.vertices {
            let w = pose.apply(*v);
            let near = samples.iter().any(|s| ((s[0] - w[0]).powi(2) + (s[1] - w[1]).powi(2) + (s[2] - w[2]).powi(2)).sqrt() < 1e-6);
            assert!(near, "{}: body {k} vertex {v:?} is not where its placement puts it ({w:?})", case.name);
        }
    }
    models
}

fn poses() -> Vec<(&'static str, Pose)> {
    let u = [0.95, 0.2, (1.0f64 - 0.95 * 0.95 - 0.2 * 0.2).sqrt()];
    vec![
        ("axis-aligned", Pose::ID),
        ("oblique A", Pose::about_line([1., 2., 3.], 37., [0.; 3]).shifted([13., -7., 5.])),
        ("oblique B", Pose::about_line([-2., 1., 0.5], 112., [20., 15., 5.]).shifted([-30., 40., 2.])),
        ("oblique C (x-dominant)", Pose::taking([0., 0., 1.], u).after(&Pose::about_line([0., 0., 1.], 53., [20., 15., 0.])).shifted([5., 5., -60.])),
    ]
}

#[test]
fn the_oblique_placements_are_oblique() {
    for (name, p) in poses().iter().skip(1) {
        // no coordinate axis of the assembly lands on a coordinate axis of the world
        for d in [[1., 0., 0.], [0., 1., 0.], [0., 0., 1.]] {
            let w = p.dir(d);
            let big = w.iter().fold(0.0f64, |m, c| m.max(c.abs()));
            assert!(big < 0.96 && big > 0.3, "{name}: {w:?}");
            assert!(w.iter().filter(|c| c.abs() < 1e-3).count() == 0, "{name}: {w:?}");
        }
    }
    // the x-dominant pose really takes the hole axis (assembly z) to a mostly-x direction
    assert!(poses()[3].1.dir([0., 0., 1.])[0] > 0.9);
}

#[test]
fn the_synthetic_corpus_has_no_false_positives_and_no_unexpected_misses_on_or_off_axis() {
    let mut by_class: BTreeMap<(&'static str, &'static str), Score> = BTreeMap::new();
    let mut total = Score::default();
    let mut dirty = Vec::new();
    let mut runs = 0;
    for case in cases() {
        for (label, global) in poses() {
            let models = read_placed(&case, &global);
            let chord = models.iter().map(|m| m.chord_tolerance_mm()).fold(0.0, f64::max);
            let f = detect(&models).unwrap();
            assert!(f.unresolved.is_empty(), "{} [{label}]: {:?}", case.name, f.unresolved);
            if let Some(n) = case.concave_cylinders {
                let got = f.faces.iter().filter(|x| matches!(x.geometry, FaceGeometry::Cylinder { side: Side::Concave, .. })).count();
                assert_eq!(got, n, "{} [{label}]: concave cylinder faces", case.name);
            }
            let s = score(&case, &global, &f, chord);
            if !s.clean() {
                dirty.push(format!("{} [{label}]: {s:?}\n  holes {:?}\n  sites {:?}\n  interfaces {:?}", case.name, f.holes, f.sites, f.interfaces));
            }
            by_class.entry((case.class, label)).or_default().add(&s);
            total.add(&s);
            runs += 1;
        }
    }
    let row = |t: &Tally| format!("{:>3} {:>3} {:>3} {:>3}", t.tp, t.fp, t.missed, t.limit);
    println!("synthetic corpus: {runs} case runs");
    println!("{:<26} {:<24} | holes tp fp mi lim | sites tp fp mi lim | interfaces tp fp mi lim", "class", "placement");
    for ((class, label), s) in &by_class {
        println!("{class:<26} {label:<24} | {} | {} | {}   mislabelled {}", row(&s.holes), row(&s.sites), row(&s.ifaces), s.mislabelled);
    }
    println!("{:<26} {:<24} | {} | {} | {}", "TOTAL", "", row(&total.holes), row(&total.sites), row(&total.ifaces));
    assert!(dirty.is_empty(), "{}", dirty.join("\n"));
}
