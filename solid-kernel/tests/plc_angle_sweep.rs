//! Where does constrained recovery stop terminating as the smallest input angle shrinks? A sweep over two generic families
//! (an isosceles triangular prism with apex angle theta; a square slab of falling thickness), each run with the shipped
//! options and with the missing-only heuristic. The registry's gate (`NATIVE_MIN_SHELL_ANGLE_DEG`) is an evidence bound; this
//! measures the boundary on inputs that are not the NEMA17 bracket. Results: `docs/issue-46-phase-83.md`.
//! `cargo test -p solid-kernel --no-default-features --release --test plc_angle_sweep -- --ignored --nocapture`

use solid_kernel::{
    plc_conform::{conform, min_shell_angle_deg, Options},
    V3,
};

fn sub(a: V3, b: V3) -> V3 {
    [a[0] - b[0], a[1] - b[1], a[2] - b[2]]
}
fn cross(a: V3, b: V3) -> V3 {
    [a[1] * b[2] - a[2] * b[1], a[2] * b[0] - a[0] * b[2], a[0] * b[1] - a[1] * b[0]]
}

/// Orient every facet away from the centroid (valid for convex inputs).
fn outward(v: &[V3], mut t: Vec<[usize; 3]>) -> Vec<[usize; 3]> {
    let c: V3 = std::array::from_fn(|k| v.iter().map(|p| p[k]).sum::<f64>() / v.len() as f64);
    for f in &mut t {
        let n = cross(sub(v[f[1]], v[f[0]]), sub(v[f[2]], v[f[0]]));
        let d = sub(v[f[0]], c);
        if n[0] * d[0] + n[1] * d[1] + n[2] * d[2] < 0.0 {
            f.swap(1, 2);
        }
    }
    t
}

/// Isosceles triangle with apex angle `theta` (degrees) and legs of length 4, extruded by 2.
fn prism(theta_deg: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let h = theta_deg.to_radians() / 2.0;
    let tri = [[0.0, 0.0], [4.0 * h.sin(), 4.0 * h.cos()], [-4.0 * h.sin(), 4.0 * h.cos()]];
    let mut v = Vec::new();
    for z in [0.0, 2.0] {
        for p in tri {
            v.push([p[0], p[1], z]);
        }
    }
    let mut t = vec![[0, 1, 2], [3, 4, 5]];
    for i in 0..3 {
        let j = (i + 1) % 3;
        t.push([i, j, 3 + j]);
        t.push([i, 3 + j, 3 + i]);
    }
    let t = outward(&v, t);
    (v, t)
}

fn slab(thickness: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let mut v = Vec::new();
    for z in [0.0, thickness] {
        for p in [[0.0, 0.0], [4.0, 0.0], [4.0, 4.0], [0.0, 4.0]] {
            v.push([p[0], p[1], z]);
        }
    }
    let mut t = vec![[0, 1, 2], [0, 2, 3], [4, 5, 6], [4, 6, 7]];
    for i in 0..4 {
        let j = (i + 1) % 4;
        t.push([i, j, 4 + j]);
        t.push([i, 4 + j, 4 + i]);
    }
    let t = outward(&v, t);
    (v, t)
}

/// A slab whose top and bottom are `n x n` grids (all cells split along the same diagonal) and whose walls are one layer of
/// `n` quads, as a surface mesh of a thin plate looks.
fn slab_grid(n: usize, thickness: f64) -> (Vec<V3>, Vec<[usize; 3]>) {
    let id = |layer: usize, i: usize, j: usize| layer * (n + 1) * (n + 1) + j * (n + 1) + i;
    let mut v = Vec::new();
    for z in [0.0, thickness] {
        for j in 0..=n {
            for i in 0..=n {
                v.push([4.0 * i as f64 / n as f64, 4.0 * j as f64 / n as f64, z]);
            }
        }
    }
    let mut t = Vec::new();
    for layer in 0..2 {
        for j in 0..n {
            for i in 0..n {
                t.push([id(layer, i, j), id(layer, i + 1, j), id(layer, i + 1, j + 1)]);
                t.push([id(layer, i, j), id(layer, i + 1, j + 1), id(layer, i, j + 1)]);
            }
        }
    }
    let ring: Vec<(usize, usize)> = (0..n).map(|i| (i, 0)).chain((0..n).map(|j| (n, j))).chain((0..n).map(|i| (n - i, n))).chain((0..n).map(|j| (0, n - j))).collect();
    for (k, &(i, j)) in ring.iter().enumerate() {
        let (i2, j2) = ring[(k + 1) % ring.len()];
        let _ = (i2, j2);
        let next = {
            let (a, b) = ring[(k + 1) % ring.len()];
            (a, b)
        };
        t.push([id(0, i, j), id(0, next.0, next.1), id(1, next.0, next.1)]);
        t.push([id(0, i, j), id(1, next.0, next.1), id(1, i, j)]);
    }
    let t = outward(&v, t);
    (v, t)
}

