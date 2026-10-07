//! Problem -> mesh: a sketch meshed by `fea-core`'s unstructured mesher (optionally extruded), or an
//! imported Gmsh / Abaqus mesh.

use crate::problem::{Analysis, Geometry, Problem, Shape};
use fea_core::adapt::SizeField;
use fea_core::delaunay::MeshOptions;
use fea_core::geometry::{Curve, Loop, Region, Segment};
use fea_core::import::{read_abaqus, read_gmsh, ImportOptions};
use fea_core::mesh2d::mesh_region;
use fea_core::sweep::extrude;
use fea_core::{Mesh, Physics};

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
    }
}

/// The meshable region of a sketch problem.
pub fn region_of(p: &Problem) -> Result<Region, String> {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Err("not a sketch problem".into()) };
    let outer_loop = shape_loop(outer, true, true, "outer")?;
    let hole_loops = holes.iter().enumerate().map(|(i, h)| shape_loop(h, false, false, &format!("hole{}", i + 1))).collect::<Result<Vec<_>, _>>()?;
    Region::new(outer_loop, hole_loops, p.material.elastic())
}

/// Size function: `size` away from holes, `size * hole_factor` at a hole edge, blended over two hole
/// reaches.
fn size_function(p: &Problem) -> Box<dyn Fn([f64; 2]) -> f64> {
    let (h, f) = (p.mesh.size, p.mesh.hole_factor);
    let holes: Vec<([f64; 2], f64)> = match &p.geometry {
        Geometry::Sketch { holes, .. } => holes.iter().map(|s| (s.centre(), s.reach())).collect(),
        Geometry::Imported { .. } => Vec::new(),
    };
    Box::new(move |x| {
        let mut k = 1.0f64;
        for (c, r) in &holes {
            let d = ((x[0] - c[0]).hypot(x[1] - c[1]) - r).max(0.0);
            let t = (d / (2.0 * r)).min(1.0);
            k = k.min(f + (1.0 - f) * t);
        }
        h * k
    })
}

/// Append the bushings of a plane problem to the plate mesh: each is an annulus (outer circle = its hole, bore
/// possibly offset) meshed with the plate's element type and registered as surfaces `bN/od` (outer) and `bN/id` (bore).
/// The plate's own hole surface is kept as `plate_holeH`, and the name `holeH` is re-pointed at the bore so that a load or
/// support on the hole acts on the bushing.
pub fn add_bushings(p: &Problem, mesh: &mut Mesh) -> Result<(), String> {
    let Geometry::Sketch { holes, .. } = &p.geometry else { return Ok(()) };
    for (i, b) in p.bushings.iter().enumerate() {
        let n = i + 1;
        let Some(Shape::Circle { cx, cy, r }) = holes.get(b.hole.wrapping_sub(1)) else { return Err(format!("bushing {n}: hole {} is not a circle", b.hole)) };
        let ri = 0.5 * b.inner_diameter;
        let wall = r - ri - b.offset[0].hypot(b.offset[1]);
        let region = Region::new(Loop::circle([*cx, *cy], *r, "od")?, vec![Loop::circle([cx + b.offset[0], cy + b.offset[1]], ri, "id")?], b.material.elastic())?;
        // Fine enough for the thin side of the wall, never coarser than the plate's own hole refinement.
        let h = (p.mesh.size * p.mesh.hole_factor).min(wall.max(r / 25.0));
        let bm = mesh_region(&region, physics_of(p), p.mesh.element.kind(), &move |_| h, MeshOptions::default())?;
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
    p.validate_geometry()?;
    let region = region_of(p)?;
    let Geometry::Sketch { depth, layers, .. } = &p.geometry else { return Err("not a sketch problem".into()) };
    let est = region.area() / (p.mesh.size * p.mesh.size) * if p.mesh.element.is_quad() { 3.0 } else { 2.0 };
    if est * if p.analysis == Analysis::Solid { *layers as f64 } else { 1.0 } > MAX_ELEMENTS {
        return Err(format!("about {:.0} elements: that is too fine (limit {MAX_ELEMENTS:.0}); increase the mesh size", est));
    }
    let physics_2d = if p.analysis == Analysis::Solid { Physics::PlaneStress { thickness: 1.0 } } else { physics_of(p) };
    let base = size_function(p);
    let size = |x: [f64; 2]| match field {
        Some(sf) => sf.at(x),
        None => base(x),
    };
    let mut mesh = mesh_region(&region, physics_2d, p.mesh.element.kind(), &size, MeshOptions::default())?;
    add_bushings(p, &mut mesh)?;
    if p.analysis == Analysis::Solid {
        let levels: Vec<f64> = (0..=*layers).map(|i| depth * i as f64 / *layers as f64).collect();
        return extrude(&mesh, &levels);
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
