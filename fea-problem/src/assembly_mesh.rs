//! Viewer-only surface mesh of a multi-body STEP assembly (epic `docs/epic-assembly-feature-advice.md`, Phase B).
//!
//! A `Problem` holds one body, so this does not go through `Geometry::Solid`: it reads every placed instance with
//! `AnalyticModel::read_all` (the same bodies, in the same root frame and order, that `solid_kernel::features::detect` sees),
//! samples each body's faces and concatenates the triangles. Each triangle is tagged with its body and its STEP face id, which is
//! exactly a `solid_kernel::features::FaceKey`, so an advice item's faces highlight without a number translation. Nothing here is
//! persisted and nothing feeds a solve: the face ids are valid for this file only.
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cylindrical_step::ChartPolicy;
/// Re-exported so a front end names faces without a direct `solid-kernel` dependency.
pub use solid_kernel::features::FaceKey;
use solid_kernel::features::{detect, Features};
use crate::solid::SolidCache;
use solid_kernel::native_surface::{Limits, SampledSurface};
use solid_kernel::SurfaceMesh;
use solid_kernel::step::{inventory, Limits as StepLimits};

/// The sampling chord used when the caller has none (millimetres), as in `native_solid`.
pub const DEFAULT_CHORD: f64 = 0.05;

/// Read every body of `step` and run `f` on them: a strict read first, and a file from another CAD kernel that disagrees with
/// itself by less than a fiftieth of the chord gets the tolerant read (`Options::foreign_cad`, chart canonicalization) on a second
/// try, the order `native_solid::sample` uses for one body. `f` gets the chart policy that goes with the read it is given.
pub(crate) fn with_bodies<T>(step: &str, chord: f64, f: impl Fn(&[AnalyticModel], ChartPolicy) -> Result<T, String>) -> Result<T, String> {
    let data = inventory(step, StepLimits::default()).map_err(|e| e.to_string())?;
    let run = |opt: Options, policy: ChartPolicy| -> Result<T, String> {
        let models = AnalyticModel::read_all(&data, opt).map_err(|e| e.to_string())?;
        f(&models, policy)
    };
    run(Options { chord_mm: chord, ..Options::default() }, ChartPolicy::Strict)
        .or_else(|first| run(Options::foreign_cad(chord), ChartPolicy::CanonicalizeWithinTolerance).map_err(|second| format!("{first}; tolerant read: {second}")))
}

/// The triangles of every body of an assembly, in the root frame, millimetres.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct AssemblyMesh {
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    /// Body (instance order of `read_all`) and STEP face id of each triangle.
    pub face: Vec<FaceKey>,
    /// Triangles per body, in body order; they are stored body by body.
    pub body_triangles: Vec<usize>,
    /// The sampling chord, millimetres.
    pub chord_mm: f64,
}

impl AssemblyMesh {
    pub fn bodies(&self) -> usize {
        self.body_triangles.len()
    }
    /// The triangle indices of the faces named by `keys` (an advice item's `faces()`), for highlighting.
    pub fn triangles_of(&self, keys: &[FaceKey]) -> Vec<usize> {
        let keys: std::collections::HashSet<&FaceKey> = keys.iter().collect();
        self.face.iter().enumerate().filter(|(_, k)| keys.contains(k)).map(|(t, _)| t).collect()
    }
}

/// Every body sampled on its own (face tags local to the body, `source_faces` the STEP face ids).
fn sample_bodies(models: &[AnalyticModel], policy: ChartPolicy) -> Result<Vec<SampledSurface>, String> {
    models.iter().enumerate().map(|(body, model)| {
        let limits = Limits { chart_policy: policy, ..Limits::default() };
        model.sampled_surface(limits, |_| true).map_err(|e| format!("body {}: {e}", body + 1))
    }).collect()
}

fn sample_models(models: &[AnalyticModel], policy: ChartPolicy, chord: f64) -> Result<AssemblyMesh, String> {
    let mut out = AssemblyMesh { chord_mm: chord, ..AssemblyMesh::default() };
    for (body, s) in sample_bodies(models, policy)?.into_iter().enumerate() {
        let base = out.vertices.len();
        out.vertices.extend(s.mesh.vertices.iter().copied());
        out.triangles.extend(s.mesh.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
        for &tag in &s.mesh.face {
            let face = *s.source_faces.get(tag).ok_or_else(|| format!("body {}: triangle tagged with face {tag} of {}", body + 1, s.source_faces.len()))?;
            out.face.push(FaceKey { body, face });
        }
        out.body_triangles.push(s.mesh.triangles.len());
    }
    Ok(out)
}

/// The cache of an assembly of two or more bodies at `chord` mm: the bodies' surfaces side by side in the root frame, face tags
/// numbered on from one body to the next (`SolidCache::bodies` holds each body's count). A file with fewer than two bodies is not an
/// assembly and is refused, so the single-body import keeps its own errors.
pub(crate) fn assembly_cache(step: &str, chord: f64) -> Result<SolidCache, String> {
    let chord = chord_or_default(chord);
    with_bodies(step, chord, |models, policy| {
        if models.len() < 2 {
            return Err("the STEP file holds one body, not an assembly".into());
        }
        let mut mesh = SurfaceMesh { vertices: Vec::new(), triangles: Vec::new(), face: Vec::new() };
        let mut bodies = Vec::new();
        for s in sample_bodies(models, policy)? {
            let (base, offset) = (mesh.vertices.len(), bodies.iter().sum::<usize>());
            mesh.vertices.extend(s.mesh.vertices.iter().copied());
            mesh.triangles.extend(s.mesh.triangles.iter().map(|t| [t[0] + base, t[1] + base, t[2] + base]));
            mesh.face.extend(s.mesh.face.iter().map(|f| f + offset));
            bodies.push(s.source_faces.len());
        }
        let mut cache = SolidCache::new(step, mesh, chord);
        cache.bodies = bodies;
        Ok(cache)
    })
}

fn chord_or_default(chord: f64) -> f64 {
    if chord.is_finite() && chord > 0.0 { chord } else { DEFAULT_CHORD }
}

/// Sample every body of the STEP assembly `step` at `chord` mm (a non-positive or non-finite value means [`DEFAULT_CHORD`]).
pub fn assembly_mesh(step: &str, chord: f64) -> Result<AssemblyMesh, String> {
    let chord = chord_or_default(chord);
    with_bodies(step, chord, |models, policy| sample_models(models, policy, chord))
}

/// The features of an assembly and its viewer mesh, from one read of the bodies.
#[derive(Clone, Debug)]
pub struct AssemblyRead {
    pub features: Features,
    pub mesh: AssemblyMesh,
}

/// [`detect_step`](crate::assembly_advice::detect_step) and [`assembly_mesh`] over one parse and one `read_all`: the features and
/// the mesh come from the same read (strict, else the tolerant one), so they cannot disagree about the bodies. A read that detects
/// but cannot be sampled (or the reverse) falls to the tolerant read for both.
pub fn read_assembly(step: &str, chord: f64) -> Result<AssemblyRead, String> {
    let chord = chord_or_default(chord);
    with_bodies(step, chord, |models, policy| {
        let features = detect(models).map_err(|e| e.to_string())?;
        Ok(AssemblyRead { features, mesh: sample_models(models, policy, chord)? })
    })
}
