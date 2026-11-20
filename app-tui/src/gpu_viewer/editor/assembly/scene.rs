//! The drawing of a multi-body assembly: the viewer mesh of `fea_problem::assembly_mesh` as a `Scene`, one primary-scalar code per
//! vertex (the geometry view's state codes 0..3: body, support, load, selected), optionally exploded from the assembly's centre.
//!
//! Vertices are shared inside one face and never across faces, so a highlighted face carries its colour to its own outline only and
//! `Scene::feature_edges` draws face outlines and real creases, not the tessellation.
use crate::gpu_viewer::Scene;
use fea_problem::assembly_mesh::AssemblyMesh;
use solid_kernel::features::FaceKey;
use std::collections::HashMap;

/// State code of a selected face (the colour key's fourth colour).
pub const CODE_SELECTED: f32 = 3.0;
/// How far each body moves from the assembly's centre when exploded, as a fraction of the distance of its centre from it.
pub const EXPLODE: f64 = 0.8;

/// Bounding-box centre of every body (`None` for a body without triangles).
fn centres(mesh: &AssemblyMesh) -> Vec<Option<[f64; 3]>> {
    let mut lo = vec![[f64::INFINITY; 3]; mesh.bodies()];
    let mut hi = vec![[f64::NEG_INFINITY; 3]; mesh.bodies()];
    for (t, key) in mesh.triangles.iter().zip(&mesh.face) {
        for &i in t {
            for k in 0..3 {
                lo[key.body][k] = lo[key.body][k].min(mesh.vertices[i][k]);
                hi[key.body][k] = hi[key.body][k].max(mesh.vertices[i][k]);
            }
        }
    }
    lo.iter().zip(&hi).map(|(l, h)| (l[0] <= h[0]).then(|| std::array::from_fn(|k| 0.5 * (l[k] + h[k])))).collect()
}

/// The shift of every body when exploded: [`EXPLODE`] times the vector from the middle of all bodies' centres to its own.
pub fn explode_offsets(mesh: &AssemblyMesh) -> Vec<[f64; 3]> {
    let c = centres(mesh);
    let present: Vec<[f64; 3]> = c.iter().flatten().copied().collect();
    if present.is_empty() {
        return vec![[0.0; 3]; mesh.bodies()];
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &present {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let mid: [f64; 3] = std::array::from_fn(|k| 0.5 * (lo[k] + hi[k]));
    c.iter().map(|c| c.map_or([0.0; 3], |c| std::array::from_fn(|k| EXPLODE * (c[k] - mid[k])))).collect()
}

/// The scene of `mesh`, centred and scaled to the unit span like every scene. Bodies alternate the body and support colours; the faces
/// `is_selected` accepts take the selected colour.
pub fn scene(mesh: &AssemblyMesh, exploded: bool, is_selected: impl Fn(FaceKey) -> bool, label: String) -> Result<Scene, String> {
    let offsets = if exploded { explode_offsets(mesh) } else { vec![[0.0; 3]; mesh.bodies()] };
    let mut index: HashMap<(FaceKey, usize), u32> = HashMap::new();
    let (mut positions, mut codes, mut triangles) = (Vec::<[f64; 3]>::new(), Vec::<f32>::new(), Vec::<u32>::new());
    for (t, key) in mesh.triangles.iter().zip(&mesh.face) {
        let code = if is_selected(*key) { CODE_SELECTED } else { (key.body % 2) as f32 };
        for &v in t {
            let id = *index.entry((*key, v)).or_insert_with(|| {
                positions.push(std::array::from_fn(|k| mesh.vertices[v][k] + offsets[key.body][k]));
                codes.push(code);
                (positions.len() - 1) as u32
            });
            triangles.push(id);
        }
    }
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in &positions {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    let span = (0..3).map(|k| hi[k] - lo[k]).fold(0.0, f64::max);
    if !span.is_finite() || span <= 0.0 {
        return Err("nothing to draw".into());
    }
    let mid: [f64; 3] = std::array::from_fn(|k| 0.5 * (lo[k] + hi[k]));
    let scene = Scene {
        label,
        positions: positions.iter().map(|p| std::array::from_fn(|k| ((p[k] - mid[k]) / span) as f32)).collect(),
        samples: codes.iter().map(|c| [0.0, 0.0, 0.0, *c]).collect(),
        triangles,
        times: vec![0.0],
        range: [0.0, 3.0],
        gain: 1.0,
        harmonic_period: None,
        field_label: "entity state (see color key)".into(),
        fields: Vec::new(),
    };
    scene.validate()?;
    Ok(scene)
}
