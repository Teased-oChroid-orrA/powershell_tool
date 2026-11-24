//! The pin-near-an-edge bracket: the same contact plate solved as 2D plane stress, 2D plane strain and as a 3D solid, so the
//! only difference between the numbers is the idealisation. This is the driver of `tests/edge_3d_bracket.rs` (evidence:
//! `docs/edge-distance-crosscheck.md`, "3D bracket") moved into the library so the Plate Edge Workbench runs it.
//!
//! * the 2D mesh is `FeaLug`'s (Q9, graded about the bore, bushing annulus when there is one);
//! * `Idealisation::Strain` / `Stress` solve that mesh as it is; `Idealisation::Solid` extrudes it into Hex27 layers (one
//!   quarter of the plate: symmetric about the load line `y = 0` and the mid-plane `z = 0`);
//! * the housing is elastic-perfectly-plastic J2 (or a hardening law), the bushing elastic, the pin an analytic rigid circle
//!   / cylinder (uniform through the thickness) driven toward the edge by a prescribed travel;
//! * a bushing is installed first (interference by two-pass frictional contact, plastic if the housing yields), then the pin
//!   is made to fit the post-fit bore, as `edge-check`'s contact FE does;
//! * the load is read at a pin travel of [`D_OVER_D`] of the bore diameter, `edge-check`'s collapse rule.
//!
//! A solid pin pressed into a bare hole is not modelled in 3D (only a bushing's interference is; the bushed 3D pin stage did
//! not converge in trials, `docs/edge-distance-crosscheck.md`).

use crate::fea::FeaLug;
use crate::{BushingSpec, Hardening, LugGeometry, Material, MeshSpec, PlaneMode};
use fea_core::contact::{ContactRule, ContactSpec, RigidMaster, RigidShape};
use fea_core::fit::{interference_contacts, start_after_fit, Tuning};
use fea_core::material::J2;
use fea_core::nonlinear::{Interrupt, NlOptions};
use fea_core::sweep::extrude;
use fea_core::{Loads, Mesh, Model};
use std::sync::{atomic::AtomicBool, Arc};
use std::time::Instant;

/// The collapse load is read at this pin travel from first touch, in hole diameters.
pub const D_OVER_D: f64 = edge_check::contact::COLLAPSE_D_OVER_D;

#[derive(Clone, Copy, Debug)]
pub struct Housing {
    pub e: f64,
    pub nu: f64,
    /// Perfectly plastic flow stress, psi.
    pub flow: f64,
    /// A hardening law instead (small strain, flat beyond its last point); `None` = perfectly plastic at `flow`.
    pub law: Option<Hardening>,
}

#[derive(Clone, Copy, Debug)]
pub struct Bush {
    pub inner_dia: f64,
    pub e: f64,
    pub nu: f64,
    /// Diametral interference.
    pub interference_dia: f64,
    pub mu: f64,
}

#[derive(Clone, Copy, Debug)]
pub struct Case {
    /// Hole (bore) diameter, in.
    pub dia: f64,
    /// Diametral clearance of an unbushed pin (its diameter is `dia - pin_clearance`; the NACA tests used 0.0004 in on a 0.5 in hole).
    pub pin_clearance: f64,
    /// Edge distance over hole diameter.
    pub ed: f64,
    /// Thickness over hole diameter.
    pub td: f64,
    /// Plate width (y extent) and the distance from the hole centre to the clamped far end.
    pub width: f64,
    pub length: f64,
    /// Elements around the full bore circle of the 2D mesh.
    pub around: usize,
    pub housing: Housing,
    pub bush: Option<Bush>,
}

#[derive(Clone, Copy, Debug)]
pub enum Idealisation {
    Stress,
    Strain,
    /// `layers` Hex27 layers over half the thickness; `lock_z` pins `uz = 0` everywhere (plane strain in 3D).
    Solid { layers: usize, lock_z: bool },
}

impl Case {
    pub fn thickness(&self) -> f64 {
        self.td * self.dia
    }

