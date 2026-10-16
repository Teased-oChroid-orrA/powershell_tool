//! Opt-in benchmark collection, one independently verified template per invocation.
//! JSONL goes to stdout; redirect explicitly to a local file to retain it. No network.
use fea_problem::{
    mesh_telemetry::{run_verified, BenchmarkStore},
    templates::templates,
};
fn main() {
    if let Err(e) = run() {
        eprintln!("benchmark: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if args.as_slice() == ["--list"] {
        println!("Cantilever beam\nThick cylinder (axisymmetric)\nPlate with a hole");
        return Ok(());
    }
    if args.len() != 4 || args[0] != "--template" || args[2] != "--revision" {
        return Err(
            "usage: mesh-benchmark --list | --template NAME --revision EXACT_COMMIT_SHA".into(),
        );
    }
    let problem = templates()
        .into_iter()
        .find(|(n, _)| *n == args[1])
        .ok_or("unknown template")?
        .1;
    eprintln!(
        "Running one template through meshing, solve and independent verification: {}",
        problem.name
    );
    let (_, proof) = run_verified(&problem, None, &args[3])?;
    let mut store = BenchmarkStore::default();
    store.record(&proof);
    eprintln!(
        "Verified one benchmark in {:.3}s; unknown peak memory remains null",
        proof.record().total_seconds()
    );
    print!("{}", store.encode()?);
    Ok(())
}
