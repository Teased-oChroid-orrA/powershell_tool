use solid_kernel::{
    step::{inventory, ErrorKind, Limits},
    step_body::BodyContext,
    step_curve_audit::{audit, LocusCheck, Options},
};
const SOURCE: &str = include_str!("../../nema17-motor-bracket.step");
#[test]
fn complete_fixture_locus_audit_has_bounded_totals_and_monotonic_progress() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let mut history = Vec::new();
    let report = audit(&c, &d, Options::default(), |p| {
        history.push(p);
        true
    })
    .unwrap();
    assert_eq!(report.checks.len(), 228);
    assert_eq!(report.spline_spans, 45);
    assert_eq!(report.curve_controls, 105);
    assert_eq!(report.source_body.entity, c.body().id);
    let mut kinds = [0; 3];
    for check in report.checks {
        match check {
            LocusCheck::FiniteLine(r) => {
                kinds[0] += 1;
                assert!(r.curve_to_pcurve_error_mm.into_iter().all(|x| x <= 1e-7));
            }
            LocusCheck::UnsignedCircle(r) => {
                kinds[1] += 1;
                assert!(r.hausdorff_upper_mm <= 1e-7);
            }
            LocusCheck::ClosedQuadraticCircle(r) => {
                kinds[2] += 1;
                assert!(
                    r.full_circle_coverage
                        && r.single_winding
                        && r.curve_to_circle_upper_mm <= 1e-7
                );
            }
        }
    }
    assert_eq!(kinds, [176, 37, 15]);
    assert!(history
        .windows(2)
        .all(|p| p[0].edge_uses_seen <= p[1].edge_uses_seen
            && p[0].associations_done <= p[1].associations_done
            && p[0].spline_spans_done <= p[1].spline_spans_done
            && p[0].curve_controls_done <= p[1].curve_controls_done));
    let last = history.last().unwrap();
    assert_eq!(last.associations_done, 228);
    assert_eq!(last.total_associations, Some(228));
}
#[test]
fn all_aggregate_budgets_are_hard_limits() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    for options in [
        Options {
            max_edge_uses: 0,
            ..Options::default()
        },
        Options {
            max_associations: 227,
            ..Options::default()
        },
        Options {
            max_spline_spans: 44,
            ..Options::default()
        },
        Options {
            max_curve_controls: 104,
            ..Options::default()
        },
    ] {
        assert_eq!(
            audit(&c, &d, options, |_| true).unwrap_err().kind,
            ErrorKind::Limit
        );
    }
}
#[test]
fn cancellation_through_discovery_spans_and_final_association_is_atomic() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    assert_eq!(
        audit(&c, &d, Options::default(), |_| false)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        audit(&c, &d, Options::default(), |p| p.edge_uses_seen < 3)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        audit(&c, &d, Options::default(), |p| p.spline_spans_done < 2)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
    assert_eq!(
        audit(&c, &d, Options::default(), |p| p.associations_done < 228)
            .unwrap_err()
            .kind,
        ErrorKind::Cancelled
    );
}
#[test]
fn stale_context_and_absent_surface_association_do_not_produce_partial_success() {
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    let changed = inventory(&format!("{SOURCE}\n"), Limits::default()).unwrap();
    assert!(audit(&c, &changed, Options::default(), |_| true).is_err());
    let source = SOURCE.replace("#235 = PCURVE('',#32,#236);", "#235 = PCURVE('',#1,#236);");
    assert_ne!(source, SOURCE);
    let changed = inventory(&source, Limits::default()).unwrap();
    let cc = BodyContext::read(&changed).unwrap();
    assert!(audit(&cc, &changed, Options::default(), |_| true).is_err());
}
#[test]
fn finite_arc_audit_resolves_every_circle_edge_and_stays_opt_in() {
    use solid_kernel::step_pcurve_arc::{Arc, Resolved};
    let d = inventory(SOURCE, Limits::default()).unwrap();
    let c = BodyContext::read(&d).unwrap();
    assert!(audit(&c, &d, Options::default(), |_| true)
        .unwrap()
        .finite_arcs
        .is_empty());
    let options = Options {
        finite_arcs: true,
        ..Options::default()
    };
    let mut last = None;
    let report = audit(&c, &d, options, |p| {
        assert!(last.is_none_or(|n| p.associations_done >= n));
        last = Some(p.associations_done);
        true
    })
    .unwrap();
    assert_eq!(report.checks.len(), 228);
    let closed = report
        .finite_arcs
        .iter()
        .filter(|a| matches!(a.resolved, Resolved::Closed))
        .count();
    let interior = report
        .finite_arcs
        .iter()
        .filter(|a| matches!(&a.resolved, Resolved::Arc(r) if r.arc == Arc::Interior))
        .count();
    assert_eq!((report.finite_arcs.len(), closed, interior), (52, 20, 20));
    assert_eq!(
        audit(&c, &d, options, |_| false).unwrap_err().kind,
        ErrorKind::Cancelled
    );
}
