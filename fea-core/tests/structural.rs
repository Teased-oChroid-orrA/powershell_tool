use fea_core::fields::Field::{Rotation as R, Translation as U};
use fea_core::structural::*;

fn section(shear: bool) -> FrameSection {
    FrameSection {
        e: 2e7,
        g: 8e6,
        area: 0.5,
        iy: 0.02,
        iz: 0.03,
        torsion: 0.01,
        shear_areas: shear.then_some([0.4, 0.4]),
    }
}
fn cantilever(
    nodes: Vec<[f64; 3]>,
    members: Vec<Member>,
) -> (MemberModel, fea_core::fields::FieldConstraints) {
    let model = MemberModel::new(nodes, members).unwrap();
    let mut bc = model.fields().constraints();
    for axis in 0..3 {
        bc.prescribe(model.fields(), 0, U(axis), 0.0).unwrap();
        if model.fields().index(0, R(axis)).is_ok() {
            bc.prescribe(model.fields(), 0, R(axis), 0.0).unwrap();
        }
    }
    (model, bc)
}
#[test]
fn inclined_truss_matches_axial_extension_reactions_and_stress() {
    let (model, mut bc) = cantilever(
        vec![[0.0; 3], [3.0, 4.0, 0.0]],
        vec![Member::Truss {
            nodes: [0, 1],
            e: 200.0,
            area: 2.0,
        }],
    );
    // Lock y,z: projected axial stiffness in x is EA/L * (3/5)^2.
    bc.prescribe(model.fields(), 1, U(1), 0.0).unwrap();
    bc.prescribe(model.fields(), 1, U(2), 0.0).unwrap();
    let out = model.solve(&bc, &[(1, U(0), 12.0)], 1e-12).unwrap();
    let ux = 12.0 * 5.0 / (200.0 * 2.0 * 0.6_f64.powi(2));
    assert!((out.fields.value(1, U(0)).unwrap() - ux).abs() < 1e-12);
    assert!((out.members[0].axial_stress - 10.0).abs() < 1e-12);
    assert!((out.fields.reactions[0] + 12.0).abs() < 1e-12);
    assert!((out.fields.reactions[1] + 16.0).abs() < 1e-12);
}
#[test]
fn frame_tip_bending_torsion_and_axial_displacement_match_theory() {
    let l = 3.0;
    let s = section(true);
    let (model, bc) = cantilever(
        vec![[0.0; 3], [l, 0.0, 0.0]],
        vec![Member::Frame {
            nodes: [0, 1],
            section: s,
            reference: [0.0, 1.0, 0.0],
            uniform_load: [0.0; 3],
        }],
    );
    let p = 100.0;
    let torque = 30.0;
    let out = model
        .solve(
            &bc,
            &[(1, U(0), p), (1, U(1), p), (1, U(2), p), (1, R(0), torque)],
            1e-12,
        )
        .unwrap();
    let expected = [
        p * l / (s.e * s.area),
        p * l.powi(3) / (3.0 * s.e * s.iz) + p * l / (s.g * 0.4),
        p * l.powi(3) / (3.0 * s.e * s.iy) + p * l / (s.g * 0.4),
    ];
    for axis in 0..3 {
        assert!((out.fields.value(1, U(axis)).unwrap() / expected[axis] - 1.0).abs() < 1e-12);
    }
    assert!(
        (out.fields.value(1, R(0)).unwrap() / (torque * l / (s.g * s.torsion)) - 1.0).abs() < 1e-12
    );
    assert!((out.members[0].local_end_actions[5] + p * l).abs() < 1e-9);
    assert!((out.members[0].local_end_actions[4] - p * l).abs() < 1e-9);
}
#[test]
fn uniform_frame_load_recovers_correct_support_shear_and_moment() {
    let l = 4.0;
    let q = 20.0;
    let s = section(false);
    let (model, bc) = cantilever(
        vec![[0.0; 3], [l, 0.0, 0.0]],
        vec![Member::Frame {
            nodes: [0, 1],
            section: s,
            reference: [0.0, 1.0, 0.0],
            uniform_load: [0.0, q, 0.0],
        }],
    );
    let out = model.solve(&bc, &[], 1e-12).unwrap();
    assert!(
        (out.fields.value(1, U(1)).unwrap() / (q * l.powi(4) / (8.0 * s.e * s.iz)) - 1.0).abs()
            < 1e-12
    );
    let actions = out.members[0].local_end_actions;
    assert!((actions[1] + q * l).abs() < 1e-9);
    assert!((actions[5] + q * l * l / 2.0).abs() < 1e-9);
    assert!(actions[6..].iter().all(|f| f.abs() < 1e-9));
}
#[test]
fn rotated_frame_preserves_local_solution_and_rigid_body_patch() {
    let s = section(false);
    let l = 2.0;
    let p = 50.0;
    // Local x -> global y, local y -> global z, local z -> global x.
    let (model, bc) = cantilever(
        vec![[2.0, 3.0, 4.0], [2.0, 3.0 + l, 4.0]],
        vec![Member::Frame {
            nodes: [0, 1],
            section: s,
            reference: [0.0, 0.0, 1.0],
            uniform_load: [0.0; 3],
        }],
    );
    let out = model.solve(&bc, &[(1, U(2), p)], 1e-12).unwrap();
    assert!(
        (out.fields.value(1, U(2)).unwrap() / (p * l.powi(3) / (3.0 * s.e * s.iz)) - 1.0).abs()
            < 1e-12
    );
    let mut rigid = model.fields().constraints();
    let omega = [0.03, -0.02, 0.01];
    for (n, x) in model.nodes().iter().enumerate() {
        let u = [
            0.2 + omega[1] * x[2] - omega[2] * x[1],
            -0.1 + omega[2] * x[0] - omega[0] * x[2],
            0.3 + omega[0] * x[1] - omega[1] * x[0],
        ];
        for axis in 0..3 {
            rigid
                .prescribe(model.fields(), n, U(axis), u[axis])
                .unwrap();
            rigid
                .prescribe(model.fields(), n, R(axis), omega[axis])
                .unwrap();
        }
    }
    let out = model.solve(&rigid, &[], 1e-12).unwrap();
    assert!(out.members[0]
        .local_end_actions
        .iter()
        .all(|f| f.abs() < 1e-8));
}
#[test]
fn subdividing_an_euler_beam_preserves_the_analytical_solution() {
    let s = section(false);
    let l = 3.0;
    let q = 20.0;
    for div in [1, 2, 4, 8] {
        let nodes = (0..=div)
            .map(|n| [l * n as f64 / div as f64, 0.0, 0.0])
            .collect();
        let members = (0..div)
            .map(|n| Member::Frame {
                nodes: [n, n + 1],
                section: s,
                reference: [0.0, 1.0, 0.0],
                uniform_load: [0.0, q, 0.0],
            })
            .collect();
        let (model, bc) = cantilever(nodes, members);
        let out = model.solve(&bc, &[], 1e-10).unwrap();
        assert!(
            (out.fields.value(div, U(1)).unwrap() / (q * l.powi(4) / (8.0 * s.e * s.iz)) - 1.0)
                .abs()
                < 1e-9
        );
    }
}
#[test]
fn mechanisms_and_bad_geometry_are_refused() {
    assert!(MemberModel::new(
        vec![[0.0; 3]; 2],
        vec![Member::Truss {
            nodes: [0, 1],
            e: 1.0,
            area: 1.0
        }]
    )
    .is_err());
    assert!(MemberModel::new(
        vec![[0.0; 3], [1.0, 0.0, 0.0]],
        vec![Member::Frame {
            nodes: [0, 1],
            section: section(false),
            reference: [1.0, 0.0, 0.0],
            uniform_load: [0.0; 3]
        }]
    )
    .is_err());
    let (model, _) = cantilever(
        vec![[0.0; 3], [1.0, 0.0, 0.0]],
        vec![Member::Truss {
            nodes: [0, 1],
            e: 1.0,
            area: 1.0,
        }],
    );
    assert!(model
        .solve(&model.fields().constraints(), &[(1, U(0), 1.0)], 1e-8)
        .is_err());
}

