//! The housing, the offset bushing and the pin load as an FEA `Problem`, for the picture before an analysis has run (the
//! mesh, the bodies in their own colours, the load arrow). The analysis itself is the `eccentric-bushing` solver's, on its
//! own mesh; once it has run, the window shows that solved model (`vtu`) instead.
use crate::gpu_viewer::editor::bushing::fe::material;
use crate::toolboxes::bushing::model::BushingModel;
use crate::toolboxes::eccentric_bushing::model::{build_input, EccentricUi};
use fea_problem::{templates::templates, Analysis, Bushing, Geometry, Load, MeshSpec, Problem, Shape};

pub fn preview_problem(bushing: &BushingModel, ui: &EccentricUi) -> Result<Problem, String> {
    let i = build_input(bushing, ui)?;
    let r = i.bore_radius();
    let outer = match i.edge_distance {
        None => Shape::Circle { cx: 0.0, cy: 0.0, r: i.housing_od / 2.0 },
        Some(ed) => {
            let far = (3.0 * ed).max(5.0 * i.bore_dia);
            Shape::Rect { x0: -ed, y0: -far, x1: far, y1: far }
        }
    };
    let mut p = templates().remove(4).1; // the lug with an eccentric bushing: everything below is replaced
    p.name = "Eccentric bushing".into();
    p.analysis = Analysis::PlaneStress;
    p.thickness = i.thickness;
    p.material = material(bushing.housing_material());
    p.geometry = Geometry::Sketch { outer, holes: vec![Shape::Circle { cx: 0.0, cy: 0.0, r }], depth: 1.0, layers: 4, regions: Vec::new() };
    let size = r / 1.5;
    p.mesh = MeshSpec { size, hole_factor: ((r / 5.0) / size).clamp(0.05, 1.0), ..MeshSpec::default() };
    p.bushings = vec![Bushing { hole: 1, inner_diameter: i.bushing_id, offset: [i.offset, 0.0], interference: i.interference_dia.max(0.0), friction: i.friction, material: material(bushing.bushing_material()) }];
    p.supports.clear();
    let phi = i.load_angle_deg.to_radians();
    p.loads = if i.load_lbf > 0.0 { vec![Load::Bearing { hole: 1, fx: i.load_lbf * phi.cos(), fy: i.load_lbf * phi.sin() }] } else { Vec::new() };
    p.cases.clear();
    p.active_case = 0;
    Ok(p)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_offset_bore_the_boss_and_the_pin_load_reach_the_picture() {
        let b = BushingModel::default();
        let mut ui = EccentricUi::default();
        ui.offset = 0.03;
        ui.load_angle = 90.0;
        let p = preview_problem(&b, &ui).unwrap();
        assert_eq!(p.bushings[0].offset, [0.03, 0.0]);
        let Geometry::Sketch { outer, .. } = &p.geometry else { panic!() };
        assert!(matches!(outer, Shape::Circle { r, .. } if (*r - ui.boss_factor * 0.25).abs() < 1e-9), "{outer:?}");
        assert!(matches!(p.loads[..], [Load::Bearing { fx, fy, .. }] if fx.abs() < 1e-9 && (fy - b.load).abs() < 1e-9));
        ui.edge_limited = true;
        let q = preview_problem(&b, &ui).unwrap();
        let Geometry::Sketch { outer, .. } = &q.geometry else { panic!() };
        assert!(matches!(outer, Shape::Rect { x0, .. } if (*x0 + b.edge_dist).abs() < 1e-12), "the free edge is the edge distance on the -x side");
    }

    #[test]
    fn inputs_the_solver_refuses_are_refused_with_its_reason() {
        let mut ui = EccentricUi::default();
        ui.offset = 0.5;
        assert!(preview_problem(&BushingModel::default(), &ui).is_err());
    }
}
