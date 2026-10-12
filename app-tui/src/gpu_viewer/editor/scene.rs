//! Member problem (and optional solution) to a renderable `Scene`: members become square tubes, nodes
//! small cubes and an axis triad marks the origin so even an empty model draws something.
use super::camera::Frame;
use crate::gpu_viewer::scene::{ScalarField, Scene};
use fea_problem::members::{MemberProblem, MemberSolved};
use std::collections::BTreeMap;

/// Primary-scalar codes of an editing scene (range 0..=4 through the contour colour map).
pub const CODE_TRUSS: f32 = 0.0;
pub const CODE_FRAME: f32 = 1.0;
pub const CODE_SUPPORT: f32 = 2.0;
pub const CODE_LOAD: f32 = 3.0;
pub const CODE_SELECTED: f32 = 4.0;

fn sub(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: [f32; 3], b: [f32; 3]) -> [f32; 3] {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}
fn unit(a: [f32; 3]) -> [f32; 3] {
    let n = (a[0] * a[0] + a[1] * a[1] + a[2] * a[2]).sqrt().max(1e-20);
    [a[0] / n, a[1] / n, a[2] / n]
}

pub(crate) struct Builder {
    pub(crate) positions: Vec<[f32; 3]>,
    pub(crate) triangles: Vec<u32>,
    /// Source node index per vertex (displacement lookup), `None` for decoration.
    owner: Vec<Option<usize>>,
    pub(crate) value: Vec<f32>,
}
impl Builder {
    pub(crate) fn new() -> Self {
        Self { positions: vec![], triangles: vec![], owner: vec![], value: vec![] }
    }
    /// A static scene coloured by the state codes (`CODE_*`), no displacements.
    pub(crate) fn into_scene(self, label: String) -> Result<Scene, String> {
        let scene = Scene {
            label,
            samples: self.value.iter().map(|v| [0.0, 0.0, 0.0, *v]).collect(),
            positions: self.positions,
            triangles: self.triangles,
            times: vec![0.0],
            range: [CODE_TRUSS, CODE_SELECTED],
            gain: 1.0,
            harmonic_period: None,
            field_label: "entity state (see color key)".into(),
            fields: Vec::new(),
        };
        scene.validate()?;
        Ok(scene)
    }
    /// Square tube from `a` to `b` (scene space) with half-width `w`; vertices are tagged with their end node.
    pub(crate) fn tube(&mut self, a: [f32; 3], b: [f32; 3], w: f32, owners: [Option<usize>; 2], value: f32) {
        let dir = unit(sub(b, a));
        let helper = if dir[1].abs() < 0.9 { [0.0, 1.0, 0.0] } else { [1.0, 0.0, 0.0] };
        let u = unit(cross(dir, helper));
        let v = cross(dir, u);
        let base = self.positions.len() as u32;
        for (end, p) in [a, b].into_iter().enumerate() {
            for (su, sv) in [(-1.0, -1.0), (1.0, -1.0), (1.0, 1.0), (-1.0, 1.0)] {
                self.positions.push(std::array::from_fn(|k| p[k] + w * (su * u[k] + sv * v[k])));
                self.owner.push(owners[end]);
                self.value.push(value);
            }
        }
        for i in 0..4u32 {
            let j = (i + 1) % 4;
            self.triangles.extend([base + i, base + j, base + 4 + j, base + i, base + 4 + j, base + 4 + i]);
        }
        self.triangles.extend([base, base + 2, base + 1, base, base + 3, base + 2, base + 4, base + 5, base + 6, base + 4, base + 6, base + 7]);
    }
    pub(crate) fn cube(&mut self, c: [f32; 3], h: f32, owner: Option<usize>, value: f32) {
        let base = self.positions.len() as u32;
        for i in 0..8u32 {
            self.positions.push([c[0] + if i & 1 == 0 { -h } else { h }, c[1] + if i & 2 == 0 { -h } else { h }, c[2] + if i & 4 == 0 { -h } else { h }]);
            self.owner.push(owner);
            self.value.push(value);
        }
        for f in [[0, 1, 3, 2], [4, 6, 7, 5], [0, 4, 5, 1], [2, 3, 7, 6], [0, 2, 6, 4], [1, 5, 7, 3]] {
            self.triangles.extend([base + f[0], base + f[1], base + f[2], base + f[0], base + f[2], base + f[3]]);
        }
    }
}