fn eb_section() -> FrameSection {
    FrameSection {
        shear_areas: None,
        ..section(false)
    }
}
fn released(nodes: [usize; 2], releases: EndReleases, q: [f64; 3]) -> Member {
    Member::FrameReleased {
        nodes,
        section: eb_section(),
        reference: [0.0, 1.0, 0.0],
        uniform_load: q,
        releases,
    }
}
fn fix(model: &MemberModel, bc: &mut fea_core::fields::FieldConstraints, node: usize, rot: bool) {
    for axis in 0..3 {
        bc.prescribe(model.fields(), node, U(axis), 0.0).unwrap();
        if rot {
            bc.prescribe(model.fields(), node, R(axis), 0.0).unwrap();
        }
    }
}

#[test]
fn pinned_fixed_beam_under_uniform_load_matches_closed_form() {
    // Pin at the start (node 0, bending released), wall at the end: wall moment qL^2/8,
    // reactions 3qL/8 at the pin and 5qL/8 at the wall.
    let (l, q) = (4.0, -10.0);
    let model = MemberModel::new(
        vec![[0.0; 3], [l, 0.0, 0.0]],
        vec![released([0, 1], EndReleases::pin_start(), [0.0, q, 0.0])],
    )
    .unwrap();
    let mut bc = model.fields().constraints();
    fix(&model, &mut bc, 0, false);
    fix(&model, &mut bc, 1, true);
    let out = model.solve(&bc, &[], 1e-13).unwrap();
    let reaction = |n, f| out.fields.reactions[model.fields().index(n, f).unwrap()];
    let tol = 1e-9 * q.abs() * l * l;
    assert!((reaction(0, U(1)) - 3.0 * -q * l / 8.0).abs() < tol);
    assert!((reaction(1, U(1)) - 5.0 * -q * l / 8.0).abs() < tol);
    assert!((reaction(1, R(2)).abs() - q.abs() * l * l / 8.0).abs() < tol);
    // The released start carries no moment; the member end action at the wall is the wall moment.
    let a = out.members[0].local_end_actions;
    assert!(a[3].abs() < tol && a[4].abs() < tol && a[5].abs() < tol);
    assert!((a[11].abs() - q.abs() * l * l / 8.0).abs() < tol);
    // Free-end deflection of a pinned-fixed beam at x = L(1 - 1/sqrt(5)) is the maximum,
    // qL^4/(185 EI); check the midpoint instead through equilibrium of the end forces.
    assert!((a[1] + a[7] + q * l).abs() < tol);
}

