//! Analytical lug cross-check: the Air Force method (Melcon and Hoblit, 1953; AFFDL-TR-69-42 Section 9, reproduced as
//! Bruhn Chapter D1) for the ultimate load of a pin-loaded lug in axial, transverse and oblique loading.
//!
//! This is the *handbook* answer the finite-element collapse is compared with, not another solver: every coefficient is
//! read off a test-derived chart (`affdl_data`, digitized point for point from the scanned figures, linearly
//! interpolated, clamped to the nearest edge value outside a chart). It uses no stress analysis, so it knows nothing about
//! a bushing's fit, friction or the pin's stiffness. Its limits (the charts are drawn for a lug in single or double shear
//! whose mating parts are as strong as the lug; no out-of-plane buckling; constant-width head) are the method's own.
//!
//! Equations (AFFDL numbers): `Fu = min(Ftu, 1.304 Fty)`;
//! bearing `Pbru = Kbr Fu D t` (9-3; Fig. 9-2 for `D/t <= 5`, Fig. 9-3 above);
//! net tension `Pnu = Kn Fu (W - D) t` (9-4..9-6, Fig. 9-4);
//! axial allowable `min(Pbru, Pnu)`; transverse `Ptru = Ktru Ftu D t` at `h_av/D` (9-28..9-30, Figs. 9-8, 9-10);
//! oblique `(Pax/Pu)^1.6 + (Ptr/Ptru)^1.6 = 1` (9-31). The ductility factor `B / B(0.05)` (Fig. 9-17) scales both
//! allowables when the elongation is below 5 %.
//!
//! Source of the digitized arrays: <https://kaspercalc.com/LugPinAnalysis.html> (`js/lug-math.js`); regenerate with
//! `tools/gen_affdl_lug.py`. Accuracy is "a percent or two" of each read-off; the two-digit agreement with the
//! worked example that site quotes is pinned in the tests.
use crate::affdl_data::{FIG9_10, FIG9_17, FIG9_2A, FIG9_2B, FIG9_3, FIG9_3_CUT_A, FIG9_3_CUT_B, FIG9_4B, FIG9_4C, FIG9_4D, FIG9_8U, FIG9_8Y};

/// A family of curves by a parameter (`params` ascending, one curve per parameter), each `[x, y]` ascending in x.
pub struct Family {
    pub params: &'static [f64],
    pub curves: &'static [&'static [[f64; 2]]],
}

/// Piecewise-linear `y(x)`, held flat past either end.
fn lin(pts: &[[f64; 2]], x: f64) -> f64 {
    let (first, last) = (pts[0], pts[pts.len() - 1]);
    if x <= first[0] {
        return first[1];
    }
    if x >= last[0] {
        return last[1];
    }
    let i = pts.partition_point(|p| p[0] < x).max(1);
    let (a, b) = (pts[i - 1], pts[i]);
    a[1] + (b[1] - a[1]) * (x - a[0]) / (b[0] - a[0])
}

/// Bracketing indices of `v` in an ascending list, the blend fraction, and whether `v` lies outside the list.
fn bracket(list: &[f64], v: f64) -> (usize, usize, f64, bool) {
    let n = list.len();
    if v <= list[0] {
        return (0, 0, 0.0, v < list[0]);
    }
    if v >= list[n - 1] {
        return (n - 1, n - 1, 0.0, v > list[n - 1]);
    }
    let j = list.partition_point(|p| *p < v);
    (j - 1, j, (v - list[j - 1]) / (list[j] - list[j - 1]), false)
}

impl Family {
    /// Read at `x` on the curve for `param`: each bracketing curve at `x`, then blended. The flag is set when `param` is
    /// outside the family.
    fn at(&self, param: f64, x: f64) -> (f64, bool) {
        let (i, j, f, clamped) = bracket(self.params, param);
        let (ya, yb) = (lin(self.curves[i], x), lin(self.curves[j], x));
        (ya + (yb - ya) * f, clamped)
    }
}

/// Aluminium grain-direction cutoff of Fig. 9-3 (it caps `Kbr`; other metals are unaffected).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Default)]
pub enum Grain {
    /// No cutoff.
    #[default]
    None,
    /// Curve A: hand-forged billet with the long transverse grain across the lug.
    HandForgedLongTransverse,
    /// Curve B: plate, bar, short-transverse billet, die forgings.
    PlateBarOrDieForging,
}

/// What the method needs of the lug material (psi; `elongation` a fraction, 0 = not tabulated).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LugMaterial {
    pub ftu: f64,
    pub fty: f64,
    pub e: f64,
    pub elongation: f64,
    pub aluminium: bool,
    /// Tabulated bearing ultimate at e/D = 2.0 and its yield counterpart (0 = none): the allowable never exceeds
    /// `min(Fbru, 1.304 Fbry) D t`.
    pub fbru20: f64,
    pub fbry20: f64,
}

