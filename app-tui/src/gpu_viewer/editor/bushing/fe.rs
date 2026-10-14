//! The finite-element picture of a bushing in its housing, derived from the workbench inputs: a plane-stress housing
//! with the bore and the edge at the stated distance, the bushing pressed into it with the achieved interference, and the
//! pin load pulling the bushing toward the edge. It is the FEA Workbench's own `Problem` with a bushing (frictional
//! interference contact, `fea_problem::solve`), so the solve, mesh and results are the ones the FEA Workbench already
//! trusts. It is a view and a cross-check: the pass/fail checks stay `bushing-solver`'s, and nothing here feeds back.
use crate::toolboxes::bushing::model::BushingModel;
use fea_problem::{
    templates::templates, Analysis, Bushing, Geometry, Load, MaterialSpec, MeshSpec, Problem, Shape, Support,
};

/// Weight density to mass density in the inch / lbf / second system.
const G_INCH: f64 = 386.089;

fn material(m: &mechanics_core::materials::Material) -> MaterialSpec {
    let density = m.extra.map_or(0.1, |x| if x.density_lb_in3 > 0.0 { x.density_lb_in3 } else { 0.1 });
    MaterialSpec { name: m.name.to_string(), e: m.e_ksi * 1000.0, nu: m.nu, alpha: m.alpha_u_f * 1e-6, yield_stress: Some(m.sy_ksi * 1000.0), density: density / G_INCH, plastic: false, hardening: 0.0 }
}

/// Where the model's parts sit, in inches: the housing rectangle `[0, length] x [0, height]` with its free edge at `x = 0`
/// and the bore centre at `(edge, height / 2)`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Layout {
    pub length: f64,
    pub height: f64,
    pub bore_radius: f64,
    pub edge: f64,
}

pub fn layout(model: &BushingModel) -> Result<Layout, String> {
    let a = model.output.bore_tol.nominal / 2.0;
    let e = model.edge_dist;
    if !(a.is_finite() && a > 0.0) {
        return Err("the bore diameter must be positive".into());
    }
    if !(e.is_finite() && e > a) {
        return Err("the edge distance must exceed the bore radius".into());
    }
    if !(model.id_bushing > 0.0 && model.id_bushing / 2.0 < a) {
        return Err("the bushing ID must be positive and smaller than the bore".into());
    }
    // Wide enough that the bore has a wall on both sides, and as wide as the housing says.
    let height = model.housing_width.max(2.5 * a);
    // Far enough past the bore that the far end is not what the stress sees.
    let length = e + (1.5 * e).max(3.0 * a);
    Ok(Layout { length, height, bore_radius: a, edge: e })
}

