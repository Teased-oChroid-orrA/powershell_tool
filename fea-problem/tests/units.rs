//! Unit systems: a conversion rescales every number so the same physical problem gives the same answer, declaring
//! rescales nothing, and a modulus that no structural material has is caught before a solve.
use fea_problem::diagnose::{diagnose, FailureClass};
use fea_problem::{solve, templates, Analysis, Load, Problem, Support, UnitSystem};

fn plate() -> Problem {
    let mut p = templates::templates().remove(0).1;
    p.mesh.size = 1.0;
    p.mesh.adapt_passes = 0;
    // These tests are about the conversion, and the answer is compared at its peak node: the all-unstructured mesher breaks ties
    // by round-off, so the converted problem differs from the original by a fraction of a percent; a hybrid mesh (structured
    // blocks plus a graded unstructured fill) differs by about 3 %. Pin the strategy so a change of the default is not a unit bug.
    p.mesh.strategy = fea_problem::MeshStrategy::Unstructured;
    p.loads.push(Load::Pressure { edge: "top".into(), p: 500.0 });
    p
}

#[test]
fn the_system_tables_are_consistent() {
    let (i, m, s) = (UnitSystem::InchPsi, UnitSystem::MmMpa, UnitSystem::MeterPa);
    assert!((i.stress_pa() - 6894.757).abs() < 0.01, "1 psi in Pa");
    assert!((m.stress_pa() - 1e6).abs() < 1e-6, "1 MPa");
    assert_eq!(s.stress_pa(), 1.0);
    // Steel: E = 29e6 psi = 199.9 GPa; weight density 0.283 lb/in3 = 7833 kg/m3.
    assert!((i.stress_to_pa(29.0e6) / 1e9 - 199.9).abs() < 0.2);
    let rho = i.from_weight_density(0.283) * i.density_kg_m3();
    assert!((rho - 7833.0).abs() < 5.0, "{rho}");
    // The library's ksi and weight density land on the same physical value in every system.
    for u in UnitSystem::ALL {
        assert!((u.stress_to_pa(u.from_ksi(29_000.0)) / 1e9 - 199.9).abs() < 0.2, "{u:?}");
        assert!((u.from_weight_density(0.283) * u.density_kg_m3() - 7833.0).abs() < 5.0, "{u:?}");
    }
    // 6.5 microstrain per deg F is 11.7 per K.
    assert!((m.from_microstrain_per_f(6.5) - 11.7e-6).abs() < 0.05e-6);
}

#[test]
fn a_converted_problem_gives_the_same_physical_answer_in_every_system() {
    let inch = plate();
    let base = solve(&inch, None).unwrap();
    for to in [UnitSystem::MmMpa, UnitSystem::MeterPa] {
        let mut p = inch.clone();
        p.convert_units(to).unwrap();
        assert_eq!(p.units, to);
        let s = solve(&p, None).unwrap();
        // Displacement is a length, stress a stress, the reaction a force.
        let (lf, sf, ff) = (inch.units.length_m() / to.length_m(), inch.units.stress_pa() / to.stress_pa(), inch.units.force_n() / to.force_n());
        let rel = |a: f64, b: f64| (a - b).abs() / b.abs().max(1e-300);
        // The mesher is not bit-for-bit scale invariant (the node count differs by a few percent), so the fields agree to a
        // fraction of a percent; the applied force is an exact integral of the load and agrees to rounding.
        assert!(rel(s.summary.max_displacement.value, base.summary.max_displacement.value * lf) < 1e-2, "{to:?} displacement");
        assert!(rel(s.summary.max_von_mises.value, base.summary.max_von_mises.value * sf) < 2e-2, "{to:?} stress");
        assert!(rel(s.summary.applied[0], base.summary.applied[0] * ff) < 1e-6, "{to:?} force");
        // And back again is the original.
        let mut back = p.clone();
        back.convert_units(UnitSystem::InchPsi).unwrap();
        let r = solve(&back, None).unwrap();
        assert!(rel(r.summary.max_displacement.value, base.summary.max_displacement.value) < 1e-2);
        assert!((back.material.e - inch.material.e).abs() / inch.material.e < 1e-12);
    }
}

