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
    let usage = "usage: step-inventory FILE.step [--analytic|--surface|--canonical-surface] [--repair-orientation]";
    let path = args.next().ok_or(usage)?;
    let mut mode = 0;
    let mut repair_orientation = false;
    for flag in args {
        if flag == "--analytic" {
            mode = mode.max(1);
        } else if flag == "--surface" {
            mode = mode.max(2);
        } else if flag == "--canonical-surface" {
            mode = 3;
        } else if flag == "--repair-orientation" {
            repair_orientation = true;
            mode = mode.max(2);
        } else {
            return Err(usage.into());
        }
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
        if mode >= 2 {
            let started = std::time::Instant::now();
            let surface_limits = solid_kernel::native_surface::Limits {
                orientation_policy: if repair_orientation {
                    solid_kernel::orientation_repair::OrientationPolicy::RepairWithReport
                } else {
                    solid_kernel::orientation_repair::OrientationPolicy::Strict
                },
                chart_policy: if mode == 3 {
                    solid_kernel::cylindrical_step::ChartPolicy::CanonicalizeWithinTolerance
                } else {
                    solid_kernel::cylindrical_step::ChartPolicy::Strict
                },
                ..solid_kernel::native_surface::Limits::default()
            };
            let surface = model
                .sampled_surface(surface_limits, |p| {
                    use solid_kernel::native_surface::GenerationPhase;
                    if p.phase == GenerationPhase::RepairingOrientation {
                        if p.completed_work % 8192 == 0 || p.completed_work == p.total_work {
                            eprintln!(
                                "Native orientation repair: {}/{} work units",
                                p.completed_work, p.total_work
                            );
                        }
                    } else {
                        eprintln!(
                            "Native surface generation: {}/{} faces",
                            p.completed_faces, p.total_faces
                        );
                    }
                    true
                })
                .map_err(|e| e.to_string())?;
            println!("Closed sampled surface: {} vertices, {} triangles, {} source faces, signed volume {} mm³ ({:.3} s)",
                surface.mesh.vertices.len(),surface.mesh.triangles.len(),surface.source_faces.len(),surface.signed_volume_mm3,started.elapsed().as_secs_f64());
            if mode == 3 {
                let largest = surface
                    .chart_adjustments
                    .iter()
                    .map(|a| a.world_deviation_mm)
                    .fold(0., f64::max);
                println!("Explicit chart canonicalization: {} changed samples; max surface projection deviation {} mm (geometry tolerance {} mm); world boundary coordinates unchanged",surface.chart_adjustments.len(),largest,model.geometry_tolerance_mm());
            }
            if repair_orientation {
                println!(
                    "Explicit orientation repair: {} source faces changed",
                    surface.face_orientation_adjustments.len()
                );
                for change in &surface.face_orientation_adjustments {
                    println!(
                        "Face #{}: same_sense {} -> {}",
                        change.source_face, change.before_same_sense, change.after_same_sense
                    );
                }
            }
            let checked = surface
                .check_intersections(2_000_000, |pairs| {
                    if pairs % 8192 == 0 {
                        eprintln!("Native facet intersection check: {pairs} candidate pairs");
                    }
                    true
                })
                .map_err(|e| e.to_string())?;
            println!(
                "Sampled facet intersection check passed: {} candidate pairs",
                checked.pairs_checked()
            );
            println!("Exact trimmed CAD validity remains uncertified.");
        }
    }
    for (name, count) in result.counts() {
        println!("{name}: {count}");
    }
    Ok(())
}
