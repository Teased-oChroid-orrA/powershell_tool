//! Plain-text report of a solved problem.

use crate::problem::{Geometry, Load, Support};
use crate::solve::Solved;
use std::fmt::Write;

pub fn describe_support(s: &Support) -> String {
    let c = s.comps();
    let comp = |v: Option<f64>, n: &str| v.map(|v| if v == 0.0 { format!("{n}=0") } else { format!("{n}={v:.6}") });
    let held: Vec<String> = [comp(c[0], "ux"), comp(c[1], "uy"), comp(c[2], "uz")].into_iter().flatten().collect();
    let all_held = c[0] == Some(0.0) && c[1] == Some(0.0) && c[2].is_none_or(|v| v == 0.0);
    let held = if all_held { "fixed".to_string() } else { held.join(" ") };
    match s {
        Support::Edge { edge, .. } => format!("{edge}: {held}"),
        Support::Point { x, y, z, .. } => format!("point ({x}, {y}, {z}): {held}"),
    }
}

pub fn describe_load(l: &Load) -> String {
    match l {
        Load::Pressure { edge, p } => format!("{edge}: pressure {p}"),
        Load::Traction { edge, tx, ty, tz } => format!("{edge}: traction ({tx}, {ty}, {tz})"),
        Load::Force { edge, fx, fy, fz } => format!("{edge}: total force ({fx}, {fy}, {fz})"),
        Load::Bearing { hole, fx, fy } => format!("hole {hole}: bearing load ({fx}, {fy})"),
        Load::Point { x, y, z, fx, fy, fz } => format!("point ({x}, {y}, {z}): force ({fx}, {fy}, {fz})"),
        Load::Body { bx, by, bz } => format!("body force ({bx}, {by}, {bz})"),
    }
}

pub fn report(s: &Solved) -> String {
    let (p, sm) = (&s.problem, &s.summary);
    let mut o = String::new();
    let _ = writeln!(o, "FEA Workbench: {}", p.name);
    let _ = writeln!(o, "Analysis: {}{}", p.analysis.label(), if matches!(p.analysis, crate::Analysis::PlaneStress | crate::Analysis::PlaneStrain) { format!(", thickness {}", p.thickness) } else { String::new() });
    let _ = writeln!(o, "Material: {}  E = {}  nu = {}  alpha = {}{}", p.material.name, p.material.e, p.material.nu, p.material.alpha, p.material.yield_stress.map(|y| format!("  yield = {y}")).unwrap_or_default());
    match &p.geometry {
        Geometry::Sketch { outer, holes, depth, layers } => {
            let _ = writeln!(o, "Geometry: {} with {} hole(s){}", outer.label().to_lowercase(), holes.len(), if p.analysis == crate::Analysis::Solid { format!(", extruded {depth} in {layers} layer(s)") } else { String::new() });
        }
        Geometry::Imported { path } => {
            let _ = writeln!(o, "Geometry: imported mesh {path}");
        }
    }
    if p.delta_t != 0.0 {
        let _ = writeln!(o, "Temperature change: {}", p.delta_t);
    }
    let _ = writeln!(o, "\nSupports");
    for s in &p.supports {
        let _ = writeln!(o, "  {}", describe_support(s));
    }
    let _ = writeln!(o, "Loads");
    if p.loads.is_empty() && p.delta_t == 0.0 {
        let _ = writeln!(o, "  (none)");
    }
    for l in &p.loads {
        let _ = writeln!(o, "  {}", describe_load(l));
    }
    let _ = writeln!(o, "\nMesh: {:?}, {} nodes, {} elements, {} dofs", p.mesh.element, sm.nodes, sm.elements, sm.dofs);
    for (i, h) in s.history.iter().enumerate() {
        let _ = writeln!(o, "  pass {}: {} nodes, {} elements, error estimate {:.2} %, peak von Mises {:.1}", i + 1, h.nodes, h.elements, 100.0 * h.zz_error, h.max_von_mises);
    }
    let at = |e: &crate::solve::Extreme| if s.model.mesh.dim() == 3 { format!("({:.4}, {:.4}, {:.4})", e.at[0], e.at[1], e.at[2]) } else { format!("({:.4}, {:.4})", e.at[0], e.at[1]) };
    let _ = writeln!(o, "\nResults");
    let _ = writeln!(o, "  Max displacement   {:.6e} at {}", sm.max_displacement.value, at(&sm.max_displacement));
    let _ = writeln!(o, "  Max von Mises      {:.2} at {}", sm.max_von_mises.value, at(&sm.max_von_mises));
    let _ = writeln!(o, "  Max principal      {:.2} at {}", sm.max_principal.value, at(&sm.max_principal));
    let _ = writeln!(o, "  Min principal      {:.2} at {}", sm.min_principal.value, at(&sm.min_principal));
    if let Some(m) = sm.margin {
        let _ = writeln!(o, "  Yield margin       {m:+.3} (yield / peak von Mises - 1)");
    }
    let _ = writeln!(o, "  Strain energy      {:.6e}", sm.strain_energy);
    let _ = writeln!(o, "\nChecks");
    let _ = writeln!(o, "  Reactions          ({:.4}, {:.4}, {:.4})", sm.reaction[0], sm.reaction[1], sm.reaction[2]);
    let _ = writeln!(o, "  Applied loads      ({:.4}, {:.4}, {:.4})", sm.applied[0], sm.applied[1], sm.applied[2]);
    let _ = writeln!(o, "  Equilibrium error  {:.2e}", sm.equilibrium_error);
    let _ = writeln!(o, "  Mesh error (ZZ)    {:.2} % of the energy norm", 100.0 * sm.zz_error);
    let _ = writeln!(o, "  Solve time         {:.0} ms", sm.solve_ms);
    if !sm.interfaces.is_empty() {
        let _ = writeln!(o, "\nInterference fits");
        for f in &sm.interfaces {
            let _ = writeln!(o, "  hole {}: mean pressure {:.0}, peak {:.0}, open over {:.0} deg, friction torque capacity {:.2}, {:.0} % of the force at its friction limit", f.hole, f.mean_pressure, f.peak_pressure, f.open_arc_deg, f.torque_capacity, 100.0 * f.slip_share);
        }
    }
    if !sm.notes.is_empty() {
        let _ = writeln!(o, "\nNotes");
        for n in &sm.notes {
            let _ = writeln!(o, "  - {n}");
        }
    }
    o
}
