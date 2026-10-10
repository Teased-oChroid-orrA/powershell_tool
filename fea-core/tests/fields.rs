use fea_core::fields::*;
use fea_core::*;

#[test]
fn mixed_fields_preserve_node_major_mapping_and_reject_ambiguity() {
    let map = DofMap::new(vec![
        vec![Field::Translation(0), Field::Rotation(2)],
        vec![Field::Temperature],
    ])
    .unwrap();
    assert_eq!(map.n_dofs(), 3);
    assert_eq!(map.index(0, Field::Rotation(2)).unwrap(), 1);
    assert_eq!(map.index(1, Field::Temperature).unwrap(), 2);
    assert!(map.index(1, Field::Translation(0)).is_err());
    assert!(DofMap::new(vec![vec![Field::Temperature, Field::Temperature]]).is_err());
    assert!(DofMap::uniform(1, &[Field::Rotation(3)]).is_err());
    let mut constraints = map.constraints();
    constraints
        .prescribe(&map, 1, Field::Temperature, 20.0)
        .unwrap();
    assert!(constraints
        .prescribe(&map, 1, Field::Temperature, 30.0)
        .is_err());
}

#[test]
fn generalized_sparse_assembly_solves_a_bar_with_nonzero_prescription() {
    let map = DofMap::uniform(3, &[Field::Translation(0)]).unwrap();
    let mut bc = map.constraints();
    bc.prescribe(&map, 0, Field::Translation(0), 0.2).unwrap();
    let mut assembly = FieldAssembly::new(map);
    for pair in [[0, 1], [1, 2]] {
        assembly
            .add_element(&pair, &[4.0, -4.0, -4.0, 4.0])
            .unwrap();
    }
    assembly.add_load(2, 2.0).unwrap();
    let system = assembly.finish();
    assert_eq!(system.nonzeros(), 5);
    let solution = system.solve(&bc, 1e-12).unwrap();
    fea_core::report::AcceptanceReport::fields(&solution, 1e-12).require().unwrap();
    for (got, want) in solution.values.iter().zip([0.2, 0.7, 1.2]) {
        assert!((got - want).abs() < 1e-12);
    }
    assert!((solution.reactions[0] + 2.0).abs() < 1e-12);
    assert!((solution.quadratic_energy - 1.0).abs() < 1e-12);
}

#[test]
fn continuum_migration_preserves_matrix_solution_and_reactions() {
    let mesh = generate::grid(
        Physics::PlaneStress { thickness: 1.0 },
        ElementKind::Quad4,
        Elastic::new(1e5, 0.25),
        [3, 2, 1],
        &|p| p,
    )
    .unwrap();
    let model = Model::new(mesh).unwrap();
    let map = model.displacement_fields();
    let mut bc = model.dirichlet();
    for &node in model.mesh.node_set("u0").unwrap() {
        bc.fix_node(node);
    }
    let tip = *model.mesh.node_set("u1").unwrap().last().unwrap();
    let loads = Loads {
        nodal: vec![(tip, [1.0, 0.3, 0.0])],
        ..Default::default()
    };
    let old = model.solve_static(&loads, &bc).unwrap();
    let k = model.assemble().unwrap();
    let n = map.n_dofs();
    let mut dense = vec![0.0; n * n];
    for j in 0..n {
        let mut basis = vec![0.0; n];
        basis[j] = 1.0;
        let mut col = vec![0.0; n];
        k.matvec_add(&model.pattern, &basis, &mut col);
        for i in 0..n {
            dense[i * n + j] = col[i];
        }
    }
    let mut assembly = FieldAssembly::new(map.clone());
    assembly
        .add_element(&(0..n).collect::<Vec<_>>(), &dense)
        .unwrap();
    for (i, f) in loads::assemble(&model.mesh, &loads)
        .unwrap()
        .into_iter()
        .enumerate()
    {
        assembly.add_load(i, f).unwrap();
    }
    let new = assembly
        .finish()
        .solve(
            &FieldConstraints::from_continuum(&model, &bc).unwrap(),
            1e-12,
        )
        .unwrap();
    for (a, b) in old.u.iter().zip(&new.values) {
        assert!((a - b).abs() < 1e-12);
    }
    for (a, b) in old.reactions.iter().zip(&new.reactions) {
        assert!((a - b).abs() < 1e-10);
    }
    for node in 0..model.mesh.nodes.len() {
        for axis in 0..2 {
            assert_eq!(
                map.index(node, Field::Translation(axis)).unwrap(),
                node * 2 + axis
            );
        }
    }
}

