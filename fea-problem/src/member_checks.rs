//! Stress and column-buckling margins of a solved frame / truss model (#234: "column buckling and stress margins").
//!
//! A post-process of [`crate::members::MemberSolved`]: it adds no unknowns and changes no solve. The allowable stress and the
//! fibre distances are *inputs* ([`MemberAllowable`]); nothing is defaulted from a material table.
//!
//! - **Stress margin** `MS = sigma_allow / sigma_max - 1` with `sigma_max = |N|/A + |M_y| c_z / I_y + |M_z| c_y / I_z` at the worse
//!   end of the member (the corner of the section: conservative for biaxial bending). A frame under a uniform load can exceed its end
//!   moments in the span; this is an end-action check, like [`crate::members::MemberResult::max_bending_moment`].
//! - **Buckling margin** (compression only) `MS = P_allow / P - 1` with `P_allow` from the Euler load `pi^2 E I_min / (K L)^2`, or the
//!   Johnson parabola when `K L / r` is below `C_c = sqrt(2 pi^2 E / sigma_y)` (the Euler stress would exceed half the yield).
//!   Frames only: a pin-jointed truss member carries no section inertia in [`crate::members::MemberKind::Truss`], so it gets the
//!   stress margin and no buckling margin (`buckling: None`).
use crate::members::{MemberKind, MemberProblem, MemberSolved};

