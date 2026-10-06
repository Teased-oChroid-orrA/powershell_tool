//! Gmsh and Abaqus import: node orders built from each format's own edge / face definitions,
//! rejection of a wrong order, and a full round trip through a Gmsh 4.1 file.

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::import::{read_abaqus, read_gmsh, ImportOptions};
use fea_core::sweep::extrude;
use fea_core::*;
use std::fmt::Write;

const E: f64 = 10.0e6;

fn opts() -> ImportOptions {
    ImportOptions::new(Physics::PlaneStress { thickness: 1.0 }, Elastic::new(E, 0.3))
}

/// An affine skew so no element is a box.
fn skew(p: [f64; 3]) -> [f64; 3] {
    [2.0 * p[0] + 0.3 * p[1] + 0.1 * p[2], 0.2 * p[0] + 1.5 * p[1] + 0.2 * p[2], 0.1 * p[0] - 0.1 * p[1] + 1.2 * p[2]]
}

const HEX_CORNERS: [[f64; 3]; 8] = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [1., 1., 1.], [0., 1., 1.]];

/// Gmsh's own definition of the second-order node positions of one element: corners, then the
/// edge midpoints (pairs of corner indices, in Gmsh order), then face centres and the body centre.
fn gmsh_positions(corners: &[[f64; 3]], edges: &[(usize, usize)], faces: &[&[usize]], centre: bool) -> Vec<[f64; 3]> {
    let mid = |ids: &[usize]| -> [f64; 3] { std::array::from_fn(|i| ids.iter().map(|&k| corners[k][i]).sum::<f64>() / ids.len() as f64) };
    let mut v: Vec<[f64; 3]> = corners.to_vec();
    v.extend(edges.iter().map(|&(a, b)| mid(&[a, b])));
    v.extend(faces.iter().map(|f| mid(f)));
    if centre {
        v.push(mid(&(0..corners.len()).collect::<Vec<_>>()));
    }
    v
}

fn msh41_single(ty: u32, dim: usize, pts: &[[f64; 3]]) -> String {
    let mut s = String::from("$MeshFormat\n4.1 0 8\n$EndMeshFormat\n");
    let n = pts.len();
    let _ = writeln!(s, "$Nodes\n1 {n} 1 {n}\n{dim} 1 0 {n}");
    for k in 1..=n {
        let _ = writeln!(s, "{k}");
    }
    for p in pts {
        let _ = writeln!(s, "{} {} {}", p[0], p[1], p[2]);
    }
    let _ = writeln!(s, "$EndNodes\n$Elements\n1 1 1 1\n{dim} 1 {ty} 1");
    let ids: Vec<String> = (1..=n).map(|k| k.to_string()).collect();
    let _ = writeln!(s, "1 {}\n$EndElements", ids.join(" "));
    s
}

fn assert_straight_sided(m: &Mesh) {
    for blk in &m.blocks {
        let kind = blk.kind;
        let lin = match (kind.dim(), kind.is_simplex()) {
            (3, true) => ElementKind::Tet4,
            (3, false) => ElementKind::Hex8,
            (_, true) => ElementKind::Tri3,
            _ => ElementKind::Quad4,
        };
        for conn in blk.conn.chunks_exact(kind.n_nodes()) {
            for (k, xi) in kind.node_coords().iter().enumerate() {
                let (n, _) = lin.shape(*xi);
                let want: [f64; 3] = std::array::from_fn(|i| (0..kind.n_corners()).map(|a| n[a] * m.nodes[conn[a]][i]).sum());
                assert!((0..3).all(|i| (m.nodes[conn[k]][i] - want[i]).abs() < 1e-12), "{kind:?} node {k}");
            }
        }
    }
}

