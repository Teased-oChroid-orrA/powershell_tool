//! Problem -> mesh: a sketch meshed by `fea-core`'s unstructured mesher (optionally extruded), or an
//! imported Gmsh / Abaqus mesh.

use crate::problem::{Analysis, Geometry, MeshStrategy, Problem, Shape};
use fea_core::meshing::hybrid::{mesh_hybrid_2d, ElementFamily, HybridOptions2d, Strategy};
use fea_core::adapt::SizeField;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Curve, Loop, Region, Segment};
use fea_core::import::{read_abaqus, read_gmsh, ImportOptions};
use fea_core::mesh2d::mesh_region;
use crate::history::Body;
use fea_core::sweep::{extrude, revolve, sweep_along};
use fea_core::{Mesh, Physics};

/// The mesher's own budget: vertices it may create before it gives up. The default (2 million) lets a sliver feature keep the
/// refinement running for minutes before it says anything; this is about twice what `MAX_ELEMENTS` can use, reached in seconds.
pub const MAX_VERTICES: usize = 250_000;

fn mesh_options() -> MeshOptions {
    MeshOptions { max_vertices: MAX_VERTICES, ..MeshOptions::default() }
}

/// Most elements a sketch is allowed to produce (a guard against a size typo freezing the app).
pub const MAX_ELEMENTS: f64 = 120_000.0;

pub fn physics_of(p: &Problem) -> Physics {
    match p.analysis {
        Analysis::PlaneStress => Physics::PlaneStress { thickness: p.thickness },
        Analysis::PlaneStrain => Physics::PlaneStrain { thickness: p.thickness },
        Analysis::Axisymmetric => Physics::Axisymmetric,
        Analysis::Solid => Physics::Solid,
    }
}

fn seg(curve: Curve, name: &str) -> Segment {
    Segment { curve, name: name.to_string() }
}

fn shape_loop(s: &Shape, rect_names: bool, polygon_names: bool, name: &str) -> Result<Loop, String> {
    match s {
        Shape::Rect { x0, y0, x1, y1 } if rect_names => Loop::rectangle(*x0, *y0, *x1, *y1),
        Shape::Rect { x0, y0, x1, y1 } => Loop::polygon(&[[*x0, *y0], [*x1, *y0], [*x1, *y1], [*x0, *y1]], name),
        Shape::Circle { cx, cy, r } => Loop::circle([*cx, *cy], *r, name),
        Shape::Slot { cx, cy, length, width, angle } => {
            let a = angle.to_radians();
            let (u, n) = ([a.cos(), a.sin()], [-a.sin(), a.cos()]);
            let (r, h) = (0.5 * width, 0.5 * (length - width));
            let (c1, c2) = ([cx - h * u[0], cy - h * u[1]], [cx + h * u[0], cy + h * u[1]]);
            let off = |c: [f64; 2], k: f64| [c[0] + k * r * n[0], c[1] + k * r * n[1]];
            let half = std::f64::consts::FRAC_PI_2;
            Loop::new(vec![
                seg(Curve::line(off(c1, -1.0), off(c2, -1.0)), name),
                seg(Curve::arc(c2, r, a - half, a + half), name),
                seg(Curve::line(off(c2, 1.0), off(c1, 1.0)), name),
                seg(Curve::arc(c1, r, a + half, a + 3.0 * half), name),
            ])
        }
        Shape::Polygon { pts } if polygon_names => {
            let names: Vec<String> = (1..=pts.len()).map(|i| format!("edge{i}")).collect();
            Loop::polygon_named(pts, &names.iter().map(String::as_str).collect::<Vec<_>>())
        }
        Shape::Polygon { pts } => Loop::polygon(pts, name),
        Shape::Outline { segs } if polygon_names => Loop::new(segs.iter().enumerate().flat_map(|(i, sg)| sg.curves().into_iter().map(move |c| seg(c, &format!("edge{}", i + 1)))).collect()),
        Shape::Outline { segs } => Loop::new(segs.iter().flat_map(|sg| sg.curves().into_iter().map(|c| seg(c, name))).collect()),
    }
}