/// What a margin needs beyond the model: the yield (allowable) stress of the member, the effective-length factor `K` of its end
/// fixity (1.0 pinned-pinned, 0.5 fixed-fixed, 0.7 fixed-pinned, 2.0 cantilever) and the distances from the neutral axes to the
/// extreme fibres (`fibre_y` along local y for bending about z, `fibre_z` along local z for bending about y).
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberAllowable {
    pub allowable_stress: f64,
    pub effective_length_factor: f64,
    pub fibre_y: f64,
    pub fibre_z: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StressCheck {
    pub sigma_max: f64,
    pub margin: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum BucklingMode {
    /// `K L / r` at or above `C_c`: the Euler load.
    Euler,
    /// Short column: the Johnson parabola.
    Johnson,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BucklingCheck {
    pub slenderness: f64,
    pub critical_load: f64,
    pub mode: BucklingMode,
    /// `None` when the member is not in compression.
    pub margin: Option<f64>,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemberMargins {
    pub id: u32,
    pub stress: StressCheck,
    pub buckling: Option<BucklingCheck>,
}

/// Critical compressive load of a column of section `(e, area, i_min)`, length `length`, effective-length factor `k`, yield `sigma_y`.
pub fn critical_load(e: f64, area: f64, i_min: f64, length: f64, k: f64, sigma_y: f64) -> (f64, f64, BucklingMode) {
    let radius = (i_min / area).sqrt();
    let slenderness = k * length / radius;
    let c_c = (2.0 * std::f64::consts::PI.powi(2) * e / sigma_y).sqrt();
    if slenderness >= c_c {
        (std::f64::consts::PI.powi(2) * e * i_min / (k * length).powi(2), slenderness, BucklingMode::Euler)
    } else {
        let stress = sigma_y * (1.0 - sigma_y * slenderness * slenderness / (4.0 * std::f64::consts::PI.powi(2) * e));
        (stress * area, slenderness, BucklingMode::Johnson)
    }
}

/// Margins of every member of `problem` in `solved`. `allowable(id)` gives each member's inputs; a member it returns `None` for is
/// skipped. A non-positive allowable, factor or fibre distance is refused.
pub fn margins(problem: &MemberProblem, solved: &MemberSolved, allowable: impl Fn(u32) -> Option<MemberAllowable>) -> Result<Vec<MemberMargins>, String> {
    let mut out = Vec::new();
    for (spec, result) in problem.members.iter().zip(&solved.members) {
        let Some(a) = allowable(spec.id) else { continue };
        for (v, what) in [(a.allowable_stress, "allowable stress"), (a.effective_length_factor, "effective-length factor"), (a.fibre_y, "fibre distance y"), (a.fibre_z, "fibre distance z")] {
            if !(v.is_finite() && v > 0.0) {
                return Err(format!("member {}: the {what} must be positive", spec.id));
            }
        }
        let act = &result.end_actions;
        let (stress, buckling) = match spec.kind {
            MemberKind::Truss { e, area } => {
                let _ = e;
                let sigma = result.axial_force.abs() / area;
                (StressCheck { sigma_max: sigma, margin: a.allowable_stress / sigma.max(f64::MIN_POSITIVE) - 1.0 }, None)
            }
            MemberKind::Frame { section, .. } => {
                // Moments about local y / z at either end; the larger combination.
                let bending = |end: usize| act[end + 1].abs() * a.fibre_z / section.iy + act[end + 2].abs() * a.fibre_y / section.iz;
                let sigma = result.axial_force.abs() / section.area + bending(3).max(bending(9));
                let (p_cr, slenderness, mode) = critical_load(section.e, section.area, section.iy.min(section.iz), result.length, a.effective_length_factor, a.allowable_stress);
                let compression = -result.axial_force;
                let margin = (compression > 0.0).then(|| p_cr / compression - 1.0);
                (StressCheck { sigma_max: sigma, margin: a.allowable_stress / sigma.max(f64::MIN_POSITIVE) - 1.0 }, Some(BucklingCheck { slenderness, critical_load: p_cr, mode, margin }))
            }
        };
        out.push(MemberMargins { id: spec.id, stress, buckling });
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::members::{templates, Edit, MemberKind, SectionSpec};

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * b.abs(), "{a} vs {b}");
    }
    fn input(sigma: f64) -> MemberAllowable {
        MemberAllowable { allowable_stress: sigma, effective_length_factor: 1.0, fibre_y: 1.0, fibre_z: 1.0 }
    }

    #[test]
    fn euler_and_johnson_meet_at_the_transition_and_follow_the_formulas() {
        let (e, sy) = (1.0e7, 40_000.0);
        let s = SectionSpec::solid_round(e, 0.33, 1.0);
        // Slender: K L / r = 100 / 0.5 = 200 > C_c = 70.2: Euler, pi^2 E I / L^2 = 7751 lbf.
        let (p, lam, mode) = critical_load(e, s.area, s.iy, 100.0, 1.0, sy);
        assert_eq!(mode, BucklingMode::Euler);
        close(lam, 200.0, 1e-12);
        close(p, std::f64::consts::PI.powi(2) * e * s.iy / 1e4, 1e-12);
        // Short: L = 20, K L / r = 40 < C_c: Johnson, sigma_y (1 - sigma_y lam^2 / (4 pi^2 E)).
        let (p, lam, mode) = critical_load(e, s.area, s.iy, 20.0, 1.0, sy);
        assert_eq!(mode, BucklingMode::Johnson);
        close(p, sy * (1.0 - sy * lam * lam / (4.0 * std::f64::consts::PI.powi(2) * e)) * s.area, 1e-12);
        // Continuity at C_c: Euler stress is half the yield there, and so is Johnson.
        let c_c = (2.0 * std::f64::consts::PI.powi(2) * e / sy).sqrt();
        let r = (s.iy / s.area).sqrt();
        let (pj, _, _) = critical_load(e, s.area, s.iy, c_c * r * (1.0 - 1e-9), 1.0, sy);
        let (pe, _, _) = critical_load(e, s.area, s.iy, c_c * r * (1.0 + 1e-9), 1.0, sy);
        close(pj, pe, 1e-6);
        close(pe, 0.5 * sy * s.area, 1e-6);
    }

    #[test]
    fn the_cantilever_template_stress_matches_m_c_over_i() {
        let p = templates().remove(1);
        let solved = p.solve().unwrap();
        let m = margins(&p, &solved, |_| Some(input(60_000.0))).unwrap();
        // Tip load 100 lbf at 50 in, solid round r = 1: sigma = 100 * 50 * 1 / (pi / 4) = 6366.2 psi.
        close(m[0].stress.sigma_max, 100.0 * 50.0 / (std::f64::consts::PI / 4.0), 1e-6);
        close(m[0].stress.margin, 60_000.0 / m[0].stress.sigma_max - 1.0, 1e-12);
        assert!(m[0].buckling.unwrap().margin.is_none(), "a beam in bending only is not in compression");
    }

    #[test]
    fn a_compressed_pinned_column_gets_the_euler_margin_and_a_tension_member_none() {
        // A 100 in column under 1000 lbf compression (planar model along x, pinned at the far end by a roller).
        let mut p = MemberProblem::new("column");
        p.plane = Some(2);
        let s = SectionSpec::solid_round(1.0e7, 0.33, 1.0);
        let a = p.apply(Edit::AddNode { pos: [0.0; 3] }).unwrap().unwrap();
        let b = p.apply(Edit::AddNode { pos: [100.0, 0.0, 0.0] }).unwrap().unwrap();
        p.apply(Edit::AddMember { a, b, kind: MemberKind::Frame { section: s, reference: [0.0, 1.0, 0.0], uniform_load: [0.0; 3] } }).unwrap();
        p.apply(Edit::SetSupport { node: a, fixed: [true, true, true, false, false, true] }).unwrap();
        p.apply(Edit::SetSupport { node: b, fixed: [false, true, true, false, false, false] }).unwrap();
        p.apply(Edit::SetLoad { node: b, force: [-1000.0, 0.0, 0.0], moment: [0.0; 3] }).unwrap();
        let solved = p.solve().unwrap();
        let m = margins(&p, &solved, |_| Some(input(40_000.0))).unwrap();
        let b = m[0].buckling.unwrap();
        close(b.critical_load, std::f64::consts::PI.powi(2) * 1.0e7 * s.iy / 1e4, 1e-9);
        close(b.margin.unwrap(), b.critical_load / 1000.0 - 1.0, 1e-6);
        close(m[0].stress.sigma_max, 1000.0 / s.area, 1e-6);
        // Reverse the load: tension, no buckling margin.
        p.apply(Edit::SetLoad { node: 2, force: [1000.0, 0.0, 0.0], moment: [0.0; 3] }).unwrap();
        let t = margins(&p, &p.solve().unwrap(), |_| Some(input(40_000.0))).unwrap();
        assert!(t[0].buckling.unwrap().margin.is_none());
    }

    #[test]
    fn a_truss_gets_a_stress_margin_only_and_bad_inputs_are_refused() {
        let p = templates().remove(0);
        let solved = p.solve().unwrap();
        let m = margins(&p, &solved, |_| Some(input(30_000.0))).unwrap();
        assert_eq!(m.len(), 3);
        for (r, k) in solved.members.iter().zip(&m) {
            assert!(k.buckling.is_none());
            close(k.stress.sigma_max, r.axial_stress.abs(), 1e-9);
        }
        assert!(margins(&p, &solved, |_| Some(input(0.0))).unwrap_err().contains("allowable stress"));
        assert!(margins(&p, &solved, |_| Some(MemberAllowable { fibre_z: -1.0, ..input(1.0) })).unwrap_err().contains("fibre distance z"));
        assert!(margins(&p, &solved, |_| None).unwrap().is_empty());
    }
}
