//! OpenCascade-backed modelling (feature `occt`, ADR-019): extrude a sketch profile, boolean union / cut / intersect,
//! fillet, STEP in and out, and the face-tagged tessellation that feeds `SurfaceSolid`.
use crate::{SurfaceMesh, SurfaceSolid, V3};
use cadrum::{BSplineEnd, DVec3, Edge, Solid, Tessellation};
use std::path::Path;

// Compatibility exports; neutral contracts live outside the OCCT adapter.
pub use crate::cad::{Bool, FaceInfo, Loop, Profile};

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
    /// A solid of revolution about the z axis whose profile (in the half plane x >= 0) runs along a B-spline through
    /// `points` (radius, z) and closes with straight lines down to the axis: OpenCASCADE writes the spline's face as a
    /// `SURFACE_OF_REVOLUTION`.
    pub fn lathe_spline(points: &[[f64; 2]]) -> Result<Self, String> {
        if points.len() < 3 || points.iter().any(|p| p[0] <= 0.0) {
            return Err("a spline lathe needs at least three profile points with a positive radius".into());
        }
        let pts: Vec<DVec3> = points.iter().map(|p| DVec3::new(p[0], 0.0, p[1])).collect();
        let (first, last) = (pts[0], *pts.last().unwrap());
        let profile = [
            Edge::bspline(&pts, BSplineEnd::NotAKnot).map_err(err)?,
            Edge::line(last, DVec3::new(0.0, 0.0, last.z)).map_err(err)?,
            Edge::line(DVec3::new(0.0, 0.0, last.z), DVec3::new(0.0, 0.0, first.z)).map_err(err)?,
            Edge::line(DVec3::new(0.0, 0.0, first.z), first).map_err(err)?,
        ];
        let solid = Solid::revolve(&profile, DVec3::ZERO, DVec3::Z, std::f64::consts::TAU).map_err(err)?;
        Ok(Self::one(solid))
    }
    /// A prism: the plane figure bounded by a B-spline through `points` (x, y) and the chord back to its start, extruded
    /// by `depth` along +z. OpenCASCADE writes the spline's wall as a `SURFACE_OF_LINEAR_EXTRUSION`.
    pub fn prism_spline(points: &[[f64; 2]], depth: f64) -> Result<Self, String> {
        if points.len() < 3 || !(depth > 0.0 && depth.is_finite()) {
            return Err("a spline prism needs at least three points and a positive depth".into());
        }
        let pts: Vec<DVec3> = points.iter().map(|p| DVec3::new(p[0], p[1], 0.0)).collect();
        let profile = [
            Edge::bspline(&pts, BSplineEnd::NotAKnot).map_err(err)?,
            Edge::line(*pts.last().unwrap(), pts[0]).map_err(err)?,
        ];
        Ok(Self::one(Solid::extrude(&profile, DVec3::new(0.0, 0.0, depth)).map_err(err)?))
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

    /// A cone or frustum with base radius `r1` at `base`, top radius `r2` after `height` along `axis`.
    pub fn cone(base: V3, r1: f64, r2: f64, axis: V3, height: f64) -> Self {
        let dir = v(axis).normalize_or_zero() * height;
        Self::one(Solid::cone(r1, r2, dir).translate(v(base)))
    }
    pub fn sphere(centre: V3, radius: f64) -> Self {
        Self::one(Solid::sphere(radius).translate(v(centre)))
    }
    /// A torus around `axis` through the origin-translated `centre`: ring radius `major`, tube radius `minor`.
    pub fn torus(centre: V3, major: f64, minor: f64, axis: V3) -> Self {
        Self::one(Solid::torus(major, minor, v(axis)).translate(v(centre)))
    }
    /// Round (`chamfer = false`) or chamfer (`true`) the edges whose end points both lie in the plane `z = height`.
    pub fn round_edges_at_height(&self, height: f64, size: f64, chamfer: bool) -> Result<Self, String> {
        if size.is_nan() || size <= 0.0 {
            return Err("the size must be positive".into());
        }
        let s = self.single()?;
        let at = |p: DVec3| (p.z - height).abs() < 1e-9;
        let picked: Vec<_> = s.iter_edge().filter(|e| at(e.start_point()) && at(e.end_point())).collect();
        if picked.is_empty() {
            return Err(format!("no edge lies in the plane z = {height}"));
        }
        let out = if chamfer { s.chamfer_edges(size, picked) } else { s.fillet_edges(size, picked) };
        Ok(Self::one(out.map_err(err)?))
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
        // A chord across a very small curved feature can coincide with a neighbouring
        // face's edge. Refine only those interior chords against the exact trimmed
        // kernel face; global angular refinement can produce millions of facets.
        let kernel_faces: Vec<_> = self.solids.iter().flat_map(|s| s.iter_face()).collect();
        refine_coincident_chords(&mut out, &kernel_faces);
        Ok(out)
    }

    /// A [`SurfaceSolid`] of this shape, tessellated with chord deflection `deflection` (a small fraction of the element
    /// size you will mesh with).
    pub fn solid(&self, deflection: f64) -> Result<SurfaceSolid, String> {
        SurfaceSolid::new(self.tessellate(deflection)?, deflection * 1e-3)
    }
}

