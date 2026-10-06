//! Classical closed-form benchmarks: Timoshenko's end-loaded cantilever (parabolic shear traction
//! through a position-dependent face load), Kirsch's plate with a circular hole (stress
//! concentration 3), and the internally pressurised hollow sphere (axisymmetric).

#![allow(clippy::needless_range_loop)] // index-parallel numeric kernels read clearer as loops

use fea_core::generate::grid;
use fea_core::*;
use std::f64::consts::PI;

const E: f64 = 10.0e6;

fn pair(model: &Model, name: &str) -> Vec<usize> {
    model.mesh.node_set(name).unwrap().to_vec()
}

// ---------------------------------------------------------------- Timoshenko cantilever

/// Timoshenko & Goodier art. 21: free end `x = 0` carries the shear `P`, `x = l` is the support.
#[derive(Clone, Copy)]
struct Cantilever {
    p: f64,
    l: f64,
    c: f64,
    nu: f64,
}

impl Cantilever {
    fn i(&self) -> f64 {
        2.0 * self.c.powi(3) / 3.0
    }
    fn g(&self) -> f64 {
        E / (2.0 * (1.0 + self.nu))
    }
    fn u(&self, x: f64, y: f64) -> [f64; 2] {
        let (p, l, c, i, nu, g) = (self.p, self.l, self.c, self.i(), self.nu, self.g());
        [
            -p * x * x * y / (2.0 * E * i) - nu * p * y.powi(3) / (6.0 * E * i) + p * y.powi(3) / (6.0 * i * g) + (p * l * l / (2.0 * E * i) - p * c * c / (2.0 * i * g)) * y,
            nu * p * x * y * y / (2.0 * E * i) + p * x.powi(3) / (6.0 * E * i) - p * l * l * x / (2.0 * E * i) + p * l.powi(3) / (3.0 * E * i),
        ]
    }
    /// `(sigma_x, tau_xy)`.
    fn stress(&self, x: f64, y: f64) -> (f64, f64) {
        (-self.p * x * y / self.i(), -self.p / (2.0 * self.i()) * (self.c * self.c - y * y))
    }
}

fn cantilever_error(kind: ElementKind, div: [usize; 3]) -> (f64, f64) {
    let nu = 0.3;
    let beam = Cantilever { p: 100.0, l: 8.0, c: 1.0, nu };
    let (l, c) = (beam.l, beam.c);
    let mesh = grid(Physics::PlaneStress { thickness: 0.5 }, kind, Elastic::new(E, nu), div, &|p| [l * p[0], c * (2.0 * p[1] - 1.0), 0.0]).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in pair(&model, "u1") {
        let [ux, uy] = beam.u(model.mesh.nodes[n][0], model.mesh.nodes[n][1]);
        bc.fix(n, 0, ux);
        bc.fix(n, 1, uy);
    }
    // Traction on the free end x = 0 (outward normal -x): t = -(sigma_x, tau_xy), applied through the exact field.
    let field = FaceField::new(move |x, _n| {
        let (sx, txy) = beam.stress(x[0], x[1]);
        [-sx, -txy, 0.0]
    });
    let loads = Loads { field_faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), field.clone())).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let scale = beam.u(0.0, 0.0)[1].abs();
    let mut eu = 0.0f64;
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        let ex = beam.u(x[0], x[1]);
        eu = eu.max((sol.u[n * 2] - ex[0]).abs().max((sol.u[n * 2 + 1] - ex[1]).abs()) / scale);
    }
    // Bending stress at mid-span nodal values.
    let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
    let mut es = 0.0f64;
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        if (x[0] - 0.5 * l).abs() < 0.26 * l / div[0] as f64 && x[1].abs() < 0.9 * c {
            es = es.max((nodal[n][0] - beam.stress(x[0], x[1]).0).abs() / (beam.p * l * c / beam.i()));
        }
    }
    eprintln!("cantilever {kind:?} {div:?}: max displacement error {eu:.2e}, mid-span sigma_x error {es:.2e}");
    (eu, es)
}

#[test]
fn timoshenko_cantilever_displacement_and_bending_stress() {
    // Quadratic elements represent everything but the cubic `x^3` term of `v`.
    let (eu, es) = cantilever_error(ElementKind::Quad9, [8, 2, 1]);
    assert!(eu < 2e-4, "Quad9 displacement {eu:e}");
    assert!(es < 5e-3, "Quad9 bending stress {es:e}");
    let (eu, _) = cantilever_error(ElementKind::Quad8, [8, 2, 1]);
    assert!(eu < 3e-4, "Quad8 displacement {eu:e}");
    let (eu, _) = cantilever_error(ElementKind::Tri6, [8, 2, 1]);
    assert!(eu < 2e-3, "Tri6 displacement {eu:e}");
    // Bilinear quadrilaterals shear-lock on a coarse bending mesh, converge with refinement.
    let (coarse, _) = cantilever_error(ElementKind::Quad4, [8, 2, 1]);
    let (fine, _) = cantilever_error(ElementKind::Quad4, [32, 8, 1]);
    assert!(fine < coarse / 4.0, "Quad4 does not converge: {coarse:e} -> {fine:e}");
}

