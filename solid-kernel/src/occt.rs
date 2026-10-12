//! OpenCascade-backed modelling (feature `occt`, ADR-019): extrude a sketch profile, boolean union / cut / intersect,
//! fillet, STEP in and out, and the face-tagged tessellation that feeds `SurfaceSolid`.
use crate::{SurfaceMesh, SurfaceSolid, V3};
use cadrum::{DVec3, Edge, Solid, Tessellation};
use std::path::Path;

/// One closed loop of a sketch profile, in the plane of the profile.
#[derive(Clone, Debug, PartialEq)]
pub enum Loop {
    Polygon(Vec<[f64; 2]>),
    Circle { centre: [f64; 2], radius: f64 },
}

/// An outer loop and the holes cut out of it.
#[derive(Clone, Debug, PartialEq)]
pub struct Profile {
    pub outer: Loop,
    pub holes: Vec<Loop>,
}

/// A face of a solid, in the order the kernel lists them (`SurfaceMesh::face` tags refer to this index). Normals are
/// not asked of the kernel (its normal query throws for a curved face, which aborts the process): use
/// `SurfaceSolid::face_normals`, taken from the tessellation.
#[derive(Clone, Debug, PartialEq)]
pub struct FaceInfo {
    pub index: usize,
    pub area: f64,
    pub centre: V3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Bool {
    Union,
    Cut,
    Intersect,
}

/// A B-rep shape: normally one solid; a STEP file or a cut that splits the part can give several pieces.
pub struct Brep {
    solids: Vec<Solid>,
}

fn err(e: cadrum::Error) -> String {
    e.to_string()
}

fn v(p: V3) -> DVec3 {
    DVec3::from_array(p)
}

/// The edges of a loop in the plane `z`.
fn edges(l: &Loop, z: f64) -> Result<Vec<Edge>, String> {
    match l {
        Loop::Polygon(pts) => {
            if pts.len() < 3 {
                return Err("a polygon loop needs at least three points".into());
            }
            let pts: Vec<DVec3> = pts.iter().map(|p| DVec3::new(p[0], p[1], z)).collect();
            Edge::polygon(&pts).map_err(err)
        }
        Loop::Circle { centre, radius } => {
            if radius.is_nan() || *radius <= 0.0 {
                return Err("a circle loop needs a positive radius".into());
            }
            Ok(vec![Edge::circle(*radius, DVec3::Z).map_err(err)?.translate(DVec3::new(centre[0], centre[1], z))])
        }
    }
}

impl Brep {
    fn one(solid: Solid) -> Self {
        Self { solids: vec![solid] }
    }
    fn single(&self) -> Result<&Solid, String> {
        match self.solids.as_slice() {
            [s] => Ok(s),
            other => Err(format!("this operation needs a single solid but the shape has {} pieces", other.len())),
        }
    }

    /// Extrude `profile` (in the plane `z = z0`) by `depth` along +z. The holes are loops of the same profile: the kernel
    /// takes the first loop as the boundary and the rest as holes.
    pub fn extrude(profile: &Profile, z0: f64, depth: f64) -> Result<Self, String> {
        if !(depth.abs() > 0.0 && depth.is_finite()) {
            return Err("the extrusion depth must be a non-zero number".into());
        }
        let mut loops = edges(&profile.outer, z0)?;
        for hole in &profile.holes {
            loops.extend(edges(hole, z0)?);
        }
        let solid = Solid::extrude(&loops, DVec3::new(0.0, 0.0, depth)).map_err(err)?;
        Ok(Self::one(solid))
    }
    pub fn box_from_corners(lo: V3, hi: V3) -> Self {
        Self::one(Solid::cube(v(lo), v(hi)))
    }
    pub fn cylinder(base: V3, radius: f64, axis: V3, height: f64) -> Self {
        let dir = v(axis).normalize_or_zero() * height;
        Self::one(Solid::cylinder(radius, dir).translate(v(base)))
    }

    /// Union, cut (`self` minus `other`) or intersection. Nothing left is an error (the shapes did not meet, or the
    /// kernel could not combine them); a cut that splits the part gives a shape of several pieces.
    pub fn boolean(&self, op: Bool, other: &Brep) -> Result<Self, String> {
        let (a, b) = (self.single()?, other.single()?);
        let expr = match op {
            Bool::Union => a + b,
            Bool::Cut => a - b,
            Bool::Intersect => a * b,
        };
        let pieces = expr.build_vec().map_err(err)?;
        if pieces.is_empty() {
            return Err(format!("the {op:?} produced nothing"));
        }
        Ok(Self { solids: pieces })
    }

