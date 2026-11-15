//! Code-rule thickness of shells and formed heads under internal pressure (ASME BPVC Section VIII Division 1,
//! UG-27 for shells, UG-32 / Appendix 1-4 for heads) - the "heads and UG-27 code check" of #234.
//!
//! This is a *code-rule* check, separate from the exact Lame/failure-mode analysis of [`crate::failure`]: it takes
//! the allowable stress `S` and the joint efficiency `E` as inputs (never defaults them: the allowable comes from
//! the owner's material source) and answers "required thickness at design pressure P" and "maximum allowable working
//! pressure at thickness t". Dimensions are in the corroded condition: pass the inside radius / diameter and the
//! thickness left after corrosion and mill undertolerance (see [`crate::geometry::WallAllowance`]).
//!
//! The formulas are the published thin-wall rules of UG-27(c)(1)-(3), UG-32(d)-(f) and 1-4(d); each carries its limit
//! of applicability, and a case outside it returns [`HeadError::OutsideRules`] with the reason instead of a number
//! (UG-27 then points to Appendix 1-2). Check the coefficients against the owner's code edition before relying on a
//! result for a stamped vessel.

/// Why a code check could not produce a number.
#[derive(Debug, Clone, PartialEq)]
pub enum HeadError {
    /// A dimension, stress, efficiency or pressure was not a positive finite number (efficiency in (0, 1]).
    InvalidInput(&'static str),
    /// The rule's limit of applicability is exceeded (thick wall, high pressure, bad crown/knuckle ratio).
    OutsideRules(String),
}

/// A formed or cylindrical component under internal pressure.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Component {
    /// UG-27(c)(1): circumferential stress governs. `inside_radius` R.
    CylinderCircumferential { inside_radius: f64 },
    /// UG-27(c)(2): longitudinal stress governs (a circumferential joint).
    CylinderLongitudinal { inside_radius: f64 },
    /// UG-27(c)(3): spherical shell or hemispherical head. `inside_radius` is the sphere radius.
    Sphere { inside_radius: f64 },
    /// UG-32(d) / 1-4(c): ellipsoidal head, inside diameter D and inside depth h (2:1 is h = D/4).
    Ellipsoidal { inside_diameter: f64, inside_depth: f64 },
    /// UG-32(e) / 1-4(d): torispherical head, inside crown radius L and inside knuckle radius r.
    Torispherical { crown_radius: f64, knuckle_radius: f64 },
}

/// Allowable stress `S` (same units as pressure) and joint efficiency `E` of the part.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Joint {
    pub allowable_stress: f64,
    pub efficiency: f64,
}

/// UG-32(e): torispherical heads are within the rules only up to L/r = 16.67, which is the same limit as a knuckle radius
/// of at least 6 % of the crown radius.
const MAX_CROWN_TO_KNUCKLE: f64 = 16.67;

fn positive(v: f64, what: &'static str) -> Result<f64, HeadError> {
    if v.is_finite() && v > 0.0 { Ok(v) } else { Err(HeadError::InvalidInput(what)) }
}

fn check_joint(j: Joint) -> Result<f64, HeadError> {
    positive(j.allowable_stress, "the allowable stress must be positive")?;
    if !(j.efficiency.is_finite() && j.efficiency > 0.0 && j.efficiency <= 1.0) {
        return Err(HeadError::InvalidInput("the joint efficiency must be in (0, 1]"));
    }
    Ok(j.allowable_stress * j.efficiency)
}

/// Ellipsoidal-head factor `K = (2 + (D / 2h)^2) / 6` (UG-32(d); 1.0 for 2:1).
pub fn ellipsoidal_k(inside_diameter: f64, inside_depth: f64) -> f64 {
    (2.0 + (inside_diameter / (2.0 * inside_depth)).powi(2)) / 6.0
}

/// Torispherical-head factor `M = (3 + sqrt(L / r)) / 4` (1-4(d); 1.77 at L/r = 16.67).
pub fn torispherical_m(crown_radius: f64, knuckle_radius: f64) -> f64 {
    0.25 * (3.0 + (crown_radius / knuckle_radius).sqrt())
}