#[test]
fn gmsh_second_order_orders_follow_gmsh_definitions() {
    // Tet10 (Gmsh type 11): edge nodes 4..9 on (0,1) (1,2) (2,0) (3,0) (3,2) (3,1).
    let tet = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let p = gmsh_positions(&tet.map(skew), &[(0, 1), (1, 2), (2, 0), (3, 0), (3, 2), (3, 1)], &[], false);
    let m = read_gmsh(&msh41_single(11, 3, &p), &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Tet10);
    assert_straight_sided(&m);
    Model::new(m).unwrap().assemble().unwrap();
    // Hex20 (type 17): edges (0,1) (0,3) (0,4) (1,2) (1,5) (2,3) (2,6) (3,7) (4,5) (4,7) (5,6) (6,7).
    let corners = HEX_CORNERS.map(skew);
    let e20 = [(0, 1), (0, 3), (0, 4), (1, 2), (1, 5), (2, 3), (2, 6), (3, 7), (4, 5), (4, 7), (5, 6), (6, 7)];
    let m = read_gmsh(&msh41_single(17, 3, &gmsh_positions(&corners, &e20, &[], false)), &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Hex20);
    assert_straight_sided(&m);
    Model::new(m).unwrap().assemble().unwrap();
    // Hex27 (type 12): faces (0,1,2,3) (0,1,5,4) (0,3,7,4) (1,2,6,5) (2,3,7,6) (4,5,6,7), then the centre.
    let faces: [&[usize]; 6] = [&[0, 1, 2, 3], &[0, 1, 5, 4], &[0, 3, 7, 4], &[1, 2, 6, 5], &[2, 3, 7, 6], &[4, 5, 6, 7]];
    let m = read_gmsh(&msh41_single(12, 3, &gmsh_positions(&corners, &e20, &faces, true)), &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Hex27);
    assert_straight_sided(&m);
    Model::new(m).unwrap().assemble().unwrap();
    // 2D second order: Quad9 (type 10) is corners, edges (0,1) (1,2) (2,3) (3,0), centre; Tri6 (type 9) edges (0,1) (1,2) (2,0).
    let quad = [[0., 0., 0.], [1., 0., 0.], [1., 1., 0.], [0., 1., 0.]].map(|p| [2.0 * p[0] + 0.2 * p[1], 1.5 * p[1], 0.0]);
    let m = read_gmsh(&msh41_single(10, 2, &gmsh_positions(&quad, &[(0, 1), (1, 2), (2, 3), (3, 0)], &[], true)), &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Quad9);
    assert_straight_sided(&m);
    let tri = [[0., 0., 0.], [2., 0., 0.], [0., 1., 0.]];
    let m = read_gmsh(&msh41_single(9, 2, &gmsh_positions(&tri, &[(0, 1), (1, 2), (2, 0)], &[], false)), &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Tri6);
    assert_straight_sided(&m);
}

#[test]
fn a_wrong_node_order_is_rejected_not_silently_distorted() {
    // A Tet10 written in VTK order (edges (0,3) (1,3) (2,3) last) is Gmsh's order with 8 and 9 swapped.
    let tet = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let p = gmsh_positions(&tet, &[(0, 1), (1, 2), (2, 0), (3, 0), (1, 3), (2, 3)], &[], false);
    let err = read_gmsh(&msh41_single(11, 3, &p), &opts()).unwrap_err();
    assert!(err.contains("node ordering mismatch"), "{err}");
    // A prism has no element in the library.
    let prism = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.], [1., 0., 1.], [0., 1., 1.]];
    assert!(read_gmsh(&msh41_single(6, 3, &prism), &opts()).unwrap_err().contains("unsupported element type 6"));
}

