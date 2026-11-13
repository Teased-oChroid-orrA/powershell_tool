//! The bushing in its housing drawn to scale: the housing plate (the FE plate of `fe::layout`, the bore axis vertical and
//! the plate as thick as the housing length) with the bushing sleeve in its bore, and the flange when it has one. A quarter
//! is cut away so the section shows (the same quarter as `editor/drawing.rs`), or the whole joint is drawn.
//!
//! The colours are `bushing-solver`'s own closed-form Lamé field (`bushing_stress_field` / `housing_stress_field`), read at
//! each point's radius from the bore axis and held at the field's end values beyond the radii it was sampled over (the
//! housing's equivalent OD; the flange). Nothing here is a stress formula and nothing is meshed or solved.
use super::fe;
use crate::gpu_viewer::{editor::drawing::Mesh, Scene};
use crate::toolboxes::bushing::model::BushingModel;
use bushing_solver::geometry::BushingType;
use mechanics_core::lame::LameSample;
use std::f64::consts::{PI, TAU};

/// What the colour key says for each part code.
pub const KEY_LABELS: [&str; 4] = ["Housing", "Bushing", "Bore", "Cut face"];
const CODE_HOUSING: f32 = 0.0;
const CODE_BUSHING: f32 = 1.0;
const CODE_BORE: f32 = 2.0;
const CODE_CUT: f32 = 3.0;

/// What the picture is coloured by.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Field {
    Hoop,
    Radial,
    Parts,
}
impl Field {
    pub const ALL: [Field; 3] = [Field::Hoop, Field::Radial, Field::Parts];
    pub fn label(self) -> &'static str {
        match self {
            Field::Hoop => "Hoop stress",
            Field::Radial => "Radial stress",
            Field::Parts => "Parts",
        }
    }
}

/// Facets around the drawn sweep, and points from the bore out to the plate's edge.
const AROUND: usize = 96;
const ACROSS: usize = 14;
/// Points across the bushing wall.
const WALL: usize = 6;

fn at(r: f64, y: f64, a: f64) -> [f64; 3] {
    [r * a.cos(), y, r * a.sin()]
}
fn radius(p: &[f64; 3]) -> f64 {
    (p[0] * p[0] + p[2] * p[2]).sqrt()
}

/// A sampled Lamé field at radius `r` (linear between samples), held at its end values outside the sampled radii.
pub fn sample(field: &[LameSample], r: f64, pick: fn(&LameSample) -> f64) -> f64 {
    match field {
        [] => 0.0,
        [only] => pick(only),
        _ => {
            let i = field.partition_point(|s| s.r < r);
            if i == 0 {
                return pick(&field[0]);
            }
            let Some(b) = field.get(i) else { return pick(&field[field.len() - 1]) };
            let a = &field[i - 1];
            let t = if b.r > a.r { (r - a.r) / (b.r - a.r) } else { 0.0 };
            pick(a) + (pick(b) - pick(a)) * t
        }
    }
}

/// The drawn angles about the bore axis (the quarter facing the opening camera left out when `cut`), with the angles of
/// the plate's corners added so its outline is exact.
fn angles(cut: bool, corners: &[f64]) -> Vec<f64> {
    let (start, sweep) = if cut { (1.75 * PI, 1.5 * PI) } else { (0.0, TAU) };
    let mut v: Vec<f64> = (0..=AROUND).map(|k| start + sweep * k as f64 / AROUND as f64).collect();
    for &c in corners {
        let a = start + (c - start).rem_euclid(TAU);
        if a > start && a < start + sweep {
            v.push(a);
        }
    }
    v.sort_by(f64::total_cmp);
    v.dedup_by(|a, b| (*a - *b).abs() < 1e-9);
    v
}

