use bushing_solver::stackup::{calculate, Inputs, Member};
use std::{hint::black_box, time::Instant};
fn main() {
    println!("members,uniform,recalculations,microseconds_per_calculation");
    for size in [2, 10, 100, 1000] {
        for uniform in [false, true] {
            let mut input = Inputs::default();
            input.bushing_enabled = true;
            input.members = (1..=size)
                .map(|id| {
                    let mut m = Member::new(id as u64);
                    m.seating_max = 1e6;
                    m
                })
                .collect();
            input.uniform_od = uniform;
            input.pilot_offset = 0.01;
            black_box(calculate(&input).unwrap());
            let repetitions = 10000 / size + 100;
            let start = Instant::now();
            for k in 0..repetitions {
                input.pilot_offset = 0.01 + (k % 2) as f64 * 0.001;
                black_box(calculate(black_box(&input)).unwrap());
            }
            println!(
                "{size},{uniform},{repetitions},{:.3}",
                start.elapsed().as_secs_f64() * 1e6 / repetitions as f64
            );
        }
    }
}
