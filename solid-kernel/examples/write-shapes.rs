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
    let shapes = [
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
    ];
    for (name, shape) in shapes {
        shape.write_step(&dir.join(format!("{name}.step"))).unwrap();
        println!("{name}: {} faces", shape.face_count());
    }
}
