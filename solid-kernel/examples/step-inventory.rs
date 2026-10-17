//! Read-only native syntax/reference audit. No CAD kernel and no geometry-success claim.
use solid_kernel::step::{inventory, Limits};
use std::io::Read;
fn report_work(stage: &str, done: usize, total: usize, last: &mut Option<(usize, usize)>) {
    if *last != Some((done, total)) && (done % 8192 == 0 || done == total) {
        eprintln!("{stage}: {done}/{total}");
        *last = Some((done, total));
    }
}
fn main() {
    if let Err(e) = run() {
        eprintln!("STEP inventory: {e}");
        std::process::exit(1);
    }
}
fn run() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: step-inventory FILE.step [--analytic|--surface|--canonical-surface] [--pcurve-audit] [--finite-arcs] [--repair-orientation] [--plc] [--conform [--conform-max-steiner N] [--conform-missing-only]] [--star-seed X,Y,Z] [--trial-kernel] (kernel coordinates in mm)";
    let path = args.next().ok_or(usage)?;
    if path == "--help" {
        println!("{usage}");
        return Ok(());
    }
    let mut pcurve_audit = false;
    let mut plc_requested = false;
    let mut conform_requested = false;
    let mut conform_max_steiner = 4000;
    let mut conform_missing_only = false;
    let mut kernel = None;
    let mut trial_kernel = false;
    let mut mode = 0;
    let mut repair_orientation = false;
    let mut finite_arcs = false;
    while let Some(flag) = args.next() {
        if flag == "--pcurve-audit" {
            pcurve_audit = true;
        } else if flag == "--finite-arcs" {
            pcurve_audit = true;
            finite_arcs = true;
        } else if flag == "--analytic" {
            mode = mode.max(1);
        } else if flag == "--surface" {
            mode = mode.max(2);
        } else if flag == "--canonical-surface" {
            mode = 3;
        } else if flag == "--repair-orientation" {
            repair_orientation = true;
            mode = mode.max(2);
        } else if flag == "--conform-missing-only" {
            conform_missing_only = true;
        } else if flag == "--conform-max-steiner" {
            conform_max_steiner = args
                .next()
                .and_then(|v| v.to_str().and_then(|v| v.parse().ok()))
                .ok_or("--conform-max-steiner needs a count")?;
        } else if flag == "--conform" {
            plc_requested = true;
            conform_requested = true;
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
    if pcurve_audit {
        let context =
            solid_kernel::step_body::BodyContext::read(&result).map_err(|e| e.to_string())?;
        let mut last = None;
        let report = solid_kernel::step_curve_audit::audit(
            &context,
            &result,
            solid_kernel::step_curve_audit::Options {
                finite_arcs,
                ..Default::default()
            },
            |p| {
                let status = (p.edge_uses_seen, p.associations_done, p.spline_spans_done);
                if last != Some(status)
                    && (if p.total_associations.is_none() {
                        p.edge_uses_seen % 32 == 0
                    } else {
                        p.associations_done % 32 == 0
                            || p.total_associations == Some(p.associations_done)
                    })
                {
                    let total = p
                        .total_associations
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "?".into());
                    eprintln!(
                        "P-curve audit: {} edge uses seen, {} / {} associations, {} spline spans",
                        p.edge_uses_seen, p.associations_done, total, p.spline_spans_done
                    );
                    last = Some(status);
                }
                true
            },
        )
        .map_err(|e| e.to_string())?;
        println!("Scoped p-curve locus audit: {} associations; {} spline spans. Master semantics, trims and body validity are not certified.",report.checks.len(),report.spline_spans);
        if finite_arcs {
            use solid_kernel::step_pcurve_arc::Resolved;
            let count = |f: fn(&Resolved) -> bool| {
                report.finite_arcs.iter().filter(|a| f(&a.resolved)).count()
            };
            println!(
                "Finite trims: {} segments, {} oriented arcs, {} closed edges without an arc.",
                count(|r| matches!(r, Resolved::Segment(_))),
                count(|r| matches!(r, Resolved::Arc(_))),
                count(|r| matches!(r, Resolved::Closed))
            );
        }
    }
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
                let mut last_plc = None;
                let plc = ValidatedPlc::new(checked, PlcLimits::default(), |done, total| {
                    report_work("Native PLC admission (facets)", done, total, &mut last_plc);
                    true
                })
                .map_err(|e| e.to_string())?;
                println!(
                    "Checked triangulated PLC: {} vertices, {} facets, {} segment constraints",
                    plc.vertices().len(),
                    plc.facets().len(),
                    plc.segments().len()
                );
                if conform_requested {
                    let started = std::time::Instant::now();
                    let mut last = (usize::MAX, 0);
                    let mesh = solid_kernel::plc_conform::conform(
                        plc.vertices(),
                        &plc.facets().to_vec(),
                        solid_kernel::plc_conform::Options {
                            max_steiner: conform_max_steiner,
                            protect_encroached: !conform_missing_only,
                            ..Default::default()
                        },
                        |p| {
                            if (p.phase as usize, p.steiner / 25) != last {
                                last = (p.phase as usize, p.steiner / 25);
                                eprintln!(
                                    "Conforming recovery {:?}: {} Steiner points, {} segments / {} sub-facets pending",
                                    p.phase, p.steiner, p.missing_segments, p.missing_facets
                                );
                            }
                            true
                        },
                    )
                    .map_err(|e| e.to_string())?;
                    let r = &mesh.report;
                    println!("Conforming Delaunay recovery: {} input vertices + {} segment and {} facet Steiner points; {} interior cells (of {} hull cells); volume error {:e}; max Steiner offset {:e} (relative); {} rounds ({:.1} s). Cell quality and slivers are not bounded.",r.input_vertices,r.segment_steiner,r.facet_steiner,r.interior_cells,r.hull_cells,r.volume_error_relative,r.max_steiner_offset_relative,r.rounds,started.elapsed().as_secs_f64());
                }
                if let Some(point) = kernel {
                    let mut last_seed = None;
                    let mut seed = StarSeed::new(plc, point, 100_000, |done, total| {
                        report_work(
                            "Native star seed (boundary facets)",
                            done,
                            total,
                            &mut last_seed,
                        );
                        true
                    })
                    .map_err(|e| e.to_string())?;
                    println!("Strict star-kernel seed: {} points, {} tetrahedra, {} preserved boundary facets; supplied kernel {:?} mm",seed.topology().points().len(),seed.topology().cells().len(),seed.topology().boundary().len(),point);
                    if trial_kernel {
                        let mut last_progress = None;
                        let (updated, report) = seed
                            .try_vertex_mean_kernel(|p| {
                                let key = (p.phase, p.completed, p.total);
                                if last_progress != Some(key)
                                    && (p.completed % 8192 == 0 || p.completed == p.total)
                                {
                                    eprintln!(
                                        "Native kernel trial {:?}: {}/{}",
                                        p.phase, p.completed, p.total
                                    );
                                    last_progress = Some(key);
                                }
                                true
                            })
                            .map_err(|e| e.to_string())?;
                        seed = updated;
                        println!("Kernel trial: accepted={}; before {:?} mm; candidate {:?} mm; minimum scaled Jacobian {} -> {}; worst edge ratio {} -> {}; refusal {:?}",report.accepted,report.before_point,report.candidate_point,report.before_quality.min_scaled_jacobian,report.after_quality.min_scaled_jacobian,report.before_quality.max_edge_ratio,report.after_quality.max_edge_ratio,report.refusal);
                        if let Some(quality) = report.candidate_quality {
                            println!("Candidate quality: minimum scaled Jacobian {}; worst edge ratio {}",quality.min_scaled_jacobian,quality.max_edge_ratio);
                        }
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
