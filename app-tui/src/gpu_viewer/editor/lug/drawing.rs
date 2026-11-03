//! The lug drawn flat in the xy plane (the front view looks at it from +z) on the finite-element mesh itself: every Q9
//! element split into four sub-quads of its nine nodes, two triangles each, a half model mirrored about the lug axis
//! (`y = 0`) to the full lug. The hole centre is the origin, the clamped end is at `x` = the model length.
//!
//! Solved, the picture is the nodal field the kernel read its results from (`lug_solver::LugField`): von Mises and hoop
//! stress (about the hole centre) in psi, and the displacement magnitude in inches. All three are unchanged by the
//! mirror, so a mirrored node carries its original's values. Before that field is in for these inputs the picture is the
//! mesh of the current geometry in a colour key (lug, bushing, the pin at its contact side, the clamped end); nothing
//! here computes a stress.
use crate::gpu_viewer::{
    editor::drawing::{self, Mesh},
    scene::ScalarField,
    Scene,
};
use crate::toolboxes::lug_analysis::model::{LugRun, LugUiModel};
use lug_solver::{stress::Stress, LugField};

/// What the colour key says for each part code.
pub const KEY_LABELS: [&str; 4] = ["Lug", "Bushing", "Pin", "Clamped end"];
const CODE_PIN: f64 = 2.0;
const CODE_CLAMP: f64 = 3.0;

/// The fields the solved picture carries; the first is the one it opens on.
pub const FIELDS: [&str; 3] = ["von Mises stress", "Hoop stress", "Displacement"];

/// Facets of the pin disc in the geometry view.
const PIN_FACETS: usize = 72;

/// A flat Q9 mesh with values per node, drawn into `mesh` (and `columns`, one value list per field), mirrored about
/// `y = 0` when `half`. `value(i)` gives node `i`'s values.
fn draw_q9<const N: usize>(mesh: &mut Mesh, columns: &mut [Vec<f64>; N], nodes: &[[f64; 2]], elems: &[[usize; 9]], half: bool, z: f64, value: impl Fn(usize) -> [f64; N]) {
    let span = nodes.iter().fold(0.0f64, |m, p| m.max(p[0].abs()).max(p[1].abs()));
    let on_axis = |p: &[f64; 2]| p[1].abs() <= 1e-9 * span;
    let base = mesh.positions.len() as u32;
    let mut put = |p: [f64; 2], v: [f64; N]| {
        mesh.positions.push([p[0], p[1], z]);
        for (c, x) in columns.iter_mut().zip(v) {
            c.push(x);
        }
    };
    for (i, p) in nodes.iter().enumerate() {
        put(*p, value(i));
    }
    // The mirror image of every node off the axis; one on it is shared by both halves.
    let mirror: Vec<u32> = if half {
        let mut next = base + nodes.len() as u32;
        nodes
            .iter()
            .enumerate()
            .map(|(i, p)| {
                if on_axis(p) {
                    base + i as u32
                } else {
                    put([p[0], -p[1]], value(i));
                    next += 1;
                    next - 1
                }
            })
            .collect()
    } else {
        Vec::new()
    };
    for e in elems {
        let n = |a: usize, b: usize| e[3 * b + a];
        for b in 0..2 {
            for a in 0..2 {
                let q = [n(a, b), n(a + 1, b), n(a + 1, b + 1), n(a, b + 1)];
                let own = q.map(|i| base + i as u32);
                mesh.triangles.extend([own[0], own[1], own[2], own[0], own[2], own[3]]);
                if half {
                    // Reflected, so wound the other way round.
                    let m = q.map(|i| mirror[i]);
                    mesh.triangles.extend([m[0], m[2], m[1], m[0], m[3], m[2]]);
                }
            }
        }
    }
    debug_assert!(columns.iter().all(|c| c.len() == mesh.positions.len()));
}

/// The picture of `m`: the solved field of `run` when it has one, else the parts in a colour key; or why there is none.
pub fn scene(m: &LugUiModel, name: &str, run: Option<&LugRun>) -> Result<Scene, String> {
    match run.and_then(|r| r.field.as_deref().map(|f| (r, f))) {
        Some((run, field)) => solved(name, run, field),
        None => parts(m, name),
    }
}

