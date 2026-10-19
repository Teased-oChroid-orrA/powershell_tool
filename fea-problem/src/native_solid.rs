//! The solid editor without OpenCascade: read, tessellate and edit a STEP solid with `solid-kernel`'s native pipeline
//! (`AnalyticModel` for the face-tagged surface mesh, `NativeBrep` for the edits). The default build uses this in place of the
//! `occt` functions of `solid`; an `occt` build keeps OpenCascade and these stay available for comparison.
//!
//! Face numbers: the native face order is the order of the face entities in the STEP text, which is also the order of the
//! surface mesh this module produces. A cache made by another kernel numbers faces differently, so an edit finds its flat face by
//! plane (point and normal), never by number.
use crate::solid::{finish_import, flat_face, SolidCache, SolidOp, StepImport};
use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::cylindrical_step::ChartPolicy;
use solid_kernel::native_brep::NativeBrep;
use solid_kernel::native_surface::{Limits as SurfaceLimits, SampledSurface};
use solid_kernel::step::{inventory, Limits};

/// The sampling chord used when the caller has none (millimetres).
const DEFAULT_CHORD: f64 = 0.05;

/// Read and sample `step` at `chord` mm. A strict read is tried first; a file from another CAD kernel that disagrees with itself
/// by less than a fiftieth of the chord gets the tolerant read (`Options::foreign_cad`, chart canonicalization) on a second try.
fn sample(step: &str, chord: f64) -> Result<(AnalyticModel, SampledSurface), String> {
    let data = inventory(step, Limits::default()).map_err(|e| e.to_string())?;
    let attempt = |opt: Options, policy: ChartPolicy| -> Result<(AnalyticModel, SampledSurface), String> {
        let model = AnalyticModel::read(&data, opt).map_err(|e| e.to_string())?;
        let limits = SurfaceLimits { chart_policy: policy, ..SurfaceLimits::default() };
        let surface = model.sampled_surface(limits, |_| true).map_err(|e| e.to_string())?;
        Ok((model, surface))
    };
    attempt(Options { chord_mm: chord, ..Options::default() }, ChartPolicy::Strict)
        .or_else(|first| attempt(Options::foreign_cad(chord), ChartPolicy::CanonicalizeWithinTolerance).map_err(|second| format!("{first}; tolerant read: {second}")))
}

fn chord_of(deflection: f64) -> f64 {
    if deflection.is_finite() && deflection > 0.0 {
        deflection
    } else {
        DEFAULT_CHORD
    }
}

/// Tessellate STEP text natively (chord deflection `deflection` mm; a non-positive value means the default 0.05 mm).
pub fn tessellate_step_native(step: &str, deflection: f64) -> Result<SolidCache, String> {
    let chord = chord_of(deflection);
    let (_, surface) = sample(step, chord)?;
    Ok(SolidCache::new(step, surface.mesh, chord))
}

/// Read a STEP solid natively: its cache at a thousandth of the bounding-box diagonal and the element size the editor starts with.
pub fn import_step_native(step: &str) -> Result<StepImport, String> {
    // A first, coarse read gives the extent.
    let (model, _) = sample(step, DEFAULT_CHORD)?;
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for id in model.body().edges.keys() {
        for p in model.edge_points(*id).unwrap_or(&[]) {
            for k in 0..3 {
                lo[k] = lo[k].min(p[k]);
                hi[k] = hi[k].max(p[k]);
            }
        }
    }
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    if !(diag.is_finite() && diag > 0.0) {
        return Err("the STEP solid has no extent".into());
    }
    let cache = tessellate_step_native(step, diag * 1e-3)?;
    Ok(finish_import(cache, lo, hi, diag))
}

/// Apply `op` natively; returns the new STEP text and its tessellation (the same contract as the OpenCascade `apply_op`).
pub fn apply_op_native(step: &str, cache: &SolidCache, op: &SolidOp) -> Result<(String, SolidCache), String> {
    let brep = NativeBrep::from_step_text(step)?;
    let scale = cache.coordinate_scale;
    let positive = |v: f64, what: &str| if v.is_finite() && v > 0.0 { Ok(()) } else { Err(format!("the {what} must be a positive number")) };
    // The flat face the cache calls `face`, found in the B-rep by its plane and position.
    let flat = |face: usize| -> Result<(usize, [f64; 3]), String> {
        let (mut c, n) = flat_face(cache, face)?;
        for x in &mut c {
            *x /= scale;
        }
        let candidates = brep.planar_faces_through(c, n);
        match candidates.as_slice() {
            [] => Err(format!("face{face} is not a flat face of the solid")),
            [one] => Ok((*one, c)),
            // Coplanar faces (a plate's two halves): the one that holds the centre.
            many => many.iter().copied().find(|f| brep.face_holds(*f, c)).map(|f| (f, c)).ok_or_else(|| format!("the middle of face{face} is not on the face (it has a hole there): the native hole / boss needs a spot on the material")),
        }
    };
    let result = match *op {
        SolidOp::Fillet { radius } => {
            positive(radius, "fillet radius")?;
            brep.fillet_all_convex(radius / scale)?
        }
        SolidOp::Hole { face, diameter } => {
            positive(diameter, "hole diameter")?;
            let (f, c) = flat(face)?;
            brep.drill(f, c, 0.5 * diameter / scale, None)?
        }
        SolidOp::Boss { face, diameter, height } => {
            positive(diameter, "boss diameter")?;
            positive(height, "boss height")?;
            let (f, c) = flat(face)?;
            brep.boss(f, c, 0.5 * diameter / scale, height / scale)?
        }
    };
    let new_step = result.to_step_text();
    let deflection = cache.deflection;
    let (_, surface) = sample(&new_step, chord_of(deflection / scale))?;
    let mut new_cache = SolidCache::new(&new_step, surface.mesh, deflection);
    new_cache.coordinate_scale = scale;
    for v in &mut new_cache.vertices {
        for x in v {
            *x *= scale;
        }
    }
    Ok((new_step, new_cache))
}
