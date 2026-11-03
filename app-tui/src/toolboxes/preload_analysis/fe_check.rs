//! Finite-element cross-check of the clamped-member compliance, on the general kernel (`fea-core`
//! through `fea_problem::joint`): the stack is meshed as an axisymmetric solid and its compliance
//! compared with the pressure-cone estimate the solver uses (`C_m`), with the joint load fraction
//! each implies and the cone half angle that would reproduce the FE value.
//!
//! The solver's own numbers are untouched: this is a read-only second opinion that runs on a
//! worker a moment after the joint stops changing (`PreloadSession::start_fe`, `gpu_viewer/editor/preload`).

use fastened_joint_solver::compliance::{member_stack_compliance, Member, MemberStack};
use fastened_joint_solver::service::joint_load_fraction;
use fea_problem::joint::{member_field, Layer, MemberFe, MemberField};

/// What the check is made for; two equal inputs give one result.
#[derive(Debug, Clone, PartialEq)]
pub struct FeInput {
    pub members: Vec<Member>,
    pub poisson_ratios: Vec<f64>,
    /// Bearing diameter at the head and the nut (the solver uses one for both).
    pub contact_diameter: f64,
    pub cone_half_angle_deg: f64,
    /// Fastener axial compliance (for the load fractions).
    pub fastener_compliance: f64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct FeResult {
    pub fe: MemberFe,
    /// The cone estimate of the member compliance at the input angle.
    pub cone: f64,
    /// Cone half angle (degrees) whose compliance equals the FE one; `None` when no angle in 5..=60 does.
    pub equivalent_angle_deg: Option<f64>,
    pub fraction_cone: f64,
    pub fraction_fe: f64,
}

impl FeResult {
    /// `FE / cone - 1`.
    pub fn difference(&self) -> f64 {
        self.fe.compliance / self.cone - 1.0
    }
}

pub fn run(input: &FeInput) -> Result<FeResult, String> {
    run_with_field(input).map(|(r, _)| r)
}

/// [`run`] together with the solved member stack it measured (stress per unit clamp force anywhere in it), so one solve
/// feeds both the cross-check and a picture of the joint.
pub fn run_with_field(input: &FeInput) -> Result<(FeResult, MemberField), String> {
    if input.members.len() != input.poisson_ratios.len() {
        return Err("one Poisson ratio is required per member".into());
    }
    let layers: Vec<Layer> = input.members.iter().zip(&input.poisson_ratios).map(|(m, &nu)| Layer { thickness: m.thickness, e: m.e, nu, hole_diameter: m.hole_diameter, outer_diameter: m.outer_diameter }).collect();
    let field = member_field(&layers, input.contact_diameter, input.contact_diameter)?;
    let fe = field.fe;
    let stack = MemberStack { members: input.members.clone() };
    let cone_at = |angle: f64| member_stack_compliance(&stack, input.contact_diameter, input.contact_diameter, angle);
    let cone = cone_at(input.cone_half_angle_deg);
    // Cone compliance falls as the angle grows: bisect for the angle that matches the FE value.
    let (mut lo, mut hi) = (5.0f64, 60.0f64);
    let equivalent_angle_deg = if cone_at(lo) >= fe.compliance && cone_at(hi) <= fe.compliance {
        for _ in 0..60 {
            let mid = 0.5 * (lo + hi);
            if cone_at(mid) > fe.compliance {
                lo = mid;
            } else {
                hi = mid;
            }
        }
        Some(0.5 * (lo + hi))
    } else {
        None
    };
    let fraction = |c_m: f64| {
        if input.fastener_compliance > 0.0 && c_m > 0.0 {
            joint_load_fraction(1.0 / input.fastener_compliance, 1.0 / c_m)
        } else {
            0.0
        }
    };
    Ok((FeResult { fe, cone, equivalent_angle_deg, fraction_cone: fraction(cone), fraction_fe: fraction(fe.compliance) }, field))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn input() -> FeInput {
        FeInput {
            members: vec![Member { thickness: 0.5, e: 10.3e6, hole_diameter: 0.3125, outer_diameter: None }, Member { thickness: 0.5, e: 10.3e6, hole_diameter: 0.3125, outer_diameter: None }],
            poisson_ratios: vec![0.3, 0.3],
            contact_diameter: 0.75,
            cone_half_angle_deg: 30.0,
            fastener_compliance: 4.0e-8,
        }
    }

    #[test]
    fn the_fe_value_brackets_a_cone_angle_and_that_angle_reproduces_it() {
        let r = run(&input()).unwrap();
        let angle = r.equivalent_angle_deg.expect("an equivalent angle exists for a typical joint");
        assert!((5.0..=60.0).contains(&angle));
        let stack = MemberStack { members: input().members };
        let c = member_stack_compliance(&stack, 0.75, 0.75, angle);
        assert!((c / r.fe.compliance - 1.0).abs() < 1e-6, "{c:e} vs {:e}", r.fe.compliance);
        // A softer member (higher compliance) lowers the bolt's share of an external load.
        assert!(r.fraction_fe.abs() > 0.0 && r.fraction_cone.abs() > 0.0);
        assert_eq!(r.fraction_fe > r.fraction_cone, r.fe.compliance > r.cone);
    }

    #[test]
    fn the_field_crosses_threads_and_is_the_solve_the_cross_check_measured() {
        fn send<T: Send + 'static>(_: &T) {}
        let (r, field) = run_with_field(&input()).unwrap();
        send(&field);
        assert_eq!(field.fe, r.fe);
        // A unit clamp force through the bearing ring: compression just under the head, inside the ring.
        let s = std::thread::spawn(move || field.stress_at(0.25, 0.01)).join().unwrap().expect("inside the stack");
        assert!(s[1] < 0.0, "{s:?}");
    }

    #[test]
    fn a_bad_stack_is_an_error_not_a_result() {
        let mut i = input();
        i.contact_diameter = 0.2; // smaller than the hole
        assert!(run(&i).is_err());
    }

    #[test]
    fn each_member_poisson_ratio_reaches_the_fe_model() {
        let mut i = input();
        let baseline = run(&i).unwrap().fe.compliance;
        i.poisson_ratios = vec![0.22, 0.38];
        let actual = run(&i).unwrap().fe.compliance;
        assert!((actual / baseline - 1.0).abs() > 1e-4);
        let layers: Vec<Layer> = i.members.iter().zip(&i.poisson_ratios).map(|(m, &nu)| Layer { thickness: m.thickness, e: m.e, nu, hole_diameter: m.hole_diameter, outer_diameter: m.outer_diameter }).collect();
        let expected = fea_problem::joint::member_compliance(&layers, i.contact_diameter, i.contact_diameter).unwrap().compliance;
        assert!((actual / expected - 1.0).abs() < 1e-10);
        i.poisson_ratios.pop();
        assert!(run(&i).is_err());
    }
}
