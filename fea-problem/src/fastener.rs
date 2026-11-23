//! A preloaded fastener on a STEP assembly (epic `docs/epic-assembly-feature-advice.md`, Phase C follow-up 4).
//!
//! The bolt is one `fea_core::Coupling` between the area-weighted mean axial displacement of the head's bearing faces and that of the
//! nut's: a spring of the bolt's axial stiffness `k_b` (`fastened-joint-solver`'s `fastener_axial_compliance`, VDI 2230 head and
//! thread additional lengths included) whose unloaded length is set so that it carries exactly the preload in the clamped state.
//! The preload comes from the same crate: a force, or a torque turned into a force by the thread and bearing friction equilibrium
//! (`fastened_joint_solver::solve::compute`); nothing of that maths is repeated here.
//!
//! The solve has two stages (`solve::solve_mesh_contact`): the bolt cut and the clamp force `P` applied to the head and nut faces
//! (the members compress by `delta_P`), then the bolt reconnected with offset `o = delta_P - P / k_b` and the service loads applied
//! from that state. The bolt tension is `k_b (delta - o)`.
use crate::problem::Problem;
use crate::ties::{dot, len, sub};
use fastened_joint_solver::bearing::BearingPressureModel;
use fastened_joint_solver::compliance::{FastenerSegment, Member, MemberStack};
use fastened_joint_solver::solve::{compute, AnalysisMode, FrictionInputs, JointGeometry, JointInputs, SolverStatus, TighteningMember};
use fastened_joint_solver::thread::ThreadGeometry;
use fastened_joint_solver::thread_catalog::{basic_minor_diameter, basic_pitch_diameter};
use fea_core::Coupling;
use serde::{Deserialize, Serialize};
use solid_kernel::features::size_info;
use std::collections::BTreeMap;

/// The head's and the nut's bearing diameter, in shank diameters, when none is given: the across-flats or head diameter of ISO 4014 /
/// 4762 bolts runs 1.55 to 1.7 d from M5 to M12 (the size table has no head or washer dimension). An assumption, reported in the
/// result's notes.
pub const BEARING_DIAMETER_FACTOR: f64 = 1.6;
/// Thread and bearing friction coefficient when none is given (the Preload Analysis default); it enters only a torque preload.
pub const DEFAULT_FRICTION: f64 = 0.15;
/// Young's modulus of a steel bolt in pascals and its Poisson ratio, when none is given.
const STEEL_E_PA: f64 = 200e9;
const STEEL_NU: f64 = 0.29;

/// How tight the bolt is: the clamp force, or the installation torque (force times length).
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum Preload {
    Force(f64),
    Torque(f64),
}

/// A bolt through a clamped stack, by its axis and the two bearing planes (problem units). `axis` points from the nut toward the head;
/// `head` and `nut` are the axial coordinates of the bearing planes measured from `axis_point` along `axis` (`head > nut`): the outer
/// faces of the outermost plates the head and the nut seat on.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Fastener {
    pub axis_point: [f64; 3],
    pub axis: [f64; 3],
    pub head: f64,
    pub nut: f64,
    /// A size of the tables (`"M6"`, `"1/4-20"`): the thread, the bore clearance and the shank diameter.
    pub size: String,
    /// Bearing diameters of the head and the nut: `None` is [`BEARING_DIAMETER_FACTOR`] shank diameters.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub head_diameter: Option<f64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub nut_diameter: Option<f64>,
    pub preload: Preload,
    /// Thread and bearing friction coefficient of the torque-to-preload equilibrium.
    #[serde(default = "default_friction")]
    pub friction: f64,
    /// Young's modulus of the bolt: `None` is steel.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bolt_modulus: Option<f64>,
}

fn default_friction() -> f64 {
    DEFAULT_FRICTION
}

impl Fastener {
    /// A fastener of `size` along an axis between two bearing planes with the default washer, friction and a steel bolt.
    pub fn new(axis_point: [f64; 3], axis: [f64; 3], nut: f64, head: f64, size: &str, preload: Preload) -> Fastener {
        Fastener { axis_point, axis, head, nut, size: size.into(), head_diameter: None, nut_diameter: None, preload, friction: DEFAULT_FRICTION, bolt_modulus: None }
    }

