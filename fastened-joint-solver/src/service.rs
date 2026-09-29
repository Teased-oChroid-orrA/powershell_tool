//! External service-load sharing, joint separation, and post-separation
//! behavior (spec sections 22, 39-41), plus transverse-load slip capacity
//! (spec section 42).

/// `C = k_b / (k_b + k_m)` (spec section 22) - the fastener's share of any
/// additional external axial load, while the joint remains in contact.
/// Compliances (not stiffnesses) are the primitive quantities elsewhere in
/// this crate, so this takes `k_b`/`k_m` directly rather than requiring the
/// caller to invert compliances twice.
pub fn joint_load_fraction(k_b: f64, k_m: f64) -> f64 {
    if k_b + k_m <= 0.0 {
        return 0.0;
    }
    k_b / (k_b + k_m)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ServiceLoadState {
    pub external_load: f64,
    pub bolt_load: f64,
    pub member_load: f64,
    pub separation_load: f64,
    /// `separation_load / external_load - 1` - `f64::INFINITY` when there
    /// is no external load to compare against.
    pub separation_margin: f64,
    pub separated: bool,
}

/// Applies an external axial service load `p` to a joint at initial
/// preload `f_i` with load fraction `c` (spec sections 39-41). Before
/// separation: `F_b = F_i + C*P`, `F_m = F_i - (1-C)*P`. At and beyond the
/// separation load `P_sep = F_i/(1-C)`, contact has fully opened - the
/// piecewise model spec section 41 requires, not a continued linear
/// extrapolation of the pre-separation stiffness fraction: member load
/// clamps at zero and the fastener alone carries the full external load
/// from that point on (`F_b = P`, not `F_i + C*P`, since `F_i + C*P_sep`
/// only equals `P` in the boundary case `P == P_sep` - beyond it the
/// contact interface can no longer contribute the `(1-C)` share the
/// pre-separation formula assumed).
pub fn apply_external_load(preload: f64, c: f64, external_load: f64) -> ServiceLoadState {
    let separation_load = if c < 1.0 { preload / (1.0 - c) } else { f64::INFINITY };
    let separated = external_load >= separation_load;
    let (bolt_load, member_load) = if separated { (external_load, 0.0) } else { (preload + c * external_load, preload - (1.0 - c) * external_load) };
    let separation_margin = if external_load > 0.0 { separation_load / external_load - 1.0 } else { f64::INFINITY };
    ServiceLoadState { external_load, bolt_load, member_load, separation_load, separation_margin, separated }
}

/// Friction-grip slip resistance across one or more clamped interfaces
/// (spec section 42): `F_slip = sum(mu_interface * N_interface)`, using
/// each interface's own actual clamping force - never one coefficient
/// multiplied blindly across the whole stack.
pub fn slip_capacity(interfaces: &[(f64, f64)]) -> f64 {
    interfaces.iter().map(|(mu, n)| mu * n).sum()
}

/// `F_slip / F_applied_shear - 1` - `f64::INFINITY` when there is no
/// applied shear load to compare against.
pub fn slip_margin(capacity: f64, applied_shear: f64) -> f64 {
    if applied_shear <= 0.0 {
        return f64::INFINITY;
    }
    capacity / applied_shear - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn joint_load_fraction_is_the_bolt_stiffness_share() {
        let c = joint_load_fraction(1000.0, 4000.0);
        assert!((c - 0.2).abs() < 1e-9);
    }

    #[test]
    fn pre_separation_load_sharing_matches_the_closed_form() {
        let state = apply_external_load(5000.0, 0.2, 1000.0);
        assert!(!state.separated);
        assert!((state.bolt_load - (5000.0 + 0.2 * 1000.0)).abs() < 1e-9);
        assert!((state.member_load - (5000.0 - 0.8 * 1000.0)).abs() < 1e-9);
    }

    #[test]
    fn member_load_reaches_zero_exactly_at_the_separation_load() {
        let c = 0.2;
        let preload = 5000.0;
        let separation_load = preload / (1.0 - c);
        let state = apply_external_load(preload, c, separation_load);
        assert!(state.member_load.abs() < 1e-6, "member load must be (numerically) zero right at separation");
    }

    #[test]
    fn beyond_separation_the_fastener_alone_carries_the_full_external_load() {
        let c = 0.2;
        let preload = 5000.0;
        let separation_load = preload / (1.0 - c);
        let beyond = separation_load * 1.5;
        let state = apply_external_load(preload, c, beyond);
        assert!(state.separated);
        assert_eq!(state.member_load, 0.0);
        assert!((state.bolt_load - beyond).abs() < 1e-9, "post-separation bolt load must equal the applied external load, not the pre-separation linear extrapolation");
    }

    #[test]
    fn separation_margin_is_positive_below_the_separation_load_and_negative_beyond() {
        let c = 0.2;
        let preload = 5000.0;
        let below = apply_external_load(preload, c, 1000.0);
        assert!(below.separation_margin > 0.0);
        let separation_load = preload / (1.0 - c);
        let beyond = apply_external_load(preload, c, separation_load * 1.5);
        assert!(beyond.separation_margin < 0.0);
    }

    #[test]
    fn slip_capacity_sums_every_interface_independently() {
        let capacity = slip_capacity(&[(0.3, 5000.0), (0.2, 5000.0)]);
        assert!((capacity - (0.3 * 5000.0 + 0.2 * 5000.0)).abs() < 1e-9);
    }

    #[test]
    fn slip_margin_with_no_applied_shear_is_infinite() {
        assert_eq!(slip_margin(1000.0, 0.0), f64::INFINITY);
    }
}
