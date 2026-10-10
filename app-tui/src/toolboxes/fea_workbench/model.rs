//! Rows of the FEA Workbench field list and the edits they make to the `Problem`. Nothing here
//! computes mechanics: it lists, labels, reads and writes `fea_problem::Problem` fields.

use fea_problem::{Analysis, ElementChoice, Geometry, Load, Problem, Shape, Support};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FieldRow {
    Header(&'static str),
    Template,
    Analysis,
    Thickness,
    Material,
    Young,
    Poisson,
    Alpha,
    Density,
    Yield,
    DeltaT,
    Source,
    ImportPath,
    OuterKind,
    Outer(usize),
    PolyPoint(usize, usize),
    PolyAdd,
    PolyRemove,
    HoleKind(usize),
    Hole(usize, usize),
    HoleRemove(usize),
    HoleAdd,
    Depth,
    Layers,
    Element,
    MeshSize,
    HoleFactor,
    AdaptPasses,
    TargetError,
    SupportKind(usize),
    SupportEdge(usize),
    SupportCoord(usize, usize),
    SupportComp(usize, usize),
    SupportRemove(usize),
    SupportAdd,
    LoadKind(usize),
    LoadEdge(usize),
    LoadParam(usize, usize),
    LoadRemove(usize),
    LoadAdd,
    BushingParam(usize, usize),
    BushingRemove(usize),
    BushingAdd,
}

/// How a row is edited.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EditKind {
    /// Typed number.
    Number,
    /// Typed number, or `free` to release the component.
    Component,
    /// Typed text.
    Text,
    /// Space / Enter performs it.
    Action,
}

pub fn edit_kind(row: FieldRow) -> EditKind {
    use FieldRow::*;
    match row {
        Thickness | Young | Poisson | Alpha | Density | Yield | DeltaT | Outer(..) | PolyPoint(..) | Hole(..) | Depth | Layers | MeshSize | HoleFactor | AdaptPasses | TargetError | SupportCoord(..) | LoadParam(..) | BushingParam(..) => EditKind::Number,
        SupportComp(..) => EditKind::Component,
        ImportPath => EditKind::Text,
        _ => EditKind::Action,
    }
}

fn dim(p: &Problem) -> usize {
    if p.analysis == Analysis::Solid {
        3
    } else {
        2
    }
}

pub fn is_sketch(p: &Problem) -> bool {
    matches!(p.geometry, Geometry::Sketch { .. })
}

pub fn field_rows(p: &Problem) -> Vec<FieldRow> {
    use FieldRow::*;
    let d = dim(p);
    let mut r = vec![Header("Problem"), Template, Analysis];
    if matches!(p.analysis, fea_problem::Analysis::PlaneStress | fea_problem::Analysis::PlaneStrain) {
        r.push(Thickness);
    }
    r.extend([Header("Material"), Material, Young, Poisson, Alpha, Density, Yield, DeltaT, Header("Geometry"), Source]);
    match &p.geometry {
        Geometry::Sketch { outer, holes, .. } => {
            r.push(OuterKind);
            match outer {
                Shape::Polygon { pts } => {
                    for i in 0..pts.len() {
                        r.extend([PolyPoint(i, 0), PolyPoint(i, 1)]);
                    }
                    r.push(PolyAdd);
                    if pts.len() > 3 {
                        r.push(PolyRemove);
                    }
                }
                s => r.extend((0..s.params().len()).map(Outer)),
            }
            for (h, s) in holes.iter().enumerate() {
                r.push(HoleKind(h));
                r.extend((0..s.params().len()).map(|i| Hole(h, i)));
                r.push(HoleRemove(h));
            }
            r.push(HoleAdd);
            if d == 3 {
                r.extend([Depth, Layers]);
            }
        }
        Geometry::Imported { .. } => r.push(ImportPath),
    }
    r.push(Header("Mesh"));
    if is_sketch(p) {
        r.push(Element);
    }
    r.push(MeshSize);
    if is_sketch(p) {
        if matches!(&p.geometry, Geometry::Sketch { holes, .. } if !holes.is_empty()) {
            r.push(HoleFactor);
        }
        if d == 2 {
            r.push(AdaptPasses);
            if p.mesh.adapt_passes > 0 {
                r.push(TargetError);
            }
        }
    }
    r.push(Header("Supports"));
    for (s, sup) in p.supports.iter().enumerate() {
        r.push(SupportKind(s));
        match sup {
            Support::Edge { .. } => r.push(SupportEdge(s)),
            Support::Point { .. } => r.extend((0..d).map(|i| SupportCoord(s, i))),
        }
        r.extend((0..d).map(|c| SupportComp(s, c)));
        r.push(SupportRemove(s));
    }
    r.push(SupportAdd);
    r.push(Header("Loads"));
    for (l, load) in p.loads.iter().enumerate() {
        r.push(LoadKind(l));
        if load.edge().is_some() {
            r.push(LoadEdge(l));
        }
        r.extend((0..load.params(d).len()).map(|i| LoadParam(l, i)));
        r.push(LoadRemove(l));
    }
    r.push(LoadAdd);
    // Interference-fit bushings (plane problems with a circular hole).
    let plane = matches!(p.analysis, fea_problem::Analysis::PlaneStress | fea_problem::Analysis::PlaneStrain);
    if plane && matches!(&p.geometry, Geometry::Sketch { holes, .. } if holes.iter().any(|h| matches!(h, Shape::Circle { .. }))) {
        r.push(Header("Bushings"));
        for (b, bu) in p.bushings.iter().enumerate() {
            r.extend((0..bu.params().len()).map(|i| BushingParam(b, i)));
            r.push(BushingRemove(b));
        }
        r.push(BushingAdd);
    }
    r
}

