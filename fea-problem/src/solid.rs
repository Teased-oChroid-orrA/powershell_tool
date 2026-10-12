//! `Geometry::Solid`: a B-rep solid stored as its STEP text plus the cached face-tagged surface mesh the solve needs.
//! The solve path (`solid_mesh`) uses only `solid-kernel`'s surface mesh and volume mesher, so it works in a build without
//! OpenCascade; reading or editing the STEP text (`tessellate_step`) needs the `occt` feature and says so when it is absent.
use crate::problem::{Geometry, Problem};
use fea_core::tetmesh::MeshReport;
use fea_core::Mesh;
use serde::{Deserialize, Serialize};
use solid_kernel::{mesh_volume, SurfaceMesh, SurfaceSolid, VolumeParams};

/// Elements the volume mesh may have (a tetrahedral mesh of a part is far bigger than a sketch mesh, so it has its own cap).
pub const MAX_TETS: f64 = 400_000.0;

/// The tessellation of a STEP solid at a chord deflection, tied to the text it came from by `hash`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SolidCache {
    /// [`text_hash`] of the STEP text the mesh was made from.
    pub hash: String,
    pub deflection: f64,
    pub vertices: Vec<[f64; 3]>,
    pub triangles: Vec<[usize; 3]>,
    /// B-rep face index of each triangle.
    pub face: Vec<usize>,
}

