//! Offline axisymmetric contact pilot for directly tapped Unified threads.
//! Revolved truncated V profiles are rings, not a helical thread. Elastic
//! reactions and contact forces are diagnostics, not pull-out capacity.

use fea_core::{
    contact::ContactSpec,
    delaunay::MeshOptions,
    geometry::{Loop, Region},
    mesh2d::mesh_region,
    Dirichlet, Elastic, ElementKind, Loads, Model, NlOptions, Physics,
};

#[derive(Clone, Copy, Debug)]
pub struct ContactInputs {
    pub major_in: f64,
    pub bolt_minor_in: f64,
    pub female_minor_in: f64,
    /// Radial clearance between bolt crest and modeled internal root, in.
    pub radial_clearance_in: f64,
    pub tpi: f64,
    pub turns: usize,
    /// Pilot-only radial wall measured from nominal major radius, in.
    pub wall_from_major_in: f64,
    pub bolt_e_psi: f64,
    pub bolt_nu: f64,
    pub parent_e_psi: f64,
    pub parent_nu: f64,
    /// Prescribed upward travel at the bolt's loaded face, in.
    pub travel_in: f64,
    /// Target boundary edge size as a fraction of pitch.
    pub edge_fraction: f64,
}

#[derive(Clone, Debug)]
pub struct ContactResult {
    pub axial_reaction_lbf: f64,
    pub contact_axial_lbf: f64,
    pub min_gap_in: f64,
    pub mesh_nodes: usize,
    pub mesh_elements: usize,
    pub turn_load_lbf: Vec<f64>,
}

#[derive(Clone, Debug)]
pub struct MeshCheckedContact {
    pub result: ContactResult,
    pub mesh_change_fraction: f64,
    pub mesh_elements: [usize; 3],
}

fn profile(root: f64, crest: f64, pitch: f64, turns: usize) -> Result<Vec<[f64; 2]>, String> {
    let flank = (crest - root) / (3.0_f64.sqrt() * pitch);
    let flat = (0.5 - flank) / 2.0;
    if !(flat > 0.0 && flat < 0.25) {
        return Err("thread diameters cannot form a truncated 60-degree V at this pitch".into());
    }
    let mut p = vec![[root, 0.0]];
    for i in 0..turns {
        let z = i as f64 * pitch;
        p.extend([
            [root, z + flat * pitch],
            [crest, z + (0.5 - flat) * pitch],
            [crest, z + (0.5 + flat) * pitch],
            [root, z + (1.0 - flat) * pitch],
            [root, z + pitch],
        ]);
    }
    Ok(p)
}