/// The picture of `m` coloured by `field`, the quarter cut away when `cut`, or why there is none.
pub fn scene(m: &BushingModel, name: &str, field: Field, cut: bool) -> Result<Scene, String> {
    let l = fe::layout(m)?;
    let out = &m.output;
    let (a, ri, t) = (l.bore_radius, m.id_bushing / 2.0, m.housing_len);
    if !(t.is_finite() && t > 0.0) {
        return Err("the housing length must be positive".into());
    }
    // The plate about the bore axis: x from the free edge to the far end, z across its width.
    let (x0, x1, z0, z1) = (-l.edge, l.length - l.edge, -l.height / 2.0, l.height / 2.0);
    let reach = |g: f64| {
        let (c, s) = (g.cos(), g.sin());
        let along = |d: f64, lo: f64, hi: f64| if d > 1e-12 { hi / d } else if d < -1e-12 { lo / d } else { f64::INFINITY };
        along(c, x0, x1).min(along(s, z0, z1))
    };
    let corners: Vec<f64> = [(x0, z0), (x1, z0), (x1, z1), (x0, z1)].iter().map(|&(x, z)| z.atan2(x)).collect();
    let angs = angles(cut, &corners);
    let is_corner = |g: f64| corners.iter().any(|&c| (g - c).rem_euclid(TAU) < 1e-9 || (c - g).rem_euclid(TAU) < 1e-9);
    let pick: Option<fn(&LameSample) -> f64> = match field {
        Field::Hoop => Some(|s| s.sigma_theta),
        Field::Radial => Some(|s| s.sigma_r),
        Field::Parts => None,
    };
    let housing = |r: f64| pick.map_or(CODE_HOUSING, |f| sample(&out.housing_stress_field, r, f) as f32);
    let bushing = |r: f64| pick.map_or(CODE_BUSHING, |f| sample(&out.bushing_stress_field, r, f) as f32);
    let housing_cut = |r: f64| if pick.is_some() { housing(r) } else { CODE_CUT };
    let bushing_cut = |r: f64| if pick.is_some() { bushing(r) } else { CODE_CUT };
    // Graded toward the bore, where the field changes fastest.
    let across = |g: f64| -> Vec<f64> {
        let outer = reach(g);
        (0..=ACROSS).map(|k| a + (outer - a) * (k as f64 / ACROSS as f64).powf(1.6)).collect()
    };
    let wall = |r1: f64| -> Vec<f64> { (0..=WALL).map(|k| ri + (r1 - ri) * k as f64 / WALL as f64).collect() };

    let mut mesh = Mesh::default();
    // The housing's faces.
    for y in [0.0, t] {
        let rows: Vec<Vec<[f64; 3]>> = angs.iter().map(|&g| across(g).into_iter().map(|r| at(r, y, g)).collect()).collect();
        mesh.grid(&rows, |p| housing(radius(p)));
    }
    // Its sides, one surface per side of the rectangle so the corners stay sharp.
    let mut run: Vec<f64> = Vec::new();
    for (k, &g) in angs.iter().enumerate() {
        run.push(g);
        if (is_corner(g) || k + 1 == angs.len()) && run.len() > 1 {
            let rows: Vec<Vec<[f64; 3]>> = run.iter().map(|&g| [0.0, t].iter().map(|&y| at(reach(g), y, g)).collect()).collect();
            mesh.grid(&rows, |p| housing(radius(p)));
            run = vec![g];
        }
    }
    // The bushing: its bore, its ends and (flanged) the flange.
    let flange = (m.bushing_type == BushingType::Flanged && m.flange_od / 2.0 > a && m.flange_thk > 0.0).then_some((m.flange_od / 2.0, m.flange_thk));
    let top = t + flange.map_or(0.0, |f| f.1);
    let rows: Vec<Vec<[f64; 3]>> = angs.iter().map(|&g| vec![at(ri, 0.0, g), at(ri, top, g)]).collect();
    mesh.grid(&rows, |p| if pick.is_some() { bushing(radius(p)) } else { CODE_BORE });
    let ends: [(f64, f64); 2] = [(0.0, a), (top, flange.map_or(a, |f| f.0))];
    for (y, r1) in ends {
        let rows: Vec<Vec<[f64; 3]>> = angs.iter().map(|&g| wall(r1).into_iter().map(|r| at(r, y, g)).collect()).collect();
        mesh.grid(&rows, |p| bushing(radius(p)));
    }
    if let Some((rf, _)) = flange {
        let rows: Vec<Vec<[f64; 3]>> = angs.iter().map(|&g| vec![at(rf, t, g), at(rf, top, g)]).collect();
        mesh.grid(&rows, |p| bushing(radius(p)));
    }
    // The cut faces: the section through the housing and the bushing at both ends of the sweep.
    if cut {
        for g in [angs[0], angs[angs.len() - 1]] {
            let rows: Vec<Vec<[f64; 3]>> = [0.0, t].iter().map(|&y| across(g).into_iter().map(|r| at(r, y, g)).collect()).collect();
            mesh.grid(&rows, |p| housing_cut(radius(p)));
            let rows: Vec<Vec<[f64; 3]>> = [0.0, t].iter().map(|&y| wall(a).into_iter().map(|r| at(r, y, g)).collect()).collect();
            mesh.grid(&rows, |p| bushing_cut(radius(p)));
            if let Some((rf, _)) = flange {
                let rows: Vec<Vec<[f64; 3]>> = [t, top].iter().map(|&y| wall(rf).into_iter().map(|r| at(r, y, g)).collect()).collect();
                mesh.grid(&rows, |p| bushing_cut(radius(p)));
            }
        }
    }
    match pick {
        None => mesh.into_scene(format!("{name} | parts"), "part (see color key)", [CODE_HOUSING, CODE_CUT], Vec::new()),
        Some(_) => {
            let lo = mesh.values.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = mesh.values.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let range = if hi - lo > 1e-3 * hi.abs().max(lo.abs()).max(1.0) { [lo, hi] } else { [lo - 1.0, hi + 1.0] };
            mesh.into_scene(format!("{name} | {}, closed-form Lame", field.label().to_lowercase()), field.label(), range, Vec::new())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::bushing::model::NumberTarget;

    fn extent(s: &Scene, k: usize) -> f32 {
        s.positions.iter().map(|p| p[k]).fold(f32::NEG_INFINITY, f32::max) - s.positions.iter().map(|p| p[k]).fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn the_picture_is_the_fe_plate_to_scale_coloured_by_the_solvers_own_field() {
        let m = BushingModel::default();
        let l = fe::layout(&m).unwrap();
        let s = scene(&m, "case", Field::Hoop, false).unwrap();
        // Scaled to a unit span: the plate's length is the span; its width and the bore length keep their proportions.
        let span = l.length.max(l.height);
        assert!((extent(&s, 0) as f64 - l.length / span).abs() < 1e-4, "{} vs {}", extent(&s, 0), l.length / span);
        assert!((extent(&s, 2) as f64 - l.height / span).abs() < 1e-4);
        assert!((extent(&s, 1) as f64 - m.housing_len / span).abs() < 1e-4);
        // The colour range is the solver's hoop stresses: the bushing's compression to the housing's bore tension.
        let out = &m.output;
        assert!((s.range[0] as f64 - out.bushing_stress_field.iter().map(|x| x.sigma_theta).fold(f64::INFINITY, f64::min)).abs() < 1.0);
        assert!((s.range[1] as f64 - out.stress_hoop_housing).abs() < 0.01 * out.stress_hoop_housing.abs(), "{:?} vs {}", s.range, out.stress_hoop_housing);
        assert_eq!(s.field_label, "Hoop stress");
    }

    #[test]
    fn parts_use_the_colour_key_and_the_cut_shows_the_section() {
        let m = BushingModel::default();
        let whole = scene(&m, "case", Field::Parts, false).unwrap();
        let cut = scene(&m, "case", Field::Parts, true).unwrap();
        let codes = |s: &Scene| -> Vec<f32> { s.samples.iter().map(|v| v[3]).collect() };
        for c in [CODE_HOUSING, CODE_BUSHING, CODE_BORE] {
            assert!(codes(&whole).contains(&c));
        }
        assert!(!codes(&whole).contains(&CODE_CUT), "nothing is cut");
        assert!(codes(&cut).contains(&CODE_CUT));
        assert_eq!(cut.range, [0.0, 3.0]);
    }

    #[test]
    fn a_flange_stands_above_the_housing_and_inputs_with_no_bore_draw_nothing() {
        let mut m = BushingModel::default();
        let plain = scene(&m, "case", Field::Hoop, true).unwrap();
        m.bushing_type = BushingType::Flanged;
        m.flange_od = 0.8;
        m.flange_thk = 0.1;
        m.recompute();
        let flanged = scene(&m, "case", Field::Hoop, true).unwrap();
        assert!(flanged.positions.len() > plain.positions.len());
        m.commit_number(NumberTarget::EdgeDist, 0.1);
        assert!(scene(&m, "case", Field::Hoop, true).unwrap_err().contains("edge distance"));
    }

    #[test]
    fn the_field_is_held_at_its_ends_and_linear_between_samples() {
        let f = [LameSample { r: 1.0, sigma_r: -10.0, sigma_theta: 30.0, sigma_axial: 0.0 }, LameSample { r: 2.0, sigma_r: 0.0, sigma_theta: 10.0, sigma_axial: 0.0 }];
        assert_eq!(sample(&f, 0.5, |s| s.sigma_theta), 30.0);
        assert_eq!(sample(&f, 1.5, |s| s.sigma_theta), 20.0);
        assert_eq!(sample(&f, 9.0, |s| s.sigma_r), 0.0);
    }
}
