//! The hole drawn to scale at its nominal dimensions: a three-quarter section of a round plate (a quarter is cut away so
//! the hole's profile shows on the two cut faces). Regular: two stacked parts, the upper bored at Hole 1, the lower at
//! Hole 2. Countersunk: one plate with the solved primary countersink over its straight hole. The plate's outer size and
//! thickness are only for the picture (the workbench has no plate inputs); the hole itself is the inputs' nominal shape.
use crate::gpu_viewer::Scene;
use crate::toolboxes::fastener_hole::model::{FastenerHoleModel, HoleType};
use std::f64::consts::PI;

/// Colour codes (the colour key, `key_labels`).
pub const CODE_PART: f32 = 0.0;
pub const CODE_WALL_A: f32 = 1.0;
pub const CODE_WALL_B: f32 = 2.0;
pub const CODE_SECTION: f32 = 3.0;

/// Facets around the drawn three quarters.
const SEGMENTS: usize = 72;
/// The drawn sweep starts at this angle (the cut quarter is the one before it), so the cut faces the opening (isometric)
/// camera.
const SWEEP_START: f64 = 1.75 * PI;
const SWEEP: f64 = 1.5 * PI;

/// What the colour key says for each code, by hole type.
pub fn key_labels(hole: HoleType) -> [&'static str; 4] {
    match hole {
        HoleType::Regular => ["Parts", "Hole 1 wall", "Hole 2 wall", "Cut face"],
        HoleType::Countersunk => ["Plate", "Straight hole", "Countersink", "Cut face"],
    }
}

/// One revolved block: the hole wall from the top face down (`(radius, height)`, height not increasing), the code of
/// each wall segment, and the plate's outer radius.
struct Block<'a> {
    wall: &'a [(f64, f64)],
    codes: &'a [f32],
    outer: f64,
}

#[derive(Default)]
struct Mesh {
    positions: Vec<[f64; 3]>,
    triangles: Vec<u32>,
    codes: Vec<f32>,
}
impl Mesh {
    /// A surface through `rows` of points (each row the same length): neighbouring quads share their vertices, so only
    /// its outline and real creases read as edges.
    fn grid(&mut self, rows: &[Vec<[f64; 3]>], code: f32) {
        let base = self.positions.len() as u32;
        let n = rows[0].len() as u32;
        for row in rows {
            self.positions.extend(row);
            self.codes.extend(std::iter::repeat_n(code, row.len()));
        }
        for i in 0..rows.len() as u32 - 1 {
            for j in 0..n - 1 {
                let (a, b) = (base + i * n + j, base + (i + 1) * n + j);
                self.triangles.extend([a, a + 1, b + 1, a, b + 1, b]);
            }
        }
    }
    fn block(&mut self, b: &Block) {
        let at = |r: f64, z: f64, a: f64| [r * a.cos(), z, r * a.sin()];
        let angles: Vec<f64> = (0..=SEGMENTS).map(|k| SWEEP_START + SWEEP * k as f64 / SEGMENTS as f64).collect();
        // A surface of revolution through the profile points `line`, swept over the drawn angles.
        let revolve = |m: &mut Mesh, line: &[(f64, f64)], code: f32| {
            let rows: Vec<Vec<[f64; 3]>> = angles.iter().map(|&a| line.iter().map(|&(r, z)| at(r, z, a)).collect()).collect();
            m.grid(&rows, code);
        };
        let (top, bottom) = (b.wall[0], b.wall[b.wall.len() - 1]);
        for (w, code) in b.wall.windows(2).zip(b.codes) {
            revolve(self, w, *code);
        }
        revolve(self, &[top, (b.outer, top.1)], CODE_PART);
        revolve(self, &[bottom, (b.outer, bottom.1)], CODE_PART);
        revolve(self, &[(b.outer, top.1), (b.outer, bottom.1)], CODE_PART);
        for a in [angles[0], angles[SEGMENTS]] {
            let rows: Vec<Vec<[f64; 3]>> = b.wall.iter().map(|&(r, z)| vec![at(r, z, a), at(b.outer, z, a)]).collect();
            self.grid(&rows, CODE_SECTION);
        }
    }
    /// Centred and scaled to the unit span, like every other scene.
    fn into_scene(self, label: String) -> Result<Scene, String> {
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in &self.positions {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let span = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        if !span.is_finite() || span <= 0.0 {
            return Err("nothing to draw".into());
        }
        let mid: [f64; 3] = std::array::from_fn(|a| 0.5 * (lo[a] + hi[a]));
        let scene = Scene {
            label,
            positions: self.positions.iter().map(|p| std::array::from_fn(|a| ((p[a] - mid[a]) / span) as f32)).collect(),
            samples: self.codes.iter().map(|c| [0.0, 0.0, 0.0, *c]).collect(),
            triangles: self.triangles,
            times: vec![0.0],
            range: [CODE_PART, CODE_SECTION],
            gain: 1.0,
            harmonic_period: None,
            field_label: "part (see color key)".into(),
            fields: Vec::new(),
        };
        scene.validate()?;
        Ok(scene)
    }
}

/// A scene with nothing visible in it (one degenerate triangle), for a case that cannot be drawn.
pub fn empty(name: &str) -> Scene {
    Scene { label: format!("{name} | nothing to draw"), positions: vec![[0.0; 3]; 3], triangles: vec![0, 1, 2], samples: vec![[0.0; 4]; 3], times: vec![0.0], range: [CODE_PART, CODE_SECTION], gain: 1.0, harmonic_period: None, field_label: String::new(), fields: Vec::new() }
}

/// What is drawn, in inches (for the inspector's note and the tests).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub outer_radius: f64,
    /// Each part's thickness (regular) or the plate's (countersunk).
    pub thickness: f64,
}