/// FNV-1a 64 of the text as 16 hex digits: stable across Rust versions (unlike `DefaultHasher`) and platforms, which a
/// stored cache key needs. It detects an edited file, it is not a security measure.
pub fn text_hash(text: &str) -> String {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in text.bytes() {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    format!("{h:016x}")
}

impl SolidCache {
    pub fn new(step: &str, mesh: SurfaceMesh, deflection: f64) -> Self {
        Self { hash: text_hash(step), deflection, vertices: mesh.vertices, triangles: mesh.triangles, face: mesh.face }
    }
    /// Whether the cache was made from exactly this STEP text.
    pub fn is_current(&self, step: &str) -> bool {
        self.hash == text_hash(step)
    }
    pub fn surface(&self) -> SurfaceMesh {
        SurfaceMesh { vertices: self.vertices.clone(), triangles: self.triangles.clone(), face: self.face.clone() }
    }
    /// `face1`.. for every B-rep face index that has a triangle, up to the largest.
    pub fn face_names(&self) -> Vec<String> {
        (1..=self.face.iter().copied().max().map_or(0, |m| m + 1)).map(|i| format!("face{i}")).collect()
    }
}

fn cache_of(p: &Problem) -> Result<&SolidCache, String> {
    p.validate_geometry()?;
    match &p.geometry {
        Geometry::Solid { cache: Some(c), .. } => Ok(c),
        _ => Err("not a B-rep solid problem".into()),
    }
}

/// The closed, face-tagged surface of a solid problem (refused with the reason when the cached mesh is open or broken).
pub fn solid_surface(p: &Problem) -> Result<SurfaceSolid, String> {
    let c = cache_of(p)?;
    SurfaceSolid::new(c.surface(), c.deflection * 1e-3)
}

/// Tetrahedral volume mesh of a solid problem at the problem's mesh size (Tet10 for a quadratic element choice). Refinement
/// zones are sketch-plane features and do not apply. Faces are named `face1`.. as `Problem::edge_names` lists them.
pub fn solid_mesh(p: &Problem) -> Result<(Mesh, MeshReport), String> {
    let solid = solid_surface(p)?;
    let h = p.mesh.size;
    let est = solid.volume() / (0.1179 * h * h * h);
    if est > MAX_TETS {
        return Err(format!("about {est:.0} elements: that is too fine (limit {MAX_TETS:.0}); increase the mesh size"));
    }
    let params = VolumeParams { base: 2.0 * h, boundary_size: h, quadratic: p.mesh.element.is_quadratic() };
    mesh_volume(&solid, p.material.elastic(), params, &|_| h)
}

/// Tessellate STEP text into a cache (chord deflection `deflection`). Needs the OpenCascade kernel.
#[cfg(feature = "occt")]
pub fn tessellate_step(step: &str, deflection: f64) -> Result<SolidCache, String> {
    let brep = solid_kernel::occt::Brep::from_step_text(step)?;
    Ok(SolidCache::new(step, brep.tessellate(deflection)?, deflection))
}

#[cfg(not(feature = "occt"))]
pub fn tessellate_step(_step: &str, _deflection: f64) -> Result<SolidCache, String> {
    Err("reading or editing a STEP solid needs the OpenCascade build (cargo feature `occt`); this build can only solve solids whose surface mesh is already stored".into())
}

impl Geometry {
    /// A solid geometry from STEP text, tessellated now (needs the `occt` feature).
    pub fn solid_from_step(step: &str, deflection: f64) -> Result<Geometry, String> {
        let cache = tessellate_step(step, deflection)?;
        Ok(Geometry::Solid { step: step.to_string(), cache: Some(cache) })
    }
}

impl Problem {
    /// Make the problem a B-rep solid: the geometry, a 3D solid analysis and no sketch history (a solid is not built from
    /// one). Supports and loads are kept: they name faces (`faceN`), so the caller decides whether they still apply.
    pub fn set_solid(&mut self, step: &str, cache: SolidCache) {
        self.geometry = Geometry::Solid { step: step.to_string(), cache: Some(cache) };
        self.analysis = crate::problem::Analysis::Solid;
        self.history = None;
    }
}

// ------------------------------------------------------------------------------------------ editing a stored solid

/// An edit of a stored solid. These need the OpenCascade kernel (feature `occt`); without it they say so.
#[derive(Clone, Debug, PartialEq)]
pub enum SolidOp {
    /// Round every edge.
    Fillet { radius: f64 },
    /// A round hole through the part at the middle of flat face `face` (1-based, `faceN`), perpendicular to it.
    Hole { face: usize, diameter: f64 },
    /// A round boss standing on flat face `face`.
    Boss { face: usize, diameter: f64, height: f64 },
}

/// A solid read from STEP: its cache and a sensible element size (a twelfth of the bounding-box diagonal).
#[derive(Clone, Debug)]
pub struct StepImport {
    pub cache: SolidCache,
    pub size: f64,
}

/// Middle and outward normal of a flat face of a cache, from its triangles (`Err`: no such face, or not flat).
pub fn flat_face(c: &SolidCache, face: usize) -> Result<([f64; 3], [f64; 3]), String> {
    let f = face.checked_sub(1).ok_or("faces are numbered from face1")?;
    let (mut sum, mut area, mut normal) = ([0.0; 3], 0.0, [0.0; 3]);
    let mut tri_normals = Vec::new();
    for (t, tf) in c.triangles.iter().zip(&c.face) {
        if *tf != f {
            continue;
        }
        let [a, b, d] = t.map(|i| c.vertices[i]);
        let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [d[0] - a[0], d[1] - a[1], d[2] - a[2]]);
        let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
        let ar = 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
        if ar <= 0.0 {
            continue;
        }
        for k in 0..3 {
            sum[k] += ar * (a[k] + b[k] + d[k]) / 3.0;
            normal[k] += 0.5 * n[k];
        }
        tri_normals.push([n[0] / (2.0 * ar), n[1] / (2.0 * ar), n[2] / (2.0 * ar)]);
        area += ar;
    }
    if tri_normals.is_empty() || area <= 0.0 {
        return Err(format!("the solid has no face{face}"));
    }
    let len = (normal[0] * normal[0] + normal[1] * normal[1] + normal[2] * normal[2]).sqrt();
    let n = [normal[0] / len, normal[1] / len, normal[2] / len];
    if tri_normals.iter().any(|t| t[0] * n[0] + t[1] * n[1] + t[2] * n[2] < 0.999) {
        return Err(format!("face{face} is curved: pick a flat face"));
    }
    // The tessellation's winding is the kernel's: orient the normal outward by the sign of the enclosed volume.
    let mut vol = 0.0;
    for t in &c.triangles {
        let [a, b, d] = t.map(|i| c.vertices[i]);
        vol += a[0] * (b[1] * d[2] - b[2] * d[1]) + a[1] * (b[2] * d[0] - b[0] * d[2]) + a[2] * (b[0] * d[1] - b[1] * d[0]);
    }
    let s = if vol < 0.0 { -1.0 } else { 1.0 };
    Ok(([sum[0] / area, sum[1] / area, sum[2] / area], [s * n[0], s * n[1], s * n[2]]))
}

