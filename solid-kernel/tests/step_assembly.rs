//! STEP product structure: the placements of a two-instance assembly, checked against OpenCASCADE reading the same file
//! (feature `occt`), the independent reference for the transformation direction convention.

use solid_kernel::step::{inventory, Limits};
use solid_kernel::step_assembly::{read, Placement};

const SOURCE: &str = include_str!("data/assembly-two-instances.step");

fn bounds(points: impl Iterator<Item = [f64; 3]>) -> ([f64; 3], [f64; 3]) {
    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
    for p in points {
        for k in 0..3 {
            lo[k] = lo[k].min(p[k]);
            hi[k] = hi[k].max(p[k]);
        }
    }
    (lo, hi)
}

type Extent = ([f64; 3], [f64; 3]);

fn placed(source: &str) -> Vec<(Placement, Extent)> {
    let data = inventory(source, Limits::default()).unwrap();
    let assembly = read(&data).unwrap();
    let bodies = data.raw_bodies().unwrap();
    assembly
        .parts
        .iter()
        .map(|p| {
            let body = bodies.iter().find(|b| b.id == p.solid).unwrap();
            (p.placement, bounds(body.vertices.values().map(|v| p.placement.apply(v.coordinates))))
        })
        .collect()
}

fn near(a: [f64; 3], b: [f64; 3]) -> bool {
    (0..3).all(|k| (a[k] - b[k]).abs() < 1e-9)
}

#[test]
fn each_instance_lands_where_the_placements_say() {
    let parts = placed(SOURCE);
    assert_eq!(parts.len(), 2);
    // Instance A: local x -> +y, local y -> -x, local z -> +z, at (10, 5, 0): the 2x1x3 box lands on x 9..10, y 5..7, z 0..3.
    let a = parts.iter().find(|(_, b)| near(b.0, [9., 5., 0.])).expect("instance A");
    assert!(near(a.1 .1, [10., 7., 3.]), "{:?}", a.1);
    assert_eq!(a.0.rotation, [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]]);
    // Instance B: z' = +y, x' = +x (so y' = -z) at (-6, 3, 4): x -4..-6 wait x -6..-4, y 3..6, z 3..4.
    let b = parts.iter().find(|(_, b)| near(b.0, [-6., 3., 3.])).expect("instance B");
    assert!(near(b.1 .1, [-4., 6., 4.]), "{:?}", b.1);
}

#[test]
fn the_order_of_the_two_placements_and_of_the_two_representations_does_not_change_the_result() {
    let reference = placed(SOURCE);
    for (from, to) in [
        ("ITEM_DEFINED_TRANSFORMATION('','',#11,#931)", "ITEM_DEFINED_TRANSFORMATION('','',#931,#11)"),
        ("REPRESENTATION_RELATIONSHIP('','',#10,#904)", "REPRESENTATION_RELATIONSHIP('','',#904,#10)"),
    ] {
        assert!(SOURCE.contains(from), "{from}");
        let swapped = placed(&SOURCE.replace(from, to));
        for (r, s) in reference.iter().zip(&swapped) {
            assert!(near(r.1 .0, s.1 .0) && near(r.1 .1, s.1 .1), "{from}: {:?} vs {:?}", r.1, s.1);
        }
    }
}

#[test]
fn ambiguous_unsupported_and_inconsistent_structure_is_refused_with_a_reason() {
    let refuse = |source: String, expect: &str| {
        let data = inventory(&source, Limits::default()).unwrap();
        let err = read(&data).err().unwrap_or_else(|| panic!("expected a refusal containing {expect:?}"));
        assert!(err.detail.contains(expect), "{}: {}", expect, err.detail);
    };
    // A placement that is an item of neither representation.
    refuse(SOURCE.replace("(#905,#931,#941)", "(#905,#941)"), "cannot tell");
    // A placement that is an item of both representations.
    refuse(SOURCE.replace("#904 = SHAPE_REPRESENTATION('',(#905,#931,#941)", "#904 = SHAPE_REPRESENTATION('',(#905,#931,#941,#11)"), "cannot tell");
    // A missing transformation.
    refuse(SOURCE.replace("#933 = CONTEXT_DEPENDENT_SHAPE_REPRESENTATION(#932,#921);", ""), "no placement transformation");
    // A relationship that does not join the two representations.
    refuse(SOURCE.replace("REPRESENTATION_RELATIONSHIP('','',#10,#904)", "REPRESENTATION_RELATIONSHIP('','',#10,#10)"), "does not join");
    // Mapped items.
    refuse(SOURCE.replace("ENDSEC;\nEND-ISO", "#990 = MAPPED_ITEM('',#991,#905);\n#991 = REPRESENTATION_MAP(#905,#904);\nENDSEC;\nEND-ISO"), "mapped items");
    // A degenerate frame: reference direction parallel to the axis.
    refuse(SOURCE.replace("#2934 = DIRECTION('',(0.,1.,0.));", "#2934 = DIRECTION('',(0.,0.,1.));"), "parallel");
    // A usage that makes the part its own parent is inconsistent with its relationship (and would be a cycle).
    refuse(SOURCE.replace("NEXT_ASSEMBLY_USAGE_OCCURRENCE('920','','',#902,#5,$)", "NEXT_ASSEMBLY_USAGE_OCCURRENCE('920','','',#5,#5,$)"), "does not join");
    // Not an assembly at all.
    let single = inventory(include_str!("../../nema17-motor-bracket.step"), Limits::default()).unwrap();
    assert!(read(&single).unwrap_err().detail.contains("not an assembly"));
}

#[test]
fn placements_compose_and_invert() {
    let a = Placement { rotation: [[0., -1., 0.], [1., 0., 0.], [0., 0., 1.]], translation: [10., 5., 0.] };
    let b = Placement { rotation: [[1., 0., 0.], [0., 0., -1.], [0., 1., 0.]], translation: [-6., 3., 4.] };
    let p = [0.3, -1.2, 2.5];
    assert!(near(a.after(&b).apply(p), a.apply(b.apply(p))));
    assert!(near(a.inverse().apply(a.apply(p)), p));
    assert!(near(a.after(&a.inverse()).apply(p), p));
}

#[cfg(feature = "occt")]
#[test]
fn the_native_placements_agree_with_opencascade_reading_the_same_file() {
    let parts = placed(SOURCE);
    let (lo, hi) = bounds(parts.iter().flat_map(|(_, b)| [b.0, b.1]));
    let reference = solid_kernel::occt::Brep::from_step_text(SOURCE).unwrap();
    let (rlo, rhi) = reference.bounds();
    // OpenCASCADE pads its bounding box by its tolerance (1e-7).
    assert!((0..3).all(|k| (lo[k] - rlo[k]).abs() < 1e-6 && (hi[k] - rhi[k]).abs() < 1e-6), "native {lo:?}..{hi:?} vs OpenCASCADE {rlo:?}..{rhi:?}");
}