/// The nodal von Mises, hoop and displacement magnitude of `field`, contoured on its mesh.
fn solved(name: &str, run: &LugRun, field: &LugField) -> Result<Scene, String> {
    let mut mesh = Mesh::default();
    let mut columns: [Vec<f64>; 3] = Default::default();
    let value = |i: usize| {
        let (s, p, u) = (field.stress[i], field.nodes[i], field.u[i]);
        let hoop = Stress { sx: s[0], sy: s[1], txy: s[3] }.polar(p[1].atan2(p[0])).0;
        [fea_core::kernel::von_mises(&s), hoop, u[0].hypot(u[1])]
    };
    draw_q9(&mut mesh, &mut columns, &field.nodes, &field.elems, field.half, 0.0, value);
    let primary = ScalarField::from_f64(FIELDS[0], &columns[0])?;
    mesh.values = primary.values;
    let fields = FIELDS[1..].iter().zip(&columns[1..]).map(|(label, v)| ScalarField::from_f64(label, v)).collect::<Result<Vec<_>, _>>()?;
    let case = run.input.case;
    mesh.into_scene(format!("{name} | {:.0} lbf at {:.0} deg", case.load_lbf, case.angle_deg), FIELDS[0], primary.range, fields)
}

/// The mesh of the current geometry coloured by part, the pin in the bore on the side the load drives it to and the
/// clamped end as a strip past the far edge.
fn parts(m: &LugUiModel, name: &str) -> Result<Scene, String> {
    let input = m.input()?;
    let g = input.geometry;
    let half = input.case.is_axial();
    let bushing = input.bushing.as_ref().map(|b| lug_solver::solve::bushing_mesh(&g, input.mesh, &b.spec())).transpose()?;
    let fe = lug_solver::mesh::Mesh::build_with(&g, input.mesh, half, bushing)?;
    let mut mesh = Mesh::default();
    let mut codes: [Vec<f64>; 1] = Default::default();
    // A node takes its element's group (lug 0, bushing 1); the two bodies do not share nodes.
    let mut group = vec![0.0; fe.nodes.len()];
    for (e, grp) in fe.elems.iter().zip(&fe.elem_group) {
        for &n in e {
            group[n] = f64::from(*grp);
        }
    }
    draw_q9(&mut mesh, &mut codes, &fe.nodes, &fe.elems, half, 0.0, |i| [group[i]]);
    // The pin: a disc of its own diameter, moved across the clearance in the load's direction, just above the lug.
    let (r_pin, r_bore) = (0.5 * input.pin.diameter, 0.5 * m.contact_dia());
    let d = input.case.direction();
    let shift = (r_bore - r_pin).max(0.0);
    let centre = [shift * d[0], shift * d[1]];
    let z = 1e-3 * g.width;
    let hub = mesh.positions.len() as u32;
    mesh.positions.push([centre[0], centre[1], z]);
    for k in 0..PIN_FACETS {
        let a = std::f64::consts::TAU * k as f64 / PIN_FACETS as f64;
        mesh.positions.push([centre[0] + r_pin * a.cos(), centre[1] + r_pin * a.sin(), z]);
    }
    for k in 0..PIN_FACETS as u32 {
        mesh.triangles.extend([hub, hub + 1 + k, hub + 1 + (k + 1) % PIN_FACETS as u32]);
    }
    codes[0].extend(std::iter::repeat_n(CODE_PIN, PIN_FACETS + 1));
    // The clamped end: a thin strip across the width at x = L.
    let (x0, x1, w) = (g.length, g.length + 0.04 * g.width, 0.5 * g.width);
    let c = mesh.positions.len() as u32;
    mesh.positions.extend([[x0, -w, 0.0], [x1, -w, 0.0], [x1, w, 0.0], [x0, w, 0.0]]);
    mesh.triangles.extend([c, c + 1, c + 2, c, c + 2, c + 3]);
    codes[0].extend([CODE_CLAMP; 4]);
    mesh.values = codes[0].iter().map(|c| *c as f32).collect();
    mesh.into_scene(format!("{name} | the lug while the analysis runs"), "part (see color key)", [0.0, CODE_CLAMP as f32], Vec::new())
}