/// The meshable region of a sketch problem.
pub fn region_of(p: &Problem) -> Result<Region, String> {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Err("not a sketch problem".into()) };
    let outer_loop = shape_loop(outer, true, true, "outer")?;
    let hole_loops = holes.iter().enumerate().map(|(i, h)| shape_loop(h, false, false, &format!("hole{}", i + 1))).collect::<Result<Vec<_>, _>>()?;
    Region::new(outer_loop, hole_loops, p.material.elastic())
}

/// The meshable regions of the sketch's other regions (islands, separate outlines), in the order of `r2/`, `r3/`, ...
pub fn extra_regions_of(p: &Problem) -> Result<Vec<Region>, String> {
    let Geometry::Sketch { regions, .. } = &p.geometry else { return Ok(Vec::new()) };
    regions
        .iter()
        .map(|r| {
            let outer = shape_loop(&r.outer, true, true, "outer")?;
            let holes = r.holes.iter().enumerate().map(|(i, h)| shape_loop(h, false, false, &format!("hole{}", i + 1))).collect::<Result<Vec<_>, _>>()?;
            Region::new(outer, holes, p.material.elastic())
        })
        .collect()
}

/// Holes (1-based, as their `holeN` names) that do not fit the outline: outside it, crossing it, or overlapping another
/// hole, each with the geometry kernel's reason. Empty for a sound sketch and for any other geometry.
pub fn hole_problems(p: &Problem) -> Vec<(usize, String)> {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Vec::new() };
    let Ok(outer_loop) = shape_loop(outer, true, true, "outer") else { return Vec::new() };
    let loops: Vec<Option<Loop>> = holes.iter().enumerate().map(|(i, h)| shape_loop(h, false, false, &format!("hole{}", i + 1)).ok()).collect();
    let mut out = Vec::new();
    for (i, l) in loops.iter().enumerate() {
        let Some(l) = l else { continue };
        if let Err(e) = Region::new(outer_loop.clone(), vec![l.clone()], p.material.elastic()) {
            out.push((i + 1, e.replace("hole 1", &format!("hole {}", i + 1))));
            continue;
        }
        for (j, o) in loops.iter().enumerate().skip(i + 1) {
            let Some(o) = o else { continue };
            if Region::new(outer_loop.clone(), vec![l.clone(), o.clone()], p.material.elastic()).is_err() && !out.iter().any(|(k, _)| *k == j + 1) {
                out.push((j + 1, format!("hole {} overlaps hole {}", j + 1, i + 1)));
            }
        }
    }
    out
}

/// Size function: `size` away from holes, `size * hole_factor` at a hole edge, blended over two hole
/// reaches.
pub fn size_function(p: &Problem) -> Box<dyn Fn([f64; 2]) -> f64> {
    let (h, f) = (p.mesh.size, p.mesh.hole_factor);
    let holes: Vec<([f64; 2], f64)> = match &p.geometry {
        Geometry::Sketch { holes, .. } => holes.iter().map(|s| (s.centre(), s.reach())).collect(),
        Geometry::Imported { .. } | Geometry::Solid { .. } => Vec::new(),
    };
    let zones = p.mesh.refine.clone();
    Box::new(move |x| {
        let mut k = 1.0f64;
        for (c, r) in &holes {
            let d = ((x[0] - c[0]).hypot(x[1] - c[1]) - r).max(0.0);
            let t = (d / (2.0 * r)).min(1.0);
            k = k.min(f + (1.0 - f) * t);
        }
        h * k.min(zone_factor(&zones, x))
    })
}

/// Size multiplier of the refinement zones at `x` (1 outside them).
fn zone_factor(zones: &[crate::problem::RefineRegion], x: [f64; 2]) -> f64 {
    zones.iter().fold(1.0, |k, z| k.min(z.factor(x)))
}

