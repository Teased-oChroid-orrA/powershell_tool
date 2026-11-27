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
use fea_core::material::J2;
use fea_core::mesh::Mesh;
use fea_core::nonlinear::{FailureMonitor, NlState, Stop};
use fea_core::report::Profile;
use mechanics_core::hardening::Hardening;
use std::sync::Arc;

#[derive(Clone, Copy, Debug)]
pub struct ContactInputs {
    pub major_in: f64,
    pub bolt_minor_in: f64,
    pub female_minor_in: f64,
    /// Radial clearance between bolt crest and modeled internal root, in.
    pub radial_clearance_in: f64,
    pub tpi: f64,
    pub turns: usize,
    /// Radial wall measured from the outermost parent thread root, in.
    pub radial_wall_in: f64,
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
    /// Actual converged travel; may be below the requested travel at rupture onset.
    pub travel_reached_in: f64,
    pub axial_reaction_lbf: f64,
    pub contact_axial_lbf: f64,
    pub min_gap_in: f64,
    pub mesh_nodes: usize,
    pub mesh_elements: usize,
    pub turn_load_lbf: Vec<f64>,
    pub profile: Profile,
    pub elapsed_ms: f64,
    pub factorisations: usize,
}

#[derive(Clone, Debug)]
pub struct MeshCheckedContact {
    pub result: ContactResult,
    pub mesh_change_fraction: f64,
    pub mesh_elements: [usize; 3],
}

/// Explicit constitutive and rupture-onset inputs for an offline tapped-hole study.
/// The strain limits are material data, not fitted from the elastic reaction.
#[derive(Clone, Copy, Debug)]
pub struct PlasticInputs {
    pub contact: ContactInputs,
    pub bolt_law_psi: Hardening,
    pub parent_law_psi: Hardening,
    pub bolt_rupture_ep: f64,
    pub parent_rupture_ep: f64,
}

#[derive(Clone, Debug)]
pub struct PlasticContactResult {
    pub contact: ContactResult,
    pub bolt_max_ep: f64,
    pub parent_max_ep: f64,
    /// Nonlocal strain in a fixed 0.3-pitch Gaussian gauge around the loaded roots.
    pub bolt_gauge_ep: f64,
    pub parent_gauge_ep: f64,
    pub bolt_yielded_fraction: f64,
    pub parent_yielded_fraction: f64,
    pub bolt_rupture_onset: bool,
    pub parent_rupture_onset: bool,
}

struct GaugeTurn {
    points: Vec<(usize, usize, f64)>,
    weight: f64,
}

struct RootGauge {
    turns: [Vec<GaugeTurn>; 2],
}

impl RootGauge {
    fn new(mesh: &Mesh, bolt_blocks: usize, inputs: ContactInputs) -> Result<Self, String> {
        let pitch = 1.0 / inputs.tpi;
        let roots = [inputs.bolt_minor_in / 2.0, inputs.major_in / 2.0 + inputs.radial_clearance_in];
        let sigma = 0.3 * pitch;
        let mut turns = [Vec::new(), Vec::new()];
        for side in 0..2 {
            let blocks = if side == 0 { 0..bolt_blocks } else { bolt_blocks..mesh.blocks.len() };
            for turn in 0..inputs.turns {
                let center = [roots[side], (turn as f64 + 0.5) * pitch];
                let mut points = Vec::new();
                let mut weight = 0.0;
                for b in blocks.clone() {
                    let block = &mesh.blocks[b];
                    let table = block.kind.table();
                    for e in 0..block.n_elems() {
                        let nodes = block.elem(e);
                        for g in 0..table.ngp {
                            let (n, dn) = (table.n_at(g), table.dn_at(g));
                            let (mut r, mut z, mut dr_dxi, mut dr_deta, mut dz_dxi, mut dz_deta) = (0.0, 0.0, 0.0, 0.0, 0.0, 0.0);
                            for (a, &node) in nodes.iter().enumerate() {
                                let x = mesh.nodes[node];
                                r += n[a] * x[0];
                                z += n[a] * x[1];
                                dr_dxi += dn[2 * a] * x[0];
                                dr_deta += dn[2 * a + 1] * x[0];
                                dz_dxi += dn[2 * a] * x[1];
                                dz_deta += dn[2 * a + 1] * x[1];
                            }
                            let det = dr_dxi * dz_deta - dr_deta * dz_dxi;
                            if !(det > 0.0 && r >= 0.0) {
                                return Err("invalid axisymmetric gauge element".into());
                            }
                            let d2 = (r - center[0]).powi(2) + (z - center[1]).powi(2);
                            if d2 > 9.0 * sigma * sigma { continue; }
                            let w = table.w[g] * det * r * (-0.5 * d2 / (sigma * sigma)).exp();
                            weight += w;
                            points.push((b, e * table.ngp + g, w));
                        }
                    }
                }
                if weight <= 0.0 { return Err("thread-root gauge has no integration points".into()); }
                turns[side].push(GaugeTurn { points, weight });
            }
        }
        Ok(Self { turns })
    }