    pub fn geometry(&self) -> LugGeometry {
        LugGeometry { hole_dia: self.dia, width: self.width, edge: self.ed * self.dia, length: self.length, thickness: self.thickness(), head_corner_radius: 0.0, far_corner_radius: 0.0 }
    }

    pub fn lug(&self, mode: PlaneMode) -> FeaLug {
        let bushing = self.bush.map(|b| BushingSpec { inner_dia: b.inner_dia, material: Material { e_psi: b.e, nu: b.nu }, interference_dia: b.interference_dia, friction: b.mu });
        let spec = MeshSpec { elements_around: self.around, ..MeshSpec::default() };
        FeaLug::build_bushed(&self.geometry(), Material { e_psi: self.housing.e, nu: self.housing.nu }, spec, true, mode, bushing).unwrap()
    }
}

/// Result of one run.
pub struct Out {
    /// `(travel / D, load lbf)` after every step.
    pub pts: Vec<(f64, f64)>,
    /// Mean bushing/housing fit pressure, psi (bushed cases).
    pub fit_pressure: Option<f64>,
    pub secs: f64,
    pub dofs: usize,
}

impl Out {
    /// Load at the collapse travel (the run ends there).
    pub fn at_limit(&self) -> f64 {
        let (t, p) = *self.pts.last().unwrap();
        assert!((t - D_OVER_D).abs() < 1e-9, "the curve ends at {t} D, not {D_OVER_D} D");
        p
    }

    pub fn peak(&self) -> f64 {
        self.pts.iter().fold(0.0, |m, p| m.max(p.1))
    }

    /// Initial stiffness: the largest secant `P / travel` of the curve (the first step can sit in the contact's settling).
    fn stiffness(&self) -> Option<f64> {
        self.pts.iter().filter(|(t, p)| *t > 0.0 && *p > 0.0).map(|(t, p)| p / t).max_by(|a, b| a.total_cmp(b))
    }

    /// The load at which the curve crosses the line of its initial stiffness shifted right by `frac` of the bore diameter
    /// of pin travel: the bearing-yield definition (MIL-HDBK-5 uses `frac = 0.02`). `None` if the curve ends before it
    /// crosses. Linear interpolation between the two steps that straddle it. With a perfectly plastic housing the curve
    /// is flat by about 0.05 D, so this lands within a few percent of the collapse load: the housing law has no hardening.
    pub fn offset_load(&self, frac: f64) -> Option<f64> {
        let k = self.stiffness()?;
        let gap = |&(t, p): &(f64, f64)| p - k * (t - frac);
        let mut prev = (0.0, 0.0);
        for cur in &self.pts {
            if gap(cur) <= 0.0 && gap(&prev) > 0.0 {
                let w = gap(&prev) / (gap(&prev) - gap(cur));
                return Some(prev.1 + w * (cur.1 - prev.1));
            }
            prev = *cur;
        }
        None
    }

    /// The load at which the secant stiffness first falls `drop` (a fraction, 0.05) below the initial stiffness: where the
    /// curve leaves the elastic line, the onset of yielding in the load-travel response. `None` if it never does.
    pub fn departure_load(&self, drop: f64) -> Option<f64> {
        let k = self.stiffness()?;
        let sec = |&(t, p): &(f64, f64)| p / t;
        let i = self.pts.iter().position(|q| q.0 > 0.0 && sec(q) < (1.0 - drop) * k && q.1 > 0.0 && self.pts.iter().take_while(|r| r.0 <= q.0).any(|r| sec(r) >= k))?;
        let (a, b) = if i == 0 { ((0.0, 0.0), self.pts[0]) } else { (self.pts[i - 1], self.pts[i]) };
        let target = (1.0 - drop) * k;
        let (fa, fb) = (if a.0 > 0.0 { sec(&a) - target } else { k - target }, sec(&b) - target);
        let w = if fa == fb { 1.0 } else { fa / (fa - fb) };
        Some(a.1 + w * (b.1 - a.1))
    }
}

