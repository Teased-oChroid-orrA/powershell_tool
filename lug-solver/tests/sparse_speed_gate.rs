//! Speed gate for the lug solver's own sparse Cholesky against the general kernel's faer-based one,
//! on the same reduced matrix (ignored: timings, not assertions). It lives here because `lug-solver`
//! depends on `fea-core` (the kernel bridge, `src/fea.rs`), not the other way round.
//!
//! `cargo test -p lug-solver --release --test sparse_speed_gate -- --ignored --nocapture`

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use std::time::{Duration, Instant};

fn best<T>(n: usize, mut f: impl FnMut() -> T) -> (Duration, T) {
    let mut out = None;
    let mut min = Duration::MAX;
    for _ in 0..n {
        let t = Instant::now();
        let v = f();
        min = min.min(t.elapsed());
        out = Some(v);
    }
    (min, out.unwrap())
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1e3
}

/// Speed gate: faer supernodal Cholesky against lug-solver's own nested-dissection supernodal one
/// on the same reduced matrix.
#[test]
#[ignore]
fn faer_against_lug_solver_cholesky() {
    use fea_core::generate::grid;
    use fea_core::linear::Reduced;
    use fea_core::{ElementKind, Elastic, Model, Physics};
    use std::sync::Arc;
    use lug_solver::sparse::{nested_dissection, Cholesky, Pattern, SparseSym, Symbolic};
    for n in [20usize, 50, 100] {
        let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, ElementKind::Quad9, Elastic::new(10.3e6, 0.33), [n, n, 1], &|p| p).unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for &nd in model.mesh.node_set("u0").unwrap() {
            bc.fix(nd, 0, 0.0);
        }
        for &nd in model.mesh.node_set("v0").unwrap() {
            bc.fix(nd, 1, 0.0);
        }
        let k = model.assemble().unwrap();
        let red = Reduced::new(&model.pattern, &bc).unwrap();
        let (t_faer, _) = best(3, || red.factor(&k).unwrap());
        let t_faer = ms(t_faer);
        let (cp, ri, vals) = red.csc(&k);
        let free = red.free_dofs();
        // lug-solver wants a node graph for the ordering and works on dof indices 0..ndof.
        let nn = model.mesh.nodes.len();
        let coords: Vec<[f64; 2]> = model.mesh.nodes.iter().map(|x| [x[0], x[1]]).collect();
        let mut adj: Vec<Vec<usize>> = vec![Vec::new(); nn];
        for j in 0..nn {
            for q in model.pattern.col_ptr[j]..model.pattern.col_ptr[j + 1] {
                let i = model.pattern.row_idx[q] as usize;
                if i != j {
                    adj[i].push(j);
                    adj[j].push(i);
                }
            }
        }
        let t0 = Instant::now();
        let node_order = nested_dissection(&coords, &adj);
        // Reduced numbering of the free dofs.
        let m = free.len();
        let mut new_of = vec![usize::MAX; nn * 2];
        for (r, &dof) in free.iter().enumerate() {
            new_of[dof as usize] = r;
        }
        let order: Vec<usize> = node_order.iter().flat_map(|&nd| (0..2).map(move |c| 2 * nd + c)).filter_map(|dof| (new_of[dof] != usize::MAX).then_some(new_of[dof])).collect();
        let mut pairs = Vec::new();
        for c in 0..m {
            for q in cp[c] as usize..cp[c + 1] as usize {
                pairs.push((ri[q] as usize, c));
            }
        }
        let pat = Arc::new(Pattern::new(m, &order, pairs.iter().copied()));
        let sym = Arc::new(Symbolic::new(&pat));
        let t_setup = ms(t0.elapsed());
        let mut a = SparseSym::zeros(&pat);
        for c in 0..m {
            for q in cp[c] as usize..cp[c + 1] as usize {
                a.add(ri[q] as usize, c, vals[q]);
            }
        }
        let (t_lug, f) = best(3, || Cholesky::factor(&a, &sym));
        assert!(f.is_some());
        eprintln!("BENCH gate Quad9 {n}x{n}: faer factor {t_faer:.2} ms | lug-solver factor {:.2} ms (ordering+symbolic {t_setup:.1} ms) | factor nnz {} flops {:.2e}", ms(t_lug), sym.nnz(), sym.flops());
    }
}