    /// Scale every length, force and stress of the fastener (a unit change of the problem).
    pub(crate) fn scale(&mut self, length: f64, force: f64, stress: f64) {
        self.axis_point = self.axis_point.map(|x| x * length);
        self.head *= length;
        self.nut *= length;
        self.head_diameter = self.head_diameter.map(|d| d * length);
        self.nut_diameter = self.nut_diameter.map(|d| d * length);
        self.bolt_modulus = self.bolt_modulus.map(|e| e * stress);
        self.preload = match self.preload {
            Preload::Force(f) => Preload::Force(f * force),
            Preload::Torque(t) => Preload::Torque(t * force * length),
        };
    }

    fn validate(&self, i: usize) -> Result<(), String> {
        let n = i + 1;
        let finite = self.axis_point.iter().chain(&self.axis).chain([&self.head, &self.nut]).all(|v| v.is_finite());
        if !finite || len(self.axis) < 1e-12 {
            return Err(format!("fastener {n}: the axis and the bearing planes need finite numbers and a nonzero axis direction"));
        }
        if self.head <= self.nut {
            return Err(format!("fastener {n}: the head plane must lie beyond the nut plane along the axis"));
        }
        if size_info(&self.size).is_none() {
            return Err(format!("fastener {n}: \"{}\" is not a size of the tables (M3 to M24, #4-40 to 1/2-13)", self.size));
        }
        for (what, d) in [("head", self.head_diameter), ("nut", self.nut_diameter)] {
            if d.is_some_and(|d| !(d.is_finite() && d > 0.0)) {
                return Err(format!("fastener {n}: the {what} bearing diameter must be positive"));
            }
        }
        let load = match self.preload {
            Preload::Force(v) | Preload::Torque(v) => v,
        };
        if !(load.is_finite() && load > 0.0) {
            return Err(format!("fastener {n}: the preload must be positive"));
        }
        if !(self.friction.is_finite() && self.friction >= 0.0) || self.bolt_modulus.is_some_and(|e| !(e.is_finite() && e > 0.0)) {
            return Err(format!("fastener {n}: the friction must not be negative and the bolt modulus must be positive"));
        }
        Ok(())
    }
}

impl Problem {
    /// Check the fasteners: a linear-elastic structural study of a STEP assembly with several bodies, and fasteners that describe a
    /// real bolt of the tables.
    pub(crate) fn validate_fasteners(&self) -> Result<(), String> {
        if self.fasteners.is_empty() {
            return Ok(());
        }
        if !self.contacts.is_empty() || !self.bushings.is_empty() {
            return Err("fasteners and 2D contact pairs or bushings do not combine: fasteners are for a static study of an assembly".into());
        }
        if self.any_plastic() || self.study.is_thermal() {
            return Err("fasteners are for a linear-elastic structural study: not an elastic-plastic or thermal one".into());
        }
        match &self.geometry {
            crate::problem::Geometry::Solid { cache: Some(c), .. } if c.bodies.len() >= 2 => {}
            _ => return Err("fasteners clamp the bodies of a STEP assembly with several bodies".into()),
        }
        self.fasteners.iter().enumerate().try_for_each(|(i, f)| f.validate(i))
    }
}

/// What one bolt carries in the solved state.
#[derive(Debug, Clone, PartialEq)]
pub struct FastenerResult {
    pub size: String,
    /// The clamp force the bolt was tightened to (from the force, or from the torque).
    pub preload: f64,
    /// The bolt's axial stiffness (`fastener_axial_compliance`, VDI 2230 head and thread lengths included).
    pub stiffness: f64,
    /// The bolt's tension after the service loads: `P` plus its share of them.
    pub tension: f64,
    /// The installation torque that gives the preload (the given torque, or the one the joint solver finds for a given force).
    pub torque: f64,
}

/// A fastener resolved on a mesh: its preload and stiffness from the joint solver, and the bearing nodes with their weights.
pub(crate) struct Bolt {
    pub axis: [f64; 3],
    pub head: Vec<(usize, f64)>,
    pub nut: Vec<(usize, f64)>,
    /// The clamp force.
    pub preload: f64,
    /// The bolt's axial stiffness.
    pub stiffness: f64,
    pub torque: f64,
    pub notes: Vec<String>,
}

impl Bolt {
    /// `(dof, coefficient)` of the elongation `c . u` (head mean minus nut mean, along the axis).
    fn terms(&self) -> Vec<(usize, f64)> {
        let side = |nodes: &[(usize, f64)], sign: f64| nodes.iter().flat_map(|&(n, w)| (0..3).map(move |c| (n * 3 + c, sign * w * self.axis[c]))).collect::<Vec<_>>();
        side(&self.head, 1.0).into_iter().chain(side(&self.nut, -1.0)).collect()
    }

