//! The clamped joint drawn to scale as its finite-element model sees it: each member a revolved double frustum (two cones
//! meeting at the mid-plane, from the bearing face at the cone angle that reproduces the model's compliance, cut by the
//! member's outer diameter), a quarter cut away (`editor/drawing.rs`), the shank in the bore, the head on top.
//!
//! Every stress comes from the solved member model (`MemberField::stress_at`, per unit clamp force, scaled by the
//! solver's preload) or, in the shank, from the solver's own stress section; nothing here is a stress formula. Before the
//! model's answer for this joint arrives the picture shows the parts in a colour key instead.
//!
//! The members' revolved surfaces come last in the triangle list and are drawn at `MEMBER_OPACITY` so the bolt shows
//! through them; their cut faces stay solid, so the section reads exactly.
use crate::gpu_viewer::{
    editor::drawing::{self, Mesh, SEGMENTS},
    scene::ScalarField,
    Scene,
};
use crate::toolboxes::preload_analysis::{fe_check::FeResult, joint_templates::LayerRole, model::PreloadModel};
use fastened_joint_solver::solve::JointSolution;
use fea_problem::joint::MemberField;

/// What the colour key says for each part code.
pub const KEY_LABELS: [&str; 4] = ["Member", "Washer", "Bolt", "Cut face"];
const CODE_MEMBER: f64 = 0.0;
const CODE_WASHER: f64 = 1.0;
const CODE_BOLT: f64 = 2.0;
const CODE_CUT: f64 = 3.0;

/// How opaque the members' revolved surfaces are drawn (owner choice: a fixed 35%, no control).
pub const MEMBER_OPACITY: f32 = 0.35;

/// The fields the picture carries once solved; the first is the one it opens on.
pub const FIELDS: [&str; 4] = ["Axial stress", "Radial stress", "Hoop stress", "von Mises stress"];

/// Points across a member from its bore to the cone, on the cut faces and the bearing faces.
const ACROSS: usize = 16;
/// Stations along the whole stack (each member gets its share, at least `MIN_STATIONS`).
const STATIONS: f64 = 24.0;
const MIN_STATIONS: usize = 4;

/// The answer the picture is coloured by: the member field and its cross-check, and the solve that sets the preload.
pub struct Solved<'a> {
    pub field: &'a MemberField,
    pub fe: &'a FeResult,
    pub solution: &'a JointSolution,
}

/// The values one drawn point carries: `[axial, radial, hoop, von Mises]` in psi, or the part code in all four.
type Values = [f64; 4];

/// The mesh with its four value columns kept parallel to the positions.
struct Picture {
    mesh: Mesh,
    columns: [Vec<f64>; 4],
}
impl Picture {
    /// Values for the vertices the last mesh call added: `values` (one per point it was given) repeated `copies` times.
    fn fill(&mut self, values: &[Values], copies: usize) {
        for _ in 0..copies {
            for v in values {
                for (c, x) in self.columns.iter_mut().zip(v) {
                    c.push(*x);
                }
            }
        }
        debug_assert!(self.columns[0].len() == self.mesh.positions.len());
    }
    fn revolve(&mut self, line: &[(f64, f64)], values: &[Values]) {
        self.mesh.revolve(line, |_| 0.0);
        self.fill(values, SEGMENTS + 1);
    }
    fn cut(&mut self, rows: &[Vec<(f64, f64)>], values: &[Values]) {
        self.mesh.cut(rows, |_| 0.0);
        self.fill(values, 2);
    }
}

