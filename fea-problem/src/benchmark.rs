//! Closed-form checks for the workbench's starting templates: an unmodified template is a benchmark problem with a known
//! answer, and the result view shows how close the finite-element value is. A problem edited in any way is not checked
//! (nothing is known about it).

use crate::solve::Solved;
use crate::templates::templates;

/// One FE quantity against its closed-form value.
#[derive(Debug, Clone, PartialEq)]
pub struct Check {
    pub quantity: &'static str,
    pub fe: f64,
    pub reference: f64,
}

impl Check {
    /// `fe / reference - 1`.
    pub fn error(&self) -> f64 {
        self.fe / self.reference - 1.0
    }
}

/// What the template is checked against and the checks.
#[derive(Debug, Clone, PartialEq)]
pub struct Benchmark {
    pub source: &'static str,
    pub checks: Vec<Check>,
    /// The error beyond which the result deserves a second look (the closed form's own accuracy and the mesh).
    pub tolerance: f64,
}

impl Benchmark {
    pub fn passed(&self) -> bool {
        self.checks.iter().all(|c| c.error().abs() <= self.tolerance)
    }

    /// Text lines for a report or a readout.
    pub fn lines(&self) -> Vec<String> {
        let mut out = vec![format!("Benchmark ({}): {}", self.source, if self.passed() { "within tolerance" } else { "OUTSIDE TOLERANCE" })];
        for c in &self.checks {
            out.push(format!("  {:<28} FE {:>12.5e}  closed form {:>12.5e}  {:+.2} %", c.quantity, c.fe, c.reference, 100.0 * c.error()));
        }
        out.push(format!("  (tolerance {:.0} %)", 100.0 * self.tolerance));
        out
    }
}

fn template(name: &str) -> Option<crate::Problem> {
    templates().into_iter().find(|(n, _)| *n == name).map(|(_, p)| p)
}

/// The benchmark of `s`, when its problem is exactly one of the starting templates that has a closed form.
pub fn check(s: &Solved) -> Option<Benchmark> {
    let name = s.problem.name.as_str();
    if template(name).as_ref() != Some(&s.problem) {
        return None;
    }
    let (e, nu) = (s.problem.material.e, s.problem.material.nu);
    match name {
        "Cantilever beam" => {
            // Tip load P at the free end of a beam L long, depth h, width t: Euler-Bernoulli plus the shear deflection
            // (Timoshenko, k = 5/6). The clamped end restrains the section's warping, which stiffens the FE beam slightly.
            let (p, l, h, t) = (100.0f64, 10.0f64, 1.0f64, s.problem.thickness);
            let i = t * h.powi(3) / 12.0;
            let g = e / (2.0 * (1.0 + nu));
            let tip = p * l.powi(3) / (3.0 * e * i) + p * l / (5.0 / 6.0 * g * t * h);
            Some(Benchmark { source: "Timoshenko beam theory", checks: vec![Check { quantity: "tip deflection", fe: s.summary.max_displacement.value, reference: tip }], tolerance: 0.02 })
        }
        "Thick cylinder (axisymmetric)" => {
            // Lame: hoop stress at the bore of a cylinder under internal pressure p (the radial stress there is -p).
            let (p, ri, ro) = (1000.0, 1.0f64, 2.0f64);
            let hoop = p * (ro * ro + ri * ri) / (ro * ro - ri * ri);
            Some(Benchmark { source: "Lame thick cylinder", checks: vec![Check { quantity: "hoop stress at the bore", fe: s.summary.max_principal.value, reference: hoop }], tolerance: 0.01 })
        }
        "Plate with a hole" => {
            // Heywood's net-section stress concentration of a hole in a finite-width plate: Ktn = 3 - 3.14 l + 3.667 l^2 -
            // 1.527 l^3 with l = d / W, on the net-section stress.
            let (w, d, gross) = (4.0f64, 1.0f64, 10_000.0f64);
            let l = d / w;
            // (3.14 is Heywood's fitted coefficient, not pi.)
            #[allow(clippy::approx_constant)]
            let ktn = 3.0 - 3.14 * l + 3.667 * l * l - 1.527 * l.powi(3);
            let peak = ktn * gross * w / (w - d);
            Some(Benchmark { source: "Heywood hole in a finite-width plate", checks: vec![Check { quantity: "peak stress at the hole", fe: s.summary.max_principal.value, reference: peak }], tolerance: 0.03 })
        }
        _ => None,
    }
}
