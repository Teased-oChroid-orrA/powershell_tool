//! Nested dissection on a mesh of disconnected bodies (regression: an empty separator between two
//! disjoint halves used to stop the recursion and leave the whole mesh in natural order).

use fea_core::generate::grid;
use fea_core::linear::{Ordering, Reduced};
use fea_core::*;

const MAT: Elastic = Elastic { e: 10.3e6, nu: 0.33, alpha: 0.0, aniso: None };

fn nd_fill(mesh: Mesh) -> usize {
    let model = Model::new(mesh).unwrap();
    let bc = model.dirichlet();
    Reduced::with_ordering(&model.pattern, &bc, Some(&model.mesh.nodes), Ordering::NestedDissection).unwrap().factor_nnz()
}

#[test]
fn nested_dissection_recurses_into_disconnected_bodies() {
    let div = [10, 10, 10];
    let one = grid(Physics::Solid, ElementKind::Hex8, MAT, div, &|p| p).unwrap();
    let single = nd_fill(one.clone());
    // Second copy shifted along x past the first: the first bisection falls in the gap.
    let mut two = one.clone();
    let offset = two.nodes.len();
    let xmax = one.nodes.iter().map(|p| p[0]).fold(f64::MIN, f64::max);
    let xmin = one.nodes.iter().map(|p| p[0]).fold(f64::MAX, f64::min);
    let span = xmax - xmin;
    two.nodes.extend(one.nodes.iter().map(|p| [p[0] + 1.5 * span, p[1], p[2]]));
    let mut b = one.blocks[0].clone();
    b.conn.iter_mut().for_each(|c| *c += offset);
    b.name = format!("{}_copy", b.name);
    two.blocks.push(b);
    let double = nd_fill(two);
    let ratio = double as f64 / single as f64;
    assert!((1.8..2.3).contains(&ratio), "two disjoint blocks: fill {double} vs one block {single} (ratio {ratio:.2})");
}