/// The picture of `m` (drawn in inches; coloured in psi when `solved`, else by part), with the index in its triangles
/// where the members' revolved surfaces begin (drawn translucent), or why there is none.
pub fn scene(m: &PreloadModel, name: &str, solved: Option<Solved>) -> Result<(Scene, u32), String> {
    let solution = m.output.as_ref().map_err(|e| format!("the inputs make no joint ({e:?})"))?;
    let total: f64 = m.members.iter().map(|l| l.thickness).sum();
    if !(total > 0.0) || m.members.iter().any(|l| !(l.thickness > 0.0 && l.hole_diameter >= 0.0)) {
        return Err("a member has no thickness".into());
    }
    let angle = solved.as_ref().and_then(|s| s.fe.equivalent_angle_deg).unwrap_or(m.cone_half_angle_deg);
    let tan = angle.to_radians().tan();
    let rc = m.bearing_outer_radius;
    let mut pic = Picture { mesh: Mesh::default(), columns: Default::default() };
    // The members' revolved surfaces, drawn after everything solid.
    let mut shells: Vec<(Vec<(f64, f64)>, Vec<Values>)> = Vec::new();
    // FE coordinates: y = 0 at the head face, down the stack; drawn height z = total - y (the head on top).
    let mut y0 = 0.0;
    for member in &m.members {
        let y1 = y0 + member.thickness;
        let bore = 0.5 * member.hole_diameter;
        let od = (member.outer_diameter > 0.0).then_some(0.5 * member.outer_diameter);
        // The cone grows from each bearing face to the mid-plane, cut by the outer diameter, never inside the bore.
        let cone = |y: f64| rc + y.min(total - y).max(0.0) * tan;
        let outer = |y: f64| {
            let r = od.map_or(cone(y), |d| cone(y).min(d));
            r.max(bore + 0.02 * total).min(od.unwrap_or(f64::INFINITY))
        };
        // Stations: even through the member, plus the mid-plane and where the outer diameter starts cutting the cone.
        let n = ((STATIONS * member.thickness / total).ceil() as usize).max(MIN_STATIONS);
        let mut ys: Vec<f64> = (0..=n).map(|k| y0 + member.thickness * k as f64 / n as f64).collect();
        let mut kinks = vec![0.5 * total];
        if let Some(d) = od {
            let reach = (d - rc) / tan;
            kinks.extend([reach, total - reach]);
        }
        ys.extend(kinks.into_iter().filter(|&y| y > y0 && y < y1));
        ys.sort_by(f64::total_cmp);
        ys.dedup_by(|a, b| (*a - *b).abs() < 1e-9 * total);

        let grid: Vec<Vec<(f64, f64)>> = ys.iter().map(|&y| (0..=ACROSS).map(|k| (bore + (outer(y) - bore) * k as f64 / ACROSS as f64, y)).collect()).collect();
        let part = if member.name == LayerRole::Washer.name() { CODE_WASHER } else { CODE_MEMBER };
        // One value per grid point, read from the solved model just inside the member (rounding at its faces).
        let values: Vec<Vec<Values>> = match &solved {
            Some(s) => {
                let eps = 1e-6 * total;
                grid.iter()
                    .map(|row| {
                        row.iter()
                            .map(|&(r, y)| {
                                let at = s.field.stress_at(r.clamp(bore + eps, outer(y) - eps), y.clamp(y0 + eps, y1 - eps)).unwrap_or([0.0; 6]);
                                let st: [f64; 6] = std::array::from_fn(|i| at[i] * s.solution.preload);
                                [st[1], st[0], st[2], fea_core::kernel::von_mises(&st)]
                            })
                            .collect()
                    })
                    .collect()
            }
            None => grid.iter().map(|row| vec![[part; 4]; row.len()]).collect(),
        };
        let drawn = |p: &(f64, f64)| (p.0, total - p.1);
        let rows: Vec<Vec<(f64, f64)>> = grid.iter().map(|row| row.iter().map(drawn).collect()).collect();
        let last = rows.len() - 1;
        let column = |k: usize| -> (Vec<(f64, f64)>, Vec<Values>) { (rows.iter().map(|r| r[k]).collect(), values.iter().map(|v| v[k]).collect()) };
        let cut_values: Vec<Values> = match solved {
            Some(_) => values.concat(),
            None => vec![[CODE_CUT; 4]; rows.len() * (ACROSS + 1)],
        };
        pic.cut(&rows, &cut_values);
        shells.push(column(ACROSS));
        let (mut line, mut vals) = column(0);
        line.reverse();
        vals.reverse();
        shells.push((line, vals));
        shells.push((rows[0].clone(), values[0].clone()));
        shells.push((rows[last].clone(), values[last].clone()));
        y0 = y1;
    }
    // The shank in the bore, from the head face to the nut face, at the solver's shank stress (just clear of the
    // narrowest bore so the two surfaces never coincide).
    let rb = (0.5 * m.shank_diameter).min(0.98 * m.members.iter().map(|l| 0.5 * l.hole_diameter).fold(f64::INFINITY, f64::min));
    if rb > 0.0 {
        let sec = &solution.stress.shank;
        let v: Values = if solved.is_some() { [sec.axial, 0.0, 0.0, sec.von_mises] } else { [CODE_BOLT; 4] };
        let cut: Values = if solved.is_some() { v } else { [CODE_CUT; 4] };
        pic.revolve(&[(rb, 0.0), (rb, total)], &[v; 2]);
        pic.revolve(&[(0.0, total), (rb, total)], &[v; 2]);
        pic.revolve(&[(rb, 0.0), (0.0, 0.0)], &[v; 2]);
        pic.cut(&[vec![(0.0, total), (rb, total)], vec![(0.0, 0.0), (rb, 0.0)]], &[cut; 4]);
    }
    let translucent_from = pic.mesh.triangles.len() as u32;
    for (line, values) in &shells {
        pic.revolve(line, values);
    }

    let Picture { mut mesh, columns } = pic;
    match solved {
        None => {
            mesh.values = columns[0].iter().map(|c| *c as f32).collect();
            mesh.into_scene(format!("{name} | the parts while the member model runs"), "part (see color key)", [CODE_MEMBER as f32, CODE_CUT as f32], Vec::new()).map(|s| (s, translucent_from))
        }
        Some(_) => {
            let primary = ScalarField::from_f64(FIELDS[0], &columns[0])?;
            mesh.values = primary.values;
            let fields = FIELDS[1..].iter().zip(&columns[1..]).map(|(label, v)| ScalarField::from_f64(label, v)).collect::<Result<Vec<_>, _>>()?;
            mesh.into_scene(format!("{name} | preload {:.0} lbf", solution.preload), FIELDS[0], primary.range, fields).map(|s| (s, translucent_from))
        }
    }
}