/// Rendering options for the edit view.
pub struct EditView<'a> {
    pub frame: Frame,
    pub selected: Option<u32>,
    /// Further selected node / member ids (ids are unique across both).
    pub group: &'a [u32],
    pub solved: Option<&'a MemberSolved>,
    pub show_supports: bool,
    pub show_loads: bool,
}

/// Vertices in the scene before the members: three origin-axis tubes of 8 vertices.
const TRIAD_VERTICES: usize = 24;

/// Build the scene. With a solution the primary scalar is member axial stress (end bending moment when
/// no member carries axial stress, e.g. a pure cantilever) and the displacements are included (normalised
/// by the frame span, like every other scene); without one it is the state code (`CODE_*`) and the scene
/// is static. Layout: triad tubes, then 8 vertices per member, then 8 per node.
pub fn build(problem: &MemberProblem, view: &EditView) -> Result<Scene, String> {
    let index: BTreeMap<u32, usize> = problem.nodes.iter().enumerate().map(|(i, n)| (n.id, i)).collect();
    let frame = view.frame;
    let pos = |id: u32| frame.to_scene(problem.node(id).expect("validated problem").pos);
    let mut b = Builder { positions: vec![], triangles: vec![], owner: vec![], value: vec![] };
    let member_w = 0.006;
    let node_h = 0.012;
    let solved = view.solved.filter(|s| s.displacement.len() == problem.nodes.len() && s.members.len() == problem.members.len());
    let o = frame.to_scene([0.0; 3]);
    for axis in 0..3 {
        let mut e = o;
        e[axis] += 0.08;
        b.tube(o, e, 0.002, [None, None], CODE_FRAME);
    }
    for m in &problem.members {
        let selected = view.selected == Some(m.id) || view.group.contains(&m.id);
        let code = if selected { CODE_SELECTED } else if m.kind.is_frame() { CODE_FRAME } else { CODE_TRUSS };
        b.tube(pos(m.nodes[0]), pos(m.nodes[1]), if selected { member_w * 1.6 } else { member_w }, [Some(index[&m.nodes[0]]), Some(index[&m.nodes[1]])], code);
    }
    for (i, n) in problem.nodes.iter().enumerate() {
        let selected = view.selected == Some(n.id) || view.group.contains(&n.id);
        let code = if selected {
            CODE_SELECTED
        } else if view.show_loads && problem.load(n.id).is_some() {
            CODE_LOAD
        } else if view.show_supports && problem.support(n.id).is_some() {
            CODE_SUPPORT
        } else {
            CODE_TRUSS
        };
        b.cube(frame.to_scene(n.pos), if selected { node_h * 1.5 } else { node_h }, Some(i), code);
    }
    let count = b.positions.len();
    let Some(s) = solved else {
        let scene = Scene {
            label: format!("{} | editing", problem.name),
            positions: b.positions,
            triangles: b.triangles,
            samples: (0..count).map(|i| [0.0, 0.0, 0.0, b.value[i]]).collect(),
            times: vec![0.0],
            range: [CODE_TRUSS, CODE_SELECTED],
            gain: 1.0,
            harmonic_period: None,
            field_label: "entity state (see color key)".into(),
            fields: Vec::new(),
        };
        scene.validate()?;
        return Ok(scene);
    };
    // Per-vertex result scalars. Members interpolate end values along their length; nodes average their members.
    let node0 = TRIAD_VERTICES + 8 * problem.members.len();
    let mut axial = vec![0.0f64; count];
    let mut moment = vec![0.0f64; count];
    let mut acc = vec![(0.0f64, 0.0f64, 0u32); problem.nodes.len()];
    for (k, m) in problem.members.iter().enumerate() {
        let r = &s.members[k];
        let ends = [r.end_actions[4].hypot(r.end_actions[5]), r.end_actions[10].hypot(r.end_actions[11])];
        for end in 0..2 {
            for v in 0..4 {
                axial[TRIAD_VERTICES + 8 * k + 4 * end + v] = r.axial_stress;
                moment[TRIAD_VERTICES + 8 * k + 4 * end + v] = ends[end];
            }
            let e = &mut acc[index[&m.nodes[end]]];
            *e = (e.0 + r.axial_stress, e.1 + ends[end], e.2 + 1);
        }
    }
    for (i, (sa, sm, c)) in acc.iter().enumerate() {
        for v in 0..8 {
            axial[node0 + 8 * i + v] = if *c > 0 { sa / *c as f64 } else { 0.0 };
            moment[node0 + 8 * i + v] = if *c > 0 { sm / *c as f64 } else { 0.0 };
        }
    }
    let structure = |a: &[f64]| a.iter().zip(&b.owner).filter(|(_, o)| o.is_some()).map(|(v, _)| *v).collect::<Vec<_>>();
    let axial_peak = structure(&axial).iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    let moment_peak = structure(&moment).iter().fold(0.0_f64, |m, v| m.max(v.abs()));
    let moment_primary = axial_peak <= 1e-9 * moment_peak;
    let mag: Vec<f64> = (0..count)
        .map(|v| b.owner[v].map_or(0.0, |n| s.displacement[n][..3].iter().map(|x| x * x).sum::<f64>().sqrt()))
        .collect();
    let (primary, others, name, others_names): (&Vec<f64>, [&Vec<f64>; 2], &str, [&str; 2]) = if moment_primary {
        (&moment, [&mag, &axial], "end bending moment (members; nodes average their members)", ["displacement magnitude", "axial stress (members)"])
    } else {
        (&axial, [&mag, &moment], "axial stress (members; nodes average their members)", ["displacement magnitude", "end bending moment (members)"])
    };
    // The colour range covers the structure; the origin triad is painted with the cold end.
    let real = structure(primary);
    let lo = real.iter().copied().fold(f64::INFINITY, f64::min);
    let hi = real.iter().copied().fold(f64::NEG_INFINITY, f64::max);
    let (lo, hi) = if lo <= hi { (lo as f32, hi as f32) } else { (0.0, 0.0) };
    let samples: Vec<[f32; 4]> = (0..count)
        .map(|v| match b.owner[v] {
            Some(n) => [
                (s.displacement[n][0] / frame.span) as f32,
                (s.displacement[n][1] / frame.span) as f32,
                (s.displacement[n][2] / frame.span) as f32,
                primary[v] as f32,
            ],
            None => [0.0, 0.0, 0.0, lo],
        })
        .collect();
    let peak = samples.iter().flat_map(|s| s[..3].iter()).fold(0.0_f32, |a, v| a.max(v.abs()));
    let scene = Scene {
        label: format!("{} | static solve, equilibrium accepted", problem.name),
        positions: b.positions,
        triangles: b.triangles,
        samples,
        times: vec![0.0],
        range: [lo, hi],
        gain: if peak > 0.0 { (0.08 / peak).min(1e12) } else { 1.0 },
        harmonic_period: None,
        field_label: name.into(),
        fields: vec![ScalarField::from_f64(others_names[0], others[0])?, ScalarField::from_f64(others_names[1], others[1])?],
    };
    scene.validate()?;
    Ok(scene)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_problem::members::{templates, Edit};

    #[test]
    fn empty_model_still_builds_a_valid_scene() {
        let p = MemberProblem::new("e");
        let s = build(&p, &EditView { frame: Frame::fit(&p), selected: None, solved: None, show_supports: true, show_loads: true, group: &[] }).unwrap();
        assert_eq!(s.positions.len(), 24, "three origin-axis tubes");
        assert!(s.validate().is_ok());
    }
    #[test]
    fn geometry_counts_and_state_codes() {
        let p = templates().remove(0);
        let apex = p.nodes.last().unwrap().id;
        let s = build(&p, &EditView { frame: Frame::fit(&p), selected: Some(apex), solved: None, show_supports: true, show_loads: true, group: &[] }).unwrap();
        // 3 axis tubes + 3 members (8 vertices each) + 4 node cubes (8 each)
        assert_eq!(s.positions.len(), 3 * 8 + 3 * 8 + 4 * 8);
        assert_eq!(s.triangles.len() / 3, 3 * 12 + 3 * 12 + 4 * 12);
        let v = |i: usize| s.samples[i][3];
        let first_cube = 48;
        assert_eq!(v(first_cube), CODE_SUPPORT, "supported base node");
        assert_eq!(v(first_cube + 3 * 8), CODE_SELECTED, "selected apex");
        assert_eq!(v(24), CODE_TRUSS);
        assert!(s.fields.is_empty() && s.times == vec![0.0]);
    }
    #[test]
    fn loaded_unselected_node_is_coded_as_load() {
        let mut p = templates().remove(0);
        let apex = p.nodes.last().unwrap().id;
        p.apply(Edit::SetSupport { node: apex, fixed: [false; 6] }).unwrap();
        let s = build(&p, &EditView { frame: Frame::fit(&p), selected: None, solved: None, show_supports: true, show_loads: true, group: &[] }).unwrap();
        assert_eq!(s.samples[48 + 3 * 8][3], CODE_LOAD);
    }
    #[test]
    fn solved_scene_carries_stress_displacement_and_fields() {
        let p = templates().remove(0);
        let solved = p.solve().unwrap();
        let frame = Frame::fit(&p);
        let s = build(&p, &EditView { frame, selected: None, solved: Some(&solved), show_supports: true, show_loads: true, group: &[] }).unwrap();
        assert_eq!(s.fields.len(), 2);
        assert!(s.field_label.starts_with("axial stress"));
        // Apex-end vertex of member 0 moves down by the solved displacement / span.
        let apex_idx = p.nodes.len() - 1;
        let v = 24 + 4; // member 0, second end
        let expect = (solved.displacement[apex_idx][2] / frame.span) as f32;
        assert!((s.samples[v][2] - expect).abs() < 1e-9);
        // All three members carry the same compressive stress.
        let stress = solved.members[0].axial_stress as f32;
        assert!(stress < 0.0 && (s.samples[24][3] - stress).abs() < 1e-3);
        assert!(s.gain > 0.0 && s.range[0] <= stress + 1e-3);
        // The origin triad does not stretch the colour range (tripod members are uniformly compressed).
        assert!(s.range[1] < 0.0, "range {:?}", s.range);
        assert!(s.samples[..24].iter().all(|v| v[3] == s.range[0]));
        assert_eq!(s.fields[0].label, "displacement magnitude");
        // Mismatched solution (stale) is ignored rather than mis-indexed.
        let mut other = p.clone();
        other.apply(Edit::AddNode { pos: [1.0, 1.0, 1.0] }).unwrap();
        let stale = build(&other, &EditView { frame, selected: None, solved: Some(&solved), show_supports: true, show_loads: true, group: &[] }).unwrap();
        assert!(stale.fields.is_empty());
    }
    #[test]
    fn bending_only_structure_colours_by_end_moment_with_a_gradient() {
        let p = templates().remove(1);
        let solved = p.solve().unwrap();
        let s = build(&p, &EditView { frame: Frame::fit(&p), selected: None, solved: Some(&solved), show_supports: true, show_loads: true, group: &[] }).unwrap();
        assert!(s.field_label.starts_with("end bending moment"), "{}", s.field_label);
        assert_eq!(s.fields[1].label, "axial stress (members)");
        // Cantilever: moment P L at the root end, 0 at the tip end (linear along the member).
        let (root, tip) = (s.samples[24][3], s.samples[28][3]);
        assert!((root as f64 / (100.0 * 50.0) - 1.0).abs() < 1e-6, "root {root}");
        assert!(tip.abs() < 1e-3, "tip {tip}");
        assert!((s.range[0]..=s.range[1]).contains(&root) && s.range[1] > 0.0);
    }
}