fn run_with(name: &str, v: &[V3], t: &[[usize; 3]], radius_edge: Option<f64>) {
    let angle = min_shell_angle_deg(v, t);
    for (label, protect) in [("shipped", true), ("missing-only", false)] {
        let opts = Options { protect_encroached: protect, max_radius_edge: radius_edge, ..Options::default() };
        let start = std::time::Instant::now();
        let r = conform(v, t, opts, |_| true);
        let ms = start.elapsed().as_secs_f64() * 1e3;
        match r {
            Ok(c) => println!("{name:<22} min angle {angle:>6.2} deg {label:<12}: recovered, {} steiner (+{} quality), {} cells, {ms:.0} ms", c.report.segment_steiner + c.report.facet_steiner, c.report.quality_steiner, c.cells.len()),
            Err(e) => println!("{name:<22} min angle {angle:>6.2} deg {label:<12}: {:?} ({ms:.0} ms)", e.kind),
        }
    }
}

fn run(name: &str, v: &[V3], t: &[[usize; 3]]) {
    let angle = min_shell_angle_deg(v, t);
    for (label, protect) in [("shipped", true), ("missing-only", false)] {
        let opts = Options { protect_encroached: protect, ..Options::default() };
        let start = std::time::Instant::now();
        let r = conform(v, t, opts, |_| true);
        let ms = start.elapsed().as_secs_f64() * 1e3;
        match r {
            Ok(c) => println!("{name:<18} min angle {angle:>6.2} deg {label:<12}: recovered, {} steiner, {} cells, {ms:.0} ms", c.report.segment_steiner + c.report.facet_steiner, c.cells.len()),
            Err(e) => println!("{name:<18} min angle {angle:>6.2} deg {label:<12}: {:?} ({ms:.0} ms)", e.kind),
        }
    }
}

#[test]
#[ignore = "sweep: run with --release --ignored --nocapture"]
fn the_termination_boundary_over_two_input_families() {
    for theta in [60.0, 40.0, 30.0, 25.0, 20.0, 18.0, 16.0, 14.0, 12.0, 10.0, 8.0, 6.0, 4.0] {
        let (v, t) = prism(theta);
        run(&format!("prism {theta}"), &v, &t);
    }
    for thickness in [4.0, 2.0, 1.0, 0.6, 0.4, 0.25, 0.15, 0.08, 0.04] {
        let (v, t) = slab(thickness);
        run(&format!("slab t={thickness}"), &v, &t);
    }
}

#[test]
#[ignore = "sweep: run with --release --ignored --nocapture"]
fn surface_mesh_like_inputs_with_and_without_quality_refinement() {
    for n in [4usize, 8] {
        for thickness in [2.0, 1.0, 0.5, 0.25, 0.1] {
            let (v, t) = slab_grid(n, thickness);
            run_with(&format!("grid {n}x{n} t={thickness}"), &v, &t, None);
            run_with(&format!("grid {n}x{n} t={thickness} re2"), &v, &t, Some(2.0));
        }
    }
}

#[test]
#[ignore = "sweep: run with --release --ignored --nocapture"]
fn the_quality_refinement_boundary_between_fourteen_and_twenty_seven_degrees() {
    for n in [6usize, 8, 10] {
        let h = 4.0 / n as f64;
        for deg in [14.0f64, 16.0, 18.0, 20.0, 22.0, 24.0] {
            let (v, t) = slab_grid(n, h * deg.to_radians().tan());
            run_with(&format!("grid {n}x{n} wall {deg}"), &v, &t, Some(2.0));
        }
    }
}

/// Fast pins of the sweep's two findings (the full tables are the ignored tests above).
#[test]
fn recovery_alone_terminates_at_small_angles_but_quality_refinement_needs_a_larger_one() {
    // 5.7 degrees between wall and face of a surface-mesh-like slab: plain recovery still terminates.
    let (v, t) = slab_grid(4, 0.1);
    assert!(min_shell_angle_deg(&v, &t) < 6.0);
    assert!(conform(&v, &t, Options::default(), |_| true).is_ok());
    // 24 degrees: the radius-edge-2 refinement terminates as well.
    let (v, t) = slab_grid(8, 0.5 * 24f64.to_radians().tan());
    let opts = Options { max_radius_edge: Some(2.0), ..Options::default() };
    assert!(conform(&v, &t, opts, |_| true).is_ok());
}