#[test]
fn released_end_passes_no_moment_to_the_joint() {
    // Joint 1 joins A (released at 1) and B (rigid). Moment M at the joint is carried by B
    // alone: theta = M L / (4 E I) (B fixed far end), not M L / (8 E I).
    let l = 3.0;
    let s = eb_section();
    let model = MemberModel::new(
        vec![[0.0; 3], [l, 0.0, 0.0], [2.0 * l, 0.0, 0.0]],
        vec![
            released([0, 1], EndReleases::pin_end(), [0.0; 3]),
            released([1, 2], EndReleases::default(), [0.0; 3]),
        ],
    )
    .unwrap();
    let mut bc = model.fields().constraints();
    fix(&model, &mut bc, 0, true);
    fix(&model, &mut bc, 2, true);
    fix(&model, &mut bc, 1, false);
    let m = 7.0;
    let out = model.solve(&bc, &[(1, R(2), m)], 1e-13).unwrap();
    let theta = m * l / (4.0 * s.e * s.iz);
    assert!((out.fields.value(1, R(2)).unwrap() - theta).abs() < 1e-12 * theta.abs());
    let a = out.members[0].local_end_actions;
    assert!(a[11].abs() < 1e-9 && a[10].abs() < 1e-9 && a[9].abs() < 1e-9);
}

#[test]
fn both_ends_pinned_reduces_to_the_truss_bar() {
    let nodes = vec![[0.0; 3], [4.0, 0.0, 0.0], [2.0, 3.0, 0.0]];
    let pairs = [[0, 2], [1, 2], [0, 1]];
    let s = eb_section();
    let solve = |members: Vec<Member>| {
        let model = MemberModel::new(nodes.clone(), members).unwrap();
        let mut bc = model.fields().constraints();
        for n in 0..3 {
            bc.prescribe(model.fields(), n, U(2), 0.0).unwrap();
        }
        for axis in 0..2 {
            bc.prescribe(model.fields(), 0, U(axis), 0.0).unwrap();
        }
        bc.prescribe(model.fields(), 1, U(1), 0.0).unwrap();
        let out = model
            .solve(&bc, &[(2, U(0), 5.0), (2, U(1), -9.0)], 1e-13)
            .unwrap();
        let u: Vec<f64> = (0..3)
            .flat_map(|n| (0..2).map(move |a| (n, a)))
            .map(|(n, a)| out.fields.value(n, U(a)).unwrap())
            .collect();
        (
            u,
            out.members
                .iter()
                .map(|m| m.axial_stress)
                .collect::<Vec<_>>(),
        )
    };
    let (ut, st) = solve(
        pairs
            .iter()
            .map(|&nodes| Member::Truss {
                nodes,
                e: s.e,
                area: s.area,
            })
            .collect(),
    );
    let (uf, sf) = solve(
        pairs
            .iter()
            .map(|&nodes| Member::FrameReleased {
                nodes,
                section: s,
                reference: [0.0, 0.0, 1.0],
                uniform_load: [0.0; 3],
                releases: EndReleases::PINNED_BOTH,
            })
            .collect(),
    );
    for (a, b) in ut.iter().zip(&uf).chain(st.iter().zip(&sf)) {
        assert!((a - b).abs() <= 1e-12 * a.abs().max(1e-6), "{a} vs {b}");
    }
}

#[test]
fn no_release_equals_plain_frame_and_torsion_release_carries_no_torque() {
    let l = 2.5;
    let run = |m: Member| {
        let (model, bc) = cantilever(vec![[0.0; 3], [l, 0.0, 0.0]], vec![m]);
        let out = model
            .solve(&bc, &[(1, U(1), 3.0), (1, R(0), 2.0)], 1e-13)
            .unwrap();
        (
            out.fields.value(1, U(1)).unwrap(),
            out.fields.reactions[model.fields().index(0, R(0)).unwrap()],
        )
    };
    let plain = Member::Frame {
        nodes: [0, 1],
        section: eb_section(),
        reference: [0.0, 1.0, 0.0],
        uniform_load: [0.0; 3],
    };
    let none = released([0, 1], EndReleases::default(), [0.0; 3]);
    assert_eq!(run(plain), run(none));
    // Torsion released at one end: the wall sees no torque from the member.
    let t = released(
        [0, 1],
        EndReleases {
            start: [false; 3],
            end: [true, false, false],
        },
        [0.0; 3],
    );
    assert!(run(t).1.abs() < 1e-12);
}
