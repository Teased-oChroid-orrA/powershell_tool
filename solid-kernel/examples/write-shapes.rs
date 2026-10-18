//! Writes a few OpenCASCADE-built shapes as STEP files into the directory given (default `.`), the inputs of the native
//! quality studies (`step-inventory --grade`). `cargo run -p solid-kernel --features occt --example write-shapes -- DIR`
use solid_kernel::occt::{Bool, Brep};

fn main() {
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| ".".into()));
    let block = Brep::box_from_corners([0., 0., 0.], [40., 30., 20.]);
    let hole = Brep::cylinder([20., 15., -1.], 6., [0., 0., 1.], 22.);
    let boss = Brep::cylinder([8., 8., 20.], 4., [0., 0., 1.], 6.);
    let plate = Brep::box_from_corners([0., 0., 0.], [12., 4., 0.25]);
    let pin = |x: f64| Brep::cylinder([x, 2., -1.], 0.5, [0., 0., 1.], 2.);
    // Faces that an edit moves, adds or resizes (tests/face_identity.rs).
    let slab = |h: f64| Brep::box_from_corners([0., 0., 0.], [40., 30., h]);
    let drill = |x: f64, y: f64, r: f64| Brep::cylinder([x, y, -1.], r, [0., 0., 1.], 20.);
    let shapes = [
        ("edit-plate", slab(10.).boolean(Bool::Cut, &drill(12., 15., 4.)).unwrap()),
        ("edit-plate-hole-moved", slab(10.).boolean(Bool::Cut, &drill(14., 15., 4.)).unwrap()),
        ("edit-plate-second-hole", slab(10.).boolean(Bool::Cut, &drill(12., 15., 4.)).unwrap().boolean(Bool::Cut, &drill(30., 15., 3.)).unwrap()),
        ("edit-plate-thicker", slab(12.).boolean(Bool::Cut, &drill(12., 15., 4.)).unwrap()),
        ("edit-plate-centre-hole", slab(10.).boolean(Bool::Cut, &drill(20., 15., 4.)).unwrap()),
        ("edit-plate-twin-holes", slab(10.).boolean(Bool::Cut, &drill(15., 15., 4.)).unwrap().boolean(Bool::Cut, &drill(25., 15., 4.)).unwrap()),
        ("edit-plate-hole-resized", slab(10.).boolean(Bool::Cut, &drill(12., 15., 5.)).unwrap()),
    ]
    .into_iter()
    .collect::<Vec<_>>();
    let mut shapes = shapes;
    // Intersections that are not circles or lines (tests/curved_edges.rs).
    let rot = |b: Brep| b; // placeholder to keep the list readable
    let _ = rot;
    // Faces on spline surfaces: the rolling-ball fillets of edges that are not lines or circles.
    for (name, brep, r) in [
        ("t-junction-filleted", Brep::cylinder([0., 0., 0.], 6., [0., 0., 1.], 30.).boolean(Bool::Union, &Brep::cylinder([-10., 0., 15.], 4., [1., 0., 0.], 20.)).unwrap(), 1.0),
        ("tilted-hole-filleted", block.boolean(Bool::Cut, &Brep::cylinder([20., 15., -4.], 4., [0.5, 0.2, 1.], 40.)).unwrap(), 0.5),
        ("cross-bore-filleted", Brep::cylinder([0., 0., 0.], 8., [0., 0., 1.], 30.).boolean(Bool::Cut, &Brep::cylinder([-10., 0., 15.], 3., [1., 0., 0.], 20.)).unwrap(), 0.5),
    ] {
        match brep.fillet_all(r) {
            Ok(b) => shapes.push((name, b)),
            Err(e) => eprintln!("{name}: fillet failed: {e}"),
        }
    }
    // Surfaces of revolution and of linear extrusion: a spline profile turned about an axis, a spline outline pushed along one.
    for (name, made) in [
        ("vase", Brep::lathe_spline(&[[4., 0.], [6., 5.], [3., 10.], [5., 15.], [4.5, 20.]])),
        ("wavy-prism", Brep::prism_spline(&[[0., 0.], [4., 3.], [9., 2.], [14., 4.], [20., 0.]], 8.)),
    ] {
        match made {
            Ok(b) => shapes.push((name, b)),
            Err(e) => eprintln!("{name}: {e}"),
        }
    }
    shapes.extend([
        ("tilted-hole", block.boolean(Bool::Cut, &Brep::cylinder([20., 15., -4.], 4., [0.5, 0.2, 1.], 40.)).unwrap()),
        ("t-junction", Brep::cylinder([0., 0., 0.], 6., [0., 0., 1.], 30.).boolean(Bool::Union, &Brep::cylinder([-10., 0., 15.], 4., [1., 0., 0.], 20.)).unwrap()),
        ("cross-bore", Brep::cylinder([0., 0., 0.], 8., [0., 0., 1.], 30.).boolean(Bool::Cut, &Brep::cylinder([-10., 0., 15.], 3., [1., 0., 0.], 20.)).unwrap()),
        ("slot", block.boolean(Bool::Cut, &Brep::box_from_corners([10., -1., 12.], [30., 31., 16.])).unwrap()),
    ]);
    shapes.extend([
        ("block-with-hole", block.boolean(Bool::Cut, &hole).unwrap()),
        (
            "block-hole-boss",
            block
                .boolean(Bool::Cut, &hole)
                .unwrap()
                .boolean(Bool::Union, &boss)
                .unwrap(),
        ),
        (
            "block-pocket",
            block
                .boolean(
                    Bool::Cut,
                    &Brep::box_from_corners([10., 5., 12.], [30., 25., 25.]),
                )
                .unwrap(),
        ),
        (
            "plate-two-holes",
            plate
                .boolean(Bool::Cut, &pin(3.))
                .unwrap()
                .boolean(Bool::Cut, &pin(9.))
                .unwrap(),
        ),
        ("rod", Brep::cylinder([0., 0., 0.], 2., [0., 0., 1.], 60.)),
        ("cylinder-filleted-top", Brep::cylinder([0., 0., 0.], 5., [0., 0., 1.], 10.).round_edges_at_height(10., 1.5, false).unwrap()),
        ("cylinder-chamfered-top", Brep::cylinder([0., 0., 0.], 5., [0., 0., 1.], 10.).round_edges_at_height(10., 1.5, true).unwrap()),
        ("cylinder-both-ends-filleted", Brep::cylinder([0., 0., 0.], 5., [0., 0., 1.], 10.).round_edges_at_height(10., 1.5, false).unwrap().round_edges_at_height(0., 1.0, false).unwrap()),
        ("frustum", Brep::cone([0., 0., 0.], 6., 3., [0., 0., 1.], 8.)),
        ("cone", Brep::cone([0., 0., 0.], 5., 0., [0., 0., 1.], 8.)),
        ("hemisphere", Brep::sphere([0., 0., 0.], 4.).boolean(Bool::Intersect, &Brep::box_from_corners([-5., -5., 0.], [5., 5., 5.])).unwrap()),
        ("capsule", Brep::cylinder([0., 0., 0.], 3., [0., 0., 1.], 8.).boolean(Bool::Union, &Brep::sphere([0., 0., 0.], 3.)).unwrap().boolean(Bool::Union, &Brep::sphere([0., 0., 8.], 3.)).unwrap()),
        ("ball", Brep::sphere([0., 0., 0.], 4.)),
        ("torus", Brep::torus([0., 0., 0.], 6., 2., [0., 0., 1.])),
        ("filleted-box", Brep::box_from_corners([0., 0., 0.], [2., 3., 4.]).fillet_all(0.2).unwrap()),
    ]);
    for (name, shape) in shapes {
        shape.write_step(&dir.join(format!("{name}.step"))).unwrap();
        println!("{name}: {} faces", shape.face_count());
    }
}