/// Elements per unit area at element size 1, measured against the mesher (see `estimate_mesh`).
pub fn elements_per_area(quad: bool) -> f64 {
    if quad { 9.5 } else { 3.2 }
}

/// Element-count forecast of a sketch problem's base mesh (before any adaptive pass), from the size function sampled
/// over the region: what the Mesh panel shows before Generate.
#[derive(Debug, Clone, PartialEq)]
pub struct MeshEstimate {
    pub elements: f64,
    /// Elements attributed to each refinement zone (the zone with the smallest factor at each sample).
    pub per_zone: Vec<f64>,
    /// Share of the region where a zone is at its full level, and where any zone changes the size (blend included).
    pub core_share: f64,
    pub touched_share: f64,
}

/// See [`MeshEstimate`]. A 2D profile is counted once even when it is extruded (`layers` multiplies the 3D count).
pub fn estimate_mesh(p: &Problem) -> Result<MeshEstimate, String> {
    let regions: Vec<Region> = std::iter::once(region_of(p)).chain(extra_regions_of(p)?.into_iter().map(Ok)).collect::<Result<_, _>>()?;
    let n = 220usize;
    // Elements per unit area at size h, measured against the mesher over element types, sizes, hole factors and zones
    // (real / estimate stayed within 10% of these): the quad mesh is a recombined triangle mesh, so it is far denser
    // than a square tiling.
    let per_area = elements_per_area(p.mesh.element.is_quad());
    let base = size_function(p);
    let zones = &p.mesh.refine;
    let mut out = MeshEstimate { elements: 0.0, per_zone: vec![0.0; zones.len()], core_share: 0.0, touched_share: 0.0 };
    let (mut cells, mut core, mut touched) = (0.0, 0.0, 0.0);
    for region in &regions {
        let (lo, hi) = region.bounds();
        let (dx, dy) = ((hi[0] - lo[0]) / n as f64, (hi[1] - lo[1]) / n as f64);
        for i in 0..n {
            for j in 0..n {
                let x = [lo[0] + (i as f64 + 0.5) * dx, lo[1] + (j as f64 + 0.5) * dy];
                if !region.contains(x) {
                    continue;
                }
                let h = base(x);
                let e = per_area * dx * dy / (h * h);
                out.elements += e;
                cells += 1.0;
                let best = zones.iter().enumerate().map(|(k, z)| (k, z.factor(x))).filter(|(_, f)| *f < 1.0 - 1e-9).min_by(|a, b| a.1.total_cmp(&b.1));
                if let Some((k, _)) = best {
                    out.per_zone[k] += e;
                    touched += 1.0;
                    if zones.iter().any(|z| z.distance(x) <= z.radius) {
                        core += 1.0;
                    }
                }
            }
        }
    }
    if cells > 0.0 {
        out.core_share = core / cells;
        out.touched_share = touched / cells;
    }
    Ok(out)
}