/// The lug's in-plane dimensions (inches); `edge` is hole centre to the end of the head, `hole` the bearing diameter
/// (the bushing's bore when there is a bushing is the *pin*; the lug's own hole is the contact surface of the lug).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LugDims {
    pub hole: f64,
    pub thickness: f64,
    pub width: f64,
    pub edge: f64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AxialMode {
    Bearing,
    NetSection,
}

#[derive(Clone, Debug, PartialEq)]
pub struct AffdlResult {
    /// `min(Ftu, 1.304 Fty)`.
    pub fu: f64,
    pub kbr: f64,
    /// Which bearing chart was read: "9-2" or "9-3".
    pub bearing_figure: &'static str,
    pub pbru: f64,
    pub kn: f64,
    pub pnu: f64,
    /// Axial allowable, `min(Pbru, Pnu)`.
    pub p_axial: f64,
    pub mode: AxialMode,
    /// Effective edge distance over D (Fig. 9-10) and the transverse coefficient read at it.
    pub hav_over_d: f64,
    pub ktru: f64,
    pub p_transverse: f64,
    /// Allowable along the load angle given to [`lug_allowable`] (equation 9-31).
    pub p_oblique: f64,
    /// Ductility factor applied to the bearing and transverse allowables (1 = none).
    pub ductility: f64,
    /// Reasons the answer is less certain (a chart edge was used, a limit of the method was met).
    pub notes: Vec<String>,
}

/// Bearing efficiency `Kbr` (Fig. 9-2 at `D/t <= 5`, Fig. 9-3 above) and the chart used.
fn kbr(e_over_d: f64, d_over_t: f64, grain: Grain, notes: &mut Vec<String>) -> (f64, &'static str) {
    let (mut k, fig) = if d_over_t <= 5.0 {
        // Fig. 9-2 is written on the nose distance a = e - D/2 below e/D = 1.5: K * a/D puts it on D. It ends at
        // e/D = 2.4 and is held flat past that, which is conservative.
        if e_over_d > 2.4 {
            notes.push("e/D is past the end of Fig. 9-2 (2.4): the value at 2.4 is used".into());
        }
        (if e_over_d < 1.5 { lin(FIG9_2A, e_over_d) * (e_over_d - 0.5).max(0.0) } else { lin(FIG9_2B, e_over_d) }, "9-2")
    } else {
        let (k, clamped) = FIG9_3.at(d_over_t.max(2.0), e_over_d);
        if clamped {
            notes.push(format!("D/t {d_over_t:.1} is outside Fig. 9-3 (2 to 30): the nearest curve is used"));
        }
        (k, "9-3")
    };
    // At e/D <= 0.5 the hole breaks out of the edge; the digitized traces start a hair under 0.5 with noise.
    k = if e_over_d <= 0.5 { 0.0 } else { k.max(0.0) };
    match grain {
        Grain::None => {}
        Grain::HandForgedLongTransverse => k = k.min(lin(FIG9_3_CUT_A, e_over_d)),
        Grain::PlateBarOrDieForging => k = k.min(lin(FIG9_3_CUT_B, e_over_d)),
    }
    (k, fig)
}

/// Net-tension coefficient `Kn` (Fig. 9-4): three charts by `Fty/Ftu` (1.0, 0.8, 0.6; 0.6 below), each a family by
/// `Ftu / (E eps_u)`, blended in both.
fn kn(d_over_w: f64, fty_over_ftu: f64, duct: f64, notes: &mut Vec<String>) -> f64 {
    let ratios = [0.6, 0.8, 1.0];
    let charts = [&FIG9_4D, &FIG9_4C, &FIG9_4B];
    let (i, j, f, clamped) = bracket(&ratios, fty_over_ftu);
    if clamped {
        notes.push(format!("Fty/Ftu {fty_over_ftu:.2} is outside Fig. 9-4 (0.6 to 1.0): the nearest chart is used"));
    }
    let p = duct.clamp(0.0, 1.0);
    let (ya, yb) = (charts[i].at(p, d_over_w).0, charts[j].at(p, d_over_w).0);
    (ya + (yb - ya) * f).clamp(0.0, 1.0)
}

