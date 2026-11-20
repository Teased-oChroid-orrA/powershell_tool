//! Time reading a STEP assembly for the Assembly Joints window: the two separate reads (`detect_step` + `assembly_mesh`) against
//! the one shared read (`read_assembly`), interleaved so machine load hits both alike; minimum and median over the runs.
//! `cargo run --release -p fea-problem --example assembly-load-bench -- FILE.step [RUNS]`.
use fea_problem::assembly_advice::detect_step;
use fea_problem::assembly_mesh::{assembly_mesh, read_assembly, DEFAULT_CHORD};
use std::time::Instant;

fn ms(f: impl FnOnce()) -> f64 {
    let t = Instant::now();
    f();
    t.elapsed().as_secs_f64() * 1e3
}

fn main() {
    let mut args = std::env::args().skip(1);
    let path = args.next().expect("usage: assembly-load-bench FILE.step [RUNS]");
    let runs: usize = args.next().map_or(101, |r| r.parse().expect("RUNS"));
    let text = std::fs::read_to_string(&path).expect("read");
    let two = || {
        detect_step(&text).unwrap();
        assembly_mesh(&text, DEFAULT_CHORD).unwrap();
    };
    let one = || {
        read_assembly(&text, DEFAULT_CHORD).unwrap();
    };
    two();
    one();
    let (mut a, mut b) = (Vec::new(), Vec::new());
    for _ in 0..runs {
        a.push(ms(two));
        b.push(ms(one));
    }
    let stat = |v: &mut Vec<f64>| {
        v.sort_by(f64::total_cmp);
        (v[0], v[v.len() / 2])
    };
    let (a, b) = (stat(&mut a), stat(&mut b));
    println!("{path} ({runs} interleaved runs)\n  two reads (detect_step + assembly_mesh): min {:.2} ms, median {:.2} ms\n  one read (read_assembly):                min {:.2} ms, median {:.2} ms\n  ratio one/two: min {:.2}, median {:.2}", a.0, a.1, b.0, b.1, b.0 / a.0, b.1 / a.1);
}
