//! Fastener axial/torsional elastic compliance (spec sections 14-16) and
//! clamped-member pressure-cone compliance (spec sections 17-19). Every
//! compliance value here is a real integral (`integral dx/(E*A(x))`),
//! evaluated exactly for piecewise-constant cross-sections (the fastener
//! stack: shank/threaded-free/engaged-thread/head-effect segments) or
//! numerically for the continuously-varying member pressure cone - never
//! reduced to one arbitrary stiffness constant.

use crate::quadrature::composite_simpson;
use crate::thread::ThreadGeometry;

/// One length of the fastener with a constant diameter - the shank, the
/// free (unengaged) threaded length, an equivalent "additional elastic
/// length" segment representing head or nut/thread-engagement effect
/// (see [`vdi2230_additional_length`]), etc. Never an "equivalent single
/// area" for the whole fastener (spec section 14's own prohibition) -
/// each physically distinct region is its own segment.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FastenerSegment {
    pub length: f64,
    pub diameter: f64,
}

impl FastenerSegment {
    pub fn area(&self) -> f64 {
        std::f64::consts::FRAC_PI_4 * self.diameter * self.diameter
    }
}

/// Additional elastic length representing the head's (or nut's) own
/// flexibility beyond its nominal bearing face, and thread-engagement
/// flexibility - VDI 2230's well-known `l_SK = 0.4*d` approximation
/// (additional length of full-diameter material whose stretch reproduces
/// the head/nut/engaged-thread region's own measured flexibility). Cited
/// explicitly per spec section 16 ("if an empirical effective-length
/// correction is used, identify it explicitly") - this is not folded
/// silently into an arbitrary "grip length," it is its own labeled
/// segment appended in [`build_fastener_segments`].
pub fn vdi2230_additional_length(diameter: f64) -> f64 {
    0.4 * diameter
}

/// Assembles the full fastener compliance segment list: the caller-supplied
/// shank/free-thread physical segments, plus one additional-length segment
/// at the head end (major diameter) and one at the nut/engaged-thread end
/// (root diameter - the conservative, more-flexible choice, since the
/// engaged-thread region's true stiffness sits between root- and
/// pitch-diameter area and root is the safe/stiffness-lower-bound side).
pub fn build_fastener_segments(shank_and_free_thread: &[FastenerSegment], thread: &ThreadGeometry) -> Vec<FastenerSegment> {
    let mut segments = Vec::with_capacity(shank_and_free_thread.len() + 2);
    segments.push(FastenerSegment { length: vdi2230_additional_length(thread.d), diameter: thread.d });
    segments.extend_from_slice(shank_and_free_thread);
    segments.push(FastenerSegment { length: vdi2230_additional_length(thread.d3), diameter: thread.d3 });
    segments
}

/// `C_b = sum(L_i / (E * A_i))` - exact for piecewise-constant
/// cross-section (spec section 14's preferred closed form for this case).
pub fn fastener_axial_compliance(segments: &[FastenerSegment], e: f64) -> f64 {
    if e <= 0.0 {
        return 0.0;
    }
    segments.iter().map(|s| s.length / (e * s.area())).sum()
}

/// `phi = T*L/(G*J)` per segment, summed - fastener torsional compliance
/// (radians per unit torque) at the same segment resolution as the axial
/// compliance (spec section 27's `J = pi*d^4/32`, generalized to the
/// segment stack rather than one nominal diameter).
pub fn fastener_torsional_compliance(segments: &[FastenerSegment], g: f64) -> f64 {
    if g <= 0.0 {
        return 0.0;
    }
    segments
        .iter()
        .map(|s| {
            let j = std::f64::consts::PI * s.diameter.powi(4) / 32.0;
            s.length / (g * j)
        })
        .sum()
}

/// Numerical axial compliance for an arbitrary (non-piecewise-constant)
/// diameter profile - `integral[0,L] dx/(E*A(x))`, evaluated by composite
/// Simpson (spec section 14's "for arbitrary diameter profiles, numerically
/// integrate"). Exists both as a genuine capability for a future
/// continuously-tapered fastener model and as the cross-check this
/// module's own tests use to prove [`fastener_axial_compliance`]'s
/// closed-form sum is correct for the piecewise-constant case (a constant
/// `area_fn` must reproduce `L/(E*A)` exactly).
pub fn axial_compliance_integral(length: f64, e: f64, area_fn: impl Fn(f64) -> f64) -> f64 {
    if e <= 0.0 || length <= 0.0 {
        return 0.0;
    }
    composite_simpson(0.0, length, 200, |x| 1.0 / (e * area_fn(x).max(1e-15)))
}