    /// The clamp force of stage 1: `P` pressing the head toward the nut and the nut toward the head, spread over the bearing nodes.
    pub fn clamp_loads(&self) -> Vec<(usize, [f64; 3])> {
        let on = |nodes: &[(usize, f64)], sign: f64| nodes.iter().map(|&(n, w)| (n, self.axis.map(|a| sign * self.preload * w * a))).collect::<Vec<_>>();
        on(&self.head, -1.0).into_iter().chain(on(&self.nut, 1.0)).collect()
    }

    /// The elongation of the bolt's span in displacement field `u`.
    pub fn elongation(&self, u: &[f64]) -> f64 {
        self.terms().iter().map(|&(dof, c)| c * u[dof]).sum()
    }

    /// The bolt as a coupling whose unloaded span is `offset`.
    pub fn coupling(&self, offset: f64) -> Coupling {
        Coupling { terms: self.terms(), k: self.stiffness, offset }
    }

    /// The offset that makes the bolt carry the preload when the span has the elongation `clamped` (stage 1's).
    pub fn offset(&self, clamped: f64) -> f64 {
        clamped - self.preload / self.stiffness
    }

    pub fn result(&self, size: &str, u: &[f64], offset: f64) -> FastenerResult {
        FastenerResult { size: size.into(), preload: self.preload, stiffness: self.stiffness, tension: self.tension(u, offset), torque: self.torque }
    }

    /// Bolt tension in displacement field `u` for a bolt of the given offset.
    pub fn tension(&self, u: &[f64], offset: f64) -> f64 {
        self.stiffness * (self.elongation(u) - offset)
    }
}

impl Fastener {
    /// The preload, the stiffness and the bearing nodes of this fastener on `mesh` (a solid), in the problem's units.
    pub(crate) fn resolve(&self, p: &Problem, mesh: &fea_core::Mesh, index: usize) -> Result<Bolt, String> {
        let n = index + 1;
        let info = size_info(&self.size).ok_or_else(|| format!("fastener {n}: \"{}\" is not a size of the tables", self.size))?;
        let mm = 1e-3 / p.units.length_m();
        let (d, pitch, clearance) = (info.nominal * mm, info.pitch * mm, info.clearance * mm);
        let axis = {
            let l = len(self.axis);
            self.axis.map(|a| a / l)
        };
        let grip = self.head - self.nut;
        let (head_dia, nut_dia) = (self.head_diameter.unwrap_or(BEARING_DIAMETER_FACTOR * d), self.nut_diameter.unwrap_or(BEARING_DIAMETER_FACTOR * d));
        let e_bolt = self.bolt_modulus.unwrap_or(STEEL_E_PA / p.units.stress_pa());
        let thread = ThreadGeometry { d, d2: basic_pitch_diameter(d, pitch), d3: basic_minor_diameter(d, pitch), pitch, starts: 1, thread_angle_deg: 60.0 };
        let inputs = JointInputs {
            geometry: JointGeometry {
                thread,
                bearing_inner_radius: 0.5 * clearance,
                bearing_outer_radius: 0.5 * nut_dia,
                head_bearing_inner_radius: Some(0.5 * clearance),
                head_bearing_outer_radius: Some(0.5 * head_dia),
            },
            friction: FrictionInputs { mu_thread: self.friction, mu_bearing: self.friction, bearing_model: BearingPressureModel::UniformPressure, prevailing_torque: 0.0 },
            tightening_from: TighteningMember::Nut,
            fastener_segments: vec![FastenerSegment { length: grip, diameter: d }],
            fastener_e: e_bolt,
            fastener_nu: STEEL_NU,
            // The members are the mesh's business; the solver wants a stack to validate and to report a member compliance this
            // analysis does not use.
            member_stack: MemberStack { members: vec![Member { thickness: grip, e: p.material.e, hole_diameter: clearance, outer_diameter: None }] },
            cone_half_angle_deg: 30.0,
            mode: match self.preload {
                Preload::Force(f) => AnalysisMode::PreloadControlled { target_preload: f },
                Preload::Torque(t) => AnalysisMode::TorqueControlled { applied_torque: t },
            },
            external_axial_load: None,
            friction_interfaces: Vec::new(),
            applied_shear_load: None,
            proof_load: None,
            yield_strength: None,
            ultimate_strength: None,
            uncertainty: None,
            monte_carlo: None,
            thread_load_distribution: None,
            embedment_settlement: None,
            thread_engagement_length: None,
            thread_shear_strength: None,
            fatigue: None,
        };
        let solved = compute(&inputs).map_err(|e| format!("fastener {n}: the joint solver refused the bolt: {e:?}"))?;
        if solved.status != SolverStatus::Converged || !(solved.preload > 0.0) {
            return Err(format!("fastener {n}: the torque does not tighten the bolt ({:?}): the prevailing torque or the friction leaves no preload", solved.status));
        }
        if !(solved.compliance.k_b.is_finite() && solved.compliance.k_b > 0.0) {
            return Err(format!("fastener {n}: the bolt has no stiffness"));
        }
        let head = bearing_nodes(mesh, self, axis, self.head, 1.0, 0.5 * head_dia).map_err(|e| format!("fastener {n} head: {e}"))?;
        let nut = bearing_nodes(mesh, self, axis, self.nut, -1.0, 0.5 * nut_dia).map_err(|e| format!("fastener {n} nut: {e}"))?;
        let mut notes = Vec::new();
        if self.head_diameter.is_none() || self.nut_diameter.is_none() {
            notes.push(format!("Fastener {n} ({}): the bearing diameter is assumed {BEARING_DIAMETER_FACTOR} x the shank diameter (the size table has no head or washer dimension).", self.size));
        }
        if self.bolt_modulus.is_none() {
            notes.push(format!("Fastener {n}: the bolt is steel ({:.0} GPa).", STEEL_E_PA / 1e9));
        }
        Ok(Bolt { axis, head, nut, preload: solved.preload, stiffness: solved.compliance.k_b, torque: solved.torque.required(), notes })
    }
}