/// Ultimate allowable of a lug at `theta_deg` from its axis (0 = axial). `None` when the inputs have no meaning (a
/// non-positive dimension or strength, or a hole as wide as the lug).
pub fn lug_allowable(m: &LugMaterial, g: &LugDims, theta_deg: f64, grain: Grain) -> Option<AffdlResult> {
    let ok = [m.ftu, m.fty, m.e, g.hole, g.thickness, g.width, g.edge].iter().all(|v| v.is_finite() && *v > 0.0) && g.width > g.hole && theta_deg.is_finite();
    if !ok {
        return None;
    }
    let (d, t, w, e) = (g.hole, g.thickness, g.width, g.edge);
    let mut notes = Vec::new();
    let fu = m.ftu.min(1.304 * m.fty);
    let yield_capped = m.ftu > 1.304 * m.fty;
    let (e_over_d, d_over_t) = (e / d, d / t);

    // Ductility factor (Fig. 9-17): only below 5 % elongation, from the yield strain Fty/E + 0.002.
    let ductility = if m.elongation > 0.0 && m.elongation < 0.05 {
        let (eu, ey, ratio) = (m.elongation.max(0.001), m.fty / m.e + 0.002, m.fty / m.ftu);
        let (b, clamped) = FIG9_17.at(ratio, ey / eu);
        if clamped {
            notes.push(format!("Fty/Ftu {ratio:.2} is outside Fig. 9-17 (0.6 to 1.0): the nearest curve is used"));
        }
        (b / FIG9_17.at(ratio, ey / 0.05).0).min(1.0)
    } else {
        1.0
    };

    // Axial: bearing, shear-out and hoop tension together (9-3), and net tension (9-4).
    let (mut k, bearing_figure) = kbr(e_over_d, d_over_t, grain, &mut notes);
    if m.aluminium && t >= 0.5 && k > 2.0 {
        k = 2.0;
        notes.push("aluminium lug 0.5 in or thicker: Kbr capped at 2.00 (the method's rule)".into());
    }
    let mut pbru = k * fu * d * t * ductility;
    if m.fbru20 > 0.0 {
        let cap = if m.fbry20 > 0.0 { m.fbru20.min(1.304 * m.fbry20) } else { m.fbru20 };
        pbru = pbru.min(cap * d * t);
    }
    let eu = m.elongation;
    let knv = kn((d / w).min(1.0), m.fty / m.ftu, if eu > 0.0 { m.ftu / (m.e * eu) } else { 1.0 }, &mut notes);
    if eu <= 0.0 {
        notes.push("no tabulated elongation: the net-section chart is read at its ductile limit".into());
    }
    let pnu = knv * fu * (w - d) * t;
    let (p_axial, mode) = if pbru <= pnu { (pbru, AxialMode::Bearing) } else { (pnu, AxialMode::NetSection) };

    // Transverse (9-28..9-30): Fig. 9-10 is for a concentric head, whose radius is both e and W/2.
    let hav_over_d = lin(FIG9_10, e.min(w / 2.0) / d).max(0.0);
    if hav_over_d > 1.5 {
        notes.push("h_av/D is past Fig. 9-8 (1.5): the value at its end is used".into());
    }
    let p_transverse = if yield_capped { 1.304 * lin(FIG9_8Y, hav_over_d).max(0.0) * m.fty } else { lin(FIG9_8U, hav_over_d).max(0.0) * m.ftu } * d * t * ductility;
    let ktru = if yield_capped { 1.304 * lin(FIG9_8Y, hav_over_d).max(0.0) * m.fty / m.ftu } else { lin(FIG9_8U, hav_over_d).max(0.0) };

    Some(AffdlResult { fu, kbr: k, bearing_figure, pbru, kn: knv, pnu, p_axial, mode, hav_over_d, ktru, p_transverse, p_oblique: interaction(p_axial, p_transverse, theta_deg), ductility, notes })
}