    /// Round every edge with `radius`.
    pub fn fillet_all(&self, radius: f64) -> Result<Self, String> {
        if radius.is_nan() || radius <= 0.0 {
            return Err("a fillet radius must be positive".into());
        }
        let s = self.single()?;
        Ok(Self::one(s.fillet_edges(radius, s.iter_edge()).map_err(err)?))
    }

    pub fn face_count(&self) -> usize {
        self.solids.iter().map(|s| s.iter_face().count()).sum()
    }
    pub fn faces(&self) -> Vec<FaceInfo> {
        self.solids
            .iter()
            .flat_map(|s| s.iter_face())
            .enumerate()
            .map(|(index, f)| FaceInfo { index, area: f.area(), centre: f.center().to_array() })
            .collect()
    }
    pub fn bounds(&self) -> (V3, V3) {
        let mut lo = DVec3::splat(f64::INFINITY);
        let mut hi = DVec3::splat(f64::NEG_INFINITY);
        for s in &self.solids {
            let [a, b] = s.bounding_box();
            lo = lo.min(a);
            hi = hi.max(b);
        }
        (lo.to_array(), hi.to_array())
    }

    pub fn write_step(&self, path: &Path) -> Result<(), String> {
        let mut f = std::fs::File::create(path).map_err(|e| e.to_string())?;
        Solid::write_step(&self.solids, &mut f).map_err(err)
    }
    pub fn read_step(path: &Path) -> Result<Self, String> {
        let mut f = std::fs::File::open(path).map_err(|e| e.to_string())?;
        let solids = Solid::read_step(&mut f).map_err(err)?;
        if solids.is_empty() {
            return Err("the STEP file contains no solid".into());
        }
        Ok(Self { solids })
    }

    /// The STEP text of the shape (what `fea-problem` stores as the source of truth of a solid).
    pub fn to_step_text(&self) -> Result<String, String> {
        let mut buf = Vec::new();
        Solid::write_step(&self.solids, &mut buf).map_err(err)?;
        String::from_utf8(buf).map_err(|e| e.to_string())
    }
    pub fn from_step_text(text: &str) -> Result<Self, String> {
        let solids = Solid::read_step(&mut text.as_bytes()).map_err(err)?;
        if solids.is_empty() {
            return Err("the STEP text contains no solid".into());
        }
        Ok(Self { solids })
    }

