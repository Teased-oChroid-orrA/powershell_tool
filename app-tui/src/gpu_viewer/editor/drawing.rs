//! Pictures built from revolved profiles, shared by the workbenches that draw their inputs to scale (Fastener Hole,
//! Pressure Vessel): a three-quarter section about the vertical (y) axis, a quarter cut away so the profile shows on the
//! two cut faces. Surfaces are shared-vertex grids, so only their outlines and real creases read as edges in the window.
use crate::gpu_viewer::{scene::ScalarField, Scene};
use std::f64::consts::PI;

/// Facets around the drawn three quarters.
pub const SEGMENTS: usize = 72;
/// The drawn sweep starts at this angle (the cut quarter is the one before it), so the cut faces the opening (isometric)
/// camera.
const SWEEP_START: f64 = 1.75 * PI;
const SWEEP: f64 = 1.5 * PI;

/// The point at radius `r`, height `z`, angle `a` about the vertical axis.
fn at(r: f64, z: f64, a: f64) -> [f64; 3] {
    [r * a.cos(), z, r * a.sin()]
}
fn angles() -> Vec<f64> {
    (0..=SEGMENTS).map(|k| SWEEP_START + SWEEP * k as f64 / SEGMENTS as f64).collect()
}

/// Positions in model units, triangles, and the primary value of each vertex (a colour code or a contour value).
#[derive(Default)]
pub struct Mesh {
    pub positions: Vec<[f64; 3]>,
    pub triangles: Vec<u32>,
    pub values: Vec<f32>,
}
impl Mesh {
    /// A surface through `rows` of points (each row the same length): neighbouring quads share their vertices, so only
    /// its outline and real creases read as edges. `value` gives each vertex its primary value.
    pub fn grid(&mut self, rows: &[Vec<[f64; 3]>], value: impl Fn(&[f64; 3]) -> f32) {
        let base = self.positions.len() as u32;
        let n = rows[0].len() as u32;
        for row in rows {
            self.positions.extend(row);
            self.values.extend(row.iter().map(&value));
        }
        for i in 0..rows.len() as u32 - 1 {
            for j in 0..n - 1 {
                let (a, b) = (base + i * n + j, base + (i + 1) * n + j);
                self.triangles.extend([a, a + 1, b + 1, a, b + 1, b]);
            }
        }
    }
    /// A surface of revolution through the profile points `line` (`(radius, height)`), swept over the drawn angles.
    pub fn revolve(&mut self, line: &[(f64, f64)], value: impl Fn(&[f64; 3]) -> f32) {
        let rows: Vec<Vec<[f64; 3]>> = angles().into_iter().map(|a| line.iter().map(|&(r, z)| at(r, z, a)).collect()).collect();
        self.grid(&rows, value);
    }
    /// The two cut faces: the grid of profile points `rows` (`(radius, height)`, each row the same length) at both ends of
    /// the sweep.
    pub fn cut(&mut self, rows: &[Vec<(f64, f64)>], value: impl Fn(&[f64; 3]) -> f32) {
        let ends = angles();
        for a in [ends[0], ends[SEGMENTS]] {
            let grid: Vec<Vec<[f64; 3]>> = rows.iter().map(|row| row.iter().map(|&(r, z)| at(r, z, a)).collect()).collect();
            self.grid(&grid, &value);
        }
    }
    /// Centred and scaled to the unit span, like every other scene. `range` is the primary value's colour range, `fields`
    /// the other nodal values the window can switch to (one per vertex each, in `positions` order).
    pub fn into_scene(self, label: String, field_label: &str, range: [f32; 2], fields: Vec<ScalarField>) -> Result<Scene, String> {
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
            samples: self.values.iter().map(|c| [0.0, 0.0, 0.0, *c]).collect(),
            triangles: self.triangles,
            times: vec![0.0],
            range,
            gain: 1.0,
            harmonic_period: None,
            field_label: field_label.into(),
            fields,
        };
        scene.validate()?;
        Ok(scene)
    }
}

/// A scene with nothing visible in it (one degenerate triangle), for a case that cannot be drawn.
pub fn empty(label: String, range: [f32; 2]) -> Scene {
    Scene { label, positions: vec![[0.0; 3]; 3], triangles: vec![0, 1, 2], samples: vec![[0.0; 4]; 3], times: vec![0.0], range, gain: 1.0, harmonic_period: None, field_label: String::new(), fields: Vec::new() }
}
