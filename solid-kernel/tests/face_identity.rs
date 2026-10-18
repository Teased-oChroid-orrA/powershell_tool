//! Persistent face identity: signatures of the faces of a model, and the association of two models (a renumbered re-export, an
//! edited part). The files are OpenCASCADE exports of a 40 x 30 x 10 plate with a 4 mm hole (see `tests/data/README.md`).
//! `docs/issue-46-phase-103.md`.

use solid_kernel::analytic_step::{AnalyticModel, Options};
use solid_kernel::face_identity::{associate, FaceSignature, Kind, Tolerances};
use solid_kernel::step::{inventory, Limits};
use std::collections::BTreeMap;

fn signatures(source: &str) -> BTreeMap<u64, FaceSignature> {
    let data = inventory(source, Limits::default()).unwrap();
    AnalyticModel::read(&data, Options::default())
        .unwrap()
        .face_signatures()
        .unwrap()
}

/// Renumber every `#n` of a STEP text (a different exporter, or another run of the same one, numbers the entities differently).
fn renumbered(source: &str) -> (String, impl Fn(u64) -> u64) {
    let map = |n: u64| n * 7 + 1013;
    let mut out = String::with_capacity(source.len() + 1024);
    let bytes = source.as_bytes();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'#' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit() {
            let start = i + 1;
            let mut j = start;
            while j < bytes.len() && bytes[j].is_ascii_digit() {
                j += 1;
            }
            let n: u64 = source[start..j].parse().unwrap();
            out.push('#');
            out.push_str(&map(n).to_string());
            i = j;
        } else {
            out.push(bytes[i] as char);
            i += 1;
        }
    }
    (out, map)
}

fn by_kind(sigs: &BTreeMap<u64, FaceSignature>, kind: Kind) -> Vec<u64> {
    sigs.iter()
        .filter(|(_, s)| s.kind == kind)
        .map(|(id, _)| *id)
        .collect()
}

const PLATE: &str = include_str!("data/edit-plate.step");

#[test]
fn a_renumbered_re_export_associates_every_face_exactly() {
    let (text, map) = renumbered(PLATE);
    let (old, new) = (signatures(PLATE), signatures(&text));
    assert_eq!(old.len(), 7);
    let a = associate(&old, &new, Tolerances::default());
    assert_eq!(
        (
            a.modified.len(),
            a.moved.len(),
            a.lost.len(),
            a.added.len(),
            a.ambiguous.len()
        ),
        (0, 0, 0, 0, 0),
        "{a:?}"
    );
    assert_eq!(a.exact.len(), 7);
    for (o, n) in &a.exact {
        assert_eq!(map(*o), *n, "face #{o} found at #{n}");
    }
}

#[test]
fn an_edited_plate_keeps_the_identity_of_the_faces_the_edit_did_not_touch() {
    let old = signatures(PLATE);
    let cylinder = by_kind(&old, Kind::Cylinder)[0];
    let planes = by_kind(&old, Kind::Plane);
    assert_eq!(planes.len(), 6);

    // The hole moved 2 mm: the six plates of the part are exactly the same faces, the cylinder is the moved face.
    let moved = signatures(include_str!("data/edit-plate-hole-moved.step"));
    let a = associate(&old, &moved, Tolerances::default());
    assert_eq!(
        (
            a.exact.len(),
            a.modified.len(),
            a.lost.len(),
            a.added.len(),
            a.ambiguous.len()
        ),
        (6, 0, 0, 0, 0),
        "{a:?}"
    );
    assert_eq!(a.moved.len(), 1);
    let (o, _, d) = a.moved[0];
    assert_eq!(o, cylinder);
    assert!((d - 2.).abs() < 1e-6, "displacement {d}");

    // A hole of another radius is not the same face: nothing claims it (a different size is a different feature).
    let resized = signatures(include_str!("data/edit-plate-hole-resized.step"));
    let a = associate(&old, &resized, Tolerances::default());
    assert_eq!(a.exact.len() + a.modified.len(), 6, "{a:?}");
    assert_eq!((a.lost, a.added.len()), (vec![cylinder], 1));

    // A second hole: the cylinder stays, the new one is added, the top and bottom plates (one more loop) are the modified faces.
    let second = signatures(include_str!("data/edit-plate-second-hole.step"));
    let a = associate(&old, &second, Tolerances::default());
    assert_eq!(a.lost.len() + a.ambiguous.len(), 0, "{a:?}");
    assert_eq!(a.added.len(), 1);
    assert_eq!(second[&a.added[0]].kind, Kind::Cylinder);
    assert!(
        a.exact.iter().any(|(o, _)| *o == cylinder),
        "the first hole is still itself"
    );
    assert_eq!(a.modified.len(), 2, "{a:?}");
    for (o, n) in &a.modified {
        assert_eq!(old[o].kind, Kind::Plane);
        assert_eq!(second[n].loops, old[o].loops + 1);
    }

    // The plate got 2 mm thicker: the four side plates and the hole keep their surfaces (modified: the extent grew), the top plate
    // moved up, the bottom one is untouched.
    let thicker = signatures(include_str!("data/edit-plate-thicker.step"));
    let a = associate(&old, &thicker, Tolerances::default());
    assert_eq!(
        (a.lost.len(), a.added.len(), a.ambiguous.len()),
        (0, 0, 0),
        "{a:?}"
    );
    assert_eq!(a.exact.len(), 1, "{a:?}");
    assert_eq!(a.modified.len() + a.moved.len(), 6);
    assert!(
        a.moved.iter().any(|(_, _, d)| (d - 2.).abs() < 1e-6),
        "the top plate rose by 2 mm: {a:?}"
    );
}

