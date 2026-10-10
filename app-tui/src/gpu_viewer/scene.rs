use fea_core::Mesh;
use serde::{Deserialize, Serialize};

const SAMPLE_BUDGET: usize = 4_000_000;

/// Normalized coordinates; displacement/scalar samples are frame-major and uploaded once.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scene {
    pub label: String,
    pub positions: Vec<[f32; 3]>,
    pub triangles: Vec<u32>,
    pub samples: Vec<[f32; 4]>,
    pub times: Vec<f32>,
    pub range: [f32; 2],
    pub gain: f32,
}
impl Scene {
    pub fn validate(&self) -> Result<(), String> {
        let n = self.positions.len();
        if n == 0
            || n > 500_000
            || self.times.is_empty()
            || self.times.len() > 1001
            || n.checked_mul(self.times.len()) != Some(self.samples.len())
            || self.samples.len() > SAMPLE_BUDGET
            || self.triangles.is_empty()
            || self.triangles.len() > 3_000_000
            || self.triangles.len() % 3 != 0
            || self.triangles.iter().any(|&i| i as usize >= n)
            || self
                .positions
                .iter()
                .flatten()
                .chain(self.samples.iter().flatten())
                .chain(self.times.iter())
                .chain(self.range.iter())
                .any(|x| !x.is_finite())
            || self.times[0] != 0.0
            || self.times.windows(2).any(|t| t[1] <= t[0])
            || !self.gain.is_finite()
            || self.gain < 0.0
            || self.range[1] < self.range[0]
            || !(self.range[1] - self.range[0]).is_finite()
            || self.label.len() > 1024
        {
            return Err("invalid or oversized GPU scene".into());
        }
        Ok(())
    }
    /// Tessellate through corner, midside and face-center nodes; all 3D boundary faces are visible.
    pub fn from_frames(
        mesh: &Mesh,
        frames: &[Vec<f64>],
        times: Vec<f32>,
        values: &[f64],
        label: String,
    ) -> Result<Self, String> {
        let n = mesh.nodes.len();
        let dim = mesh.dim();
        if n == 0
            || n > 500_000
            || frames.is_empty()
            || frames.len() > 1001
            || n.checked_mul(frames.len())
                .is_none_or(|s| s > SAMPLE_BUDGET)
            || frames.len() != times.len()
            || values.len() != n
            || frames.iter().any(|u| u.len() != n * dim)
            || mesh
                .nodes
                .iter()
                .flatten()
                .chain(values.iter())
                .chain(frames.iter().flatten())
                .any(|x| !x.is_finite())
        {
            return Err(
                "invalid frame dimensions, nonfinite values, or GPU memory budget exceeded".into(),
            );
        }
        let mut lo = [f64::INFINITY; 3];
        let mut hi = [f64::NEG_INFINITY; 3];
        for p in &mesh.nodes {
            for a in 0..3 {
                lo[a] = lo[a].min(p[a]);
                hi[a] = hi[a].max(p[a]);
            }
        }
        let span = (0..3).map(|a| hi[a] - lo[a]).fold(0.0, f64::max);
        if !span.is_finite() || span <= 0.0 {
            return Err("invalid geometry extent".into());
        }
        let positions = mesh
            .nodes
            .iter()
            .map(|p| {
                std::array::from_fn(|a| ((p[a] - lo[a] - (hi[a] - lo[a]) / 2.0) / span) as f32)
            })
            .collect();
        let faces: Vec<Vec<usize>> = if dim == 2 {
            mesh.blocks
                .iter()
                .flat_map(|b| b.conn.chunks_exact(b.kind.n_nodes()).map(|c| c.to_vec()))
                .collect()
        } else {
            mesh.boundary_faces().into_iter().map(|f| f.nodes).collect()
        };
        let mut triangles = Vec::new();
        for c in faces {
            if c.iter().any(|&i| i >= n) {
                return Err("invalid mesh face".into());
            }
            let local: &[[usize; 3]] = match c.len() {
                3 => &[[0, 1, 2]],
                4 => &[[0, 1, 2], [0, 2, 3]],
                6 => &[[0, 3, 5], [3, 1, 4], [5, 4, 2], [3, 4, 5]],
                8 => &[
                    [0, 4, 7],
                    [4, 1, 5],
                    [5, 2, 6],
                    [6, 3, 7],
                    [4, 5, 7],
                    [5, 6, 7],
                ],
                9 => &[
                    [0, 4, 8],
                    [4, 1, 8],
                    [1, 5, 8],
                    [5, 2, 8],
                    [2, 6, 8],
                    [6, 3, 8],
                    [3, 7, 8],
                    [7, 0, 8],
                ],
                _ => return Err("unsupported mesh face topology".into()),
            };
            for t in local {
                triangles.extend(t.map(|i| c[i] as u32));
            }
            if triangles.len() > 3_000_000 {
                return Err("GPU triangle budget exceeded".into());
            }
        }
        let mut peak = 0.0_f64;
        let samples = frames
            .iter()
            .flat_map(|u| {
                (0..n)
                    .map(|i| {
                        let mut v = [0.0; 4];
                        for a in 0..dim {
                            v[a] = (u[i * dim + a] / span) as f32;
                            peak = peak.max((u[i * dim + a] / span).abs());
                        }
                        v[3] = values[i] as f32;
                        v
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        let scene = Self {
            label: label.chars().take(200).collect(),
            positions,
            triangles,
            samples,
            times,
            range: [
                values.iter().copied().fold(f64::INFINITY, f64::min) as f32,
                values.iter().copied().fold(f64::NEG_INFINITY, f64::max) as f32,
            ],
            gain: if peak > 0.0 {
                (0.08 / peak).min(1e12) as f32
            } else {
                1.0
            },
        };
        scene.validate()?;
        Ok(scene)
    }
    pub fn frame_at(&self, t: f32) -> (usize, usize, f32) {
        let b = self
            .times
            .partition_point(|x| *x <= t)
            .min(self.times.len() - 1);
        let a = b.saturating_sub(1);
        let blend = if a == b {
            0.0
        } else {
            ((t - self.times[a]) / (self.times[b] - self.times[a])).clamp(0.0, 1.0)
        };
        (a, b, blend)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scene() -> Scene {
        Scene {
            label: "test".into(),
            positions: vec![[0.0; 3]; 3],
            triangles: vec![0, 1, 2],
            samples: vec![[0.0; 4]; 9],
            times: vec![0.0, 0.5, 1.0],
            range: [0.0, 1.0],
            gain: 1.0,
        }
    }
    #[test]
    fn interpolation_uses_physical_time() {
        let s = scene();
        assert_eq!(s.frame_at(0.25), (0, 1, 0.5));
        assert_eq!(s.frame_at(2.0), (1, 2, 1.0));
    }
    #[test]
    fn refuses_invalid_uploads() {
        let mut s = scene();
        assert!(s.validate().is_ok());
        s.triangles[0] = 3;
        assert!(s.validate().is_err());
        s.triangles[0] = 0;
        s.samples[0][0] = f32::NAN;
        assert!(s.validate().is_err());
    }
    #[test]
    fn refuses_nonmonotonic_time() {
        let mut s = scene();
        s.times[1] = 0.0;
        assert!(s.validate().is_err());
    }
}

#[cfg(test)]
mod mesh_tests {
    use super::*;
    use fea_core::{generate::grid, Elastic, ElementKind, Physics};
    #[test]
    fn high_order_surface_keeps_all_nodes_and_frames() {
        for kind in [ElementKind::Tri6, ElementKind::Quad8, ElementKind::Quad9] {
            let mesh = grid(
                Physics::PlaneStress { thickness: 1.0 },
                kind,
                Elastic::new(1.0, 0.3),
                [1, 1, 1],
                &|p| p,
            )
            .unwrap();
            let n = mesh.nodes.len();
            let u = vec![0.0; n * 2];
            let s = Scene::from_frames(
                &mesh,
                &[u.clone(), u],
                vec![0.0, 1.0],
                &vec![0.0; n],
                "test".into(),
            )
            .unwrap();
            assert_eq!(s.positions.len(), n);
            assert_eq!(s.samples.len(), 2 * n);
            for i in 0..n {
                assert!(
                    s.triangles.contains(&(i as u32)),
                    "unused node {i} in {kind:?}"
                );
            }
        }
    }
    #[test]
    fn solids_show_every_boundary_face() {
        let mesh = grid(
            Physics::Solid,
            ElementKind::Hex8,
            Elastic::new(1.0, 0.3),
            [1, 1, 1],
            &|p| p,
        )
        .unwrap();
        let n = mesh.nodes.len();
        let s = Scene::from_frames(
            &mesh,
            &[vec![0.0; 3 * n]],
            vec![0.0],
            &vec![1.0; n],
            "solid".into(),
        )
        .unwrap();
        assert_eq!(s.triangles.len(), 36);
    }
    #[test]
    fn large_origin_is_removed_before_float_conversion() {
        let mesh = grid(
            Physics::PlaneStress { thickness: 1.0 },
            ElementKind::Quad4,
            Elastic::new(1.0, 0.3),
            [1, 1, 1],
            &|p| [1e12 + p[0], 1e12 + p[1], 0.0],
        )
        .unwrap();
        let s = Scene::from_frames(
            &mesh,
            &[vec![0.0; 8]],
            vec![0.0],
            &[1.0; 4],
            "translated".into(),
        )
        .unwrap();
        assert!(s.positions.iter().any(|p| p[0] == -0.5));
        assert!(s.positions.iter().any(|p| p[0] == 0.5));
    }
}

/// Cheap result snapshot sent to a worker; tessellation/frame expansion stays outside the reducer.
#[derive(Debug)]
pub struct SceneSource {
    pub mesh: Mesh,
    pub displacement: Vec<f64>,
    pub values: Vec<f64>,
    pub period: Option<f32>,
    pub label: String,
}
impl SceneSource {
    pub fn build(self) -> Result<Scene, String> {
        let count = if self.period.is_some() { 121 } else { 1 };
        if self
            .mesh
            .n_dofs()
            .checked_mul(count)
            .is_none_or(|n| n > SAMPLE_BUDGET)
        {
            return Err("mode history exceeds the GPU memory budget".into());
        }
        let (frames, times) = if let Some(period) = self.period {
            if !period.is_finite() || period <= 0.0 {
                return Err("invalid modal period".into());
            }
            let frames = (0..=120)
                .map(|i| {
                    let factor = (std::f64::consts::TAU * i as f64 / 120.0).sin();
                    self.displacement.iter().map(|v| v * factor).collect()
                })
                .collect::<Vec<Vec<f64>>>();
            let times = (0..=120).map(|i| i as f32 / 120.0 * period).collect();
            (frames, times)
        } else {
            (vec![self.displacement], vec![0.0])
        };
        Scene::from_frames(&self.mesh, &frames, times, &self.values, self.label)
    }
}