/// Separate coincident interior chords without deleting facets or changing face tags.
/// The closed-manifold check still decides whether the resulting surface is usable.
fn refine_coincident_chords(mesh: &mut SurfaceMesh, faces: &[&cadrum::Face]) {
    use std::collections::{BTreeMap, HashMap};
    let origin = mesh.vertices[0];
    let extent = mesh
        .vertices
        .iter()
        .flat_map(|p| (0..3).map(move |d| (p[d] - origin[d]).abs()))
        .fold(0.0_f64, f64::max);
    let tolerance = (extent * 1e-10).max(1e-10);
    let key =
        |p: V3| std::array::from_fn::<_, 3, _>(|d| ((p[d] - origin[d]) / tolerance).round() as i64);
    for _ in 0..4 {
        let mut edges: BTreeMap<([i64; 3], [i64; 3]), Vec<(usize, usize)>> = BTreeMap::new();
        for (i, t) in mesh.triangles.iter().enumerate() {
            for k in 0..3 {
                let (mut a, mut b) = (key(mesh.vertices[t[k]]), key(mesh.vertices[t[(k + 1) % 3]]));
                if b < a {
                    std::mem::swap(&mut a, &mut b);
                }
                edges.entry((a, b)).or_default().push((i, k));
            }
        }
        let mut split: HashMap<usize, (usize, usize)> = HashMap::new();
        for members in edges.values().filter(|m| m.len() > 2) {
            let mut by_face: BTreeMap<usize, Vec<(usize, usize)>> = BTreeMap::new();
            for &(i, k) in members {
                by_face.entry(mesh.face[i]).or_default().push((i, k));
            }
            for (face, pair) in by_face {
                if pair.len() != 2 || pair.iter().any(|(i, _)| split.contains_key(i)) {
                    continue;
                }
                let (i, k) = pair[0];
                let t = mesh.triangles[i];
                let (a, b) = (mesh.vertices[t[k]], mesh.vertices[t[(k + 1) % 3]]);
                let mid = DVec3::from_array(std::array::from_fn(|d| (a[d] + b[d]) * 0.5));
                let (projected, _) = faces[face].project(mid);
                if !projected.is_finite() || projected.distance(mid) <= 10.0 * tolerance {
                    continue;
                }
                let vertex = mesh.vertices.len();
                mesh.vertices.push(projected.to_array());
                for (i, k) in pair {
                    split.insert(i, (k, vertex));
                }
            }
        }
        if split.is_empty() {
            break;
        }
        let (mut triangles, mut tags) = (Vec::new(), Vec::new());
        for (i, &t) in mesh.triangles.iter().enumerate() {
            if let Some(&(k, m)) = split.get(&i) {
                triangles.extend([
                    [t[k], m, t[(k + 2) % 3]],
                    [m, t[(k + 1) % 3], t[(k + 2) % 3]],
                ]);
                tags.extend([mesh.face[i]; 2]);
            } else {
                triangles.push(t);
                tags.push(mesh.face[i]);
            }
        }
        mesh.triangles = triangles;
        mesh.face = tags;
    }
}
