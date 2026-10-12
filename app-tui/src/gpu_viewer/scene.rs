use fea_core::Mesh;
use serde::{Deserialize, Serialize};

pub(super) const SAMPLE_BUDGET: usize = 4_000_000;
pub(super) const MAX_FIELDS: usize = 16;

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
    /// A compact, mass-normalized modal shape. The shader gain follows sin(2πt/period).
    #[serde(default)]
    pub harmonic_period: Option<f32>,
    /// Name (and meaning) of the primary scalar stored in `samples[..].w`; empty when unknown.
    #[serde(default)]
    pub field_label: String,
    /// Alternative nodal scalars for a static scene (one value per node each), selectable in the viewer.
    #[serde(default)]
    pub fields: Vec<ScalarField>,
}
/// A named nodal scalar with its own colour range; units are the model's consistent units.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct ScalarField {
    pub label: String,
    pub range: [f32; 2],
    pub values: Vec<f32>,
}
impl ScalarField {
    pub fn from_f64(label: &str, values: &[f64]) -> Result<Self, String> {
        if values.iter().any(|v| !v.is_finite()) {
            return Err("nonfinite scalar field".into());
        }
        let v: Vec<f32> = values.iter().map(|x| *x as f32).collect();
        let lo = v.iter().copied().fold(f32::INFINITY, f32::min);
        let hi = v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        Ok(Self {
            label: label.chars().take(200).collect(),
            range: if v.is_empty() { [0.0, 0.0] } else { [lo, hi] },
            values: v,
        })
    }
}
impl Scene {
    pub fn duration(&self) -> f32 {
        self.harmonic_period
            .unwrap_or_else(|| *self.times.last().unwrap_or(&0.0))
    }
    /// Append a sampled state directly in GPU precision; never retain a second full f64 history.
    pub(super) fn append_displacements(
        &mut self,
        u: &[f64],
        dim: usize,
        span: f64,
        time: f32,
    ) -> Result<(), String> {
        let n = self.positions.len();
        if !(2..=3).contains(&dim)
            || u.len() != n * dim
            || !span.is_finite()
            || span <= 0.0
            || !time.is_finite()
            || time <= *self.times.last().ok_or("missing initial frame")?
            || self
                .samples
                .len()
                .checked_add(n)
                .is_none_or(|count| count > SAMPLE_BUDGET)
        {
            return Err("invalid streamed frame or GPU sample budget exceeded".into());
        }
        let samples: Vec<[f32; 4]> = (0..n)
            .map(|i| {
                let mut sample = [0.0; 4];
                for axis in 0..dim {
                    sample[axis] = (u[i * dim + axis] / span) as f32;
                }
                sample[3] = self.samples[i][3];
                sample
            })
            .collect();
        if samples.iter().flatten().any(|x| !x.is_finite()) {
            return Err("nonfinite streamed displacement".into());
        }
        self.samples.extend(samples);
        self.times.push(time);
        Ok(())
    }
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
            || !self.triangles.len().is_multiple_of(3)
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
            || self.field_label.len() > 1024
            || self.fields.len() > MAX_FIELDS
            || n.checked_mul(self.fields.len()).is_none_or(|c| c > SAMPLE_BUDGET)
            || self.fields.iter().any(|f| {
                f.values.len() != n
                    || f.label.len() > 1024
                    || f.values.iter().chain(f.range.iter()).any(|x| !x.is_finite())
                    || f.range[1] < f.range[0]
                    || !(f.range[1] - f.range[0]).is_finite()
            })
            || self
                .harmonic_period
                .is_some_and(|p| !p.is_finite() || p <= 0.0 || self.times.len() != 1)
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
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
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
            harmonic_period: None,
            field_label: String::new(),
            fields: Vec::new(),
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
    pub mesh: std::sync::Arc<Mesh>,
    pub displacement: Vec<f64>,
    pub values: Vec<f64>,
    pub period: Option<f32>,
    pub label: String,
    /// Name of `values`, shown in the native legend.
    pub field_label: String,
    /// Further nodal scalars (static results) the native window can switch to.
    pub fields: Vec<(String, Vec<f64>)>,
}
impl SceneSource {
    /// Static result of a solved problem: `field` is the primary scalar, every other field is selectable in the window.
    pub fn from_solved(s: &fea_problem::Solved, field: fea_problem::Field) -> Self {
        // A field the result does not have (von Mises of a thermal-only study) falls back to the one it opens on.
        let available = s.fields();
        let field = if available.contains(&field) { field } else { s.default_field() };
        Self {
            mesh: std::sync::Arc::new(s.model.mesh.clone()),
            displacement: s.u.clone(),
            values: s.node_values(field),
            period: None,
            label: format!("{} | static {:?}", s.problem.name, field),
            field_label: field.label().into(),
            fields: available.iter().filter(|f| **f != field).map(|f| (f.label().to_string(), s.node_values(*f))).collect(),
        }
    }
    /// One source per computed mode shape (harmonic for natural frequencies, static for buckling).
    pub fn from_dynamic(d: &fea_problem::dynamics::DynSolved) -> Vec<Self> {
        use fea_problem::dynamics::DynSolved;
        let mesh = std::sync::Arc::new(d.model().mesh.clone());
        (0..d.n_modes())
            .map(|mode| {
                let period = match d {
                    DynSolved::Modal(m) => Some(1.0 / m.modes[mode].frequency_hz as f32),
                    _ => None,
                };
                Self {
                    mesh: mesh.clone(),
                    displacement: d.shape(mode).to_vec(),
                    values: d.magnitude(mode),
                    period,
                    field_label: if period.is_some() { "mass-normalized displacement magnitude (arbitrary amplitude)" } else { "buckling displacement magnitude (arbitrary amplitude)" }.into(),
                    fields: Vec::new(),
                    label: format!("{} | {}", d.title(mode), if period.is_some() { "harmonic mode; seconds; mass-normalized magnitude" } else { "static buckling shape" }),
                }
            })
            .collect()
    }
    pub fn build(self) -> Result<Scene, String> {
        if let Some(period) = self.period {
            if !period.is_finite() || period <= 0.0 {
                return Err("invalid modal period".into());
            }
        }
        let mut scene = Scene::from_frames(
            &self.mesh,
            &[self.displacement],
            vec![0.0],
            &self.values,
            self.label,
        )?;
        scene.harmonic_period = self.period;
        scene.field_label = self.field_label.chars().take(200).collect();
        scene.fields = self
            .fields
            .iter()
            .map(|(name, values)| ScalarField::from_f64(name, values))
            .collect::<Result<_, _>>()?;
        scene.validate()?;
        Ok(scene)
    }
}

