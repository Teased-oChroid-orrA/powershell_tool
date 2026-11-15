//! A hexahedral region meeting a tetrahedral one by sharing the interface nodes (`meshing::weld::merge`): the four nodes of an
//! interface quad are shared, but the hexahedron's face is one quadrilateral while the tetrahedra's is two triangles, so the
//! union is node-conforming and not face-conforming. What does that cost? `docs/issue-46-phase-94.md`.

use fea_core::generate::grid;
use fea_core::meshing::transition::tetrahedralize;
use fea_core::meshing::weld::merge;
use fea_core::*;

fn mat() -> Elastic {
    Elastic::new(1000.0, 0.3)
}

/// A beam [0, len] x [0, 1] x [0, 1]: `nx` hexahedra along it in the left part, the right part tetrahedra from the same grid.
fn beam(nx: usize, ny: usize, len: f64, hex_fraction: f64, tets: bool) -> Mesh {
    let part = |x0: f64, x1: f64, n: usize| -> Mesh {
        grid(Physics::Solid, ElementKind::Hex8, mat(), [n, ny, ny], &move |p| [x0 + p[0] * (x1 - x0), p[1], p[2]]).unwrap()
    };
    let split = hex_fraction * len;
    let nl = ((nx as f64 * hex_fraction).round() as usize).max(1);
    let (left, right) = (part(0.0, split, nl), part(split, len, nx - nl));
    let right = if tets { tetrahedralize(&right, &|_, _, _| true).unwrap().0 } else { right };
    merge(&[left, right], 1e-9).unwrap()
}

fn tip_deflection(mesh: &Mesh, len: f64) -> f64 {
    let model = Model::new(mesh.clone()).unwrap();
    let mut bc = model.dirichlet();
    let mut tip = Vec::new();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if x[0] < 1e-9 {
            bc.fix_node(n);
        }
        if x[0] > len - 1e-9 {
            tip.push(n);
        }
    }
    let loads = Loads { nodal: tip.iter().map(|&n| (n, [0.0, -1.0 / tip.len() as f64, 0.0])).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    tip.iter().map(|&n| sol.u[n * 3 + 1]).sum::<f64>() / tip.len() as f64
}

#[test]
fn a_linear_field_passes_through_a_hex_tet_node_tie_and_bending_stays_close_to_the_all_hex_answer() {
    let len = 4.0;
    // Linear patch test: prescribe a linear displacement on the whole boundary; interior nodes (including the tied
    // interface nodes that are not on the boundary) must come out on the field.
    let mesh = beam(8, 4, len, 0.5, true);
    let model = Model::new(mesh.clone()).unwrap();
    let field = |x: [f64; 3]| [1e-3 * x[0] + 2e-4 * x[1] - 3e-4 * x[2], -1e-4 * x[0] + 5e-4 * x[1] + 2e-4 * x[2], 3e-4 * x[0] - 2e-4 * x[1] + 4e-4 * x[2]];
    let mut bc = model.dirichlet();
    let mut interior = Vec::new();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let boundary = x[0] < 1e-9 || x[0] > len - 1e-9 || x[1] < 1e-9 || x[1] > 1.0 - 1e-9 || x[2] < 1e-9 || x[2] > 1.0 - 1e-9;
        if boundary {
            let u = field(*x);
            for c in 0..3 {
                bc.fix(n, c, u[c]);
            }
        } else {
            interior.push(n);
        }
    }
    let sol = model.solve_static(&Loads::default(), &bc).unwrap();
    let on_interface = interior.iter().filter(|&&n| (model.mesh.nodes[n][0] - 2.0).abs() < 1e-9).count();
    assert!(on_interface > 0, "the interface has interior nodes");
    let worst = interior.iter().map(|&n| (0..3).map(|c| (sol.u[n * 3 + c] - field(model.mesh.nodes[n])[c]).abs()).fold(0.0, f64::max)).fold(0.0, f64::max);
    eprintln!("patch test over {} interior nodes ({on_interface} on the interface): worst error {worst:.3e}", interior.len());
    assert!(worst < 1e-10, "{worst}");
    // Bending: tip deflection of a cantilever, all-hex, hex|tet (tied), all-tet, at two resolutions.
    for (nx, ny) in [(8usize, 2usize), (16, 4)] {
        let hex = tip_deflection(&beam(nx, ny, len, 0.5, false), len);
        let mixed = tip_deflection(&beam(nx, ny, len, 0.5, true), len);
        let all_tet = tip_deflection(&beam(nx, ny, len, 1e-9, true), len);
        eprintln!("{nx}x{ny}x{ny}: tip deflection all-hex {hex:.5e}, hex|tet tie {mixed:.5e} ({:+.2} %), all-tet {all_tet:.5e} ({:+.2} %)", 100.0 * (mixed / hex - 1.0), 100.0 * (all_tet / hex - 1.0));
        // The tied mesh lies between the all-hex and all-tet answers (the tet half is the more flexible/stiff one) and the
        // interface adds little error of its own.
        let (lo, hi) = (hex.abs().min(all_tet.abs()), hex.abs().max(all_tet.abs()));
        assert!(mixed.abs() >= lo.abs() * 0.98 && mixed.abs() <= hi.abs() * 1.02, "tied {mixed} outside [{lo}, {hi}]");
    }
}
