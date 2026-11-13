//! The two dimensioned drawings beside the 3D view: the axial section A-A through the bore (housing and bushing walls
//! hatched, the bore's length, the edge distance) and the cross-section looking down the bore axis (the plate, the bushing
//! ring, the pin load toward the free edge, e and e/D, and, when the edge-distance check fails, where the free edge would
//! have to be). Plain `sketch::Sketch` data from the inputs and the solver's results; `sketch::layout` places the text so
//! nothing clashes (the tests check it for the shapes a case can take).
use super::fe;
use crate::gpu_viewer::sketch::{Arrow, Dim, Fill, Ink, Shape, Sketch};
use crate::toolboxes::bushing::{
    advice::{self, CheckKind, Severity},
    model::BushingModel,
};
use bushing_solver::geometry::BushingType;

fn dia(d: f64) -> String {
    format!("\u{d8}{d:.4}")
}

/// The axial section and the cross-section of `m`, or none when the inputs describe no bushing in a housing.
pub fn sketches(m: &BushingModel) -> Vec<Sketch> {
    match fe::layout(m) {
        Ok(l) if m.housing_len > 0.0 => vec![axial(m, &l), cross(m, &l)],
        _ => Vec::new(),
    }
}

fn flange(m: &BushingModel, a: f64) -> Option<(f64, f64)> {
    (m.bushing_type == BushingType::Flanged && m.flange_od / 2.0 > a && m.flange_thk > 0.0).then_some((m.flange_od / 2.0, m.flange_thk))
}

fn axial(m: &BushingModel, l: &fe::Layout) -> Sketch {
    let (e, a, ri, t, len) = (l.edge, l.bore_radius, m.id_bushing / 2.0, m.housing_len, l.length);
    let fl = flange(m, a);
    let top = t + fl.map_or(0.0, |f| f.1);
    let mut shapes = vec![
        Shape::rect(0.0, 0.0, e - a, t).filled(Fill::Housing).hatched(),
        Shape::rect(e + a, 0.0, len, t).filled(Fill::Housing).hatched(),
        Shape::rect(e - a, 0.0, e - ri, t).filled(Fill::Bushing).hatched(),
        Shape::rect(e + ri, 0.0, e + a, t).filled(Fill::Bushing).hatched(),
    ];
    if let Some((rf, _)) = fl {
        shapes.push(Shape::rect(e - rf, t, e - ri, top).filled(Fill::Bushing).hatched());
        shapes.push(Shape::rect(e + ri, t, e + rf, top).filled(Fill::Bushing).hatched());
    }
    let over = 0.12 * top.max(2.0 * a);
    shapes.push(Shape::line(vec![[e, -over], [e, top + over]], Ink::Center));
    let mut dims = vec![
        Dim::Linear { a: [e - ri, top], b: [e + ri, top], horizontal: true, side: 1, tier: 0, text: format!("{} ID", dia(m.id_bushing)), ink: Ink::Thin },
        Dim::Linear { a: [e - a, top], b: [e + a, top], horizontal: true, side: 1, tier: 1, text: format!("{} bore", dia(2.0 * a)), ink: Ink::Thin },
        Dim::Linear { a: [0.0, 0.0], b: [e, 0.0], horizontal: true, side: -1, tier: 0, text: format!("e {e:.3}"), ink: Ink::Thin },
        Dim::Linear { a: [len, 0.0], b: [len, t], horizontal: false, side: 1, tier: 0, text: format!("t {t:.3}"), ink: Ink::Thin },
        Dim::Note { at: [0.0, 0.5 * t], text: "free edge".into(), ink: Ink::Thin },
        Dim::Leader { at: [e + a, 0.3 * t], text: format!("fit {:.5} in", m.output.delta_total), ink: Ink::Thin },
    ];
    if let Some((rf, ft)) = fl {
        dims.push(Dim::Leader { at: [e - rf, t + 0.5 * ft], text: format!("flange {} x {ft:.3}", dia(2.0 * rf)), ink: Ink::Thin });
    }
    Sketch { title: "Axial section A-A".into(), caption: "through the bore".into(), shapes, arrows: Vec::new(), dims }
}