/// One part of the picture: its hole wall profile and the colour code of each wall segment.
type Part = (Vec<(f64, f64)>, Vec<f32>);

/// What the picture of `m` is: the hole's wall profile per part and the plate's size, or why there is none (a
/// countersink that does not solve, a hole of no size).
fn plan(m: &FastenerHoleModel) -> Result<(Vec<Part>, Layout), String> {
    match m.hole_type {
        HoleType::Regular => {
            let (r1, r2) = (0.5 * m.regular.hole1.nominal, 0.5 * m.regular.hole2.nominal);
            if !(r1 > 0.0 && r2 > 0.0 && r1.is_finite() && r2.is_finite()) {
                return Err("both hole diameters must be positive".into());
            }
            let big = r1.max(r2);
            let l = Layout { outer_radius: 3.0 * big, thickness: 1.5 * big };
            // A small gap between the parts keeps them readable as two.
            let gap = 0.04 * l.thickness;
            let upper = vec![(r1, 2.0 * l.thickness + gap), (r1, l.thickness + gap)];
            let lower = vec![(r2, l.thickness), (r2, 0.0)];
            Ok((vec![(upper, vec![CODE_WALL_A]), (lower, vec![CODE_WALL_B])], l))
        }
        HoleType::Countersunk => {
            let g = m.countersink_primary.as_ref().map_err(|e| format!("the countersink does not solve: {e}"))?;
            let (big, small, depth) = (0.5 * g.outer_diameter.nominal, 0.5 * g.hole_diameter.nominal, g.depth.nominal);
            if !(small > 0.0 && big > small && depth > 0.0 && big.is_finite() && depth.is_finite()) {
                return Err("the countersink has no size".into());
            }
            let l = Layout { outer_radius: 2.0 * big, thickness: depth + 2.0 * small.max(0.5 * depth) };
            let wall = vec![(big, l.thickness), (small, l.thickness - depth), (small, 0.0)];
            Ok((vec![(wall, vec![CODE_WALL_B, CODE_WALL_A])], l))
        }
    }
}

/// The plate drawn for `m` (the inspector's note), without building the picture.
pub fn layout(m: &FastenerHoleModel) -> Result<Layout, String> {
    plan(m).map(|(_, l)| l)
}

/// The picture of `m`, or why there is none.
pub fn scene(m: &FastenerHoleModel, name: &str) -> Result<(Scene, Layout), String> {
    let (blocks, layout) = plan(m)?;
    let mut mesh = Mesh::default();
    for (wall, codes) in &blocks {
        mesh.block(&Block { wall, codes, outer: layout.outer_radius });
    }
    Ok((mesh.into_scene(format!("{name} | {}", m.hole_type.label()))?, layout))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::fastener_hole::model::NumberPart;
    use crate::toolboxes::fastener_hole::model::NumberTarget;

    fn codes(s: &Scene) -> Vec<f32> {
        let mut c: Vec<f32> = s.samples.iter().map(|v| v[3]).collect();
        c.sort_by(f32::total_cmp);
        c.dedup();
        c
    }

    #[test]
    fn a_countersink_is_drawn_to_its_nominal_shape_with_every_part_coloured() {
        let mut m = FastenerHoleModel { hole_type: HoleType::Countersunk, ..Default::default() };
        m.recompute();
        let (s, l) = scene(&m, "case").unwrap();
        assert_eq!(codes(&s), [CODE_PART, CODE_WALL_A, CODE_WALL_B, CODE_SECTION]);
        // Default: D 0.5, d 0.25, h 0.125 -> plate radius 0.5, thickness 0.125 + 0.25.
        assert_eq!(l, Layout { outer_radius: 0.5, thickness: 0.375 });
        // The scene is the unit span: the plate's diameter (1.0 in) is the largest extent.
        let ext = |a: usize| s.positions.iter().map(|p| p[a]).fold(f32::NEG_INFINITY, f32::max) - s.positions.iter().map(|p| p[a]).fold(f32::INFINITY, f32::min);
        assert!((ext(0) - 1.0).abs() < 1e-6 && (ext(1) - 0.375).abs() < 1e-6, "{} {}", ext(0), ext(1));
        // The sweep includes +x, where the scene's x is the radius from the axis (x is centred: the sweep also reaches -x):
        // the countersink's mouth is D/2, the straight hole d/2.
        let reach = |code: f32| s.positions.iter().zip(&s.samples).filter(|(_, v)| v[3] == code).map(|(p, _)| p[0]).fold(f32::NEG_INFINITY, f32::max);
        assert!((reach(CODE_WALL_B) - 0.25).abs() < 1e-6 && (reach(CODE_WALL_A) - 0.125).abs() < 1e-6, "{} {}", reach(CODE_WALL_B), reach(CODE_WALL_A));
    }

    #[test]
    fn a_regular_case_draws_two_parts_and_a_countersink_that_does_not_solve_draws_nothing() {
        let mut m = FastenerHoleModel::default();
        m.commit_number(NumberTarget::RegularHole2, NumberPart::Nominal, 0.3);
        let (s, l) = scene(&m, "case").unwrap();
        assert_eq!(codes(&s), [CODE_PART, CODE_WALL_A, CODE_WALL_B, CODE_SECTION]);
        assert!((l.outer_radius - 0.45).abs() < 1e-12, "{l:?}");
        m.hole_type = HoleType::Countersunk;
        m.commit_number(NumberTarget::CsOuterDiameter, NumberPart::Nominal, 0.2);
        assert!(scene(&m, "case").unwrap_err().contains("does not solve"));
    }
}