    fn scores(&self, state: &NlState) -> [f64; 2] {
        std::array::from_fn(|side| self.turns[side].iter().map(|turn| {
            turn.points.iter().map(|&(b, g, w)| w * state.gp[b][g][6]).sum::<f64>() / turn.weight
        }).fold(0.0_f64, f64::max))
    }
}

#[derive(Clone, Debug)]
pub struct MeshCheckedPlasticContact {
    pub result: PlasticContactResult,
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

fn mesh(inputs: ContactInputs, laws: Option<(Hardening, Hardening)>) -> Result<(Model, Dirichlet, Vec<ContactSpec>, Vec<usize>, usize), String> {
    let vals = [
        inputs.major_in,
        inputs.bolt_minor_in,
        inputs.female_minor_in,
        inputs.radial_clearance_in,
        inputs.tpi,
        inputs.radial_wall_in,
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
        || inputs.radial_wall_in <= 0.0
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
    let outer_r = female_crest + inputs.radial_wall_in;
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
    let bolt_root = inputs.bolt_minor_in / 2.0;
    let bolt_size = |p: [f64; 2]| h + (0.5 * (bolt_root - p[0]).max(0.0)).min(2.0 * h);
    let parent_size = |p: [f64; 2]| h + (0.5 * (p[0] - female_crest).max(0.0)).min(2.0 * h);
    let mut mesh = mesh_region(
        &bolt_region,
        Physics::Axisymmetric,
        ElementKind::Tri6,
        &bolt_size,
        options,
    )?;
    let bolt_blocks = mesh.blocks.len();
    let parent_mesh = mesh_region(
        &parent_region,
        Physics::Axisymmetric,
        ElementKind::Tri6,
        &parent_size,
        options,
    )?;
    if let Some((bolt_law, parent_law)) = laws {
        for block in 0..mesh.blocks.len() {
            mesh.set_plasticity(block, J2::finite(bolt_law))?;
        }
        let mut parent_mesh = parent_mesh;
        for block in 0..parent_mesh.blocks.len() {
            parent_mesh.set_plasticity(block, J2::finite(parent_law))?;
        }
        mesh.append(&parent_mesh, "parent_")?;
    } else {
        mesh.append(&parent_mesh, "parent_")?;
    }
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
    Ok((model, bc, vec![contact], bolt_top, bolt_blocks))
}

/// Solve a specified elastic travel; never interpret its reaction as ultimate capacity.
pub fn solve(inputs: ContactInputs) -> Result<ContactResult, String> {
    solve_scaled(inputs, None, None).map(|(result, _)| result)
}

fn scale_law(law: Hardening, stress_psi: f64) -> Result<Hardening, String> {
    Hardening::table(&law.points().iter().map(|&(ep, sy)| (ep, sy / stress_psi)).collect::<Vec<_>>())
}

fn solve_scaled(inputs: ContactInputs, laws: Option<(Hardening, Hardening)>, limits: Option<[f64; 2]>) -> Result<(ContactResult, Option<[(f64, f64, f64); 2]>), String> {
    // fea-core is unit-free. Scale length by pitch and stress by parent E,
    // then convert the axisymmetric full-revolution force at this boundary.
    if !inputs.tpi.is_finite() || inputs.tpi <= 0.0 || !inputs.parent_e_psi.is_finite() || inputs.parent_e_psi <= 0.0 {
        return Err("invalid axisymmetric thread-contact inputs".into());
    }
    let length = 1.0 / inputs.tpi;
    let stress = inputs.parent_e_psi;
    if !length.is_finite() || !(stress * length * length).is_finite() || stress * length * length <= 0.0 {
        return Err("thread-contact unit scale overflow".into());
    }
    let unit = ContactInputs {
        major_in: inputs.major_in / length,
        bolt_minor_in: inputs.bolt_minor_in / length,
        female_minor_in: inputs.female_minor_in / length,
        radial_clearance_in: inputs.radial_clearance_in / length,
        tpi: 1.0,
        radial_wall_in: inputs.radial_wall_in / length,
        bolt_e_psi: inputs.bolt_e_psi / stress,
        parent_e_psi: 1.0,
        travel_in: inputs.travel_in / length,
        ..inputs
    };
    let laws = laws.map(|(b, p)| -> Result<_, String> { Ok((scale_law(b, stress)?, scale_law(p, stress)?)) }).transpose()?;
    let (model, bc, specs, bolt_top, bolt_blocks) = mesh(unit, laws)?;
    let gauge = laws.map(|_| RootGauge::new(&model.mesh, bolt_blocks, unit).map(Arc::new)).transpose()?;
    let failure_monitor = gauge.as_ref().zip(limits).map(|(gauge, [bolt, parent])| {
        let gauge = Arc::clone(gauge);
        FailureMonitor(Arc::new(move |state| {
            let [a, b] = gauge.scores(state);
            a >= bolt || b >= parent
        }))
    });
    let solution = model.solve_nonlinear_contact(
        &Loads::default(),
        &bc,
        specs,
        &NlOptions {
            steps: if laws.is_some() { 8 } else { 1 },
            ep_step: if laws.is_some() { 0.01 } else { 0.0 },
            failure_monitor,
            ..NlOptions::default()
        },
    )?;
    if !solution.complete() && !(limits.is_some() && solution.stop == Stop::FailureStrain) {
        return Err(format!(
            "thread contact did not converge: {:?}",
            solution.stop
        ));
    }
    let reaction: f64 = bolt_top
        .iter()
        .map(|&n| solution.reactions[2 * n + 1])
        .sum::<f64>()
        .abs() * stress * length * length;
    let contact = solution.contact.as_ref().ok_or("contact result missing")?;
    let axial = contact.master_force[0][1].abs() * stress * length * length;
    if axial == 0.0 || reaction == 0.0 {
        return Err("no thread flank load at specified travel".into());
    }
    let pitch = 1.0;
    let slave_axial_direction = -contact.master_force[0][1].signum();
    let mut turn_load_lbf = vec![0.0; inputs.turns];
    for t in solution.contact_tractions(0, 0.0) {
        let turn = (t.x[1] / pitch)
            .floor()
            .clamp(0.0, (inputs.turns - 1) as f64) as usize;
        turn_load_lbf[turn] += t.pressure * t.normal[1] * slave_axial_direction * t.weight * stress * length * length;
    }
    let plastic_state = gauge.map(|gauge| {
        let scores = gauge.scores(&solution.state);
        let summary = |range: std::ops::Range<usize>| {
            let (mut count, mut yielded, mut max_ep) = (0usize, 0usize, 0.0_f64);
            for g in range.flat_map(|b| solution.state.gp[b].iter()) {
                count += 1;
                yielded += usize::from(g[6] > 0.0);
                max_ep = max_ep.max(g[6]);
            }
            (max_ep, yielded as f64 / count as f64)
        };
        let (bolt_max, bolt_fraction) = summary(0..bolt_blocks);
        let (parent_max, parent_fraction) = summary(bolt_blocks..model.mesh.blocks.len());
        [(bolt_max, bolt_fraction, scores[0]), (parent_max, parent_fraction, scores[1])]
    });
    Ok((ContactResult {
        travel_reached_in: solution.lambda * inputs.travel_in,
        axial_reaction_lbf: reaction,
        contact_axial_lbf: axial,
        min_gap_in: contact.min_gap * length,
        mesh_nodes: model.mesh.nodes.len(),
        mesh_elements: model.mesh.n_elems(),
        turn_load_lbf,
        profile: solution.profile,
        elapsed_ms: solution.elapsed_ms,
        factorisations: solution.factorisations,
    }, plastic_state))
}

/// Plastic response and local rupture initiation at a prescribed travel.
/// This does not remove failed material, follow a crack, or establish pull-out capacity.
pub fn solve_plastic(inputs: PlasticInputs) -> Result<PlasticContactResult, String> {
    for ep in [inputs.bolt_rupture_ep, inputs.parent_rupture_ep] {
        if !ep.is_finite() || ep <= 0.0 {
            return Err("rupture strains must be positive and finite".into());
        }
    }
    let (contact, state) = solve_scaled(inputs.contact, Some((inputs.bolt_law_psi, inputs.parent_law_psi)), Some([inputs.bolt_rupture_ep, inputs.parent_rupture_ep]))?;
    let [(bolt_max_ep, bolt_yielded_fraction, bolt_gauge_ep), (parent_max_ep, parent_yielded_fraction, parent_gauge_ep)] = state.expect("plastic blocks assigned");
    Ok(PlasticContactResult {
        contact,
        bolt_max_ep,
        parent_max_ep,
        bolt_gauge_ep,
        parent_gauge_ep,
        bolt_yielded_fraction,
        parent_yielded_fraction,
        bolt_rupture_onset: bolt_gauge_ep >= inputs.bolt_rupture_ep,
        parent_rupture_onset: parent_gauge_ep >= inputs.parent_rupture_ep,
    })
}

/// Require three-mesh convergence before using a plastic response as evidence.
/// Peak strain is local and can fail this gate at a sharp thread root.
pub fn solve_mesh_checked_plastic(inputs: PlasticInputs) -> Result<MeshCheckedPlasticContact, String> {
    let mut results = Vec::with_capacity(3);
    for edge_fraction in [0.2, 0.1, 0.07] {
        let result = solve_plastic(PlasticInputs {
            contact: ContactInputs { edge_fraction, ..inputs.contact },
            ..inputs
        })?;
        let c = &result.contact;
        let balance = (c.contact_axial_lbf / c.axial_reaction_lbf - 1.0).abs();
        let turns = (c.turn_load_lbf.iter().sum::<f64>() / c.axial_reaction_lbf - 1.0).abs();
        if balance > 1e-5 || turns > 1e-5 || c.min_gap_in < -1e-6 {
            return Err(format!("plastic thread-contact force balance or gap check failed at {} elements: force {:.3e}, turns {:.3e}, gap {:.3e} in", c.mesh_elements, balance, turns, c.min_gap_in));
        }
        results.push(result);
    }
    let fine = &results[1];
    let finer = &results[2];
    let relative = |a: f64, b: f64| (a - b).abs() / a.abs().max(b.abs()).max(1e-12);
    let change = [
        relative(fine.contact.axial_reaction_lbf, finer.contact.axial_reaction_lbf),
        relative(fine.contact.travel_reached_in, finer.contact.travel_reached_in),
        relative(fine.bolt_gauge_ep, finer.bolt_gauge_ep),
        relative(fine.parent_gauge_ep, finer.parent_gauge_ep),
        (fine.bolt_yielded_fraction - finer.bolt_yielded_fraction).abs(),
        (fine.parent_yielded_fraction - finer.parent_yielded_fraction).abs(),
    ].into_iter().fold(0.0_f64, f64::max);
    let turn_change = fine.contact.turn_load_lbf.iter().zip(&finer.contact.turn_load_lbf)
        .map(|(a, b)| (a - b).abs() / finer.contact.axial_reaction_lbf)
        .fold(0.0_f64, f64::max);
    if change.max(turn_change) > 0.02
        || fine.bolt_rupture_onset != finer.bolt_rupture_onset
        || fine.parent_rupture_onset != finer.parent_rupture_onset {
        return Err(format!("plastic thread-contact mesh did not converge: response {:.1}%, turn {:.1}%", 100.0 * change, 100.0 * turn_change));
    }
    let mesh_elements = [results[0].contact.mesh_elements, results[1].contact.mesh_elements, results[2].contact.mesh_elements];
    Ok(MeshCheckedPlasticContact { result: results.pop().unwrap(), mesh_change_fraction: change.max(turn_change), mesh_elements })
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
            radial_wall_in: 0.125,
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
        eprintln!("elastic contact profile: {:.1} ms total, {:.1} ms assembly, {:.1} ms factorization, {:.1} ms contact, {} factors", out.elapsed_ms, out.profile.assembly_ms, out.profile.factorisation_ms, out.profile.contact_ms, out.factorisations);
        assert!(out.axial_reaction_lbf > 0.0);
        assert!((out.contact_axial_lbf / out.axial_reaction_lbf - 1.0).abs() < 0.05);
        assert!(
            (out.turn_load_lbf.iter().sum::<f64>() / out.axial_reaction_lbf - 1.0).abs() < 1e-5
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
        assert_eq!(before.radial_wall_in, after.radial_wall_in);
        assert_eq!(before.travel_in, after.travel_in);
        let a = solve(before).unwrap();
        let b = solve(after).unwrap();
        assert_ne!(a.axial_reaction_lbf, b.axial_reaction_lbf);
    }

    #[test]
    fn parent_wall_starts_at_internal_thread_root() {
        let inputs = pilot();
        let (model, _, _, _, _) = mesh(inputs, None).unwrap();
        let outer = model
            .mesh
            .nodes
            .iter()
            .map(|p| p[0])
            .fold(0.0_f64, f64::max);
        let expected = inputs.major_in / 2.0 + inputs.radial_clearance_in + inputs.radial_wall_in;
        assert!((outer - expected).abs() < 1e-10);
    }

    #[test]
    fn travel_below_clearance_has_no_contact_result() {
        let inputs = ContactInputs {
            radial_clearance_in: 0.005,
            travel_in: 1e-6,
            ..pilot()
        };
        assert!(solve(inputs).unwrap_err().contains("no thread flank load"));
    }

    #[test]
    fn normalized_plastic_path_obeys_length_scaling() {
        let elastic = solve(pilot()).unwrap();
        assert!((elastic.axial_reaction_lbf / 10_376.38 - 1.0).abs() < 0.15);
        let high = Hardening::linear(1.0e12, 0.0, 1.0).unwrap();
        let physical = PlasticInputs {
            contact: pilot(),
            bolt_law_psi: high,
            parent_law_psi: high,
            bolt_rupture_ep: 0.1,
            parent_rupture_ep: 0.1,
        };
        let plastic = solve_plastic(physical).unwrap();
        assert_eq!(plastic.bolt_max_ep, 0.0);
        assert_eq!(plastic.parent_max_ep, 0.0);
        assert!((plastic.contact.travel_reached_in - physical.contact.travel_in).abs() < 1e-12);
        let c = physical.contact;
        let doubled = solve_plastic(PlasticInputs {
            contact: ContactInputs {
                major_in: 2.0 * c.major_in,
                bolt_minor_in: 2.0 * c.bolt_minor_in,
                female_minor_in: 2.0 * c.female_minor_in,
                radial_clearance_in: 2.0 * c.radial_clearance_in,
                tpi: c.tpi / 2.0,
                radial_wall_in: 2.0 * c.radial_wall_in,
                travel_in: 2.0 * c.travel_in,
                ..c
            },
            ..physical
        }).unwrap();
        assert!((doubled.contact.axial_reaction_lbf / plastic.contact.axial_reaction_lbf - 4.0).abs() < 1e-5);
        let stress_doubled = solve_plastic(PlasticInputs {
            contact: ContactInputs {
                bolt_e_psi: 2.0 * c.bolt_e_psi,
                parent_e_psi: 2.0 * c.parent_e_psi,
                ..c
            },
            bolt_law_psi: Hardening::linear(2.0e12, 0.0, 1.0).unwrap(),
            parent_law_psi: Hardening::linear(2.0e12, 0.0, 1.0).unwrap(),
            ..physical
        }).unwrap();
        assert!((stress_doubled.contact.axial_reaction_lbf / plastic.contact.axial_reaction_lbf - 2.0).abs() < 1e-5);
    }

    #[test]
    fn parent_yield_and_rupture_onset_are_distinct() {
        let high = Hardening::linear(1.0e12, 0.0, 1.0).unwrap();
        let soft = Hardening::linear(20_000.0, 1_000.0, 1.0).unwrap();
        let result = solve_plastic(PlasticInputs {
            contact: pilot(),
            bolt_law_psi: high,
            parent_law_psi: soft,
            bolt_rupture_ep: 0.1,
            parent_rupture_ep: 1e-8,
        }).unwrap();
        assert_eq!(result.bolt_max_ep, 0.0);
        assert!(result.parent_max_ep > 1e-8);
        assert_eq!(result.bolt_yielded_fraction, 0.0);
        assert!(result.parent_yielded_fraction > 0.0);
        assert!(!result.bolt_rupture_onset);
        assert!(result.parent_rupture_onset);
        assert!(result.contact.travel_reached_in < pilot().travel_in);
        assert!(result.contact.axial_reaction_lbf > 0.0);
    }

    #[test]
    fn bolt_rupture_onset_does_not_imply_parent_stripping() {
        let high = Hardening::linear(1.0e12, 0.0, 1.0).unwrap();
        let soft = Hardening::linear(20_000.0, 1_000.0, 1.0).unwrap();
        let result = solve_plastic(PlasticInputs {
            contact: pilot(),
            bolt_law_psi: soft,
            parent_law_psi: high,
            bolt_rupture_ep: 1e-8,
            parent_rupture_ep: 0.1,
        }).unwrap();
        assert!(result.bolt_max_ep > 1e-8);
        assert_eq!(result.parent_max_ep, 0.0);
        assert!(result.bolt_rupture_onset);
        assert!(!result.parent_rupture_onset);
        assert!(result.contact.travel_reached_in < pilot().travel_in);
    }

    #[test]
    fn plastic_response_requires_three_meshes() {
        let high = Hardening::linear(1.0e12, 0.0, 1.0).unwrap();
        let checked = solve_mesh_checked_plastic(PlasticInputs {
            contact: pilot(),
            bolt_law_psi: high,
            parent_law_psi: high,
            bolt_rupture_ep: 0.1,
            parent_rupture_ep: 0.1,
        }).unwrap();
        assert!(checked.mesh_elements[0] < checked.mesh_elements[1]);
        assert!(checked.mesh_elements[1] < checked.mesh_elements[2]);
        assert!(checked.mesh_change_fraction < 0.02);
    }

    #[test]
    fn yielding_parent_never_bypasses_the_mesh_gate() {
        let high = Hardening::linear(1.0e12, 0.0, 1.0).unwrap();
        let soft = Hardening::linear(20_000.0, 1_000.0, 1.0).unwrap();
        let outcome = solve_mesh_checked_plastic(PlasticInputs {
            contact: pilot(),
            bolt_law_psi: high,
            parent_law_psi: soft,
            bolt_rupture_ep: 0.1,
            parent_rupture_ep: 0.1,
        });
        if let Err(error) = &outcome {
            eprintln!("yielding parent mesh gate: {error}");
            assert!(error.contains("did not converge") || error.contains("force balance or gap"), "{error}");
        }
        if let Ok(checked) = outcome {
            eprintln!("yielding parent: {:.1} ms total, {:.1} ms assembly, {:.1} ms factorization, {:.1} ms contact, {} factors, {:.3} mesh change", checked.result.contact.elapsed_ms, checked.result.contact.profile.assembly_ms, checked.result.contact.profile.factorisation_ms, checked.result.contact.profile.contact_ms, checked.result.contact.factorisations, checked.mesh_change_fraction);
            assert!(checked.mesh_change_fraction <= 0.02);
        }
    }
}