// ---------------------------------------------------------------- Kirsch plate with a hole

/// Cartesian stress of the Kirsch field (uniaxial `sigma` along `x`, hole radius `a`).
fn kirsch(sig: f64, a: f64, x: f64, y: f64) -> [f64; 3] {
    let (r, th) = (x.hypot(y), y.atan2(x));
    let (q, q2) = ((a / r).powi(2), (a / r).powi(4));
    let (c2, s2) = ((2.0 * th).cos(), (2.0 * th).sin());
    let sr = 0.5 * sig * (1.0 - q) + 0.5 * sig * (1.0 - 4.0 * q + 3.0 * q2) * c2;
    let st = 0.5 * sig * (1.0 + q) - 0.5 * sig * (1.0 + 3.0 * q2) * c2;
    let tr = -0.5 * sig * (1.0 + 2.0 * q - 3.0 * q2) * s2;
    let (c, s) = (th.cos(), th.sin());
    [sr * c * c + st * s * s - 2.0 * tr * s * c, sr * s * s + st * c * c + 2.0 * tr * s * c, (sr - st) * s * c + tr * (c * c - s * s)]
}

/// Quarter of a square plate (half side `half`) with a hole of radius `a`: `u` radial (graded
/// toward the hole), `v` angle.
fn plate_with_hole(a: f64, half: f64) -> impl Fn([f64; 3]) -> [f64; 3] {
    // Mild grading: a stronger exponent makes the Jacobian vanish at the hole edge and ruins the first ring.
    let gexp = 1.2;
    move |p| {
        let th = 0.5 * PI * p[1];
        let rho = half / th.cos().abs().max(th.sin().abs());
        let r = (1.0 - p[0].powf(gexp)) * a + p[0].powf(gexp) * rho;
        [r * th.cos(), r * th.sin(), 0.0]
    }
}

/// `(max Gauss-point stress error / sigma, max hole hoop error / 3 sigma, pole stress / sigma)`.
fn kirsch_run(kind: ElementKind, m: usize) -> (f64, f64, f64) {
    let (sig, a, half) = (1000.0, 1.0, 8.0);
    let mesh = grid(Physics::PlaneStress { thickness: 1.0 }, kind, Elastic::new(E, 0.3), [6 * m, 8 * m, 1], &plate_with_hole(a, half)).unwrap();
    let model = Model::new(mesh).unwrap();
    let mut bc = model.dirichlet();
    for n in pair(&model, "v0") {
        bc.fix(n, 1, 0.0);
    }
    for n in pair(&model, "v1") {
        bc.fix(n, 0, 0.0);
    }
    // Outer boundary (u1): exact Kirsch traction sigma . n.
    let field = FaceField::new(move |x, n| {
        let s = kirsch(sig, a, x[0], x[1]);
        [s[0] * n[0] + s[2] * n[1], s[2] * n[0] + s[1] * n[1], 0.0]
    });
    let loads = Loads { field_faces: model.mesh.surfaces["u1"].iter().map(|f| (f.clone(), field.clone())).collect(), ..Loads::default() };
    let sol = model.solve_static(&loads, &bc).unwrap();
    let mut e_gauss = 0.0f64;
    let mut at = (0.0, 0.0);
    for blk in model.gauss_stresses(&sol.u, 0.0).unwrap() {
        for (x, s) in blk {
            let ex = kirsch(sig, a, x[0], x[1]);
            for (got, want) in [(s[0], ex[0]), (s[1], ex[1]), (s[3], ex[2])] {
                let e = (got - want).abs() / sig;
                if e > e_gauss {
                    e_gauss = e;
                    at = (x[0], x[1]);
                }
            }
        }
    }
    let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
    let (mut e_hole, mut pole) = (0.0f64, 0.0f64);
    for n in pair(&model, "u0") {
        let x = model.mesh.nodes[n];
        let (c, s) = (x[0] / a, x[1] / a);
        let st = nodal[n];
        let hoop = st[0] * s * s + st[1] * c * c - 2.0 * st[3] * s * c;
        let exact = sig * (1.0 - 2.0 * (2.0 * x[1].atan2(x[0])).cos());
        e_hole = e_hole.max((hoop - exact).abs() / (3.0 * sig));
        if x[0].abs() < 1e-9 {
            pole = hoop;
        }
    }
    eprintln!("Kirsch {kind:?} m={m}: Gauss stress error {e_gauss:.2e} sigma, hole hoop error {e_hole:.2e} of 3 sigma, pole {:.4} sigma, worst Gauss point at {at:?}", pole / sig);
    (e_gauss, e_hole, pole / sig)
}