#[test]
fn a_solid_with_a_history_a_body_force_and_a_prescribed_displacement_converts_whole() {
    let mut p = templates::templates().into_iter().map(|(_, p)| p).find(|p| p.analysis == Analysis::Solid).unwrap();
    p.mesh.size = 1.0;
    p.mesh.strategy = fea_problem::MeshStrategy::Unstructured; // see `plate`
    p.loads.push(Load::Body { bx: 0.0, by: -0.283, bz: 0.0 });
    p.supports.push(Support::Point { x: 0.0, y: 0.0, z: 0.0, ux: Some(0.0), uy: Some(0.001), uz: None });
    p.history = Some(fea_problem::history::History::from_problem(&p).unwrap());
    p.apply_history().unwrap();
    let before = solve(&p, None).unwrap();
    let mut q = p.clone();
    q.convert_units(UnitSystem::MmMpa).unwrap();
    // The extrusion depth, the sketch and the history all grew by 25.4.
    if let (fea_problem::Geometry::Sketch { depth: d0, .. }, fea_problem::Geometry::Sketch { depth: d1, .. }) = (&p.geometry, &q.geometry) {
        assert!((d1 / d0 - 25.4).abs() < 1e-12);
    }
    q.apply_history().unwrap();
    let after = solve(&q, None).unwrap();
    assert!((after.summary.max_displacement.value / before.summary.max_displacement.value / 25.4 - 1.0).abs() < 1e-2, "{} vs {}", after.summary.max_displacement.value, before.summary.max_displacement.value);
    let sf = UnitSystem::InchPsi.stress_pa() / UnitSystem::MmMpa.stress_pa();
    assert!((after.summary.max_von_mises.value / (before.summary.max_von_mises.value * sf) - 1.0).abs() < 2e-2);
}

#[test]
fn imported_coordinates_convert_without_rewriting_the_source_file() {
    let mut p = templates::imported("x.msh");
    let before = p.clone();
    p.convert_units(UnitSystem::MmMpa).unwrap();
    assert!(
        matches!(p.geometry, fea_problem::Geometry::Imported { scale, .. } if (scale.length-25.4).abs()<1e-12));
    p.convert_units(UnitSystem::InchPsi).unwrap();
    assert!((p.material.e / before.material.e - 1.0).abs() < 1e-12);
    p.declare_units(UnitSystem::MmMpa);
    assert_eq!(p.units, UnitSystem::MmMpa);
    assert_eq!(p.material, before.material, "declaring rescales nothing");
}

#[test]
fn the_default_system_is_not_written_and_a_file_without_it_reads_as_inch_psi() {
    let p = plate();
    let json = p.to_json().unwrap();
    assert!(!json.contains("\"units\""), "an old file stays byte-compatible");
    assert_eq!(Problem::from_json(&json).unwrap().units, UnitSystem::InchPsi);
    let mut q = p.clone();
    q.convert_units(UnitSystem::MeterPa).unwrap();
    let back = Problem::from_json(&q.to_json().unwrap()).unwrap();
    assert_eq!(back.units, UnitSystem::MeterPa);
}

#[test]
fn a_modulus_no_structural_material_has_is_caught_and_the_right_system_is_offered() {
    // Steel in MPa (210 000) typed into a model that says psi: 1.4 GPa-equivalent? No: 210 000 psi = 1.4 GPa, in range.
    // A modulus of 210 (GPa typed into an MPa model) is 210 MPa = 0.2 GPa... in range too; 2.1e11 into psi is 1.4e15 Pa: out.
    let mut p = plate();
    p.material.e = 2.1e11;
    let found: Vec<_> = diagnose(&p, None).into_iter().filter(|d| d.class == FailureClass::ScaleSuspect && d.questions.iter().any(|q| q.id == "units")).collect();
    assert_eq!(found.len(), 1);
    assert!(found[0].summary.starts_with("Before solving"));
    // 2.1e11 is a steel modulus in Pa: the question offers to declare the model in m, N, Pa.
    let options = &found[0].questions[0].options;
    assert!(options.iter().any(|(l, _)| l.contains("m, N, Pa")), "{:?}", options.iter().map(|o| &o.0).collect::<Vec<_>>());
    let mut fixed = p.clone();
    options.iter().find(|(l, _)| l.contains("m, N, Pa")).unwrap().1.apply(&mut fixed).unwrap();
    assert_eq!(fixed.units, UnitSystem::MeterPa);
    assert!(!diagnose(&fixed, None).iter().any(|d| d.questions.iter().any(|q| q.id == "units")), "declared right, nothing is raised");
    // A sound model raises nothing.
    assert!(!diagnose(&plate(), None).iter().any(|d| d.questions.iter().any(|q| q.id == "units")));
    // Steel at 210 000 in a model declared in mm, N, MPa is fine; the same number declared in inch is 1.4 GPa: still in range.
    let mut mm = plate();
    mm.declare_units(UnitSystem::MmMpa);
    mm.material.e = 210_000.0;
    assert!(!diagnose(&mm, None).iter().any(|d| d.questions.iter().any(|q| q.id == "units")));
}

