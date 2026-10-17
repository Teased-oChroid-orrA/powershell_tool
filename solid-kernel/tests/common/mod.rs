//! Shared bracket fixtures for p-curve integration tests.
#![allow(dead_code)]
use solid_kernel::{
    nurbs,
    step::{EntityReference, Inventory},
    step_body::BodyContext,
    step_parameter_map::ParameterMapRef,
    step_pcurve::{self, ParametricCurve},
    step_plane_map::PlaneParameterMap,
};
pub const SOURCE: &str = include_str!("../../../nema17-motor-bracket.step");

/// Every planar spline p-curve of the bracket: (map, edge, pcurve, active domain).
pub fn spline_pcurves(
    data: &Inventory,
    context: &BodyContext,
) -> Vec<(PlaneParameterMap, [EntityReference; 2], [f64; 2])> {
    let mut out = Vec::new();
    for face in context.body().faces.values() {
        if data.entities()[&face.surface].components[0].name != "PLANE" {
            continue;
        }
        let map = PlaneParameterMap::read(context, data, data.entity_reference(face.id).unwrap())
            .unwrap();
        let mut seen = std::collections::BTreeSet::new();
        for use_ in face.bounds.iter().flat_map(|b| &b.edges) {
            let edge = &context.body().edges[&use_.edge];
            let wrapper =
                step_pcurve::read_surface_curve(data, data.entity_reference(edge.curve).unwrap())
                    .unwrap();
            for pc in wrapper.pcurves {
                let d = step_pcurve::read_pcurve(data, pc, nurbs::Limits::default()).unwrap();
                if d.basis_surface != ParameterMapRef::from(&map).surface_reference()
                    || !seen.insert((edge.id, pc.entity))
                {
                    continue;
                }
                if let ParametricCurve::Spline(s) = &d.geometry {
                    out.push((
                        PlaneParameterMap::read(
                            context,
                            data,
                            data.entity_reference(face.id).unwrap(),
                        )
                        .unwrap(),
                        [data.entity_reference(edge.id).unwrap(), pc],
                        s.curve().domain(),
                    ));
                }
            }
        }
    }
    out
}