const AXIS: [&str; 3] = ["x", "y", "z"];

pub fn row_label(p: &Problem, row: FieldRow) -> String {
    use FieldRow::*;
    let d = dim(p);
    let shape_param = |s: &Shape, i: usize| s.params().get(i).map_or("", |(l, _)| *l);
    match row {
        Header(t) => t.to_string(),
        Template => "Template".into(),
        Analysis => "Analysis".into(),
        Thickness => "Thickness".into(),
        Material => "Material".into(),
        Young => "Young's Modulus E".into(),
        Poisson => "Poisson's Ratio".into(),
        Alpha => "Thermal Expansion".into(),
        Density => "Mass Density".into(),
        Yield => "Yield Stress".into(),
        DeltaT => "Temperature Change".into(),
        Source => "Geometry From".into(),
        ImportPath => "Mesh File".into(),
        OuterKind => "Outer Shape".into(),
        Outer(i) => match &p.geometry {
            Geometry::Sketch { outer, .. } => format!("  {}", shape_param(outer, i)),
            _ => String::new(),
        },
        PolyPoint(i, a) => format!("  Point {} {}", i + 1, AXIS[a]),
        PolyAdd => "  Add Point".into(),
        PolyRemove => "  Remove Last Point".into(),
        HoleKind(h) => format!("Hole {} Shape", h + 1),
        Hole(h, i) => match &p.geometry {
            Geometry::Sketch { holes, .. } => format!("  {}", shape_param(&holes[h], i)),
            _ => String::new(),
        },
        HoleRemove(h) => format!("  Remove Hole {}", h + 1),
        HoleAdd => "Add Hole".into(),
        Depth => "Extrusion Depth".into(),
        Layers => "Extrusion Layers".into(),
        Element => "Element Type".into(),
        MeshSize => "Element Size".into(),
        HoleFactor => "Size At Holes".into(),
        AdaptPasses => "Adaptive Passes".into(),
        TargetError => "Target Error".into(),
        SupportKind(s) => format!("Support {}", s + 1),
        SupportEdge(_) => "  Edge".into(),
        SupportCoord(_, i) => format!("  Point {}", AXIS[i]),
        SupportComp(_, c) => format!("  Displacement {}", AXIS[c]),
        SupportRemove(s) => format!("  Remove Support {}", s + 1),
        SupportAdd => "Add Support".into(),
        LoadKind(l) => format!("Load {}", l + 1),
        LoadEdge(_) => "  Edge".into(),
        LoadParam(l, i) => format!("  {}", p.loads[l].params(d).get(i).map_or("", |(lab, _)| *lab)),
        LoadRemove(l) => format!("  Remove Load {}", l + 1),
        LoadAdd => "Add Load".into(),
        BushingParam(b, i) => format!("  Bushing {} {}", b + 1, p.bushings.get(b).and_then(|x| x.params().get(i).map(|(l, _)| *l)).unwrap_or("")),
        BushingRemove(b) => format!("  Remove Bushing {}", b + 1),
        BushingAdd => "Add Bushing".into(),
    }
}