#[test]
fn thermal_units_convert_absolute_temperatures_and_heat_dimensions() {
    use fea_problem::thermal::{HeatLoad, HeatSupport};
    let mut p = plate();
    p.thermal.reference = 32.0;
    p.thermal.conductivity = 1.0;
    p.thermal.source = 2.0;
    p.thermal.supports = vec![HeatSupport::Temperature {
        edge: "left".into(),
        t: 212.0,
    }];
    p.thermal.loads = vec![HeatLoad::Convection {
        edge: "right".into(),
        h: 3.0,
        ambient: 68.0,
    }];
    let original = p.clone();
    p.convert_units(UnitSystem::MmMpa).unwrap();
    assert!(p.thermal.reference.abs() < 1e-12);
    assert!(
        matches!(p.thermal.supports[0],HeatSupport::Temperature{t,..} if (t-100.0).abs()<1e-12)
    );
    assert!(
        matches!(p.thermal.loads[0],HeatLoad::Convection{ambient,..} if (ambient-20.0).abs()<1e-12)
    );
    assert!((p.thermal.conductivity - 4.4482216152605 * 1.8).abs() < 1e-10);
    p.convert_units(UnitSystem::InchPsi).unwrap();
    assert!((p.thermal.conductivity - original.thermal.conductivity).abs() < 1e-12);
    assert!((p.thermal.reference - original.thermal.reference).abs() < 1e-12);
}

#[test]
fn external_mesh_coordinates_and_boundary_conditions_convert_together() {
    let text="*NODE\n1,0,0\n2,1,0\n3,1,1\n4,0,1\n*ELEMENT,TYPE=CPS4\n1,1,2,3,4\n*NSET,NSET=left\n1,4\n*NSET,NSET=right\n2,3\n";
    let mut p=templates::imported("bar.inp"); p.history=None;
    p.supports=vec![Support::fixed("left")];
    p.loads=vec![Load::Force{edge:"right".into(),fx:100.0,fy:0.0,fz:0.0}];
    let a=solve(&p,Some(text)).unwrap();
    let original=p.units;
    p.convert_units(UnitSystem::MmMpa).unwrap();
    let b=solve(&p,Some(text)).unwrap();
    let ua=a.summary.max_displacement.value*original.length_m();
    let ub=b.summary.max_displacement.value*p.units.length_m();
    assert!((ua/ub-1.0).abs()<1e-10,"{ua} vs {ub}");
}

#[test]
fn solve_stage_observer_can_stop_before_meshing_without_publishing_a_result() {
    let p=plate();
    let seen=std::cell::RefCell::new(Vec::new());
    let result=fea_problem::solve::solve_observed(&p,None,&|stage| {
        seen.borrow_mut().push(stage);
        if stage=="Generating analysis mesh" { Err("cancelled by caller".into()) } else { Ok(()) }
    });
    assert!(matches!(result,Err(ref e) if e=="cancelled by caller"));
    assert_eq!(*seen.borrow(),["Validating analysis inputs","Generating analysis mesh"]);
}

#[test]
fn embedded_isotropic_and_orthotropic_mesh_materials_convert_with_sections() {
    let nodes="*NODE\n1,0,0\n2,1,0\n3,1,1\n4,0,1\n*ELEMENT,TYPE=CPS4,ELSET=bar\n1,1,2,3,4\n*NSET,NSET=left\n1,4\n*NSET,NSET=right\n2,3\n";
    for elastic in ["*ELASTIC\n10000000,0\n", "*ELASTIC,TYPE=ENGINEERING CONSTANTS\n10000000,8000000,6000000,0,0,0,3000000,3000000,3000000\n"] {
        let text=format!("{nodes}*MATERIAL,NAME=alloy\n{elastic}*EXPANSION\n0.000006\n*SOLID SECTION,ELSET=bar,MATERIAL=alloy\n0.25\n");
        let mut p=templates::imported("bar.inp");
        p.supports=vec![Support::fixed("left")];
        p.loads=vec![Load::Force{edge:"right".into(),fx:100.0,fy:0.0,fz:0.0}];
        let initial=solve(&p,Some(&text)).unwrap();
        let expected=100.0/(10.0e6*0.25);
        assert!((initial.summary.max_displacement.value/expected-1.0).abs()<1e-10);
        let physical=initial.summary.max_displacement.value*p.units.length_m();
        let source_alpha=initial.model.mesh.blocks[0].material.alpha/p.units.kelvin_per_degree();
        for target in [UnitSystem::MmMpa,UnitSystem::MeterPa,UnitSystem::InchPsi] {
            p.convert_units(target).unwrap();
            p=Problem::from_json(&p.to_json().unwrap()).unwrap();
            let solved=solve(&p,Some(&text)).unwrap();
            let displacement=solved.summary.max_displacement.value*target.length_m();
            assert!((displacement/physical-1.0).abs()<1e-10,"{target:?}: {displacement} vs {physical}");
            assert!((solved.model.mesh.blocks[0].material.alpha/target.kelvin_per_degree()/source_alpha-1.0).abs()<1e-12);
        }
    }
}