/// Bearing yield is read at this offset, in bore diameters (the handbook's 2 % of the hole diameter).
pub const OFFSET_FRACTION: f64 = 0.02;
/// The onset of yielding is the load where the secant stiffness has lost this fraction.
pub const DEPARTURE_DROP: f64 = 0.05;

/// One mesh of a convergence run.
#[derive(Debug, Clone, PartialEq)]
pub struct Refinement {
    /// Elements around the full bore circle and Hex27 layers over half the thickness (1 for 2D).
    pub around: usize,
    pub layers: usize,
    /// Load at the collapse travel, lbf.
    pub collapse: f64,
    /// Bearing yield (2 % offset), lbf, if the curve reaches it.
    pub onset: Option<f64>,
    /// Onset of yielding (5 % stiffness loss), lbf.
    pub departure: Option<f64>,
    pub secs: f64,
    pub dofs: usize,
    /// Relative change of the collapse load and of the onset load against the previous mesh.
    pub collapse_change: Option<f64>,
    pub onset_change: Option<f64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Converged {
    pub steps: Vec<Refinement>,
    /// The last two meshes agree within the tolerance on every load that exists.
    pub converged: bool,
}

impl Converged {
    pub fn last(&self) -> &Refinement {
        self.steps.last().expect("a convergence run has at least one mesh")
    }
}

/// The meshes of a convergence run, coarse to fine. The bore ring is refined first (each step adds 18 elements around, the
/// step the bracket's own convergence study used), then a 3D solid also gets one more layer: layers moved the 3D answer
/// by 0.08 % from 2 to 4, the ring by 0.25 %, and a layer costs far more than a ring element.
fn schedule(case: &Case, ideal: Idealisation, max: usize) -> Vec<(usize, usize)> {
    let layers0 = match ideal {
        Idealisation::Solid { layers, .. } => layers,
        _ => 1,
    };
    let solid = matches!(ideal, Idealisation::Solid { .. });
    (0..max).map(|k| (case.around + 18 * k, if solid && k >= 2 { layers0 + 1 } else { layers0 })).collect()
}

/// Solves on a sequence of finer meshes until the collapse load (and the bearing yield, where the curve has one) changes by
/// less than `tol` (a fraction, e.g. 0.005) between two meshes, or `max_meshes` have been run. `progress` is called after
/// every mesh. The elastic-plastic contact stage is not remeshed adaptively (the kernel's contact AMR is 2D only and was
/// not exposed); this refines the graded mesh by a fixed rule and stops on the measured change, not on a count.
pub fn converge(case: Case, ideal: Idealisation, tol: f64, max_meshes: usize, cancel: Option<Arc<AtomicBool>>, progress: &mut dyn FnMut(&Refinement)) -> Result<Converged, String> {
    let mut steps: Vec<Refinement> = Vec::new();
    for (around, layers) in schedule(&case, ideal, max_meshes.max(1)) {
        let ideal_k = match ideal {
            Idealisation::Solid { lock_z, .. } => Idealisation::Solid { layers, lock_z },
            other => other,
        };
        let out = try_run(Case { around, ..case }, ideal_k, cancel.clone())?;
        let (collapse, onset) = (out.at_limit(), out.offset_load(OFFSET_FRACTION));
        let rel = |a: f64, b: f64| ((a - b) / b).abs();
        let prev = steps.last();
        let r = Refinement { around, layers, collapse, onset, departure: out.departure_load(DEPARTURE_DROP), secs: out.secs, dofs: out.dofs, collapse_change: prev.map(|p| rel(collapse, p.collapse)), onset_change: prev.and_then(|p| Some(rel(onset?, p.onset?))) };
        progress(&r);
        let done = r.collapse_change.is_some_and(|c| c < tol) && r.onset_change.is_none_or(|c| c < tol);
        steps.push(r);
        if done {
            return Ok(Converged { steps, converged: true });
        }
    }
    Ok(Converged { steps, converged: false })
}

pub fn plasticity(h: &Housing) -> J2 {
    J2::small(h.law.unwrap_or_else(|| Hardening::linear(h.flow, 0.0, 1.0).unwrap()))
}

/// Runs the case; `cancel` set from another thread stops it at the next Newton step with an error. Blocking: a 3D case
/// takes minutes.
pub fn try_run(case: Case, ideal: Idealisation, cancel: Option<Arc<AtomicBool>>) -> Result<Out, String> {
    let interrupt = Interrupt { deadline: None, cancel };
    let t0 = Instant::now();
    let fe = case.lug(if matches!(ideal, Idealisation::Stress) { PlaneMode::Stress } else { PlaneMode::Strain });
    let mut mesh2 = fe.model.mesh.clone();
    let bushed = case.bush.is_some();
    if bushed {
        mesh2.surfaces.insert("fit_lug".into(), fe.iface.iter().map(|(_, l)| l.clone()).collect());
        mesh2.surfaces.insert("fit_bush".into(), fe.iface.iter().map(|(b, _)| b.clone()).collect());
    }
    let solid = matches!(ideal, Idealisation::Solid { .. });
    let mut mesh: Mesh = match ideal {
        Idealisation::Solid { layers, .. } => {
            let half_t = 0.5 * case.thickness();
            let levels: Vec<f64> = (0..=layers).map(|i| half_t * i as f64 / layers as f64).collect();
            extrude(&mesh2, &levels)?
        }
        _ => mesh2,
    };
    mesh.set_plasticity(0, plasticity(&case.housing)).map_err(|e| e.to_string())?;
    let dim = mesh.dim();
    let lock_z = matches!(ideal, Idealisation::Solid { lock_z: true, .. });
    let model = Model::new(mesh).map_err(|e| e.to_string())?;
    let mut bc = model.dirichlet();
    for &n in model.mesh.node_set("far").unwrap() {
        bc.fix(n, 0, 0.0);
        bc.fix(n, 1, 0.0);
    }
    for (i, p) in model.mesh.nodes.iter().enumerate() {
        if p[1].abs() < 1e-12 {
            bc.fix(i, 1, 0.0);
        }
        if solid && (lock_z || p[2].abs() < 1e-12) {
            bc.fix(i, 2, 0.0);
        }
    }
    // The bushing touches the housing only through the interface: hold it weakly (it leaks `k u` of force).
    let mut ground = Vec::new();
    if let Some(b) = case.bush {
        let k = 1e-9 * b.e * case.thickness() / if solid { 4.0 } else { 1.0 };
        let mut seen = vec![false; model.mesh.nodes.len()];
        for &n in &model.mesh.blocks[1].conn {
            seen[n] = true;
        }
        for n in (0..seen.len()).filter(|&n| seen[n]) {
            for c in 0..dim {
                ground.push((dim * n + c, k));
            }
        }
    }
    let loads = Loads { ground, ..Default::default() };
    let a_contact = fe.bore_radius;
    let trace = std::env::var_os("BRACKET_TRACE").is_some();
    if trace {
        eprintln!("[{:.1}s] model built: {} nodes, {} dof", t0.elapsed().as_secs_f64(), model.mesh.nodes.len(), model.mesh.n_dofs());
    }

    // Stage 1: the interference fit.
    let overlap = case.bush.map_or(0.0, |b| 0.5 * b.interference_dia);
    let fit_specs = |eps_n: f64| -> Vec<ContactSpec> {
        let Some(b) = case.bush else { return Vec::new() };
        let fl = model.mesh.surfaces["fit_lug"].clone();
        let fb = model.mesh.surfaces["fit_bush"].clone();
        let face = fl
            .iter()
            .map(|f| {
                let mut m = 0.0f64;
                for &p in f {
                    for &q in f {
                        let (u, v) = (model.mesh.nodes[p], model.mesh.nodes[q]);
                        m = m.max((u[0] - v[0]).hypot(u[1] - v[1]));
                    }
                }
                m
            })
            .fold(0.0f64, f64::max);
        let margin = if solid { 2.0 * face } else { 6.0 * face };
        interference_contacts("fit", fl, fb, eps_n, b.mu, overlap, margin).to_vec()
    };
    let (mut fit_nl, mut fit_pressure) = (None, None);
    if bushed {
        let tune = Tuning::friction();
        let specs = fit_specs(tune.eps_n(case.housing.e, a_contact));
        let nl = model.solve_nonlinear_contact(&loads, &bc, specs, &NlOptions { interrupt: interrupt.clone(), ..tune.options(1) }).map_err(|e| e.to_string())?;
        if !nl.complete() {
            return Err(format!("the interference fit stopped early: {:?}", nl.stop));
        }
        // The two passes (each body the slave of the other) carry about half of the physical pressure each: it is their sum.
        fit_pressure = Some((0..2).map(|k| {
            let (pf, wt) = nl.state.contact[k].iter().zip(&nl.contact_weights[k]).fold((0.0, 0.0), |(a, b), (s, w)| (a + s.p * w, b + w));
            pf / wt
        }).sum());
        if trace {
            eprintln!("[{:.1}s] fit done: pressure {:.0} psi, {} factorisations", t0.elapsed().as_secs_f64(), fit_pressure.unwrap(), nl.factorisations);
        }
        fit_nl = Some(nl);
    }

    // The pin: an unbushed one is `dia - pin_clearance`; a bushed one is made to fit the post-fit bore (zero gap on the
    // loaded side; `edge-check` conforms it everywhere).
    let bore_nodes: Vec<usize> = {
        let mut v: Vec<usize> = model.mesh.surfaces["bore"].iter().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        v
    };
    let (r_pin, r_left) = match &fit_nl {
        Some(nl) => {
            let r = |n: usize| (model.mesh.nodes[n][0] + nl.u[dim * n]).hypot(model.mesh.nodes[n][1] + nl.u[dim * n + 1]);
            let r_min = bore_nodes.iter().map(|&n| r(n)).fold(f64::MAX, f64::min);
            (r_min, r_min)
        }
        None => (0.5 * (case.dia - case.pin_clearance), a_contact),
    };
    let cap = D_OVER_D * case.dia;
    let shape = if solid { RigidShape::Cylinder { p: [0.0; 3], a: [0.0, 0.0, 1.0], r: r_pin } } else { RigidShape::Circle { c: [0.0; 2], r: r_pin } };
    let master = RigidMaster { shift: [-(r_left - r_pin), 0.0, 0.0], ..RigidMaster::new(shape).with_travel([-cap, 0.0, 0.0]) };
    let tune = Tuning::collapse();
    let eps_n = tune.eps_n(case.housing.e, a_contact);
    let mut specs = vec![ContactSpec::rigid("pin", model.mesh.surfaces["bore"].clone(), master, eps_n).with_rule(ContactRule::Reduced)];
    specs.extend(fit_specs(eps_n));
    let steps = ((cap / (tune.step_fraction * a_contact)).ceil() as usize).clamp(6, 80);
    let start = fit_nl.as_ref().map(|nl| start_after_fit(nl, 1));
    let nl = model.solve_nonlinear_contact_from(&loads, &bc, specs, &NlOptions { interrupt, ..tune.options(steps) }, start.as_ref()).map_err(|e| e.to_string())?;
    if !nl.complete() {
        return Err(format!("the pin run stopped early: {:?}", nl.stop));
    }
    // Whole-plate force: x2 for the load-line symmetry, x2 again for the mid-plane in 3D.
    let share = if solid { 4.0 } else { 2.0 };
    let pts: Vec<(f64, f64)> = nl.steps.iter().map(|s| (s.lambda * cap / case.dia, share * s.master_force[0][0])).collect();
    Ok(Out { pts, fit_pressure, secs: t0.elapsed().as_secs_f64(), dofs: model.mesh.n_dofs() })
}