/// Versioned native result session; carries every already-computed mode for this problem.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ViewerProject {
    pub version: u32,
    pub scenes: Vec<Scene>,
    pub selected: usize,
    pub problem: Option<fea_problem::Problem>,
}
impl ViewerProject {
    pub fn single(scene: Scene) -> Self {
        Self {
            version: 1,
            scenes: vec![scene],
            selected: 0,
            problem: None,
        }
    }
    /// Add another project's scenes after this one's (the one native window holds every result).
    /// Fails, leaving `self` unchanged, when the combined sample budget would be exceeded.
    pub fn append(&mut self, other: ViewerProject) -> Result<(), String> {
        let kept = self.scenes.len();
        self.scenes.extend(other.scenes);
        if let Err(e) = self.validate() {
            self.scenes.truncate(kept);
            return Err(e);
        }
        Ok(())
    }
    pub fn validate(&self) -> Result<(), String> {
        if self.version != 1
            || self.scenes.is_empty()
            || self.scenes.len() > 64
            || self.selected >= self.scenes.len()
            || self
                .scenes
                .iter()
                .try_fold(0usize, |sum, s| {
                sum.checked_add(s.samples.len())?
                    .checked_add(s.positions.len().checked_mul(s.fields.len())?)
            })
            .is_none_or(|n| n > SAMPLE_BUDGET)
        {
            return Err("invalid or oversized native viewer session".into());
        }
        if self
            .scenes
            .iter()
            .try_fold(0usize, |sum, s| sum.checked_add(s.triangles.len()))
            .is_none_or(|n| n > 12_000_000)
        {
            return Err("native session geometry exceeds its aggregate index budget".into());
        }
        for scene in &self.scenes {
            scene.validate()?;
        }
        Ok(())
    }
}