/// Write a mesh as Gmsh 4.1 ASCII: node tags 1.., one volume entity, and one surface entity per named
/// surface (each face written as a Gmsh element of the matching order), with physical names.
fn write_gmsh41(m: &Mesh, vol: &str, surfaces: &[(&str, Vec<Vec<usize>>)]) -> String {
    let (kind, perm) = match m.blocks[0].kind {
        ElementKind::Hex27 => (12, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 11, 13, 9, 16, 18, 19, 17, 10, 12, 14, 15, 22, 23, 21, 24, 20, 25, 26]),
        ElementKind::Hex8 => (5, (0..8).collect()),
        k => panic!("{k:?}"),
    };
    let mut s = String::from("$MeshFormat\n4.1 0 8\n$EndMeshFormat\n$PhysicalNames\n");
    let _ = writeln!(s, "{}", 1 + surfaces.len());
    let _ = writeln!(s, "3 1 \"{vol}\"");
    for (i, (n, _)) in surfaces.iter().enumerate() {
        let _ = writeln!(s, "2 {} \"{n}\"", 2 + i);
    }
    s.push_str("$EndPhysicalNames\n");
    let _ = writeln!(s, "$Entities\n0 0 {} 1", surfaces.len());
    for i in 0..surfaces.len() {
        let _ = writeln!(s, "{} -100 -100 -100 100 100 100 1 {} 0", 1 + i, 2 + i);
    }
    let _ = writeln!(s, "1 -100 -100 -100 100 100 100 1 1 0\n$EndEntities");
    let n = m.nodes.len();
    let _ = writeln!(s, "$Nodes\n1 {n} 1 {n}\n3 1 0 {n}");
    for k in 1..=n {
        let _ = writeln!(s, "{k}");
    }
    for p in &m.nodes {
        let _ = writeln!(s, "{:e} {:e} {:e}", p[0], p[1], p[2]);
    }
    s.push_str("$EndNodes\n");
    let nel = m.n_elems() + surfaces.iter().map(|x| x.1.len()).sum::<usize>();
    let _ = writeln!(s, "$Elements\n{} {nel} 1 {nel}", 1 + surfaces.len());
    let _ = writeln!(s, "3 1 {kind} {}", m.n_elems());
    let mut tag = 1;
    for c in m.blocks[0].conn.chunks_exact(m.blocks[0].kind.n_nodes()) {
        let mut file = vec![0usize; c.len()];
        for (k, &p) in perm.iter().enumerate() {
            file[p] = c[k] + 1;
        }
        let _ = writeln!(s, "{tag} {}", file.iter().map(|x| x.to_string()).collect::<Vec<_>>().join(" "));
        tag += 1;
    }
    for (i, (_, faces)) in surfaces.iter().enumerate() {
        let fk = if faces[0].len() == 9 { 10 } else { 3 };
        let _ = writeln!(s, "2 {} {fk} {}", 1 + i, faces.len());
        for f in faces {
            // Written with the opposite orientation to prove the importer does not trust the surface
            // element's orientation: reverse the corner cycle and remap the edge nodes to match.
            let order: [usize; 9] = if f.len() == 9 { [0, 3, 2, 1, 7, 6, 5, 4, 8] } else { [0, 3, 2, 1, 0, 0, 0, 0, 0] };
            let rev: Vec<String> = order.iter().take(f.len()).map(|&k| (f[k] + 1).to_string()).collect();
            let _ = writeln!(s, "{tag} {}", rev.join(" "));
            tag += 1;
        }
    }
    s.push_str("$EndElements\n");
    s
}

#[test]
fn a_gmsh_file_round_trips_a_hex27_bar_with_surfaces_and_solves_identically() {
    let m2 = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(E, 0.3), [3, 2, 1], &|p| [3.0 * p[0], 1.0 * p[1], 0.0]).unwrap();
    let native = extrude(&m2, &[0.0, 0.5, 1.0]).unwrap();
    let text = write_gmsh41(&native, "bar", &[("fixed", native.surfaces["start"].clone()), ("loaded", native.surfaces["end"].clone())]);
    let m = read_gmsh(&text, &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Hex27);
    assert_eq!(m.blocks[0].name, "bar");
    assert_eq!(m.n_elems(), native.n_elems());
    assert_eq!(m.nodes.len(), native.nodes.len());
    assert_straight_sided(&m);
    assert_eq!(m.surfaces["fixed"].len(), native.surfaces["start"].len());
    assert!(m.surfaces["fixed"].iter().all(|f| f.len() == 9));
    // Surfaces come back in load orientation whatever the file's element orientation: a pressure on
    // the end face gives the same total force as the native mesh's surface.
    let force = |mesh: &Mesh, name: &str| -> [f64; 3] {
        let loads = Loads { faces: mesh.surfaces[name].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(1.0))).collect(), ..Loads::default() };
        let f = fea_core::loads::assemble(mesh, &loads).unwrap();
        (0..mesh.nodes.len()).fold([0.0; 3], |a, n| [a[0] + f[n * 3], a[1] + f[n * 3 + 1], a[2] + f[n * 3 + 2]])
    };
    let (fa, fb) = (force(&native, "end"), force(&m, "loaded"));
    for i in 0..3 {
        assert!((fa[i] - fb[i]).abs() < 1e-12, "end force {fa:?} vs {fb:?}");
    }
    assert!(fb[2] < -2.9, "pressure on the +z end face pushes in -z: {fb:?}");
    // Same boundary value problem on both meshes: clamp the fixed surface, push on the loaded one.
    let solve = |mesh: &Mesh, fixed: &str, loaded: &str| -> Vec<f64> {
        let model = Model::new(mesh.clone()).unwrap();
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set(fixed).unwrap() {
            bc.fix_node(n);
        }
        let loads = Loads { faces: model.mesh.surfaces[loaded].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(500.0))).collect(), ..Loads::default() };
        model.solve_static(&loads, &bc).unwrap().u
    };
    let mut native2 = native.clone();
    native2.node_sets.insert("fixed".into(), native.node_sets["start"].clone());
    native2.surfaces.insert("loaded".into(), native.surfaces["end"].clone());
    let (ua, ub) = (solve(&native2, "fixed", "loaded"), solve(&m, "fixed", "loaded"));
    let scale = ua.iter().fold(0.0f64, |a, v| a.max(v.abs()));
    let diff = ua.iter().zip(&ub).fold(0.0f64, |a, (x, y)| a.max((x - y).abs()));
    assert!(diff < 1e-12 * scale, "round-trip solution differs by {diff:e} of {scale:e}");
}

