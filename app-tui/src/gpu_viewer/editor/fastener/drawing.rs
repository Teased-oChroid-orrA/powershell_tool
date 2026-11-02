//! The hole drawn to scale at its nominal dimensions: a three-quarter section of a round plate (a quarter is cut away so
//! the hole's profile shows on the two cut faces). Regular: two stacked parts, the upper bored at Hole 1, the lower at
//! Hole 2. Countersunk: one plate with the solved primary countersink over its straight hole. The plate's outer size and
//! thickness are only for the picture (the workbench has no plate inputs); the hole itself is the inputs' nominal shape.
use crate::gpu_viewer::{editor::drawing::Mesh, Scene};
use crate::toolboxes::fastener_hole::model::{FastenerHoleModel, HoleType};

/// Colour codes (the colour key, `key_labels`).
pub const CODE_PART: f32 = 0.0;
pub const CODE_WALL_A: f32 = 1.0;
pub const CODE_WALL_B: f32 = 2.0;
pub const CODE_SECTION: f32 = 3.0;

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

fn block(mesh: &mut Mesh, b: &Block) {
    let code = |c: f32| move |_: &[f64; 3]| c;
    let (top, bottom) = (b.wall[0], b.wall[b.wall.len() - 1]);
    for (w, c) in b.wall.windows(2).zip(b.codes) {
        mesh.revolve(w, code(*c));
    }
    mesh.revolve(&[top, (b.outer, top.1)], code(CODE_PART));
    mesh.revolve(&[bottom, (b.outer, bottom.1)], code(CODE_PART));
    mesh.revolve(&[(b.outer, top.1), (b.outer, bottom.1)], code(CODE_PART));
    let rows: Vec<Vec<(f64, f64)>> = b.wall.iter().map(|&(r, z)| vec![(r, z), (b.outer, z)]).collect();
    mesh.cut(&rows, code(CODE_SECTION));
}

/// A scene with nothing visible in it (one degenerate triangle), for a case that cannot be drawn.
pub fn empty(name: &str) -> Scene {
    super::super::drawing::empty(format!("{name} | nothing to draw"), [CODE_PART, CODE_SECTION])
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
        block(&mut mesh, &Block { wall, codes, outer: layout.outer_radius });
    }
    Ok((mesh.into_scene(format!("{name} | {}", m.hole_type.label()), "part (see color key)", [CODE_PART, CODE_SECTION], Vec::new())?, layout))
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