#[derive(Debug)]
pub struct ProjectSource {
    pub sources: Vec<SceneSource>,
    pub selected: usize,
    pub problem: fea_problem::Problem,
}
impl ProjectSource {
    pub fn build(self) -> Result<ViewerProject, String> {
        if self
            .sources
            .iter()
            .try_fold(0usize, |sum, s| sum.checked_add(s.mesh.nodes.len()))
            .is_none_or(|n| n > SAMPLE_BUDGET)
        {
            return Err(
                "computed modes exceed the native session budget; use a coarser mesh".into(),
            );
        }
        let project = ViewerProject {
            version: 1,
            scenes: self
                .sources
                .into_iter()
                .map(SceneSource::build)
                .collect::<Result<_, _>>()?,
            selected: self.selected,
            problem: Some(self.problem),
        };
        project.validate()?;
        Ok(project)
    }
}

#[cfg(test)]
mod session_tests {
    use super::*;
    use fea_core::{generate::grid, Elastic, ElementKind, Physics};
    fn source() -> SceneSource {
        let mesh = grid(
            Physics::PlaneStress { thickness: 1.0 },
            ElementKind::Quad4,
            Elastic::new(1.0, 0.3),
            [1, 1, 1],
            &|p| p,
        )
        .unwrap();
        SceneSource {
            mesh: std::sync::Arc::new(mesh),
            displacement: vec![0.1; 8],
            values: vec![1.0; 4],
            period: Some(2.0),
            label: "mode".into(),
            field_label: "magnitude".into(),
            fields: vec![("other".into(), vec![-2.0, 0.0, 1.0, 3.0])],
        }
    }
    #[test]
    fn appending_projects_keeps_one_window_for_every_result_and_rolls_back_invalid_ones() {
        let mut p = ViewerProject::single(source().build().unwrap());
        let mut more = source().build().unwrap();
        more.label = "second".into();
        p.append(ViewerProject::single(more)).unwrap();
        assert_eq!(p.scenes.len(), 2);
        assert_eq!(p.scenes[1].label, "second");
        let mut broken = source().build().unwrap();
        broken.triangles = vec![0, 1, 99];
        assert!(p.append(ViewerProject::single(broken)).is_err());
        assert_eq!(p.scenes.len(), 2, "a rejected append leaves the project unchanged");
        assert!(p.validate().is_ok());
    }
    #[test]
    fn compact_modal_session_validates_version_selection_and_period() {
        let scene = source().build().unwrap();
        assert_eq!(scene.samples.len(), 4);
        assert_eq!(scene.duration(), 2.0);
        let mut project = ViewerProject::single(scene);
        project.validate().unwrap();
        project.selected = 1;
        assert!(project.validate().is_err());
        project.selected = 0;
        project.version = 2;
        assert!(project.validate().is_err());
        project.version = 1;
        project.scenes[0].harmonic_period = Some(f32::NAN);
        assert!(project.validate().is_err());
    }
    #[test]
    fn streamed_frame_preserves_scalars_and_rejects_invalid_updates_atomically() {
        let mut s = source();
        s.period = None;
        let mut scene = s.build().unwrap();
        scene.append_displacements(&[0.2; 8], 2, 1.0, 1.0).unwrap();
        assert_eq!(scene.samples[4], [0.2, 0.2, 0.0, 1.0]);
        let before = scene.samples.clone();
        assert!(scene
            .append_displacements(&[f64::INFINITY; 8], 2, 1.0, 2.0)
            .is_err());
        assert_eq!(before, scene.samples);
        assert_eq!(scene.times, vec![0.0, 1.0]);
        assert!(scene.append_displacements(&[0.0; 8], 2, 1.0, 0.5).is_err());
    }
}
