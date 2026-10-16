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
    let path = args
        .next()
        .ok_or("usage: step-inventory FILE.step [--analytic|--surface]")?;
    let mode = match args.next() {
        None => 0,
        Some(flag) if flag == "--analytic" => 1,
        Some(flag) if flag == "--surface" => 2,
        _ => return Err("usage: step-inventory FILE.step [--analytic|--surface]".into()),
    };
    if args.next().is_some() {
        return Err("usage: step-inventory FILE.step [--analytic|--surface]".into());
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
    match result.raw_bodies() {
        Ok(bodies) => {
            for b in bodies {
                println!("Raw local body #{}: {} vertices, {} edges, {} faces (not geometric validation)", b.id,b.vertices.len(),b.edges.len(),b.faces.len());
                let mut curve_kinds = std::collections::BTreeMap::<String, usize>::new();
                for edge in b.edges.values() {
                    let mut id = edge.curve;
                    let mut seen = std::collections::BTreeSet::new();
                    let kind = loop {
                        if seen.len() >= 32 || !seen.insert(id) {
                            break "UNRESOLVED_WRAPPER".into();
                        }
                        let entity = &result.entities()[&id];
                        if let [call] = entity.components.as_slice() {
                            if matches!(call.name.as_str(), "SURFACE_CURVE" | "SEAM_CURVE") {
                                if let Some(solid_kernel::step::Value::Ref(base)) = call.args.get(1)
                                {
                                    id = *base;
                                    continue;
                                }
                                break "UNRESOLVED_WRAPPER".into();
                            }
                        }
                        break entity
                            .components
                            .iter()
                            .map(|c| c.name.as_str())
                            .collect::<Vec<_>>()
                            .join("+");
                    };
                    *curve_kinds.entry(kind).or_default() += 1;
                }
                for (kind, count) in curve_kinds {
                    println!("Referenced 3D edge declaration {kind}: {count}");
                }
            }
        }
        Err(e) => println!("Raw topology extraction unavailable: {e}"),
    }
    if mode > 0 {
        use solid_kernel::analytic_step::{AnalyticModel, Options};
        let model = AnalyticModel::read(&result, Options::default()).map_err(|e| e.to_string())?;
        let samples: usize = model
            .body()
            .edges
            .keys()
            .map(|id| model.edge_points(*id).unwrap().len())
            .sum();
        println!("Native analytic subset: {} mm/source unit; {} shared-edge samples; {} untrimmed surfaces",
            model.millimetres_per_source_unit(), samples, model.body().faces.len());
        println!("Trimmed domains, shell orientation, self-intersections and solid validity remain uncertified.");
        if mode == 2 {
            let started = std::time::Instant::now();
            let surface = model
                .sampled_surface(solid_kernel::native_surface::Limits::default(), |p| {
                    eprintln!(
                        "Native surface generation: {}/{} faces",
                        p.completed_faces, p.total_faces
                    );
                    true
                })
                .map_err(|e| e.to_string())?;
            println!("Closed sampled surface: {} vertices, {} triangles, {} source faces, signed volume {} mm³ ({:.3} s)",
                surface.mesh.vertices.len(),surface.mesh.triangles.len(),surface.source_faces.len(),surface.signed_volume_mm3,started.elapsed().as_secs_f64());
            println!("Global geometric intersections and exact CAD validity are not certified.");
        }
    }
    for (name, count) in result.counts() {
        println!("{name}: {count}");
    }
    Ok(())
}