/// The rule's limit on a thickness `t` at pressure `p`, given `SE`. `Err` names the violated limit.
fn check_limits(c: Component, p: f64, t: f64, se: f64) -> Result<(), HeadError> {
    let out = |s: String| Err(HeadError::OutsideRules(s));
    match c {
        Component::CylinderCircumferential { inside_radius: r } => {
            if t > 0.5 * r { return out(format!("t = {t:.4} exceeds R/2 = {:.4} (UG-27(c)(1) is a thin-wall rule; use Appendix 1-2)", 0.5 * r)); }
            if p > 0.385 * se { return out(format!("P = {p:.4} exceeds 0.385 SE = {:.4} (UG-27(c)(1))", 0.385 * se)); }
        }
        Component::CylinderLongitudinal { inside_radius: r } => {
            if t > 0.5 * r { return out(format!("t = {t:.4} exceeds R/2 = {:.4} (UG-27(c)(2))", 0.5 * r)); }
            if p > 1.25 * se { return out(format!("P = {p:.4} exceeds 1.25 SE = {:.4} (UG-27(c)(2))", 1.25 * se)); }
        }
        Component::Sphere { inside_radius: r } => {
            if t > 0.356 * r { return out(format!("t = {t:.4} exceeds 0.356 R = {:.4} (UG-27(c)(3))", 0.356 * r)); }
            if p > 0.665 * se { return out(format!("P = {p:.4} exceeds 0.665 SE = {:.4} (UG-27(c)(3))", 0.665 * se)); }
        }
        Component::Ellipsoidal { inside_diameter: d, inside_depth: h } => {
            if h > 0.5 * d { return out("an ellipsoidal head deeper than D/2 is not a UG-32(d) head".into()); }
        }
        Component::Torispherical { crown_radius: l, knuckle_radius: r } => {
            if l / r > MAX_CROWN_TO_KNUCKLE { return out(format!("L/r = {:.2} exceeds {MAX_CROWN_TO_KNUCKLE}: the knuckle radius is under 6 % of the crown radius (UG-32(e))", l / r)); }
        }
    }
    Ok(())
}

fn validate(c: Component) -> Result<(), HeadError> {
    match c {
        Component::CylinderCircumferential { inside_radius } | Component::CylinderLongitudinal { inside_radius } | Component::Sphere { inside_radius } => {
            positive(inside_radius, "the inside radius must be positive")?;
        }
        Component::Ellipsoidal { inside_diameter, inside_depth } => {
            positive(inside_diameter, "the inside diameter must be positive")?;
            positive(inside_depth, "the head depth must be positive")?;
        }
        Component::Torispherical { crown_radius, knuckle_radius } => {
            positive(crown_radius, "the crown radius must be positive")?;
            positive(knuckle_radius, "the knuckle radius must be positive")?;
        }
    }
    Ok(())
}

/// Minimum required thickness (corroded condition) at design pressure `pressure`.
pub fn required_thickness(c: Component, pressure: f64, j: Joint) -> Result<f64, HeadError> {
    let se = check_joint(j)?;
    validate(c)?;
    positive(pressure, "the design pressure must be positive")?;
    let t = match c {
        Component::CylinderCircumferential { inside_radius: r } => pressure * r / (se - 0.6 * pressure),
        Component::CylinderLongitudinal { inside_radius: r } => pressure * r / (2.0 * se + 0.4 * pressure),
        Component::Sphere { inside_radius: r } => pressure * r / (2.0 * se - 0.2 * pressure),
        Component::Ellipsoidal { inside_diameter: d, inside_depth: h } => pressure * d * ellipsoidal_k(d, h) / (2.0 * se - 0.2 * pressure),
        Component::Torispherical { crown_radius: l, knuckle_radius: r } => pressure * l * torispherical_m(l, r) / (2.0 * se - 0.2 * pressure),
    };
    if !(t.is_finite() && t > 0.0) {
        return Err(HeadError::OutsideRules(format!("P = {pressure:.4} is not below the rule's pressure limit for SE = {se:.4}")));
    }
    check_limits(c, pressure, t, se)?;
    Ok(t)
}

