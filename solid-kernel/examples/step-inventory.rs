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
    let usage = "usage: step-inventory FILE.step [--analytic|--surface|--canonical-surface] [--repair-orientation] [--plc] [--star-seed X,Y,Z] [--trial-kernel] (kernel coordinates in mm)";
    let path = args.next().ok_or(usage)?;
    if path == "--help" {
        println!("{usage}");
        return Ok(());
    }
    let mut plc_requested = false;
    let mut kernel = None;
    let mut trial_kernel = false;
    let mut mode = 0;
    let mut repair_orientation = false;
    while let Some(flag) = args.next() {
        if flag == "--analytic" {
            mode = mode.max(1);
        } else if flag == "--surface" {
            mode = mode.max(2);
        } else if flag == "--canonical-surface" {
            mode = 3;
        } else if flag == "--repair-orientation" {
            repair_orientation = true;
            mode = mode.max(2);
        } else if flag == "--plc" {
            plc_requested = true;
            mode = mode.max(2);
        } else if flag == "--star-seed" {
            let value = args.next().ok_or("--star-seed requires X,Y,Z in mm")?;
            let coordinates = value
                .to_str()
                .ok_or("kernel point must be three finite coordinates in mm")?
                .split(',')
                .map(str::parse::<f64>)
                .collect::<Result<Vec<_>, _>>()
                .map_err(|_| "kernel point must be three finite coordinates in mm")?;
            if coordinates.len() != 3 || coordinates.iter().any(|x| !x.is_finite()) {
                return Err("kernel point must be three finite coordinates in mm".into());
            }
            kernel = Some([coordinates[0], coordinates[1], coordinates[2]]);
            plc_requested = true;
            mode = mode.max(2);
        } else if flag == "--trial-kernel" {
            trial_kernel = true;
        } else {
            return Err(usage.into());
        }
    }
    if trial_kernel && kernel.is_none() {
        return Err("--trial-kernel requires --star-seed X,Y,Z in mm".into());
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
            if plc_requested {
                use solid_kernel::{
                    plc::{Limits as PlcLimits, ValidatedPlc},
                    plc_star::StarSeed,
                };
                let plc = ValidatedPlc::new(checked, PlcLimits::default(), |done, total| {
                    if done % 8192 == 0 || done == total {
                        eprintln!("Native PLC admission: {done}/{total} facets");
                    }
                    true
                })
                .map_err(|e| e.to_string())?;
                println!(
                    "Checked triangulated PLC: {} vertices, {} facets, {} segment constraints",
                    plc.vertices().len(),
                    plc.facets().len(),
                    plc.segments().len()
                );
                if let Some(point) = kernel {
                    let mut seed = StarSeed::new(plc, point, 100_000, |done, total| {
                        if done % 8192 == 0 || done == total {
                            eprintln!("Native star seed: {done}/{total} boundary facets");
                        }
                        true
                    })
                    .map_err(|e| e.to_string())?;
                    println!("Strict star-kernel seed: {} points, {} tetrahedra, {} preserved boundary facets; supplied kernel {:?} mm",seed.topology().points().len(),seed.topology().cells().len(),seed.topology().boundary().len(),point);
                    if trial_kernel {
                        let mut last_phase = None;
                        let (updated, report) = seed
                            .try_vertex_mean_kernel(|p| {
                                if last_phase != Some(p.phase)
                                    || p.completed % 8192 == 0
                                    || p.completed == p.total
                                {
                                    eprintln!(
                                        "Native kernel trial {:?}: {}/{}",
                                        p.phase, p.completed, p.total
                                    );
                                    last_phase = Some(p.phase);
                                }
                                true
                            })
                            .map_err(|e| e.to_string())?;
                        seed = updated;
                        println!("Kernel trial: accepted={}; before {:?} mm; candidate {:?} mm; minimum scaled Jacobian {} -> {}; worst edge ratio {} -> {}; refusal {:?}",report.accepted,report.before_point,report.candidate_point,report.before_quality.min_scaled_jacobian,report.after_quality.min_scaled_jacobian,report.before_quality.max_edge_ratio,report.after_quality.max_edge_ratio,report.refusal);
                    }
                    println!("Diagnostic seed complete: {} cells. General PLC recovery and solver analysis are not performed.",seed.topology().cells().len());
                }
            }
        }
    }
    for (name, count) in result.counts() {
        println!("{name}: {count}");
    }
    Ok(())
}
