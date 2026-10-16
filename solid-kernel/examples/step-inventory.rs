//! Read-only native syntax/reference audit. No CAD kernel and no geometry-success claim.
use solid_kernel::step::{inventory, Limits};
use std::io::Read;
fn main() {
    if let Err(e) = run() {
        eprintln!("STEP inventory: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let path = args.next().ok_or("usage: step-inventory FILE.step")?;
    if args.next().is_some() {
        return Err("usage: step-inventory FILE.step".into());
    }
    let limits = Limits::default();
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut text = String::new();
    file.take(limits.bytes as u64 + 1)
        .read_to_string(&mut text)
        .map_err(|e| e.to_string())?;
    let result = inventory(&text, limits).map_err(|e| e.to_string())?;
    println!(
        "Native STEP syntax/reference inventory: {} entities",
        result.entities().len()
    );
    println!("Geometry, topology, units, healing and CAD import validity are not certified.");
    match result.length_contexts() {
        Ok(contexts) => {
            for c in contexts {
                println!(
                    "Declared length context #{}: unit #{} = {} metres",
                    c.context, c.unit, c.metres
                );
            }
        }
        Err(e) => println!("Length-unit resolution unavailable: {e}"),
    }
    for (name, count) in result.counts() {
        println!("{name}: {count}");
    }
    Ok(())
}