/// Append the bushings of a plane problem to the plate mesh: each is an annulus (outer circle = its hole, bore
/// possibly offset) meshed with the plate's element type and registered as surfaces `bN/od` (outer) and `bN/id` (bore).
/// The plate's own hole surface is kept as `plate_holeH`, and the name `holeH` is re-pointed at the bore so that a load or
/// support on the hole acts on the bushing.
pub fn add_bushings(p: &Problem, mesh: &mut Mesh, field: Option<&dyn Fn([f64; 2]) -> f64>) -> Result<(), String> {
    let Geometry::Sketch { holes, .. } = &p.geometry else { return Ok(()) };
    for (i, b) in p.bushings.iter().enumerate() {
        let n = i + 1;
        let Some(Shape::Circle { cx, cy, r }) = holes.get(b.hole.wrapping_sub(1)) else { return Err(format!("bushing {n}: hole {} is not a circle", b.hole)) };
        let ri = 0.5 * b.inner_diameter;
        let wall = r - ri - b.offset[0].hypot(b.offset[1]);
        let region = Region::new(Loop::circle([*cx, *cy], *r, "od")?, vec![Loop::circle([cx + b.offset[0], cy + b.offset[1]], ri, "id")?], b.material.elastic())?;
        // Fine enough for the thin side of the wall, never coarser than the plate's own hole refinement. An adaptive pass
        // sizes the bushing from the same field as the plate, so the two sides of the fit ask for the same element size
        // along the interface (a mesh much finer on one side converges worse than a matched pair).
        let cap = wall.max(r / 25.0);
        let h = (p.mesh.size * p.mesh.hole_factor).min(cap);
        let size = |x: [f64; 2]| field.map_or(h, |sf| sf(x).min(cap));
        let bm = mesh_region(&region, physics_of(p), p.mesh.element.kind(), &size, mesh_options())?;
        let plate = mesh.surfaces.get(&format!("hole{}", b.hole)).cloned().ok_or_else(|| format!("bushing {n}: the plate has no surface hole{}", b.hole))?;
        mesh.surfaces.insert(format!("plate_hole{}", b.hole), plate);
        mesh.append(&bm, &format!("b{n}/"))?;
        let bore = mesh.surfaces.get(&format!("b{n}/id")).cloned().ok_or("the bushing has no bore surface")?;
        mesh.surfaces.insert(format!("hole{}", b.hole), bore);
    }
    Ok(())
}

/// Mesh a sketch problem. `field` overrides the size function (an adaptive pass).
pub fn sketch_mesh(p: &Problem, field: Option<&SizeField>) -> Result<Mesh, String> {
    match field {
        Some(f) => sketch_mesh_sized(p, Some(&|x| f.at(x))),
        None => sketch_mesh_sized(p, None),
    }
}

/// One planar region meshed by the strategy of the problem. The bushing annuli are always unstructured (`add_bushings` meshes
/// them directly: they are thin rings around a bore, with no worthwhile regular area).
fn mesh_planar(p: &Problem, region: &Region, physics: Physics, size: &dyn Fn([f64; 2]) -> f64, notes: &mut Vec<String>) -> Result<Mesh, String> {
    match p.mesh.strategy {
        MeshStrategy::Unstructured => mesh_region(region, physics, p.mesh.element.kind(), size, mesh_options()),
        MeshStrategy::Hybrid => {
            let family = if p.mesh.element.is_quadratic() { ElementFamily::Quadratic } else { ElementFamily::Linear };
            let mut opt = HybridOptions2d::new(physics, family);
            opt.mesh = mesh_options();
            let h = mesh_hybrid_2d(region, size, &opt).map_err(|e| e.to_string())?;
            if let Strategy::Unstructured { reason } = &h.report.strategy {
                notes.push(format!("Hybrid meshing was not used (the plain unstructured mesh is shown): {reason}."));
            }
            Ok(h.mesh)
        }
    }
}

/// [`sketch_mesh`] with the size override as a plain function (the adaptive passes of the goal-oriented controller).
pub fn sketch_mesh_sized(p: &Problem, field: Option<&dyn Fn([f64; 2]) -> f64>) -> Result<Mesh, String> {
    sketch_mesh_noted(p, field, &mut Vec::new())
}