#[test]
fn gmsh_2_2_with_physical_names_builds_a_2d_mesh_with_boundary_edges() {
    // Two triangles forming a unit square; boundary line 'left' (nodes 1-4) and the whole domain named 'plate'.
    let text = "$MeshFormat\n2.2 0 8\n$EndMeshFormat\n$PhysicalNames\n2\n1 5 \"left\"\n2 6 \"plate\"\n$EndPhysicalNames\n\
                $Nodes\n4\n1 0 0 0\n2 1 0 0\n3 1 1 0\n4 0 1 0\n$EndNodes\n\
                $Elements\n3\n1 1 2 5 1 4 1\n2 2 2 6 1 1 2 3\n3 2 2 6 1 1 3 4\n$EndElements\n";
    let m = read_gmsh(text, &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Tri3);
    assert_eq!(m.blocks[0].name, "plate");
    assert_eq!(m.nodes.len(), 4);
    assert_eq!(m.node_sets["left"].len(), 2);
    assert_eq!(m.surfaces["left"].len(), 1);
    // The boundary edge is oriented with the body on its left: (0,1) -> (0,0) running down the left side.
    let f = &m.surfaces["left"][0];
    assert!(m.nodes[f[0]][1] > m.nodes[f[1]][1], "{f:?}");
    assert!((Model::new(m).unwrap().assemble().is_ok()));
}