/// The nodes of the outer faces on the bearing plane `plane` (axial coordinate), seen from `outward` (+1 head, -1 nut) along the axis,
/// within radius `r_out` of the axis, with the weights of their tributary area (consistent for a quadratic face), summing to one.
fn bearing_nodes(mesh: &fea_core::Mesh, f: &Fastener, axis: [f64; 3], plane: f64, outward: f64, r_out: f64) -> Result<Vec<(usize, f64)>, String> {
    if mesh.dim() != 3 {
        return Err("fasteners need a solid mesh".into());
    }
    let tol = 1e-6 * (f.head - f.nut);
    let along = |x: [f64; 3]| dot(sub(x, f.axis_point), axis);
    let mut weights: BTreeMap<usize, f64> = BTreeMap::new();
    for (name, faces) in &mesh.surfaces {
        if !name.starts_with("face") {
            continue;
        }
        for face in faces.iter().filter(|fc| fc.len() == 3 || fc.len() == 6) {
            let c: Vec<[f64; 3]> = face[..3].iter().map(|&n| mesh.nodes[n]).collect();
            if c.iter().any(|&x| (along(x) - plane).abs() > tol) {
                continue;
            }
            let (u, v) = (sub(c[1], c[0]), sub(c[2], c[0]));
            let normal = [u[1] * v[2] - u[2] * v[1], u[2] * v[0] - u[0] * v[2], u[0] * v[1] - u[1] * v[0]];
            let area = 0.5 * len(normal);
            if !(area > 0.0) || outward * dot(normal, axis) < 0.999 * len(normal) {
                continue;
            }
            let centre = sub([(c[0][0] + c[1][0] + c[2][0]) / 3.0, (c[0][1] + c[1][1] + c[2][1]) / 3.0, (c[0][2] + c[1][2] + c[2][2]) / 3.0], f.axis_point);
            let a = dot(centre, axis);
            if len([centre[0] - a * axis[0], centre[1] - a * axis[1], centre[2] - a * axis[2]]) > r_out {
                continue;
            }
            // Linear: a third to each corner; quadratic: a third to each midside node and none to the corners.
            let carriers = if face.len() == 3 { &face[..3] } else { &face[3..] };
            for &node in carriers {
                *weights.entry(node).or_insert(0.0) += area / 3.0;
            }
        }
    }
    let total: f64 = weights.values().sum();
    if !(total > 0.0) {
        return Err(format!("no outer face of the assembly lies on the plane {plane:.6} (facing {}) within {r_out:.6} of the axis", if outward > 0.0 { "along the axis" } else { "against the axis" }));
    }
    Ok(weights.into_iter().map(|(n, a)| (n, a / total)).collect())
}
