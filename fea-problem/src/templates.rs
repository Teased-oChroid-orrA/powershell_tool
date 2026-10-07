//! Starting points for the workbench: small, solvable problems of each kind.

use crate::problem::*;

fn steel() -> MaterialSpec {
    MaterialSpec { name: "Steel (generic)".into(), e: 29.0e6, nu: 0.3, alpha: 6.5e-6, yield_stress: Some(50_000.0) }
}

fn sketch(outer: Shape, holes: Vec<Shape>) -> Geometry {
    Geometry::Sketch { outer, holes, depth: 1.0, layers: 4 }
}

fn base(name: &str, analysis: Analysis, geometry: Geometry) -> Problem {
    Problem { version: FORMAT_VERSION, name: name.into(), analysis, thickness: 0.25, material: steel(), geometry, mesh: MeshSpec::default(), supports: Vec::new(), loads: Vec::new(), delta_t: 0.0, bushings: Vec::new() }
}

/// `(title, problem)` pairs; the first is the default.
pub fn templates() -> Vec<(&'static str, Problem)> {
    let mut plate = base("Plate with a hole", Analysis::PlaneStress, sketch(Shape::Rect { x0: 0.0, y0: 0.0, x1: 12.0, y1: 4.0 }, vec![Shape::Circle { cx: 6.0, cy: 2.0, r: 0.5 }]));
    plate.mesh = MeshSpec { size: 0.5, hole_factor: 0.1, ..MeshSpec::default() };
    plate.supports = vec![Support::roller("left", 0), Support::Point { x: 0.0, y: 2.0, z: 0.0, ux: None, uy: Some(0.0), uz: None }];
    plate.loads = vec![Load::Traction { edge: "right".into(), tx: 10_000.0, ty: 0.0, tz: 0.0 }];

    let mut beam = base("Cantilever beam", Analysis::PlaneStress, sketch(Shape::Rect { x0: 0.0, y0: 0.0, x1: 10.0, y1: 1.0 }, vec![]));
    beam.thickness = 1.0;
    beam.mesh = MeshSpec { size: 0.25, ..MeshSpec::default() };
    beam.supports = vec![Support::fixed("left")];
    beam.loads = vec![Load::Force { edge: "right".into(), fx: 0.0, fy: -100.0, fz: 0.0 }];

    let mut cyl = base("Thick cylinder (axisymmetric)", Analysis::Axisymmetric, sketch(Shape::Rect { x0: 1.0, y0: 0.0, x1: 2.0, y1: 1.0 }, vec![]));
    cyl.mesh = MeshSpec { size: 0.2, ..MeshSpec::default() };
    cyl.supports = vec![Support::roller("bottom", 1), Support::roller("top", 1)];
    cyl.loads = vec![Load::Pressure { edge: "left".into(), p: 1000.0 }];

    let mut lug = base("Pin-loaded lug", Analysis::PlaneStress, sketch(Shape::Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 2.0 }, vec![Shape::Circle { cx: 1.0, cy: 1.0, r: 0.5 }]));
    lug.mesh = MeshSpec { size: 0.3, hole_factor: 0.15, ..MeshSpec::default() };
    lug.supports = vec![Support::fixed("right")];
    lug.loads = vec![Load::Bearing { hole: 1, fx: -3000.0, fy: 0.0 }];

    let mut block = base("Extruded plate with a hole (3D)", Analysis::Solid, Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 2.0 }, holes: vec![Shape::Circle { cx: 2.0, cy: 1.0, r: 0.5 }], depth: 0.5, layers: 3 });
    block.mesh = MeshSpec { size: 0.5, hole_factor: 0.4, ..MeshSpec::default() };
    block.supports = vec![Support::fixed("left")];
    block.loads = vec![Load::Traction { edge: "right".into(), tx: 5_000.0, ty: 0.0, tz: 0.0 }];

    let mut bracket = base("L-bracket", Analysis::PlaneStress, sketch(Shape::Polygon { pts: vec![[0.0, 0.0], [6.0, 0.0], [6.0, 2.0], [2.0, 2.0], [2.0, 6.0], [0.0, 6.0]] }, vec![]));
    bracket.mesh = MeshSpec { size: 0.4, adapt_passes: 2, ..MeshSpec::default() };
    bracket.supports = vec![Support::fixed("edge5")];
    bracket.loads = vec![Load::Force { edge: "edge2".into(), fx: 0.0, fy: -500.0, fz: 0.0 }];

    // A lug with an eccentric steel bushing pressed into its hole (interference fit, then the pin load on its bore).
    let mut bushed = base("Lug with an eccentric bushing", Analysis::PlaneStress, sketch(Shape::Rect { x0: 0.0, y0: 0.0, x1: 4.0, y1: 2.0 }, vec![Shape::Circle { cx: 1.0, cy: 1.0, r: 0.5 }]));
    bushed.material = MaterialSpec { name: "Aluminium 7075 (generic)".into(), e: 10.3e6, nu: 0.33, alpha: 12.9e-6, yield_stress: Some(60_000.0) };
    bushed.mesh = MeshSpec { size: 0.3, hole_factor: 0.2, ..MeshSpec::default() };
    bushed.supports = vec![Support::fixed("right")];
    bushed.loads = vec![Load::Bearing { hole: 1, fx: -1500.0, fy: 0.0 }];
    bushed.bushings = vec![Bushing { hole: 1, inner_diameter: 0.75, offset: [0.03, 0.0], interference: 0.002, friction: 0.15, material: steel() }];

    vec![("Plate with a hole", plate), ("Cantilever beam", beam), ("Thick cylinder (axisymmetric)", cyl), ("Pin-loaded lug", lug), ("Lug with an eccentric bushing", bushed), ("Extruded plate with a hole (3D)", block), ("L-bracket (adaptive mesh)", bracket)]
}