/// Formats a number for display and for the edit buffer.
pub fn format_number(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e12 {
        return format!("{v:.0}");
    }
    if v != 0.0 && (v.abs() >= 1e6 || v.abs() < 1e-4) {
        return format!("{v:.4e}");
    }
    let s = format!("{v:.6}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

/// The numeric value of a `Number` or `Component` row.
pub fn number_value(p: &Problem, row: FieldRow) -> Option<f64> {
    use FieldRow::*;
    let d = dim(p);
    Some(match row {
        Thickness => p.thickness,
        Young => p.material.e,
        Poisson => p.material.nu,
        Alpha => p.material.alpha,
        Density => p.material.density,
        Yield => p.material.yield_stress.unwrap_or(0.0),
        DeltaT => p.delta_t,
        Outer(i) => match &p.geometry {
            Geometry::Sketch { outer, .. } => outer.params().get(i)?.1,
            _ => return None,
        },
        PolyPoint(i, a) => match &p.geometry {
            Geometry::Sketch { outer: Shape::Polygon { pts }, .. } => pts.get(i)?[a],
            _ => return None,
        },
        Hole(h, i) => match &p.geometry {
            Geometry::Sketch { holes, .. } => holes.get(h)?.params().get(i)?.1,
            _ => return None,
        },
        Depth => match &p.geometry {
            Geometry::Sketch { depth, .. } => *depth,
            _ => return None,
        },
        Layers => match &p.geometry {
            Geometry::Sketch { layers, .. } => *layers as f64,
            _ => return None,
        },
        MeshSize => p.mesh.size,
        HoleFactor => p.mesh.hole_factor,
        AdaptPasses => p.mesh.adapt_passes as f64,
        TargetError => p.mesh.target_error,
        SupportCoord(s, i) => p.supports.get(s)?.coords().get(i)?.1,
        SupportComp(s, c) => p.supports.get(s)?.comps()[c]?,
        LoadParam(l, i) => p.loads.get(l)?.params(d).get(i)?.1,
        BushingParam(b, i) => p.bushings.get(b)?.params().get(i)?.1,
        _ => return None,
    })
}

/// Writes a typed number into its row (out-of-range values for a row that needs a positive or
/// bounded value are ignored).
pub fn set_number(p: &mut Problem, row: FieldRow, v: f64) {
    use FieldRow::*;
    if !v.is_finite() {
        return;
    }
    let d = dim(p);
    match row {
        Thickness if v > 0.0 => p.thickness = v,
        Young if v > 0.0 => p.material.e = v,
        Poisson if v > -1.0 && v < 0.5 => p.material.nu = v,
        Alpha => p.material.alpha = v,
        Density if v >= 0.0 => p.material.density = v,
        Yield => p.material.yield_stress = (v > 0.0).then_some(v),
        DeltaT => p.delta_t = v,
        Outer(i) => {
            if let Geometry::Sketch { outer, .. } = &mut p.geometry {
                outer.set_param(i, v);
            }
        }
        PolyPoint(i, a) => {
            if let Geometry::Sketch { outer: Shape::Polygon { pts }, .. } = &mut p.geometry {
                if let Some(pt) = pts.get_mut(i) {
                    pt[a] = v;
                }
            }
        }
        Hole(h, i) => {
            if let Geometry::Sketch { holes, .. } = &mut p.geometry {
                if let Some(s) = holes.get_mut(h) {
                    s.set_param(i, v);
                }
            }
        }
        Depth if v > 0.0 => {
            if let Geometry::Sketch { depth, .. } = &mut p.geometry {
                *depth = v;
            }
        }
        Layers if v >= 1.0 => {
            if let Geometry::Sketch { layers, .. } = &mut p.geometry {
                *layers = (v.round() as usize).min(64);
            }
        }
        MeshSize if v > 0.0 => p.mesh.size = v,
        HoleFactor if v > 0.0 && v <= 1.0 => p.mesh.hole_factor = v,
        AdaptPasses if v >= 0.0 => p.mesh.adapt_passes = (v.round() as u8).min(6),
        TargetError if v > 0.0 && v < 1.0 => p.mesh.target_error = v,
        SupportCoord(s, i) => {
            if let Some(sup) = p.supports.get_mut(s) {
                sup.set_coord(i, v);
            }
        }
        SupportComp(s, c) => {
            if let Some(sup) = p.supports.get_mut(s) {
                sup.set_comp(c, Some(v));
            }
        }
        LoadParam(l, i) => {
            if let Some(load) = p.loads.get_mut(l) {
                load.set_param(d, i, v);
            }
        }
        BushingParam(b, i) => {
            if let Some(bu) = p.bushings.get_mut(b) {
                bu.set_param(i, v);
            }
        }
        _ => {}
    }
}

/// Releases a support component (`free`).
pub fn release_component(p: &mut Problem, row: FieldRow) {
    if let FieldRow::SupportComp(s, c) = row {
        if let Some(sup) = p.supports.get_mut(s) {
            sup.set_comp(c, None);
        }
    }
}

/// The value column of a row.
pub fn row_value(p: &Problem, names: &[String], import_len: Option<usize>, row: FieldRow) -> String {
    use FieldRow::*;
    let d = dim(p);
    match row {
        Header(_) => String::new(),
        Template => p.name.clone(),
        Analysis => p.analysis.label().to_string(),
        Material => p.material.name.clone(),
        Yield => p.material.yield_stress.map_or("none".to_string(), format_number),
        Source => if is_sketch(p) { "Sketch" } else { "Imported mesh file" }.to_string(),
        ImportPath => match (&p.geometry, import_len) {
            (Geometry::Imported { path }, Some(_)) if !path.is_empty() => path.clone(),
            (Geometry::Imported { path }, None) if !path.is_empty() => format!("{path}  (not read yet)"),
            _ => "(Enter to type a .msh or .inp path)".to_string(),
        },
        OuterKind => match &p.geometry {
            Geometry::Sketch { outer, .. } => outer.label().to_string(),
            _ => String::new(),
        },
        PolyAdd | PolyRemove | HoleAdd | HoleRemove(_) | SupportAdd | SupportRemove(_) | LoadAdd | LoadRemove(_) | BushingAdd | BushingRemove(_) => "Enter".to_string(),
        HoleKind(h) => match &p.geometry {
            Geometry::Sketch { holes, .. } => match &holes[h] {
                Shape::Polygon { pts } => format!("Polygon ({} points: edit in the problem file)", pts.len()),
                s => s.label().to_string(),
            },
            _ => String::new(),
        },
        Element => p.mesh.element.label().to_string(),
        SupportKind(s) => match p.supports.get(s) {
            Some(Support::Edge { .. }) => "Edge".to_string(),
            Some(Support::Point { .. }) => "Point (nearest node)".to_string(),
            None => String::new(),
        },
        SupportEdge(s) => p.supports.get(s).and_then(|x| x.edge()).map(|e| edge_text(e, names)).unwrap_or_default(),
        LoadKind(l) => p.loads.get(l).map_or(String::new(), |x| x.label().to_string()),
        LoadEdge(l) => p.loads.get(l).and_then(|x| x.edge()).map(|e| edge_text(e, names)).unwrap_or_default(),
        SupportComp(s, c) => match p.supports.get(s).and_then(|x| x.comps()[c]) {
            Some(v) if v == 0.0 => "Fixed (0)".to_string(),
            Some(v) => format!("Prescribed {}", format_number(v)),
            None => "Free".to_string(),
        },
        Layers | AdaptPasses => format!("{}", number_value(p, row).unwrap_or(0.0).round()),
        Thickness | Young | Poisson | Alpha | Density | DeltaT | Outer(..) | PolyPoint(..) | Hole(..) | Depth | MeshSize | HoleFactor | TargetError | SupportCoord(..) | LoadParam(..) | BushingParam(..) => {
            let _ = d;
            number_value(p, row).map_or(String::new(), format_number)
        }
    }
}

fn edge_text(edge: &str, names: &[String]) -> String {
    if names.is_empty() || names.iter().any(|n| n == edge) {
        edge.to_string()
    } else {
        format!("{edge}  (no such edge)")
    }
}

pub fn field_hint(row: FieldRow) -> &'static str {
    use FieldRow::*;
    match row {
        Header(_) => "",
        Template => "Enter / Right: the next starting problem (replaces the current one; j saves it first).",
        Analysis => "Plane stress (thin plate), plane strain (long prismatic body), axisymmetric (x = radius, y = axis; loads are totals over 360 degrees) or a 3D solid (an extruded sketch or an imported volume mesh).",
        Thickness => "Out-of-plane thickness; edge forces and tractions act over this thickness.",
        Material => "Enter opens the material browser: E, Poisson's ratio, thermal expansion and yield come from it.",
        Density => "Mass per volume in consistent units (inch / psi: lbf s^2/in^4 = weight density / 386.09). Natural frequencies (n) and load-release animation (t) require it.",
        Young | Poisson | Alpha | Yield => "Linear isotropic material. Units are yours (inch / psi by convention); the solver is unit-free. Yield only feeds the margin readout (0 = none).",
        DeltaT => "Uniform temperature change from the stress-free state (needs a thermal expansion coefficient).",
        Source => "Sketch: a parametric outline with holes meshed here. Imported: a Gmsh .msh or Abaqus .inp mesh from a file.",
        ImportPath => "Path of a Gmsh (.msh, ASCII 4.1 / 2.2) or Abaqus (.inp) mesh. Its node sets and surfaces become the edge names supports and loads refer to.",
        OuterKind => "Enter changes the outline: rectangle (edges bottom / right / top / left), circle or slot (edge outer), polygon (edges edge1..edgeN).",
        Outer(_) | Hole(..) => "Dimension of the shape (slot angle in degrees from the x axis).",
        PolyPoint(..) => "Polygon vertex; edge i runs from point i to point i+1 and is named edge{i}.",
        PolyAdd => "Splits the closing edge with a new vertex at its midpoint.",
        PolyRemove => "Removes the last vertex.",
        HoleKind(_) => "Enter cycles circle, slot and rectangle holes. A hole's edge is named hole{n}.",
        HoleRemove(_) => "Removes this hole.",
        HoleAdd => "Adds a circular hole near the middle of the outline.",
        Depth | Layers => "3D only: the sketch is extruded along z in this many layers; the end faces are named start (z = 0) and end.",
        Element => "Quadratic elements (Quad8 / Quad9 / Tri6) are the accurate choice; linear ones need a much finer mesh. A 3D extrusion needs a quadrilateral type.",
        MeshSize => "Target element edge length away from holes.",
        HoleFactor => "Element size at a hole edge as a fraction of the size above (blends back over about two hole radii).",
        AdaptPasses => "Remeshing passes driven by the error estimate (0 = off): each pass makes elements smaller where the error is large.",
        TargetError => "Relative energy-norm error the adaptive passes aim for.",
        SupportKind(_) => "Edge: every node of a named edge. Point: the node nearest the given coordinates.",
        SupportEdge(_) | LoadEdge(_) => "Name of the edge (or imported surface / node set). Enter or Right: next; Left: previous.",
        SupportCoord(..) => "Coordinates of the node to support.",
        SupportComp(..) => "Fixed (0), Prescribed (any number) or Free. Space toggles Free / Fixed; type a number to prescribe, or the word free.",
        SupportRemove(_) | LoadRemove(_) => "Removes this item.",
        SupportAdd => "Adds a fully fixed support on the first edge.",
        LoadKind(_) => "Enter cycles pressure, traction, edge force, bearing (pin) load on a hole, point force and body force.",
        LoadParam(..) => "Pressure acts into the body. Tractions are force per area; edge and point forces are totals (axisymmetric: over 360 degrees). A bearing load is a cosine pressure on the half of the hole facing the force.",
        LoadAdd => "Adds a pressure on the first edge.",
        BushingParam(..) => "A bushing pressed into a circular hole of a plane problem: its outer diameter is the hole's plus the diametral interference, in frictional contact with the plate. The bore may be offset from the hole centre (an eccentric bushing). A load or support on holeN then acts on the bushing's bore. A bushed problem is solved with contact (seconds) on r, not automatically.",
        BushingRemove(_) => "Removes this bushing.",
        BushingAdd => "Presses a steel bushing (bore 1.5 hole radii, diametral interference 0.8 % of the hole radius, friction 0.15) into the first circular hole that has none.",
    }
}

/// The element kind to use when the analysis becomes a 3D solid (the extrusion needs quads).
pub fn fix_element_for(p: &mut Problem) {
    if p.analysis == Analysis::Solid && !p.mesh.element.is_quad() {
        p.mesh.element = ElementChoice::Quad8;
    }
}

/// A new circular hole placed in a free-looking spot of the outline.
pub fn new_hole(p: &Problem) -> Shape {
    let Geometry::Sketch { outer, holes, .. } = &p.geometry else { return Shape::Circle { cx: 0.0, cy: 0.0, r: 0.1 } };
    let (lo, hi) = outer.bounds();
    let (w, h) = (hi[0] - lo[0], hi[1] - lo[1]);
    let n = holes.len();
    let fx = [0.5, 0.25, 0.75][n % 3];
    let fy = [0.5, 0.3, 0.7][(n / 3) % 3];
    Shape::Circle { cx: lo[0] + fx * w, cy: lo[1] + fy * h, r: 0.08 * w.min(h) }
}

/// Splits the closing edge of a polygon with a vertex at its midpoint.
pub fn add_polygon_point(p: &mut Problem) {
    if let Geometry::Sketch { outer: Shape::Polygon { pts }, .. } = &mut p.geometry {
        let (a, b) = (pts[pts.len() - 1], pts[0]);
        pts.push([0.5 * (a[0] + b[0]), 0.5 * (a[1] + b[1])]);
    }
}

pub fn remove_polygon_point(p: &mut Problem) {
    if let Geometry::Sketch { outer: Shape::Polygon { pts }, .. } = &mut p.geometry {
        if pts.len() > 3 {
            pts.pop();
        }
    }
}

/// The next (or previous) edge name after `current`.
pub fn cycle_name(current: &str, names: &[String], forward: bool) -> Option<String> {
    if names.is_empty() {
        return None;
    }
    let i = names.iter().position(|n| n == current);
    let n = names.len();
    let next = match (i, forward) {
        (Some(i), true) => (i + 1) % n,
        (Some(i), false) => (i + n - 1) % n,
        (None, _) => 0,
    };
    Some(names[next].clone())
}

pub fn default_load(first_edge: &str) -> Load {
    Load::Pressure { edge: first_edge.to_string(), p: 1000.0 }
}

/// A default bushing for the first circular hole that has none, or `None`.
pub fn new_bushing(p: &Problem) -> Option<fea_problem::Bushing> {
    let Geometry::Sketch { holes, .. } = &p.geometry else { return None };
    let (i, r) = holes.iter().enumerate().find_map(|(i, h)| match h {
        Shape::Circle { r, .. } if !p.bushings.iter().any(|b| b.hole == i + 1) => Some((i, *r)),
        _ => None,
    })?;
    Some(fea_problem::Bushing { hole: i + 1, inner_diameter: 1.5 * r, offset: [0.0, 0.0], interference: 0.008 * r, friction: 0.15, material: fea_problem::MaterialSpec { name: "Steel (bushing)".into(), e: 29.0e6, nu: 0.3, alpha: 6.5e-6, yield_stress: None, density: 0.283 / 386.089 } })
}

/// Drop the bushings of a removed hole and renumber the later ones.
pub fn forget_hole(p: &mut Problem, removed: usize) {
    p.bushings.retain(|b| b.hole != removed + 1);
    for b in &mut p.bushings {
        if b.hole > removed + 1 {
            b.hole -= 1;
        }
    }
}