/// Equation 9-31, `(P cos/Pax)^1.6 + (P sin/Ptr)^1.6 = 1`, solved for the load `P` at `theta_deg`.
pub fn interaction(p_axial: f64, p_transverse: f64, theta_deg: f64) -> f64 {
    const N: f64 = 1.6;
    let th = theta_deg.to_radians();
    let (c, s) = (th.cos().abs(), th.sin().abs());
    let d = (if c > 1e-12 { (c / p_axial).powf(N) } else { 0.0 }) + (if s > 1e-12 { (s / p_transverse).powf(N) } else { 0.0 });
    if d > 0.0 {
        d.powf(-1.0 / N)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn al() -> LugMaterial {
        // 7075-T6 plate, typical A-basis-ish numbers; elongation 7 %.
        LugMaterial { ftu: 77_000.0, fty: 66_000.0, e: 10.3e6, elongation: 0.07, aluminium: true, fbru20: 0.0, fbry20: 0.0 }
    }

    #[test]
    fn fig_9_2_is_continuous_at_e_over_d_1_5_and_starts_from_the_published_values() {
        // The handbook figure: K = 2.00 at the start of the nose-distance branch and a minimum of 1.33 at e/D 1.5.
        assert!((lin(FIG9_2A, 0.5016) - 2.0).abs() < 2e-3);
        let (a, b) = (lin(FIG9_2A, 1.5), lin(FIG9_2B, 1.5));
        assert!((a - 1.337).abs() < 0.01 && (a - b).abs() < 0.01, "{a} {b}");
    }

    #[test]
    fn the_net_section_chart_reproduces_the_worked_example() {
        // D/w 0.40, Fty/Ftu 0.625, Ftu/(E eps_u) 0.051 reads Kn = 0.74 (the digitized site's worked example).
        let k = kn(0.40, 0.625, 0.051, &mut Vec::new());
        assert!((k - 0.74).abs() < 0.015, "{k}");
    }

    #[test]
    fn a_hole_at_the_edge_carries_no_bearing_and_the_axial_load_rises_with_edge_distance() {
        let g = |e| LugDims { hole: 0.5, thickness: 0.25, width: 1.5, edge: e };
        assert_eq!(kbr(0.5, 2.0, Grain::None, &mut Vec::new()).0, 0.0);
        let p: Vec<f64> = [0.75, 1.0, 1.5].iter().map(|e| lug_allowable(&al(), &g(*e), 0.0, Grain::None).unwrap().p_axial).collect();
        assert!(p[0] < p[1] && p[1] < p[2], "{p:?}");
    }

    #[test]
    fn the_bearing_formula_is_kbr_fu_d_t_by_hand() {
        // D = 0.5, t = 0.25 (D/t = 2, Fig. 9-2), e/D = 2.0, W wide enough that bearing governs.
        let r = lug_allowable(&al(), &LugDims { hole: 0.5, thickness: 0.25, width: 3.0, edge: 1.0 }, 0.0, Grain::None).unwrap();
        assert_eq!(r.bearing_figure, "9-2");
        assert_eq!(r.mode, AxialMode::Bearing);
        let by_hand = lin(FIG9_2B, 2.0) * 77_000.0 * 0.5 * 0.25;
        assert!((r.pbru - by_hand).abs() < 1e-6 * by_hand && (r.p_axial - r.pbru).abs() < 1e-9, "{} {by_hand}", r.pbru);
        assert_eq!(r.fu, 77_000.0f64.min(1.304 * 66_000.0));
    }

    #[test]
    fn a_narrow_lug_fails_in_net_section_and_the_oblique_interaction_closes_at_both_ends() {
        let g = LugDims { hole: 0.5, thickness: 0.25, width: 0.7, edge: 1.0 };
        let r = lug_allowable(&al(), &g, 0.0, Grain::None).unwrap();
        assert_eq!(r.mode, AxialMode::NetSection);
        assert!((r.pnu - r.kn * r.fu * 0.2 * 0.25).abs() < 1e-6);
        assert!((interaction(r.p_axial, r.p_transverse, 0.0) - r.p_axial).abs() < 1e-6 * r.p_axial);
        assert!((interaction(r.p_axial, r.p_transverse, 90.0) - r.p_transverse).abs() < 1e-6 * r.p_transverse);
        // At 45 deg the load P satisfies (P cos/Pax)^1.6 + (P sin/Ptr)^1.6 = 1.
        let p = interaction(r.p_axial, r.p_transverse, 45.0);
        let c = 45f64.to_radians().cos();
        let lhs = (p * c / r.p_axial).powf(1.6) + (p * c / r.p_transverse).powf(1.6);
        assert!((lhs - 1.0).abs() < 1e-9, "{lhs}");
        assert!(p < r.p_axial.min(r.p_transverse) / c + 1e-9);
    }

    #[test]
    fn the_grain_cutoff_only_lowers_the_bearing_coefficient_and_a_low_elongation_lowers_the_allowable() {
        let g = LugDims { hole: 0.5, thickness: 0.05, width: 3.0, edge: 1.0 }; // D/t = 10: Fig. 9-3
        let none = lug_allowable(&al(), &g, 0.0, Grain::None).unwrap();
        let a = lug_allowable(&al(), &g, 0.0, Grain::HandForgedLongTransverse).unwrap();
        assert_eq!(none.bearing_figure, "9-3");
        assert!(a.kbr <= none.kbr + 1e-12);
        let brittle = LugMaterial { elongation: 0.02, ..al() };
        let b = lug_allowable(&brittle, &g, 0.0, Grain::None).unwrap();
        assert!(b.ductility < 1.0 && b.pbru < none.pbru, "{} {}", b.ductility, b.pbru);
        assert!(lug_allowable(&al(), &LugDims { hole: 1.0, thickness: 0.1, width: 1.0, edge: 1.0 }, 0.0, Grain::None).is_none());
    }
}