#[test]
fn kirsch_plate_converges_to_stress_concentration_three() {
    for kind in [ElementKind::Quad9, ElementKind::Quad8, ElementKind::Tri6] {
        let runs: Vec<_> = [1, 2, 4].iter().map(|&m| kirsch_run(kind, m)).collect();
        assert!(runs[1].0 < runs[0].0 && runs[2].0 < runs[1].0 && runs[2].0 < runs[0].0 / 2.5, "{kind:?}: Gauss stress not converging {runs:?}");
        assert!(runs[2].1 < runs[0].1 / 3.0, "{kind:?}: hole stress not converging {runs:?}");
    }
    let (g, h, pole) = kirsch_run(ElementKind::Quad9, 4);
    assert!(g < 0.03 && h < 0.008, "Quad9 m=4: Gauss {g:e}, hole {h:e}");
    assert!((pole - 3.0).abs() < 0.05, "pole stress {pole} sigma");
}

// ---------------------------------------------------------------- pressurised hollow sphere

#[test]
fn pressurised_sphere_axisymmetric_matches_lame() {
    let (a, b, p, nu) = (1.0f64, 2.0f64, 1000.0, 0.3);
    let k = p * a.powi(3) / (b.powi(3) - a.powi(3));
    let ur = |r: f64| k * ((1.0 - 2.0 * nu) * r / E + (1.0 + nu) * b.powi(3) / (2.0 * E * r * r));
    let hoop = |r: f64| k * (1.0 + b.powi(3) / (2.0 * r.powi(3)));
    let radial = |r: f64| k * (1.0 - b.powi(3) / r.powi(3));
    // `(u_r error / u_r(a), hoop error / hoop(a), radial error / p)`, nodal values.
    let run = |kind: ElementKind, m: usize| -> (f64, f64, f64) {
        let mesh = grid(Physics::Axisymmetric, kind, Elastic::new(E, nu), [m, 2 * m, 1], &move |q| {
            let (rho, ph) = (a + (b - a) * q[0], 0.5 * PI * q[1]);
            [rho * ph.cos(), rho * ph.sin(), 0.0]
        })
        .unwrap();
        let model = Model::new(mesh).unwrap();
        let mut bc = model.dirichlet();
        for n in pair(&model, "v0") {
            bc.fix(n, 1, 0.0); // equatorial plane z = 0
        }
        for n in pair(&model, "v1") {
            bc.fix(n, 0, 0.0); // axis r = 0
        }
        let loads = Loads { faces: model.mesh.surfaces["u0"].iter().map(|f| (f.clone(), SurfaceLoad::Pressure(p))).collect(), ..Loads::default() };
        let sol = model.solve_static(&loads, &bc).unwrap();
        let nodal = model.nodal_stresses(&sol.u, 0.0).unwrap();
        let (mut eu, mut eh, mut er) = (0.0f64, 0.0f64, 0.0f64);
        for (n, x) in model.mesh.nodes.iter().enumerate() {
            let r = x[0].hypot(x[1]);
            let u_r = (sol.u[n * 2] * x[0] + sol.u[n * 2 + 1] * x[1]) / r;
            eu = eu.max((u_r - ur(r)).abs() / ur(a));
            // Axisymmetric components are (rr, zz, tt, rz): the hoop stress is `tt`; the spherical
            // radial stress is the tensor projected on the radial direction.
            let (c, s) = (x[0] / r, x[1] / r);
            let st = nodal[n];
            eh = eh.max((st[2] - hoop(r)).abs() / hoop(a));
            er = er.max((st[0] * c * c + st[1] * s * s + 2.0 * st[3] * s * c - radial(r)).abs() / p);
        }
        eprintln!("sphere {kind:?} m={m}: u_r {eu:.2e}, hoop {eh:.2e}, radial {er:.2e}");
        (eu, eh, er)
    };
    for kind in [ElementKind::Quad9, ElementKind::Quad8] {
        let r: Vec<_> = [4, 8, 16].iter().map(|&m| run(kind, m)).collect();
        // Displacement is O(h^4) for quadratic elements; node-sampled stress (a derivative) is O(h^2).
        for w in r.windows(2) {
            assert!(w[1].0 < w[0].0 / 10.0, "{kind:?}: displacement convergence {r:?}");
            assert!(w[1].1 < w[0].1 / 3.0 && w[1].2 < w[0].2 / 3.0, "{kind:?}: stress convergence {r:?}");
        }
        assert!(r[2].0 < 3e-6 && r[2].1 < 6e-3 && r[2].2 < 1e-2, "{kind:?}: finest {:?}", r[2]);
    }
    let r: Vec<_> = [4, 8, 16].iter().map(|&m| run(ElementKind::Tri6, m)).collect();
    assert!(r[2].0 < r[0].0 / 20.0 && r[2].1 < r[0].1 / 4.0, "Tri6 convergence {r:?}");
}