fn cross(m: &BushingModel, l: &fe::Layout) -> Sketch {
    let (e, a, ri, len, h) = (l.edge, l.bore_radius, m.id_bushing / 2.0, l.length, l.height);
    let out = &m.output;
    let c = [e, 0.5 * h];
    let mut shapes = vec![
        Shape::rect(0.0, 0.0, len, h).filled(Fill::Housing),
        Shape::circle(c, a).filled(Fill::Bushing),
        Shape::circle(c, ri).filled(Fill::Hole),
        Shape::line(vec![[e - 1.35 * a, c[1]], [e + 1.35 * a, c[1]]], Ink::Center),
        Shape::line(vec![[e, c[1] - 1.35 * a], [e, c[1] + 1.35 * a]], Ink::Center),
    ];
    let d = 2.0 * a;
    let mut dims = vec![
        Dim::Linear { a: [0.0, h], b: [e, h], horizontal: true, side: 1, tier: 0, text: format!("e {e:.3} \u{b7} e/D {:.3}", out.ed_actual), ink: Ink::Thin },
        Dim::Linear { a: [0.0, h], b: [len, h], horizontal: true, side: 1, tier: 1, text: format!("{len:.3}"), ink: Ink::Thin },
        Dim::Linear { a: [len, 0.0], b: [len, h], horizontal: false, side: 1, tier: 0, text: format!("{h:.3}"), ink: Ink::Thin },
        Dim::Leader { at: [e + a * 0.7071, c[1] + a * 0.7071], text: format!("{} bore", dia(d)), ink: Ink::Thin },
        Dim::Leader { at: [e + ri, c[1] - 0.25 * ri], text: format!("P {:.0} lbf", m.load), ink: Ink::Load },
        Dim::Note { at: [0.0, 0.2 * h], text: "free edge".into(), ink: Ink::Thin },
    ];
    // Where the free edge would have to be for the edge-distance check that fails (sequencing first: it asks for more).
    let failing = |k: CheckKind| advice::severity_of(&m.checks, k) == Severity::Fail;
    let need = [(CheckKind::EdgeSequencing, out.ed_min_sequence, "sequencing"), (CheckKind::EdgeStrength, out.ed_min_strength, "strength")].into_iter().find(|(k, min, _)| failing(*k) && min.is_finite() && *min * d > e);
    if let Some((_, min, what)) = need {
        let x = e - min * d;
        shapes.push(Shape::line(vec![[x, 0.0], [x, h]], Ink::Ghost));
        dims.push(Dim::Linear { a: [x, 0.0], b: [e, 0.0], horizontal: true, side: -1, tier: 0, text: format!("needs e/D {min:.3} (e {:.3}) for {what}", min * d), ink: Ink::Ghost });
    }
    let arrows = if m.load > 0.0 { vec![Arrow { from: [e + 0.75 * ri, c[1]], to: [e - 0.75 * ri, c[1]] }] } else { Vec::new() };
    Sketch { title: "Cross-section".into(), caption: "looking down the bore axis".into(), shapes, arrows, dims }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::sketch::{clashes, layout, test_measure, Rect};
    use crate::toolboxes::bushing::model::NumberTarget as N;

    /// The panel sizes a drawing gets: side by side under the 3D view, stacked in a narrow window, and alone.
    const SIZES: [(f32, f32); 5] = [(420.0, 270.0), (330.0, 240.0), (560.0, 200.0), (300.0, 210.0), (980.0, 620.0)];

    fn assert_clear(m: &BushingModel, case: &str) {
        let s = sketches(m);
        assert_eq!(s.len(), 2, "{case}");
        for sk in &s {
            for (w, h) in SIZES {
                let area = Rect::new(8.0, 30.0, w, h);
                let l = layout(sk, area, &test_measure);
                assert_eq!(l.labels.len(), sk.dims.len(), "{case}: {}", sk.title);
                let bad = clashes(&l, area);
                assert!(bad.is_empty(), "{case}, {} at {w} x {h}: {bad:?}", sk.title);
            }
        }
    }

    #[test]
    fn the_default_case_is_drawn_with_every_label_clear_and_the_edge_it_would_need() {
        let m = BushingModel::default();
        assert_clear(&m, "default");
        let cross = &sketches(&m)[1];
        // The default fails edge sequencing: the drawing shows where the free edge would have to be.
        let ghost = cross.dims.iter().find_map(|d| match d {
            Dim::Linear { ink: Ink::Ghost, text, a, .. } => Some((text.clone(), a[0])),
            _ => None,
        });
        let (text, x) = ghost.expect("a ghost dimension");
        assert!(text.contains(&format!("{:.3}", m.output.ed_min_sequence)) && text.contains("sequencing"), "{text}");
        assert!((x - (m.edge_dist - m.output.ed_min_sequence * m.bore_dia)).abs() < 1e-3);
    }

    #[test]
    fn a_passing_case_has_no_ghost_and_flanged_thin_wall_and_long_cases_stay_clear() {
        let mut pass = BushingModel::default();
        pass.commit_number(N::EdgeDist, 1.2);
        assert!(sketches(&pass)[1].dims.iter().all(|d| !matches!(d, Dim::Linear { ink: Ink::Ghost, .. })));
        assert_clear(&pass, "passing");

        let mut flanged = BushingModel::default();
        flanged.bushing_type = BushingType::Flanged;
        flanged.flange_od = 0.75;
        flanged.flange_thk = 0.08;
        flanged.recompute();
        assert_eq!(sketches(&flanged)[0].dims.len(), 7, "the flange has its callout");
        assert_clear(&flanged, "flanged");

        let mut thin = BushingModel::default();
        thin.commit_number(N::IdBushing, 0.47);
        assert_clear(&thin, "thin wall");

        let mut long = BushingModel::default();
        long.commit_number(N::HousingLen, 2.0);
        assert_clear(&long, "long bore");
    }

    #[test]
    fn inputs_with_no_bushing_in_a_housing_draw_nothing() {
        let mut m = BushingModel::default();
        m.commit_number(N::EdgeDist, 0.1);
        assert!(sketches(&m).is_empty());
    }
}