#[test]
fn abaqus_2d_and_3d_with_continuation_sets_and_materials() {
    // CPS8 plate, one element; E and nu from the material, thickness from the section, a generated node set.
    let inp = "*HEADING\nplate\n*NODE\n 1, 0., 0.\n 2, 2., 0.\n 3, 2., 1.\n 4, 0., 1.\n 5, 1., 0.\n 6, 2., 0.5\n 7, 1., 1.\n 8, 0., 0.5\n\
*ELEMENT, TYPE=CPS8R, ELSET=PLATE\n 1, 1, 2, 3, 4, 5, 6, 7, 8\n\
*NSET, NSET=LEFT, GENERATE\n 4, 4, 1\n*NSET, NSET=BOTH\n LEFT, 1\n\
*MATERIAL, NAME=STEEL\n*ELASTIC\n 29.0e6, 0.31\n*EXPANSION\n 6.5e-6\n\
*SOLID SECTION, ELSET=PLATE, MATERIAL=STEEL\n 0.25,\n*STEP\n*END STEP\n";
    let m = read_abaqus(inp, &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Quad8);
    assert!(matches!(m.physics, Physics::PlaneStress { thickness } if (thickness - 0.25).abs() < 1e-15));
    assert!((m.blocks[0].material.e - 29.0e6).abs() < 1.0 && (m.blocks[0].material.nu - 0.31).abs() < 1e-15 && (m.blocks[0].material.alpha - 6.5e-6).abs() < 1e-18);
    assert_eq!(m.blocks[0].name, "PLATE");
    assert_eq!(m.node_sets["LEFT"].len(), 1);
    assert_eq!(m.node_sets["BOTH"].len(), 2, "a set listing another set's name includes its members");
    // C3D20: 21 numbers split over two data lines, Abaqus' own corner / edge definition.
    let corners = HEX_CORNERS.map(skew);
    let e20 = [(0, 1), (1, 2), (2, 3), (3, 0), (4, 5), (5, 6), (6, 7), (7, 4), (0, 4), (1, 5), (2, 6), (3, 7)];
    let p = gmsh_positions(&corners, &e20, &[], false);
    let mut inp = String::from("*NODE\n");
    for (k, x) in p.iter().enumerate() {
        let _ = writeln!(inp, "{}, {}, {}, {}", k + 1, x[0], x[1], x[2]);
    }
    inp.push_str("*ELEMENT, TYPE=C3D20\n1, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15,\n16, 17, 18, 19, 20\n");
    let m = read_abaqus(&inp, &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Hex20);
    assert_straight_sided(&m);
    // C3D10: Abaqus tet edges (1,2) (2,3) (3,1) (1,4) (2,4) (3,4).
    let tet = [[0., 0., 0.], [1., 0., 0.], [0., 1., 0.], [0., 0., 1.]];
    let p = gmsh_positions(&tet.map(skew), &[(0, 1), (1, 2), (2, 0), (0, 3), (1, 3), (2, 3)], &[], false);
    let mut inp = String::from("*NODE\n");
    for (k, x) in p.iter().enumerate() {
        let _ = writeln!(inp, "{}, {}, {}, {}", k + 1, x[0], x[1], x[2]);
    }
    inp.push_str("*ELEMENT, TYPE=C3D10\n7, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10\n");
    let m = read_abaqus(&inp, &opts()).unwrap();
    assert_eq!(m.blocks[0].kind, ElementKind::Tet10);
    assert_straight_sided(&m);
}

#[test]
fn abaqus_errors_are_reported() {
    assert!(read_abaqus("*NODE\n1,0,0\n*ELEMENT, TYPE=WEDGE6\n1,1,1,1,1,1,1\n", &opts()).unwrap_err().contains("unsupported Abaqus element type"));
    let missing = "*NODE\n1,0,0\n2,1,0\n3,0,1\n*ELEMENT, TYPE=CPS3, ELSET=A\n1,1,2,3\n*SOLID SECTION, ELSET=A, MATERIAL=NOPE\n";
    assert!(read_abaqus(missing, &opts()).unwrap_err().contains("no *ELASTIC"));
    assert!(read_abaqus("*NODE\n1,0,0\n", &opts()).unwrap_err().contains("no supported elements"));
    assert!(read_gmsh("$MeshFormat\n4.1 1 8\n$EndMeshFormat\n", &opts()).unwrap_err().contains("binary"));
}

#[test]
fn faces_can_be_selected_from_an_imported_node_set() {
    let inp = "*NODE\n1,0,0,0\n2,1,0,0\n3,0,1,0\n4,0,0,1\n*ELEMENT, TYPE=C3D4\n1,1,2,3,4\n*NSET, NSET=BASE\n1,2,3\n";
    let mut m = read_abaqus(inp, &opts()).unwrap();
    let faces = m.select_faces_by_nodes("base", "BASE").unwrap().to_vec();
    assert_eq!(faces.len(), 1);
    // The base face z = 0 of the tetrahedron, outward normal -z: pressure pushes up (+z force on the body).
    let loads = Loads { faces: vec![(faces[0].clone(), SurfaceLoad::Pressure(2.0))], ..Loads::default() };
    let f = fea_core::loads::assemble(&m, &loads).unwrap();
    let fz: f64 = (0..4).map(|n| f[n * 3 + 2]).sum();
    assert!((fz - 1.0).abs() < 1e-12, "pressure 2 on area 1/2 pushes with +1 in z: {fz}");
}