#[cfg(feature = "occt")]
pub fn import_step(step: &str) -> Result<StepImport, String> {
    let brep = solid_kernel::occt::Brep::from_step_text(step)?;
    let (lo, hi) = brep.bounds();
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    if !(diag.is_finite() && diag > 0.0) {
        return Err("the STEP solid has no extent".into());
    }
    let cache = SolidCache::new(step, brep.tessellate(diag * 1e-3)?, diag * 1e-3);
    Ok(StepImport { cache, size: diag / 12.0 })
}

#[cfg(not(feature = "occt"))]
pub fn import_step(step: &str) -> Result<StepImport, String> {
    tessellate_step(step, 0.0).map(|cache| StepImport { cache, size: 1.0 })
}

/// Apply `op` to the solid (`step`, `cache`); returns the new STEP text and its tessellation. Face numbers change with the
/// shape: supports and loads that name faces must be checked afterwards (`diagnose` finds the ones left dangling).
#[cfg(feature = "occt")]
pub fn apply_op(step: &str, cache: &SolidCache, op: &SolidOp) -> Result<(String, SolidCache), String> {
    use solid_kernel::occt::{Bool, Brep};
    let brep = Brep::from_step_text(step)?;
    let (lo, hi) = brep.bounds();
    let diag = ((hi[0] - lo[0]).powi(2) + (hi[1] - lo[1]).powi(2) + (hi[2] - lo[2]).powi(2)).sqrt();
    let positive = |v: f64, what: &str| if v.is_finite() && v > 0.0 { Ok(()) } else { Err(format!("the {what} must be a positive number")) };
    let result = match *op {
        SolidOp::Fillet { radius } => {
            positive(radius, "fillet radius")?;
            brep.fillet_all(radius)?
        }
        SolidOp::Hole { face, diameter } => {
            positive(diameter, "hole diameter")?;
            let (c, n) = flat_face(cache, face)?;
            // A cutter that passes through the whole part along the inward normal.
            let base = [c[0] + n[0] * diag, c[1] + n[1] * diag, c[2] + n[2] * diag];
            brep.boolean(Bool::Cut, &Brep::cylinder(base, 0.5 * diameter, [-n[0], -n[1], -n[2]], 3.0 * diag))?
        }
        SolidOp::Boss { face, diameter, height } => {
            positive(diameter, "boss diameter")?;
            positive(height, "boss height")?;
            let (c, n) = flat_face(cache, face)?;
            // Start slightly inside the part so the union fuses instead of touching.
            let base = [c[0] - n[0] * 1e-3 * diag, c[1] - n[1] * 1e-3 * diag, c[2] - n[2] * 1e-3 * diag];
            brep.boolean(Bool::Union, &Brep::cylinder(base, 0.5 * diameter, n, height + 1e-3 * diag))?
        }
    };
    let new_step = result.to_step_text()?;
    let deflection = cache.deflection;
    let new_cache = SolidCache::new(&new_step, result.tessellate(deflection)?, deflection);
    Ok((new_step, new_cache))
}

#[cfg(not(feature = "occt"))]
pub fn apply_op(step: &str, _cache: &SolidCache, _op: &SolidOp) -> Result<(String, SolidCache), String> {
    tessellate_step(step, 0.0).map(|c| (step.to_string(), c))
}

impl Problem {
    /// A new problem around a STEP solid (default material, no supports or loads; the caller adds boundary conditions).
    pub fn from_step(name: &str, step: &str) -> Result<Problem, String> {
        let import = import_step(step)?;
        let mut p = crate::templates::blank();
        p.name = name.to_string();
        p.mesh.size = import.size;
        p.set_solid(step, import.cache);
        p.supports.clear();
        p.loads.clear();
        Ok(p)
    }

    /// Apply a solid edit to a solid problem (a new STEP text and cache); boundary conditions are kept as they are.
    pub fn apply_solid_op(&mut self, op: &SolidOp) -> Result<(), String> {
        let Geometry::Solid { step, cache: Some(cache) } = &self.geometry else { return Err("this model is not a STEP solid".into()) };
        if !cache.is_current(step) {
            return Err("the solid's cached surface mesh is out of date".into());
        }
        let (new_step, new_cache) = apply_op(step, cache, op)?;
        self.geometry = Geometry::Solid { step: new_step, cache: Some(new_cache) };
        Ok(())
    }
}