/// Maximum allowable working pressure of thickness `t` (corroded condition).
pub fn maximum_allowable_pressure(c: Component, thickness: f64, j: Joint) -> Result<f64, HeadError> {
    let se = check_joint(j)?;
    validate(c)?;
    positive(thickness, "the thickness must be positive")?;
    let t = thickness;
    let p = match c {
        Component::CylinderCircumferential { inside_radius: r } => se * t / (r + 0.6 * t),
        Component::CylinderLongitudinal { inside_radius: r } => 2.0 * se * t / (r - 0.4 * t),
        Component::Sphere { inside_radius: r } => 2.0 * se * t / (r + 0.2 * t),
        Component::Ellipsoidal { inside_diameter: d, inside_depth: h } => 2.0 * se * t / (d * ellipsoidal_k(d, h) + 0.2 * t),
        Component::Torispherical { crown_radius: l, knuckle_radius: r } => 2.0 * se * t / (l * torispherical_m(l, r) + 0.2 * t),
    };
    if !(p.is_finite() && p > 0.0) {
        return Err(HeadError::OutsideRules("the thickness exceeds the rule's geometry".into()));
    }
    check_limits(c, p, t, se)?;
    Ok(p)
}

/// The margin of a provided thickness against the required one: `t_provided / t_required - 1`.
pub fn margin(provided: f64, required: f64) -> f64 {
    provided / required - 1.0
}

#[cfg(test)]
mod tests {
    use super::*;

    const J: Joint = Joint { allowable_stress: 20_000.0, efficiency: 1.0 };

    fn close(a: f64, b: f64, tol: f64) {
        assert!((a - b).abs() <= tol * b.abs().max(1e-12), "{a} vs {b}");
    }

    #[test]
    fn cylinder_circumferential_matches_hand_value() {
        // P 300 psi, R 24 in, SE 20000: t = 300*24/(20000 - 180) = 0.363250...
        let t = required_thickness(Component::CylinderCircumferential { inside_radius: 24.0 }, 300.0, J).unwrap();
        close(t, 7200.0 / 19_820.0, 1e-12);
    }

    #[test]
    fn longitudinal_is_about_half_the_circumferential_thickness() {
        let c = required_thickness(Component::CylinderCircumferential { inside_radius: 24.0 }, 300.0, J).unwrap();
        let l = required_thickness(Component::CylinderLongitudinal { inside_radius: 24.0 }, 300.0, J).unwrap();
        assert!((l / c - 0.5).abs() < 0.01, "{}", l / c);
        close(l, 7200.0 / 40_120.0, 1e-12);
    }

    #[test]
    fn sphere_and_hemisphere_are_the_same_rule() {
        let t = required_thickness(Component::Sphere { inside_radius: 24.0 }, 300.0, J).unwrap();
        close(t, 7200.0 / (40_000.0 - 60.0), 1e-12);
    }

    #[test]
    fn a_two_to_one_ellipsoidal_head_has_k_one_and_the_sphere_form_on_the_diameter() {
        close(ellipsoidal_k(48.0, 12.0), 1.0, 1e-12);
        let t = required_thickness(Component::Ellipsoidal { inside_diameter: 48.0, inside_depth: 12.0 }, 300.0, J).unwrap();
        close(t, 300.0 * 48.0 / (40_000.0 - 60.0), 1e-12);
        // A hemispherical head (h = D/2) has K = (2 + 1)/6 = 0.5: the sphere rule on the radius D/2.
        close(ellipsoidal_k(48.0, 24.0), 0.5, 1e-12);
        let hemi = required_thickness(Component::Ellipsoidal { inside_diameter: 48.0, inside_depth: 24.0 }, 300.0, J).unwrap();
        let sphere = required_thickness(Component::Sphere { inside_radius: 24.0 }, 300.0, J).unwrap();
        close(hemi, sphere, 1e-12);
    }

    #[test]
    fn torispherical_at_the_limit_ratio_reproduces_the_0_885_form() {
        // L/r = 16.67 gives M = 1.7706..., and PLM/(2SE-0.2P) = 0.885 PL/(SE - 0.1P) to the code's rounding.
        let (l, r) = (48.0, 48.0 / 16.6);
        let t = required_thickness(Component::Torispherical { crown_radius: l, knuckle_radius: r }, 300.0, J).unwrap();
        let rule = 0.885 * 300.0 * l / (20_000.0 - 30.0);
        assert!((t / rule - 1.0).abs() < 2e-3, "{t} vs {rule}");
        // A larger knuckle radius (a 6 % / 100 % crown head) needs less wall.
        let better = required_thickness(Component::Torispherical { crown_radius: l, knuckle_radius: 0.1 * l }, 300.0, J).unwrap();
        assert!(better < t);
    }