#[test]
fn malformed_elements_and_nonfinite_loads_are_errors() {
    let map = DofMap::uniform(2, &[Field::Temperature]).unwrap();
    let mut assembly = FieldAssembly::new(map);
    assert!(assembly.add_element(&[0, 0], &[1.0; 4]).is_err());
    assert!(assembly
        .add_element(&[0, 1], &[1.0, 1.0, 0.0, 1.0])
        .is_err());
    assert!(assembly.add_load(2, 1.0).is_err());
    assert!(assembly.add_load(0, f64::NAN).is_err());
}

#[test]
fn thermal_to_structural_coupling_uses_temperature_change_and_matches_the_bar_reference() {
    let e = 2e7;
    let alpha = 1e-5;
    let reference = 20.0;
    let mut mesh = generate::grid(
        Physics::PlaneStress { thickness: 1.0 },
        ElementKind::Quad9,
        Elastic::new(e, 0.0).with_alpha(alpha),
        [4, 1, 1],
        &|p| [2.0 * p[0], p[1], 0.0],
    )
    .unwrap();
    mesh.set_thermal(0, Conductivity::Constant(2.0), 1.0)
        .unwrap();
    let model = Model::new(mesh).unwrap();
    let mut heat_bc = model.thermal_dirichlet();
    let mut bc = model.dirichlet();
    for (n, x) in model.mesh.nodes.iter().enumerate() {
        bc.fix(n, 1, 0.0);
        if x[0] < 1e-12 {
            heat_bc.fix(n, 0, reference);
            bc.fix(n, 0, 0.0);
        }
        if (x[0] - 2.0).abs() < 1e-12 {
            heat_bc.fix(n, 0, reference + 100.0);
            bc.fix(n, 0, 0.0);
        }
    }
    let out = model
        .solve_thermomechanical(
            &HeatLoads::default(),
            &heat_bc,
            &Loads::default(),
            &bc,
            reference,
            1e-8,
        )
        .unwrap();
    assert_eq!(
        out.direction,
        fea_core::coupling::CouplingDirection::ThermalToStructural
    );
    let reaction: f64 = model
        .mesh
        .node_set("u1")
        .unwrap()
        .iter()
        .map(|&n| out.structure.reactions[n * 2])
        .sum();
    assert!((reaction / (-e * alpha * 50.0) - 1.0).abs() < 1e-9);
    assert_eq!(out.thermal_fields.n_dofs(), model.mesh.nodes.len());
    assert_eq!(out.structural_fields.n_dofs(), model.mesh.n_dofs());
}

#[test]
fn equal_sized_constraints_with_different_fields_cannot_be_reinterpreted() {
    let thermal = DofMap::uniform(2, &[Field::Temperature]).unwrap();
    let mechanical = DofMap::uniform(2, &[Field::Translation(0)]).unwrap();
    let mut bc = thermal.constraints();
    assert!(bc
        .prescribe(&mechanical, 0, Field::Translation(0), 0.0)
        .is_err());
    bc.prescribe(&thermal, 0, Field::Temperature, 0.0).unwrap();
    let mut assembly = FieldAssembly::new(mechanical);
    assembly
        .add_element(&[0, 1], &[1.0, -1.0, -1.0, 1.0])
        .unwrap();
    assert!(assembly.finish().solve(&bc, 1e-10).is_err());
}