/// The FEA problem for the current inputs, or why there is none yet.
pub fn housing_problem(model: &BushingModel) -> Result<Problem, String> {
    let l = layout(model)?;
    let out = &model.output;
    let thickness = if out.t_eff_seq > 0.0 { out.t_eff_seq } else { model.housing_len };
    if !(thickness.is_finite() && thickness > 0.0) {
        return Err("the housing length must be positive".into());
    }
    let (housing, bushing) = (model.housing_material(), model.bushing_material());
    let mut p = templates().remove(4).1; // the lug with a bushing: its sketch, mesh and bushing are replaced below
    p.name = "Bushing in housing".into();
    p.analysis = Analysis::PlaneStress;
    p.thickness = thickness;
    p.material = material(housing);
    p.geometry = Geometry::Sketch { outer: Shape::Rect { x0: 0.0, y0: 0.0, x1: l.length, y1: l.height }, holes: vec![Shape::Circle { cx: l.edge, cy: l.height / 2.0, r: l.bore_radius }], depth: 1.0, layers: 4, regions: Vec::new() };
    let size = l.height.min(l.length) / 6.0;
    p.mesh = MeshSpec { size, hole_factor: (l.bore_radius / 6.0 / size).clamp(0.05, 1.0), ..MeshSpec::default() };
    p.bushings = vec![Bushing { hole: 1, inner_diameter: model.id_bushing, offset: [0.0, 0.0], interference: out.delta_total.max(0.0), friction: model.friction.max(0.0), material: material(bushing) }];
    // The housing is held at the far end; the pin pulls the bushing toward the free edge, the case that tears the edge out.
    p.supports = vec![Support::fixed("right")];
    p.loads = if model.load.is_finite() && model.load > 0.0 { vec![Load::Bearing { hole: 1, fx: -model.load, fy: 0.0 }] } else { Vec::new() };
    p.cases.clear();
    p.active_case = 0;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_default_inputs_make_a_valid_plane_problem_with_one_bushing_and_the_pin_load() {
        let m = BushingModel::default();
        let p = housing_problem(&m).unwrap();
        p.validate().unwrap();
        assert_eq!(p.bushings.len(), 1);
        assert!((p.bushings[0].interference - m.output.delta_total).abs() < 1e-12);
        assert_eq!(p.bushings[0].inner_diameter, m.id_bushing);
        assert!(matches!(p.loads[..], [Load::Bearing { hole: 1, fx, fy }] if fx == -m.load && fy == 0.0));
        let Geometry::Sketch { holes, outer, .. } = &p.geometry else { panic!("a sketch") };
        assert!(matches!(holes[0], Shape::Circle { cx, r, .. } if (cx - m.edge_dist).abs() < 1e-12 && (r - 0.25).abs() < 1e-9));
        assert!(matches!(outer, Shape::Rect { x0, .. } if *x0 == 0.0), "the free edge is at x = 0, so the edge distance is the bore centre's x");
        assert_eq!(p.material.e, m.housing_material().e_ksi * 1000.0);
        assert_eq!(p.bushings[0].material.e, m.bushing_material().e_ksi * 1000.0);
    }

    #[test]
    fn inputs_that_describe_no_bushing_in_a_housing_say_why() {
        let mut m = BushingModel::default();
        m.commit_number(crate::toolboxes::bushing::model::NumberTarget::EdgeDist, 0.2);
        assert!(housing_problem(&m).unwrap_err().contains("edge distance"));
        let mut m = BushingModel::default();
        m.commit_number(crate::toolboxes::bushing::model::NumberTarget::IdBushing, 0.6);
        assert!(housing_problem(&m).unwrap_err().contains("bushing ID"));
    }

    #[test]
    fn no_load_leaves_the_fit_alone_and_a_narrow_housing_width_is_widened_to_hold_the_bore() {
        let mut m = BushingModel::default();
        m.commit_number(crate::toolboxes::bushing::model::NumberTarget::Load, 0.0);
        m.commit_number(crate::toolboxes::bushing::model::NumberTarget::HousingWidth, 0.1);
        let p = housing_problem(&m).unwrap();
        assert!(p.loads.is_empty());
        assert!(layout(&m).unwrap().height >= 2.5 * 0.25 - 1e-12);
    }

    /// The FE and the closed form agree on the fit: a bushing pressed into a housing wide enough not to matter loads the bore
    /// with the Lame pressure `bushing-solver` reports (plane stress against the solver's own thick-wall model, so within a few
    /// percent, not exactly).
    #[test]
    fn the_fe_fit_pressure_matches_the_closed_form_in_a_wide_housing() {
        use crate::toolboxes::bushing::model::NumberTarget as N;
        let mut m = BushingModel::default();
        m.commit_number(N::EdgeDist, 1.5);
        m.commit_number(N::HousingWidth, 3.0);
        m.commit_number(N::Load, 0.0);
        let p = housing_problem(&m).unwrap();
        let solved = fea_problem::solve(&p, None).expect("the fit solves");
        let fit = &solved.summary.interfaces[0];
        let analytic = m.output.pressure;
        let rel = (fit.mean_pressure - analytic).abs() / analytic;
        eprintln!("DBG fit {} vs {analytic}", fit.mean_pressure);
        let t = std::time::Instant::now();
        let loaded = housing_problem(&BushingModel::default()).unwrap();
        let s2 = fea_problem::solve(&loaded, None).unwrap();
        eprintln!("DBG loaded solve {:?} vm {:?} iface {:?}", t.elapsed(), s2.summary.max_von_mises.value, s2.summary.interfaces);
        assert!(rel < 0.10, "FE mean fit pressure {} psi vs closed form {analytic} psi ({:.1} % apart)", fit.mean_pressure, 100.0 * rel);
    }
}