/// A scene with nothing visible in it, for inputs that make no joint.
pub fn empty(name: &str) -> Scene {
    drawing::empty(format!("{name} | nothing to draw"), [0.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::preload_analysis::{fe_check, model::NumberTarget};

    fn extremes(v: &[f32]) -> (f32, f32) {
        (v.iter().copied().fold(f32::INFINITY, f32::min), v.iter().copied().fold(f32::NEG_INFINITY, f32::max))
    }

    #[test]
    fn before_the_fe_answer_the_parts_are_drawn_to_scale_in_the_colour_key() {
        let m = PreloadModel::default();
        let (s, _) = scene(&m, "case", None).unwrap();
        assert_eq!(s.field_label, "part (see color key)");
        assert!(s.fields.is_empty());
        let codes: Vec<f32> = s.samples.iter().map(|v| v[3]).collect();
        for c in [CODE_MEMBER, CODE_BOLT, CODE_CUT] {
            assert!(codes.contains(&(c as f32)), "code {c} drawn");
        }
        assert!(!codes.contains(&(CODE_WASHER as f32)), "the default joint has no washer");
        // To scale: the stack (0.75 in) is the height; the widest point is the cone at the mid-plane, 0.40 + 0.375 tan 30.
        let ext = |a: usize| s.positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max) - s.positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
        let r_mid = 0.40 + 0.375 * 30f64.to_radians().tan();
        let span = 2.0 * r_mid;
        assert!((ext(1) as f64 - 0.75 / span).abs() < 1e-5, "height {} vs {}", ext(1), 0.75 / span);
    }

    #[test]
    fn the_solved_picture_is_the_member_models_stress_under_the_solvers_preload() {
        let m = PreloadModel::default();
        let input = m.fe_input().unwrap();
        let (fe, field) = fe_check::run_with_field(&input).unwrap();
        let solution = m.output.as_ref().unwrap();
        let (s, first) = scene(&m, "case", Some(Solved { field: &field, fe: &fe, solution })).unwrap();
        assert_eq!(s.field_label, "Axial stress");
        assert_eq!(s.fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), FIELDS[1..]);
        let axial: Vec<f32> = s.samples.iter().map(|v| v[3]).collect();
        let (lo, hi) = extremes(&axial);
        // The members are in compression; the shank carries the solver's tension.
        assert!(lo < 0.0, "members compressed: {lo}");
        assert!((hi as f64 - solution.stress.shank.axial).abs() < 1.0, "{hi} vs shank {}", solution.stress.shank.axial);
        // Under the bearing face the compression is about the preload over the bearing annulus (FE, not uniform).
        let ring = std::f64::consts::PI * (m.bearing_outer_radius.powi(2) - 0.203f64.powi(2));
        let nominal = solution.preload / ring;
        assert!((lo as f64).abs() > 0.5 * nominal && (lo as f64).abs() < 5.0 * nominal, "{lo} vs {nominal}");
        // Drawn at the FE-equivalent cone: the mid-plane radius follows it.
        let a = fe.equivalent_angle_deg.unwrap();
        let ext = |a: usize| s.positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max) - s.positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
        let span = 2.0 * (0.40 + 0.375 * a.to_radians().tan());
        assert!((ext(1) as f64 - 0.75 / span).abs() < 1e-5, "{} vs {}", ext(1), 0.75 / span);
        let vm = &s.fields.iter().find(|f| f.label == "von Mises stress").unwrap().values;
        assert!(vm.iter().all(|v| *v >= 0.0));
        // Solid first (the shank, every cut face), then the members' revolved surfaces, drawn translucent: none of
        // those carries the shank's tension, and the shank's top lies before them.
        assert!(first % 3 == 0 && first > 0 && (first as usize) < s.triangles.len());
        let tail = &s.triangles[first as usize..];
        assert!(tail.iter().all(|&i| axial[i as usize] < 0.5 * hi), "only members after the solid part");
        assert!(s.triangles[..first as usize].iter().any(|&i| (axial[i as usize] - hi).abs() < 1.0), "the shank is solid");
    }

    #[test]
    fn washers_have_their_own_colour_and_inputs_that_make_no_joint_draw_nothing() {
        let mut m = PreloadModel::default();
        m.apply_template(&crate::toolboxes::preload_analysis::joint_templates::presets()[0]);
        let (s, _) = scene(&m, "case", None).unwrap();
        assert!(s.samples.iter().any(|v| v[3] == CODE_WASHER as f32));
        m.commit_number(NumberTarget::BearingOuterRadius, 0.0);
        assert!(scene(&m, "case", None).is_err());
        assert_eq!(empty("case").positions.len(), 3);
    }
}
