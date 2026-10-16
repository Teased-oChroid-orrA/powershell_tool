//! Explicit, atomic whole-face orientation repair. No coordinates, topology or
//! source CAD data are changed. Geometric intersection checks remain mandatory.
use crate::{
    native_surface::signed_volume_of,
    step::{Error, ErrorKind},
    SurfaceMesh,
};
use std::collections::BTreeMap;
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum OrientationPolicy {
    #[default]
    Strict,
    RepairWithReport,
}
#[derive(Debug, Clone)]
pub struct FaceOrientationChange {
    pub source_face: u64,
    pub before_same_sense: bool,
    pub after_same_sense: bool,
}
fn fail(kind: ErrorKind, text: &str) -> Error {
    Error {
        offset: 0,
        kind,
        detail: text.into(),
    }
}
pub(crate) fn repair(
    mesh: &mut SurfaceMesh,
    source_faces: &[u64],
    senses: &[bool],
    mut progress: impl FnMut(usize, usize) -> bool,
) -> Result<Vec<FaceOrientationChange>, Error> {
    let n = mesh.triangles.len();
    let total = n
        .checked_mul(3)
        .ok_or_else(|| fail(ErrorKind::Limit, "orientation work count overflow"))?;
    if n == 0
        || mesh.face.len() != n
        || senses.len() != source_faces.len()
        || mesh.face.iter().any(|f| *f >= source_faces.len())
        || mesh
            .triangles
            .iter()
            .flatten()
            .any(|i| *i >= mesh.vertices.len())
    {
        return Err(fail(
            ErrorKind::Topology,
            "invalid orientation repair input",
        ));
    }
    if !progress(0, total) {
        return Err(fail(
            ErrorKind::Cancelled,
            "face orientation repair cancelled",
        ));
    }
    let mut edges = BTreeMap::<(usize, usize), Vec<(usize, bool)>>::new();
    for (i, t) in mesh.triangles.iter().enumerate() {
        if i % 256 == 0 && !progress(i, total) {
            return Err(fail(
                ErrorKind::Cancelled,
                "face orientation repair cancelled",
            ));
        }
        for j in 0..3 {
            let a = t[j];
            let b = t[(j + 1) % 3];
            if a == b {
                return Err(fail(
                    ErrorKind::Topology,
                    "collapsed edge cannot be orientation-repaired",
                ));
            }
            let uses = edges.entry((a.min(b), a.max(b))).or_default();
            uses.push((i, a < b));
            if uses.len() > 2 {
                return Err(fail(
                    ErrorKind::Topology,
                    "nonmanifold edge cannot be orientation-repaired",
                ));
            }
        }
    }
    let mut adjacency = vec![Vec::new(); n];
    for uses in edges.values() {
        let [(a, da), (b, db)] = uses.as_slice() else {
            return Err(fail(
                ErrorKind::Topology,
                "open edge cannot be orientation-repaired",
            ));
        };
        adjacency[*a].push((*b, da == db));
        adjacency[*b].push((*a, da == db));
    }
    let mut flips = vec![None; n];
    flips[0] = Some(false);
    let mut pending = vec![0];
    let mut visited = 0;
    while let Some(a) = pending.pop() {
        visited += 1;
        if visited % 256 == 0 && !progress(n + visited, total) {
            return Err(fail(
                ErrorKind::Cancelled,
                "face orientation repair cancelled",
            ));
        }
        for &(b, toggle) in &adjacency[a] {
            let next = flips[a].unwrap() ^ toggle;
            if let Some(old) = flips[b] {
                if old != next {
                    return Err(fail(
                        ErrorKind::Topology,
                        "nonorientable shell cannot be orientation-repaired",
                    ));
                }
            } else {
                flips[b] = Some(next);
                pending.push(b);
            }
        }
    }
    if flips.iter().any(Option::is_none) {
        return Err(fail(
            ErrorKind::Topology,
            "orientation repair requires one connected shell",
        ));
    }
    let mut face_flips = vec![None; source_faces.len()];
    for (i, &face) in mesh.face.iter().enumerate() {
        if let Some(old) = face_flips[face] {
            if Some(old) != flips[i] {
                return Err(fail(
                    ErrorKind::Topology,
                    "repair would split a source face orientation",
                ));
            }
        } else {
            face_flips[face] = flips[i];
        }
    }
    if face_flips.iter().any(Option::is_none) {
        return Err(fail(
            ErrorKind::Topology,
            "empty source face cannot be orientation-repaired",
        ));
    }
    let mut cancelled = false;
    let oriented = mesh
        .triangles
        .iter()
        .copied()
        .enumerate()
        .take_while(|(i, _)| {
            if i % 256 == 0 && !progress(2 * n + i, total) {
                cancelled = true;
                false
            } else {
                true
            }
        })
        .map(|(i, mut t)| {
            if flips[i] == Some(true) {
                t.swap(1, 2);
            }
            t
        });
    let volume = signed_volume_of(&mesh.vertices, oriented)?;
    if cancelled {
        return Err(fail(
            ErrorKind::Cancelled,
            "face orientation repair cancelled",
        ));
    }
    if !volume.is_finite() || volume == 0. {
        return Err(fail(
            ErrorKind::Geometry,
            "orientation repair cannot choose a zero/nonfinite shell volume",
        ));
    }
    let global = volume < 0.;
    let mut changes = Vec::new();
    for (i, flip) in face_flips.into_iter().enumerate() {
        if flip.unwrap() ^ global {
            changes.push(FaceOrientationChange {
                source_face: source_faces[i],
                before_same_sense: senses[i],
                after_same_sense: !senses[i],
            });
        }
    }
    if !progress(total, total) {
        return Err(fail(
            ErrorKind::Cancelled,
            "face orientation repair cancelled",
        ));
    }
    // Commit only after all constraints and cancellation checks passed.
    for (i, t) in mesh.triangles.iter_mut().enumerate() {
        if flips[i].unwrap() ^ global {
            t.swap(1, 2);
        }
    }
    Ok(changes)
}
#[cfg(test)]
mod tests {
    use super::*;
    fn tetra() -> SurfaceMesh {
        SurfaceMesh {
            vertices: vec![[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]],
            triangles: vec![[1, 2, 3], [0, 3, 2], [0, 1, 3], [0, 2, 1]],
            face: vec![0, 1, 2, 3],
        }
    }
    #[test]
    fn whole_face_and_global_repairs_are_reported_without_moving_points() {
        let original = tetra();
        let mut mesh = original.clone();
        mesh.triangles[1].swap(1, 2);
        let changes = repair(
            &mut mesh,
            &[10, 20, 30, 40],
            &[true, false, true, true],
            |_, _| true,
        )
        .unwrap();
        assert_eq!(mesh, original);
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].source_face, 20);
        assert!(!changes[0].before_same_sense && changes[0].after_same_sense);
        for t in &mut mesh.triangles {
            t.swap(1, 2);
        }
        let changes = repair(&mut mesh, &[10, 20, 30, 40], &[false; 4], |_, _| true).unwrap();
        assert_eq!(changes.len(), 4);
        assert_eq!(mesh, original);
    }
    #[test]
    fn ambiguous_nonmanifold_and_cancelled_repairs_are_atomic() {
        let mut mesh = tetra();
        mesh.triangles[1].swap(1, 2);
        mesh.face.fill(0);
        let before = mesh.clone();
        assert!(repair(&mut mesh, &[10], &[true], |_, _| true)
            .unwrap_err()
            .detail
            .contains("split a source face"));
        assert_eq!(mesh, before);
        let mut mesh = tetra();
        let before = mesh.clone();
        assert_eq!(
            repair(&mut mesh, &[10, 20, 30, 40], &[true; 4], |done, _| done < 4)
                .unwrap_err()
                .kind,
            ErrorKind::Cancelled
        );
        assert_eq!(mesh, before);
        mesh.triangles.push(mesh.triangles[0]);
        mesh.face.push(0);
        let before = mesh.clone();
        assert!(
            repair(&mut mesh, &[10, 20, 30, 40], &[true; 4], |_, _| true)
                .unwrap_err()
                .detail
                .contains("nonmanifold")
        );
        assert_eq!(mesh, before);
    }
    #[test]
    fn nonorientable_closed_topology_is_not_repaired() {
        let mut mesh = SurfaceMesh {
            vertices: vec![[0.; 3]; 6],
            triangles: vec![
                [1, 2, 3],
                [1, 2, 4],
                [1, 3, 5],
                [1, 4, 6],
                [1, 5, 6],
                [2, 3, 6],
                [2, 4, 5],
                [2, 5, 6],
                [3, 4, 5],
                [3, 4, 6],
            ]
            .into_iter()
            .map(|t| t.map(|i| i - 1))
            .collect(),
            face: (0..10).collect(),
        };
        let before = mesh.clone();
        assert!(repair(
            &mut mesh,
            &(0..10).collect::<Vec<_>>(),
            &[true; 10],
            |_, _| true
        )
        .unwrap_err()
        .detail
        .contains("nonorientable"));
        assert_eq!(mesh, before);
    }
}