    /// Triangulate every face (chord deflection `deflection`) into a face-tagged surface mesh. The kernel tags each
    /// triangle with a face id; the faces are numbered in the order the kernel lists them (`faces()`), which is what
    /// `SurfaceMesh::face` refers to. One tessellation of the whole shape keeps neighbouring faces' edge discretisation
    /// consistent (meshing each face on its own leaves T-junctions). A face with no triangle is an error.
    ///
    /// The kernel's face id is not unique: the two caps of an extrusion share one (observed with cadrum 0.9). Triangles of
    /// such an id are split into their connected pieces (faces never share a vertex index in the kernel's mesh) and each
    /// piece goes to the face whose centre is nearest to the piece's area-weighted centroid.
    pub fn tessellate(&self, deflection: f64) -> Result<SurfaceMesh, String> {
        let faces = self.faces();
        let ids: Vec<u64> = self.solids.iter().flat_map(|s| s.iter_face()).map(|f| f.id()).collect();
        let options = Tessellation { deflection_linear: deflection, relative_linear: false, ..Default::default() };
        let mesh = Solid::mesh(&self.solids, options).map_err(err)?;
        let mut by_id: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
        for (k, id) in ids.iter().enumerate() {
            by_id.entry(*id).or_default().push(k);
        }
        let mut out = SurfaceMesh { vertices: mesh.vertices.iter().map(|p| p.to_array()).collect(), ..Default::default() };
        let tris: Vec<[usize; 3]> = mesh.indices.chunks_exact(3).map(|t| [t[0], t[1], t[2]]).collect();
        // Which faces each triangle can belong to.
        let mut group: std::collections::HashMap<u64, Vec<usize>> = std::collections::HashMap::new();
        for (t, id) in mesh.face_ids.iter().enumerate() {
            if !by_id.contains_key(id) {
                return Err(format!("the tessellation refers to an unknown face {id}"));
            }
            group.entry(*id).or_default().push(t);
        }
        let mut face_of = vec![usize::MAX; tris.len()];
        for (id, members) in &group {
            let candidates = &by_id[id];
            if candidates.len() == 1 {
                members.iter().for_each(|&t| face_of[t] = candidates[0]);
                continue;
            }
            // Connected pieces of the triangles of this id.
            let mut piece: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            let mut parent: Vec<usize> = (0..members.len()).collect();
            fn find(parent: &mut [usize], mut x: usize) -> usize {
                while parent[x] != x {
                    parent[x] = parent[parent[x]];
                    x = parent[x];
                }
                x
            }
            for (m, &t) in members.iter().enumerate() {
                for &vtx in &tris[t] {
                    match piece.get(&vtx) {
                        Some(&other) => {
                            let (a, b) = (find(&mut parent, m), find(&mut parent, other));
                            parent[a] = b;
                        }
                        None => {
                            piece.insert(vtx, m);
                        }
                    }
                }
            }
            let mut centroid: std::collections::BTreeMap<usize, ([f64; 3], f64)> = std::collections::BTreeMap::new();
            for (m, &t) in members.iter().enumerate() {
                let r = find(&mut parent, m);
                let [a, b, c] = tris[t].map(|i| out.vertices[i]);
                let (u, w) = ([b[0] - a[0], b[1] - a[1], b[2] - a[2]], [c[0] - a[0], c[1] - a[1], c[2] - a[2]]);
                let n = [u[1] * w[2] - u[2] * w[1], u[2] * w[0] - u[0] * w[2], u[0] * w[1] - u[1] * w[0]];
                let area = 0.5 * (n[0] * n[0] + n[1] * n[1] + n[2] * n[2]).sqrt();
                let e = centroid.entry(r).or_insert(([0.0; 3], 0.0));
                for d in 0..3 {
                    e.0[d] += area * (a[d] + b[d] + c[d]) / 3.0;
                }
                e.1 += area;
            }
            if centroid.len() != candidates.len() {
                return Err(format!("{} faces share the kernel id {id} but its triangles form {} connected pieces", candidates.len(), centroid.len()));
            }
            // Nearest centre first, each face used once.
            let mut pairs: Vec<(f64, usize, usize)> = Vec::new();
            for (&r, (sum, area)) in &centroid {
                let c = [sum[0] / area, sum[1] / area, sum[2] / area];
                for &k in candidates {
                    let f = faces[k].centre;
                    pairs.push(((c[0] - f[0]).powi(2) + (c[1] - f[1]).powi(2) + (c[2] - f[2]).powi(2), r, k));
                }
            }
            pairs.sort_by(|a, b| a.0.total_cmp(&b.0));
            let (mut used_p, mut used_f): (Vec<usize>, Vec<usize>) = (Vec::new(), Vec::new());
            let mut assign: std::collections::HashMap<usize, usize> = std::collections::HashMap::new();
            for (_, r, k) in pairs {
                if !used_p.contains(&r) && !used_f.contains(&k) {
                    used_p.push(r);
                    used_f.push(k);
                    assign.insert(r, k);
                }
            }
            for (m, &t) in members.iter().enumerate() {
                face_of[t] = assign[&find(&mut parent, m)];
            }
        }
        let mut seen = vec![false; faces.len()];
        for (t, tri) in tris.iter().enumerate() {
            seen[face_of[t]] = true;
            out.triangles.push(*tri);
            out.face.push(face_of[t]);
        }
        if let Some(k) = seen.iter().position(|s| !s) {
            return Err(format!("face {} of {} has no triangle in the tessellation", k + 1, faces.len()));
        }
        Ok(out)
    }

    /// A [`SurfaceSolid`] of this shape, tessellated with chord deflection `deflection` (a small fraction of the element
    /// size you will mesh with).
    pub fn solid(&self, deflection: f64) -> Result<SurfaceSolid, String> {
        SurfaceSolid::new(self.tessellate(deflection)?, deflection * 1e-3)
    }
}