/// A scene with nothing visible in it, for inputs that make no lug.
pub fn empty(name: &str) -> Scene {
    drawing::empty(format!("{name} | nothing to draw"), [0.0, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::lug_analysis::model::{self, NumberTarget};

    fn extent(s: &Scene, axis: usize) -> f32 {
        s.positions.iter().map(|p| p[axis]).fold(f32::NEG_INFINITY, f32::max) - s.positions.iter().map(|p| p[axis]).fold(f32::INFINITY, f32::min)
    }

    #[test]
    fn before_a_result_the_whole_lug_is_drawn_to_scale_in_the_colour_key_with_the_pin_and_the_clamp() {
        let m = LugUiModel::default();
        let s = scene(&m, "case", None).unwrap();
        assert_eq!(s.field_label, "part (see color key)");
        assert!(s.fields.is_empty());
        let codes: Vec<f32> = s.samples.iter().map(|v| v[3]).collect();
        for c in [0.0, CODE_PIN, CODE_CLAMP] {
            assert!(codes.contains(&(c as f32)), "code {c} drawn");
        }
        assert!(!codes.contains(&1.0), "no bushing by default");
        // The axial default is a half model, drawn mirrored: the full width W = 1.5 across y; along x from the round
        // head (e = W/2 before the hole) to the clamp strip past L = 3.75.
        let span_x = 0.75 + 3.75 + 0.04 * 1.5;
        assert!((extent(&s, 1) as f64 - 1.5 / span_x).abs() < 1e-4, "{} vs {}", extent(&s, 1), 1.5 / span_x);
        assert!(extent(&s, 2) < 1e-3, "flat in the xy plane");
        let mut b = m.clone();
        b.toggle_bushing();
        assert!(scene(&b, "case", None).unwrap().samples.iter().any(|v| v[3] == 1.0), "the bushing has its own colour");
    }

    #[test]
    fn the_solved_picture_is_the_kernel_field_mirrored_with_its_peak_von_mises() {
        let m = LugUiModel::default();
        let (run, _) = model::run(&m.input().unwrap(), None);
        let run = run.unwrap();
        let field = run.field.as_deref().unwrap();
        assert!(field.half);
        let s = scene(&m, "case", Some(&run)).unwrap();
        assert_eq!(s.field_label, "von Mises stress");
        assert_eq!(s.fields.iter().map(|f| f.label.as_str()).collect::<Vec<_>>(), FIELDS[1..]);
        // Mirrored: every node off the axis twice, the axis nodes once.
        let on_axis = field.nodes.iter().filter(|p| p[1].abs() < 1e-9).count();
        assert!(on_axis > 0);
        assert_eq!(s.positions.len(), 2 * field.nodes.len() - on_axis);
        assert_eq!(s.triangles.len(), field.elems.len() * 8 * 2 * 3);
        // The contour's values are the field's: the solution's peak von Mises (read from the same field, away from the
        // clamp) is among them.
        let vm = s.samples.iter().map(|v| v[3] as f64);
        let peak = run.solution.peak_von_mises;
        assert!(vm.clone().any(|v| (v - peak).abs() <= 1e-5 * peak), "{peak} is a drawn value");
        assert!(vm.fold(f64::NEG_INFINITY, f64::max) >= peak * (1.0 - 1e-5));
        let u = &s.fields.iter().find(|f| f.label == "Displacement").unwrap().values;
        assert!(u.iter().all(|v| *v >= 0.0) && u.iter().any(|v| *v > 0.0));
        // The hoop stress at the bore crown under an axial pull is tension.
        let hoop = &s.fields.iter().find(|f| f.label == "Hoop stress").unwrap().values;
        assert!(hoop.iter().copied().fold(f32::NEG_INFINITY, f32::max) > 0.0);
    }

    #[test]
    fn inputs_that_make_no_lug_draw_nothing() {
        let mut m = LugUiModel::default();
        m.commit_number(NumberTarget::HoleDia, 1.9);
        assert!(scene(&m, "case", None).is_err());
        assert_eq!(empty("case").positions.len(), 3);
    }
}