/// [`sketch_mesh_sized`] that also reports why a requested strategy was not used (`notes`).
pub fn sketch_mesh_noted(p: &Problem, field: Option<&dyn Fn([f64; 2]) -> f64>, notes: &mut Vec<String>) -> Result<Mesh, String> {
    p.validate_geometry()?;
    let region = region_of(p)?;
    let Geometry::Sketch { depth, layers, .. } = &p.geometry else { return Err("not a sketch problem".into()) };
    let est = estimate_mesh(p)?.elements;
    if est * if p.analysis == Analysis::Solid { *layers as f64 } else { 1.0 } > MAX_ELEMENTS {
        return Err(format!("about {:.0} elements: that is too fine (limit {MAX_ELEMENTS:.0}); increase the mesh size", est));
    }
    let physics_2d = if p.analysis == Analysis::Solid { Physics::PlaneStress { thickness: 1.0 } } else { physics_of(p) };
    let base = size_function(p);
    // An adaptive pass sizes the mesh from the error field, but the user's refinement zones still apply.
    let size = |x: [f64; 2]| match field {
        Some(sf) => sf(x).min(p.mesh.size * zone_factor(&p.mesh.refine, x)),
        None => base(x),
    };
    let mut mesh = mesh_planar(p, &region, physics_2d, &size, notes)?;
    // The sketch's other regions are separate bodies: meshed alone and appended with their own surface names.
    for (k, reg) in extra_regions_of(p)?.iter().enumerate() {
        let m = mesh_planar(p, reg, physics_2d, &size, notes)?;
        mesh.append(&m, &format!("r{}/", k + 2))?;
    }
    add_bushings(p, &mut mesh, field)?;
    if p.analysis == Analysis::Solid {
        let body = p.history.as_ref().map(|h| h.rebuild()).transpose()?;
        if let Some(Body::Sweep { path, layers, origin }) = body.as_ref().map(|b| &b.body) {
            // The profile's own coordinates, measured from where the path starts, ride the path's moving frame.
            let levels = path.levels(*layers);
            let o = *origin;
            return sweep_along(&mesh, &levels, &move |x, s| {
                let f = path.at(s);
                let (a, b) = (x[0] - o[0], x[1] - o[1]);
                std::array::from_fn(|i| f.p[i] + a * f.u[i] + b * f.v[i])
            });
        }
        let mut solid = match body.as_ref().map(|b| &b.body) {
            Some(Body::Revolve { angle_deg, segments }) => {
                let angles: Vec<f64> = (0..=*segments).map(|i| angle_deg.to_radians() * i as f64 / *segments as f64).collect();
                revolve(&mesh, &angles)?
            }
            _ => {
                let levels: Vec<f64> = (0..=*layers).map(|i| depth * i as f64 / *layers as f64).collect();
                extrude(&mesh, &levels)?
            }
        };
        // A sketch on another plane places the part there (names and node sets are unaffected).
        if let Some(b) = body.filter(|b| !b.frame.is_identity()) {
            for n in &mut solid.nodes {
                *n = b.frame.apply(*n);
            }
        }
        return Ok(solid);
    }
    Ok(mesh)
}

/// Read an imported mesh from its text; `path` (the extension) selects Gmsh or Abaqus.
pub fn imported_mesh(p: &Problem, text: &str) -> Result<Mesh, String> {
    let Geometry::Imported { path } = &p.geometry else { return Err("not an imported problem".into()) };
    let opt = ImportOptions::new(physics_of(p), p.material.elastic());
    let lower = path.to_ascii_lowercase();
    let mesh = if lower.ends_with(".msh") {
        read_gmsh(text, &opt)?
    } else if lower.ends_with(".inp") {
        read_abaqus(text, &opt)?
    } else {
        return Err("mesh files are Gmsh (.msh) or Abaqus (.inp)".into());
    };
    let mut mesh = mesh;
    if p.mesh.reorient {
        let r = fea_core::import::reorient_inverted(&mut mesh);
        if r.remaining > 0 {
            return Err(format!("{} element(s) are collapsed or folded and cannot be turned right side out ({} were repaired): remesh them in the program that made the file", r.remaining, r.repaired));
        }
    }
    let solid = matches!(mesh.physics, Physics::Solid);
    if solid != (p.analysis == Analysis::Solid) {
        return Err(if solid { "the file holds a 3D mesh: set the analysis to 3D solid".into() } else { "the file holds a 2D mesh: choose plane stress, plane strain or axisymmetric".into() });
    }
    Ok(mesh)
}

/// Names an imported or generated mesh offers for supports and loads (node sets and surfaces).
pub fn mesh_names(mesh: &Mesh) -> Vec<String> {
    let mut v: Vec<String> = mesh.surfaces.keys().chain(mesh.node_sets.keys()).cloned().collect();
    v.sort();
    v.dedup();
    v
}
