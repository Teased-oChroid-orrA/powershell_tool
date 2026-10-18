//! Reference measurement for the native shell-angle gate: reads a STEP file with OpenCASCADE, tessellates it at several
//! deflections and prints the smallest corner/dihedral angle (the registry's measure, `plc_conform::min_shell_angle_deg`),
//! the worst triangles and the smallest faces. `cargo run --release -p solid-kernel --features occt --example shell-angle-probe -- FILE.step`
use solid_kernel::occt::Brep;
use solid_kernel::plc_conform::min_shell_angle_deg;

fn corner_angles(v: &[[f64; 3]], t: [usize; 3]) -> [f64; 3] {
    let p = t.map(|i| v[i]);
    let mut out = [0.0; 3];
    for k in 0..3 {
        let (a, b, c) = (p[k], p[(k + 1) % 3], p[(k + 2) % 3]);
        let u = [b[0] - a[0], b[1] - a[1], b[2] - a[2]];
        let w = [c[0] - a[0], c[1] - a[1], c[2] - a[2]];
        let (nu, nw) = (
            (u[0] * u[0] + u[1] * u[1] + u[2] * u[2]).sqrt(),
            (w[0] * w[0] + w[1] * w[1] + w[2] * w[2]).sqrt(),
        );
        let d = (u[0] * w[0] + u[1] * w[1] + u[2] * w[2]) / (nu * nw);
        out[k] = d.clamp(-1., 1.).acos().to_degrees();
    }
    out
}

fn main() {
    let path = std::env::args()
        .nth(1)
        .expect("usage: shell-angle-probe FILE.step");
    let b = Brep::read_step(std::path::Path::new(&path)).unwrap();
    let mut faces = b.faces();
    faces.sort_by(|a, b| a.area.total_cmp(&b.area));
    println!("{} faces", faces.len());
    for f in faces.iter().take(14) {
        println!(
            "  face {} area {:.5} centre {:?}",
            f.index, f.area, f.centre
        );
    }
    for d in [0.5, 0.1, 0.05, 0.01, 0.005] {
        match b.tessellate(d) {
            Ok(m) => {
                let a = min_shell_angle_deg(&m.vertices, &m.triangles);
                let mut worst: Vec<(f64, usize)> = m
                    .triangles
                    .iter()
                    .enumerate()
                    .map(|(i, t)| {
                        (
                            corner_angles(&m.vertices, *t)
                                .iter()
                                .cloned()
                                .fold(180., f64::min),
                            i,
                        )
                    })
                    .collect();
                worst.sort_by(|a, b| a.0.total_cmp(&b.0));
                let w: Vec<_> = worst
                    .iter()
                    .take(3)
                    .map(|(a, i)| (*a, m.face[*i]))
                    .collect();
                println!("deflection {d}: {} vertices, {} triangles, registry angle {a:.4} deg; worst corners (deg, face) {w:?}", m.vertices.len(), m.triangles.len());
            }
            Err(e) => println!("deflection {d}: {e}"),
        }
    }
}