fn mesh(inputs: ContactInputs) -> Result<(Model, Dirichlet, Vec<ContactSpec>, Vec<usize>), String> {
    let vals = [
        inputs.major_in,
        inputs.bolt_minor_in,
        inputs.female_minor_in,
        inputs.radial_clearance_in,
        inputs.tpi,
        inputs.wall_from_major_in,
        inputs.bolt_e_psi,
        inputs.bolt_nu,
        inputs.parent_e_psi,
        inputs.parent_nu,
        inputs.travel_in,
        inputs.edge_fraction,
    ];
    if vals.iter().any(|v| !v.is_finite())
        || inputs.turns == 0
        || inputs.turns > 40
        || !(inputs.major_in > inputs.female_minor_in
            && inputs.female_minor_in > inputs.bolt_minor_in
            && inputs.bolt_minor_in > 0.0)
        || inputs.tpi <= 0.0
        || inputs.radial_clearance_in < 0.0
        || inputs.wall_from_major_in <= 0.0
        || inputs.travel_in <= 0.0
        || !(0.0 < inputs.edge_fraction && inputs.edge_fraction <= 0.25)
        || inputs.bolt_e_psi <= 0.0
        || inputs.parent_e_psi <= 0.0
        || !(-1.0..0.5).contains(&inputs.bolt_nu)
        || !(-1.0..0.5).contains(&inputs.parent_nu)
    {
        return Err("invalid axisymmetric thread-contact inputs".into());
    }
    let pitch = 1.0 / inputs.tpi;
    let length = inputs.turns as f64 * pitch;
    let major_r = 0.5 * inputs.major_in;
    let female_crest = major_r + inputs.radial_clearance_in;
    let bolt = profile(0.5 * inputs.bolt_minor_in, major_r, pitch, inputs.turns)?;
    let parent = profile(
        0.5 * inputs.female_minor_in,
        female_crest,
        pitch,
        inputs.turns,
    )?;
    let outer_r = major_r + inputs.wall_from_major_in;
    if outer_r <= female_crest + pitch * inputs.edge_fraction {
        return Err("parent wall is too thin for the contact mesh".into());
    }
    let bolt_points: Vec<[f64; 2]> = std::iter::once([0.0, 0.0])
        .chain(bolt.iter().copied())
        .chain([[0.0, length]])
        .collect();
    let mut bolt_names = vec!["bolt_top"];
    bolt_names.extend(std::iter::repeat_n("bolt_thread", bolt.len() - 1));
    bolt_names.extend(["bolt_bottom", "bolt_axis"]);
    let bolt_region = Region::new(
        Loop::polygon_named(&bolt_points, &bolt_names)?,
        vec![],
        Elastic::new(inputs.bolt_e_psi, inputs.bolt_nu),
    )?;
    let mut parent_points = vec![
        [parent[0][0], 0.0],
        [outer_r, 0.0],
        [outer_r, length],
        [parent.last().unwrap()[0], length],
    ];
    parent_points.extend(parent[1..parent.len() - 1].iter().rev().copied());
    let mut parent_names = vec!["parent_top", "parent_outer", "parent_bottom"];
    parent_names.extend(std::iter::repeat_n("parent_thread", parent.len() - 1));
    let parent_region = Region::new(
        Loop::polygon_named(&parent_points, &parent_names)?,
        vec![],
        Elastic::new(inputs.parent_e_psi, inputs.parent_nu),
    )?;
    let h = pitch * inputs.edge_fraction;
    let options = MeshOptions::default();
    let mut mesh = mesh_region(
        &bolt_region,
        Physics::Axisymmetric,
        ElementKind::Tri6,
        &|_| h,
        options,
    )?;
    let parent_mesh = mesh_region(
        &parent_region,
        Physics::Axisymmetric,
        ElementKind::Tri6,
        &|_| h,
        options,
    )?;
    mesh.append(&parent_mesh, "parent_")?;
    let model = Model::new(mesh)?;
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("bolt_axis")? {
        bc.fix(n, 0, 0.0);
    }
    let bolt_top = model.mesh.node_set("bolt_top")?.to_vec();
    for &n in &bolt_top {
        bc.fix(n, 1, -inputs.travel_in);
    }
    for &n in model.mesh.node_set("parent_parent_top")? {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    let bolt_face = model.mesh.surfaces["bolt_thread"].clone();
    let parent_face = model.mesh.surfaces["parent_parent_thread"].clone();
    let penalty = 20.0 * inputs.parent_e_psi.min(inputs.bolt_e_psi) / h;
    let contact = ContactSpec::deformable("tapped flanks", bolt_face, parent_face, penalty)
        .with_margin(2.0 * pitch);
    Ok((model, bc, vec![contact], bolt_top))
}

/// Solve a specified elastic travel; never interpret its reaction as ultimate capacity.
pub fn solve(inputs: ContactInputs) -> Result<ContactResult, String> {
    let (model, bc, specs, bolt_top) = mesh(inputs)?;
    let solution = model.solve_nonlinear_contact(
        &Loads::default(),
        &bc,
        specs,
        &NlOptions {
            steps: 1,
            ..NlOptions::default()
        },
    )?;
    if !solution.complete() {
        return Err(format!(
            "thread contact did not converge: {:?}",
            solution.stop
        ));
    }
    let reaction: f64 = bolt_top
        .iter()
        .map(|&n| solution.reactions[2 * n + 1])
        .sum::<f64>()
        .abs();
    let contact = solution.contact.as_ref().ok_or("contact result missing")?;
    let axial = contact.master_force[0][1].abs();
    let pitch = 1.0 / inputs.tpi;
    let mut turn_load_lbf = vec![0.0; inputs.turns];
    for t in solution.contact_tractions(0, 0.0) {
        let turn = (t.x[1] / pitch)
            .floor()
            .clamp(0.0, (inputs.turns - 1) as f64) as usize;
        turn_load_lbf[turn] += t.pressure * t.normal[1].abs() * t.weight;
    }
    Ok(ContactResult {
        axial_reaction_lbf: reaction,
        contact_axial_lbf: axial,
        min_gap_in: contact.min_gap,
        mesh_nodes: model.mesh.nodes.len(),
        mesh_elements: model.mesh.n_elems(),
        turn_load_lbf,
    })
}

/// Refuse a contact reaction until three meshes balance force, keep penetration
/// below 0.000001 in, and converge the total and per-turn loads.
pub fn solve_mesh_checked(inputs: ContactInputs) -> Result<MeshCheckedContact, String> {
    let mut results = Vec::with_capacity(3);
    for edge_fraction in [0.2, 0.1, 0.07] {
        let result = solve(ContactInputs {
            edge_fraction,
            ..inputs
        })?;
        let balance = (result.contact_axial_lbf - result.axial_reaction_lbf).abs()
            / result.axial_reaction_lbf.max(1.0);
        let turns = (result.turn_load_lbf.iter().sum::<f64>() - result.axial_reaction_lbf).abs()
            / result.axial_reaction_lbf.max(1.0);
        if balance > 1e-5 || turns > 1e-5 || result.min_gap_in < -1e-6 {
            return Err("thread-contact force balance or gap check failed".into());
        }
        results.push(result);
    }
    let coarse = &results[0];
    let fine = &results[1];
    let finer = &results[2];
    let delta = (finer.axial_reaction_lbf / fine.axial_reaction_lbf - 1.0).abs();
    let turn_delta = finer
        .turn_load_lbf
        .iter()
        .zip(&fine.turn_load_lbf)
        .map(|(a, b)| (a - b).abs() / finer.axial_reaction_lbf)
        .fold(0.0_f64, f64::max);
    if delta > 0.02 || turn_delta > 0.02 {
        return Err(format!(
            "thread-contact mesh did not converge: total {:.1}%, turn {:.1}%",
            100.0 * delta,
            100.0 * turn_delta
        ));
    }
    let mesh_elements = [
        coarse.mesh_elements,
        fine.mesh_elements,
        finer.mesh_elements,
    ];
    Ok(MeshCheckedContact {
        mesh_change_fraction: delta.max(turn_delta),
        mesh_elements,
        result: results.pop().unwrap(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pilot() -> ContactInputs {
        ContactInputs {
            major_in: 0.25,
            bolt_minor_in: 0.205,
            female_minor_in: 0.218,
            radial_clearance_in: 0.002 / 28.0,
            tpi: 28.0,
            turns: 2,
            wall_from_major_in: 0.125,
            bolt_e_psi: 29e6,
            bolt_nu: 0.29,
            parent_e_psi: 10e6,
            parent_nu: 0.33,
            travel_in: 0.004,
            edge_fraction: 0.2,
        }
    }

    #[test]
    fn contact_transmits_pull_and_balances_reaction() {
        let out = solve(pilot()).unwrap();
        assert!(out.axial_reaction_lbf > 0.0);
        assert!((out.contact_axial_lbf / out.axial_reaction_lbf - 1.0).abs() < 0.05);
        assert!(
            (out.turn_load_lbf.iter().sum::<f64>() / out.axial_reaction_lbf - 1.0).abs() < 0.05
        );
        assert!(out.min_gap_in > -1e-6);
    }

    #[test]
    fn refinement_changes_response_by_a_bounded_amount() {
        let checked = solve_mesh_checked(pilot()).unwrap();
        assert!(checked.mesh_elements[0] < checked.mesh_elements[1]);
        assert!(checked.mesh_elements[1] < checked.mesh_elements[2]);
        assert!(checked.mesh_change_fraction < 0.02);
    }

    #[test]
    fn female_minor_change_preserves_other_thread_geometry() {
        let before = pilot();
        let after = ContactInputs {
            female_minor_in: before.female_minor_in + 0.002,
            ..before
        };
        assert_eq!(before.major_in, after.major_in);
        assert_eq!(before.bolt_minor_in, after.bolt_minor_in);
        assert_eq!(before.tpi, after.tpi);
        assert_eq!(before.wall_from_major_in, after.wall_from_major_in);
        assert_eq!(before.travel_in, after.travel_in);
        let a = solve(before).unwrap();
        let b = solve(after).unwrap();
        assert_ne!(a.axial_reaction_lbf, b.axial_reaction_lbf);
    }
}