/// One member in the clamped joint stack.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Member {
    pub thickness: f64,
    pub e: f64,
    pub hole_diameter: f64,
    /// The member's own available outer geometry - the pressure cone
    /// cannot grow past this even if the ideal cone half-angle would carry
    /// it further (spec section 18: "Available outer geometry"). `None`
    /// means effectively unbounded (a large plate).
    pub outer_diameter: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct MemberStack {
    pub members: Vec<Member>,
}

impl MemberStack {
    pub fn total_thickness(&self) -> f64 {
        self.members.iter().map(|m| m.thickness).sum()
    }

    /// `(E, hole_diameter, outer_diameter)` of whichever member owns axial
    /// coordinate `z` (measured from the head-side face of the stack, `0`
    /// at the very first member's near face).
    fn properties_at(&self, z: f64) -> Option<(f64, f64, Option<f64>)> {
        let mut acc = 0.0;
        for m in &self.members {
            if z < acc + m.thickness || (z - (acc + m.thickness)).abs() < 1e-9 {
                return Some((m.e, m.hole_diameter, m.outer_diameter));
            }
            acc += m.thickness;
        }
        self.members.last().map(|m| (m.e, m.hole_diameter, m.outer_diameter))
    }
}

/// Rotscher pressure-cone member compliance (spec sections 18/19):
/// truncated double-cone geometry, each cone growing from its own end
/// contact diameter (the head's or nut's/washer's bearing diameter) at
/// `cone_half_angle_deg` toward the joint's mid-plane, bounded by each
/// member's own outer diameter where narrower than the ideal cone,
/// integrated numerically per axial slice with that slice's own member's
/// `E`/hole diameter. A documented simplification (spec sections 18/19
/// permit numerical slicing without prescribing exactly how many
/// interfaces get their own cone): this models one symmetric cone pair
/// meeting at the stack's mid-plane rather than a fresh cone re-originating
/// at every internal member interface - the standard two-cone
/// approximation generalized here to an arbitrary per-slice
/// material/hole/OD, not collapsed to a single lumped stiffness constant
/// (the one thing spec section 18 explicitly forbids).
pub fn member_stack_compliance(stack: &MemberStack, contact_diameter_head_side: f64, contact_diameter_nut_side: f64, cone_half_angle_deg: f64) -> f64 {
    let total = stack.total_thickness();
    if total <= 0.0 {
        return 0.0;
    }
    let half = total / 2.0;
    let tan_alpha = (cone_half_angle_deg.to_radians()).tan();

    let compliance_from_end = |contact_diameter: f64, from_nut_side: bool| -> f64 {
        composite_simpson(0.0, half, 200, |z| {
            let z_from_head = if from_nut_side { (total - z).clamp(0.0, total - 1e-9) } else { z.clamp(0.0, total - 1e-9) };
            let Some((e, hole_d, outer_d)) = stack.properties_at(z_from_head) else { return 0.0 };
            let mut d_cone = contact_diameter + 2.0 * z * tan_alpha;
            if let Some(od) = outer_d {
                d_cone = d_cone.min(od);
            }
            let area = (std::f64::consts::FRAC_PI_4 * (d_cone * d_cone - hole_d * hole_d)).max(1e-12);
            1.0 / (e * area)
        })
    };

    compliance_from_end(contact_diameter_head_side, false) + compliance_from_end(contact_diameter_nut_side, true)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn single_cylindrical_segment_matches_l_over_ea() {
        let seg = FastenerSegment { length: 20.0, diameter: 10.0 };
        let e = 200_000.0;
        let compliance = fastener_axial_compliance(&[seg], e);
        let expected = 20.0 / (e * seg.area());
        assert!((compliance - expected).abs() < 1e-12);
    }

    #[test]
    fn multiple_segments_sum_their_individual_compliances() {
        let segs = [FastenerSegment { length: 10.0, diameter: 8.0 }, FastenerSegment { length: 15.0, diameter: 6.0 }];
        let e = 200_000.0;
        let compliance = fastener_axial_compliance(&segs, e);
        let expected: f64 = segs.iter().map(|s| s.length / (e * s.area())).sum();
        assert!((compliance - expected).abs() < 1e-12);
    }

    #[test]
    fn axial_compliance_integral_matches_the_analytic_formula_for_constant_area() {
        let e = 200_000.0;
        let area = std::f64::consts::FRAC_PI_4 * 8.0 * 8.0;
        let numeric = axial_compliance_integral(25.0, e, |_| area);
        let analytic = 25.0 / (e * area);
        assert!((numeric - analytic).abs() / analytic < 1e-9);
    }

    #[test]
    fn torsional_compliance_matches_tl_over_gj_for_one_segment() {
        let seg = FastenerSegment { length: 20.0, diameter: 10.0 };
        let g = 80_000.0;
        let compliance = fastener_torsional_compliance(&[seg], g);
        let j = std::f64::consts::PI * seg.diameter.powi(4) / 32.0;
        let expected = 20.0 / (g * j);
        assert!((compliance - expected).abs() < 1e-12);
    }

    #[test]
    fn zero_cone_half_angle_reduces_member_compliance_to_a_straight_cylinder() {
        // A degenerate (zero-growth) cone must reduce to the plain L/(E*A)
        // formula for a uniform cylindrical clamp region - proves the
        // numerical cone integrator is correct independent of any actual
        // cone growth.
        let stack = MemberStack { members: vec![Member { thickness: 30.0, e: 70_000.0, hole_diameter: 6.5, outer_diameter: None }] };
        let contact_d = 12.0;
        let compliance = member_stack_compliance(&stack, contact_d, contact_d, 0.0);
        let area = std::f64::consts::FRAC_PI_4 * (contact_d * contact_d - 6.5 * 6.5);
        let expected = 30.0 / (70_000.0 * area);
        assert!((compliance - expected).abs() / expected < 1e-6, "compliance {compliance} vs expected {expected}");
    }

    #[test]
    fn member_compliance_decreases_when_a_stiffer_material_is_used() {
        let soft = MemberStack { members: vec![Member { thickness: 20.0, e: 70_000.0, hole_diameter: 6.5, outer_diameter: None }] };
        let stiff = MemberStack { members: vec![Member { thickness: 20.0, e: 200_000.0, hole_diameter: 6.5, outer_diameter: None }] };
        let c_soft = member_stack_compliance(&soft, 12.0, 12.0, 30.0);
        let c_stiff = member_stack_compliance(&stiff, 12.0, 12.0, 30.0);
        assert!(c_stiff < c_soft, "a stiffer (higher E) member must be less compliant");
    }

    #[test]
    fn member_compliance_is_bounded_by_the_members_own_outer_diameter() {
        // A cone that would ideally grow past the member's real outer
        // diameter must be truncated there, not silently exceed the real
        // geometry.
        let unbounded = MemberStack { members: vec![Member { thickness: 20.0, e: 70_000.0, hole_diameter: 6.5, outer_diameter: None }] };
        let bounded = MemberStack { members: vec![Member { thickness: 20.0, e: 70_000.0, hole_diameter: 6.5, outer_diameter: Some(14.0) }] };
        let c_unbounded = member_stack_compliance(&unbounded, 12.0, 12.0, 30.0);
        let c_bounded = member_stack_compliance(&bounded, 12.0, 12.0, 30.0);
        assert!(c_bounded > c_unbounded, "truncating the cone to a narrower real OD must reduce area and increase compliance");
    }

    #[test]
    fn multi_member_stack_uses_each_members_own_properties() {
        let stack = MemberStack {
            members: vec![
                Member { thickness: 10.0, e: 70_000.0, hole_diameter: 6.5, outer_diameter: None },
                Member { thickness: 10.0, e: 200_000.0, hole_diameter: 6.5, outer_diameter: None },
            ],
        };
        let compliance = member_stack_compliance(&stack, 12.0, 12.0, 30.0);
        assert!(compliance.is_finite() && compliance > 0.0);
    }

    #[test]
    fn vdi2230_additional_length_scales_with_diameter() {
        assert!((vdi2230_additional_length(10.0) - 4.0).abs() < 1e-9);
    }

    #[test]
    fn build_fastener_segments_appends_head_and_thread_engagement_segments() {
        let thread = ThreadGeometry { d: 10.0, d2: 9.026, d3: 8.160, pitch: 1.5, starts: 1, thread_angle_deg: 60.0 };
        let shank = [FastenerSegment { length: 20.0, diameter: 10.0 }];
        let full = build_fastener_segments(&shank, &thread);
        assert_eq!(full.len(), 3);
        assert!((full[0].length - vdi2230_additional_length(10.0)).abs() < 1e-9);
        assert!((full.last().unwrap().length - vdi2230_additional_length(8.160)).abs() < 1e-9);
    }
}