    #[test]
    fn the_pressure_and_thickness_rules_are_inverses() {
        for c in [
            Component::CylinderCircumferential { inside_radius: 24.0 },
            Component::CylinderLongitudinal { inside_radius: 24.0 },
            Component::Sphere { inside_radius: 24.0 },
            Component::Ellipsoidal { inside_diameter: 48.0, inside_depth: 12.0 },
            Component::Torispherical { crown_radius: 48.0, knuckle_radius: 4.8 },
        ] {
            let t = required_thickness(c, 250.0, J).unwrap();
            close(maximum_allowable_pressure(c, t, J).unwrap(), 250.0, 1e-9);
        }
    }

    #[test]
    fn thickness_grows_with_pressure_and_falls_with_efficiency() {
        let c = Component::CylinderCircumferential { inside_radius: 24.0 };
        assert!(required_thickness(c, 400.0, J).unwrap() > required_thickness(c, 300.0, J).unwrap());
        let weld = Joint { efficiency: 0.85, ..J };
        close(required_thickness(c, 300.0, weld).unwrap(), 7200.0 / (17_000.0 - 180.0), 1e-12);
    }

    #[test]
    fn limits_of_applicability_are_reported_not_exceeded() {
        // High pressure: 0.385 SE = 7700 psi (the same boundary as t = R/2 for this rule, so either message is right).
        let e = required_thickness(Component::CylinderCircumferential { inside_radius: 24.0 }, 8_000.0, J).unwrap_err();
        assert!(matches!(&e, HeadError::OutsideRules(s) if s.contains("UG-27(c)(1)")), "{e:?}");
        let e = required_thickness(Component::Sphere { inside_radius: 24.0 }, 14_000.0, J).unwrap_err();
        assert!(matches!(&e, HeadError::OutsideRules(s) if s.contains("UG-27(c)(3)")), "{e:?}");
        // Thick wall: R/2 limit on a thickness given for MAWP.
        let e = maximum_allowable_pressure(Component::CylinderCircumferential { inside_radius: 24.0 }, 13.0, J).unwrap_err();
        assert!(matches!(&e, HeadError::OutsideRules(s) if s.contains("R/2")), "{e:?}");
        // Torispherical ratio limits.
        let e = required_thickness(Component::Torispherical { crown_radius: 100.0, knuckle_radius: 5.9 }, 100.0, J).unwrap_err();
        assert!(matches!(&e, HeadError::OutsideRules(s) if s.contains("16.67")), "{e:?}");
        assert!(required_thickness(Component::Torispherical { crown_radius: 100.0, knuckle_radius: 6.1 }, 100.0, J).is_ok());
    }

    #[test]
    fn bad_inputs_are_named() {
        let c = Component::Sphere { inside_radius: 24.0 };
        assert!(matches!(required_thickness(c, -1.0, J), Err(HeadError::InvalidInput(_))));
        assert!(matches!(required_thickness(c, f64::NAN, J), Err(HeadError::InvalidInput(_))));
        assert!(matches!(required_thickness(c, 100.0, Joint { efficiency: 1.2, ..J }), Err(HeadError::InvalidInput(_))));
        assert!(matches!(required_thickness(c, 100.0, Joint { allowable_stress: 0.0, ..J }), Err(HeadError::InvalidInput(_))));
        assert!(matches!(required_thickness(Component::Sphere { inside_radius: 0.0 }, 100.0, J), Err(HeadError::InvalidInput(_))));
    }

    #[test]
    fn the_code_thickness_agrees_with_the_exact_lame_wall_to_a_few_percent_for_a_thin_cylinder() {
        // Against the exact solution already in this crate: at the code thickness the Lame hoop stress at the bore stays
        // near S (the code rule is the thin-wall version of it).
        let (r, p) = (24.0, 300.0);
        let t = required_thickness(Component::CylinderCircumferential { inside_radius: r }, p, J).unwrap();
        let (a, b) = (r, r + t);
        let hoop = p * (b * b + a * a) / (b * b - a * a);
        assert!((hoop / J.allowable_stress - 1.0).abs() < 0.02, "{hoop}");
    }
}