#[test]
fn identical_features_are_reported_ambiguous_not_guessed() {
    // One hole in the middle of the plate against two identical holes 5 mm to either side of the middle: the old hole is
    // equally near both, so neither is claimed.
    let one = signatures(include_str!("data/edit-plate-centre-hole.step"));
    let two = signatures(include_str!("data/edit-plate-twin-holes.step"));
    let cylinder = by_kind(&one, Kind::Cylinder)[0];
    let a = associate(
        &one,
        &two,
        Tolerances {
            search_radius: 6.,
            ..Tolerances::default()
        },
    );
    assert_eq!(a.ambiguous.len(), 1, "{a:?}");
    assert_eq!(a.ambiguous[0].0, cylinder);
    assert_eq!(a.ambiguous[0].1.len(), 2);
    assert!(a.moved.is_empty() && a.lost.is_empty(), "{a:?}");
    // within a smaller search radius neither twin is a candidate: the hole is lost and both are added
    let b = associate(
        &one,
        &two,
        Tolerances {
            search_radius: 4.,
            ..Tolerances::default()
        },
    );
    assert_eq!((b.lost, b.added.len()), (vec![cylinder], 2));
}

#[test]
fn signatures_survive_their_text_form() {
    for (_, s) in signatures(include_str!("data/edit-plate-second-hole.step")) {
        let line = s.encode();
        assert!(!line.contains('\n'));
        assert_eq!(FaceSignature::decode(&line).unwrap(), s, "{line}");
    }
    assert!(FaceSignature::decode("face-signature v2;Plane").is_err());
    assert!(FaceSignature::decode("garbage").is_err());
}

#[test]
fn spline_surface_faces_of_a_renumbered_re_export_associate_by_position_and_neighbours() {
    // the fillet faces of a rounded cross bore are rational B-spline surfaces (`Kind::Spline`)
    let source = include_str!("data/cross-bore-filleted.step");
    let (text, map) = renumbered(source);
    // OpenCASCADE's intersection-curve approximations deviate by about 1e-5 mm from the surface
    let read = |source: &str| {
        let data = inventory(source, Limits::default()).unwrap();
        AnalyticModel::read(
            &data,
            Options {
                geometry_tolerance_mm: 1e-4,
                ..Options::default()
            },
        )
        .unwrap()
        .face_signatures()
        .unwrap()
    };
    let (old, new) = (read(source), read(&text));
    let splines = by_kind(&old, Kind::Spline);
    assert!(!splines.is_empty(), "the fixture has no spline-surface face");
    let a = associate(&old, &new, Tolerances::default());
    assert_eq!(
        (
            a.modified.len(),
            a.moved.len(),
            a.lost.len(),
            a.added.len(),
            a.ambiguous.len()
        ),
        (0, 0, 0, 0, 0),
        "{a:?}"
    );
    assert_eq!(a.exact.len(), old.len());
    for (o, n) in &a.exact {
        assert_eq!(map(*o), *n, "face #{o} found at #{n}");
    }
}
