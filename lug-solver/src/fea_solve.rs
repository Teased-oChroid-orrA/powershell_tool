//! The lug's elastic analysis on the general kernel: the same result type as the condensed
//! [`LugModel::solve`](crate::LugModel::solve) (contact points, bore stresses, peaks, bushing fit,
//! verification), so the toolbox can run either solver.

use crate::contact::{IfacePoint, PointResult};
use crate::fea::{Drive, FeaLug, RunSpec, Tuning};
use crate::fe::Material;
use crate::solve::{BoreStress, BushingResult, LoadCase, LugSolution, PinBody, PinSpec, Timings, Verification};
use crate::stress::Stress;
use fea_core::contact::ContactSpec;
use fea_core::{Loads, Model, NlSolution};
use std::time::Instant;

/// Components of a kernel stress `[xx, yy, zz, xy, yz, zx]` in the plane.
fn plane(s: &[f64; 6]) -> Stress {
    Stress { sx: s[0], sy: s[1], txy: s[3] }
}

fn von_mises(s: &[f64; 6]) -> f64 {
    let (a, b, c) = (s[0], s[1], s[2]);
    (0.5 * ((a - b).powi(2) + (b - c).powi(2) + (c - a).powi(2)) + 3.0 * (s[3] * s[3] + s[4] * s[4] + s[5] * s[5])).sqrt()
}

fn angle_deg(p: &[f64; 3]) -> f64 {
    p[1].atan2(p[0]).to_degrees().rem_euclid(360.0)
}

/// A run with a meshed elastic pin: the solution on the lug + pin model and what is needed to read it.
struct ElasticRun {
    nl: NlSolution,
    /// The model the solution belongs to (lug nodes first, then the pin's).
    model: Model,
    /// Pin centre relative to the hole centre.
    centre: [f64; 2],
}

impl FeaLug {
    /// The loaded meshed elastic pin: a ring with a small core hole (the condensed model's `PinBody::Elastic`
    /// disc) loaded through its length by a body force balancing the contact, in deformable contact with the
    /// bore. A full ring is held by weak grounding springs.
    fn run_elastic(&self, material: Material, spec: RunSpec) -> Result<ElasticRun, String> {
        let RunSpec { pin, dir, drive, tune, steps, start, .. } = spec;
        let Drive::Force { load } = drive else { return Err("a meshed elastic pin is loaded by force".into()) };
        let (r_pin, overlap) = self.fit(pin);
        let r_core = 0.1 * r_pin;
        let ring = self.ring_mesh(r_pin, r_core, 6, material, !self.half)?;
        let rim_faces = ring.surfaces["rim"].clone();
        let mut fm = self.model.mesh.clone();
        if self.finite_elastic {
            for b in 0..fm.blocks.len() {
                fm.set_plasticity(b, fea_core::material::J2::elastic_finite())?;
            }
        }
        let pin_block = fm.blocks.len();
        let off = fm.append(&ring, "pin/")?;
        let rim_faces: Vec<Vec<usize>> = rim_faces.iter().map(|f| f.iter().map(|n| n + off).collect()).collect();
        // The pin touches the loaded side of the bore at the start.
        let clear = self.bore_radius - r_pin;
        for p in fm.nodes[off..].iter_mut() {
            p[0] += clear * dir[0];
            p[1] += clear * dir[1];
        }
        let model = Model::new(fm)?;
        let mut bc = self.boundary(&model)?;
        if self.half {
            for &n in model.mesh.node_set("pin/symmetry")? {
                bc.fix(n, 1, 0.0);
            }
        }
        let share = if self.half { 0.5 } else { 1.0 };
        let area = (if self.half { 0.5 } else { 1.0 }) * std::f64::consts::PI * (r_pin * r_pin - r_core * r_core);
        let body = share * load / (area * self.geom.thickness);
        // A full ring can rotate (and, before contact, translate) freely: hold it with weak springs. The bushing too.
        let mut ground = self.bushing_ground();
        if !self.half {
            let k = 1e-8 * material.e_psi * self.geom.thickness;
            ground.extend((off..model.mesh.nodes.len()).flat_map(|n| [(2 * n, k), (2 * n + 1, k)]));
        }
        let loads = Loads { block_body: vec![(pin_block, [body * dir[0], body * dir[1], 0.0])], ground, ..Default::default() };
        let eps_n = tune.eps_n(self.material.e_psi, self.bore_radius);
        let mut pin_spec = ContactSpec::deformable("pin", self.bore_faces.clone(), rim_faces, eps_n);
        if pin.friction > 0.0 {
            pin_spec = pin_spec.with_friction(pin.friction, 0.03 * eps_n);
        }
        let mut specs = vec![pin_spec];
        specs.extend(self.contacts(pin, None, overlap, tune));
        let nl = model.solve_nonlinear_contact_from(&loads, &bc, specs, &tune.options(steps), start)?;
        if !nl.complete() {
            return Err(format!("the analysis stopped early: {:?}", nl.stop));
        }
        let rim_nodes: Vec<usize> = {
            let mut v: Vec<usize> = model.mesh.surfaces["pin/rim"].iter().flatten().copied().collect();
            v.sort_unstable();
            v.dedup();
            v
        };
        let n = rim_nodes.len() as f64;
        let centre = [rim_nodes.iter().map(|&i| model.mesh.nodes[i][0] + nl.u[2 * i]).sum::<f64>() / n, rim_nodes.iter().map(|&i| model.mesh.nodes[i][1] + nl.u[2 * i + 1]).sum::<f64>() / n];
        Ok(ElasticRun { nl, model, centre })
    }

    /// Beyond this `x` the clamped far end disturbs the field (its corners are singular).
    fn clamp_zone_x(&self) -> f64 {
        (self.geom.length - 0.5 * self.geom.width).max(0.6 * self.geom.length)
    }

    /// The error estimate only looks at the region around the pin.
    fn evaluation_zone_x(&self) -> f64 {
        (1.5 * self.geom.width).min(self.clamp_zone_x())
    }

    /// Pin-contact points of interface `0` of a solution, in the condensed solver's format.
    fn pin_points(&self, nl: &NlSolution, mu: f64) -> Vec<PointResult> {
        let (st, xs, ws) = (&nl.state.contact[0], &nl.contact_points[0], &nl.contact_weights[0]);
        st.iter()
            .zip(xs)
            .zip(ws)
            .map(|((s, x), w)| {
                let th = x[1].atan2(x[0]);
                let (sn, cs) = th.sin_cos();
                // Force on the slave (the lug) is minus the friction traction of the kernel's convention.
                let shear = if s.active { -(s.fric[0] * -sn + s.fric[1] * cs) } else { 0.0 };
                PointResult { angle_deg: angle_deg(x), pressure: if s.active { s.p } else { 0.0 }, shear, gap: s.g, slipping: s.active && mu > 0.0 && (s.fric[0].hypot(s.fric[1])) >= mu * s.p * (1.0 - 1e-6), x: [x[0], x[1]], w: *w }
            })
            .collect()
    }

    /// Bushing / lug interface points: the two passes' pressures add up to the physical pressure.
    /// `first` is the index of the lug-slave pass among the solution's contacts (1 with a pin, 0 without).
    fn interface_points(&self, nl: &NlSolution, first: usize) -> Vec<IfacePoint> {
        let mu = self.bushing.map_or(0.0, |b| b.friction);
        let (a, b) = (&nl.state.contact[first], &nl.state.contact[first + 1]);
        let (xs, ws) = (&nl.contact_points[first], &nl.contact_weights[first]);
        let mut out = Vec::with_capacity(a.len());
        for i in 0..a.len() {
            let x = xs[i];
            let th = x[1].atan2(x[0]);
            let (sn, cs) = th.sin_cos();
            // Pass `a` has the lug as its slave, pass `b` the bushing: the traction on the lug is -fa + fb.
            let pa = if a[i].active { a[i].p } else { 0.0 };
            let pb = b.get(i).map_or(0.0, |s| if s.active { s.p } else { 0.0 });
            let fa = if a[i].active { -(a[i].fric[0] * -sn + a[i].fric[1] * cs) } else { 0.0 };
            let fb = b.get(i).map_or(0.0, |s| if s.active { s.fric[0] * -sn + s.fric[1] * cs } else { 0.0 });
            let p = pa + pb;
            let shear = fa + fb;
            out.push(IfacePoint { angle_deg: angle_deg(&x), pressure: p, shear, gap: a[i].g.min(b.get(i).map_or(f64::INFINITY, |s| s.g)), slipping: p > 0.0 && shear.abs() >= mu * p * (1.0 - 1e-3), w: ws[i] });
        }
        out
    }

    /// Elastic solutions for several load directions at one load, in parallel (a full model unless every
    /// angle is axial).
    pub fn analyze_sweep(&self, pin: PinSpec, load_lbf: f64, angles_deg: &[f64]) -> Vec<Result<LugSolution, String>> {
        crate::solve::parallel_map(angles_deg, |&angle_deg| self.analyze(pin, LoadCase { load_lbf, angle_deg }))
    }

    /// Elastic solutions for several loads in one direction, in parallel (the load-response curve).
    pub fn load_sweep(&self, pin: PinSpec, angle_deg: f64, loads_lbf: &[f64]) -> Vec<Result<LugSolution, String>> {
        crate::solve::parallel_map(loads_lbf, |&load_lbf| self.analyze(pin, LoadCase { load_lbf, angle_deg }))
    }

    /// Mean interface pressure the bushing's interference leaves with no pin and no load (0 without a bushing).
    pub fn fit_pressure(&self, pin: PinSpec) -> Result<f64, String> {
        if self.bushing.is_none() {
            return Ok(0.0);
        }
        let nl = self.fit_only(pin, self.tuning.unwrap_or(Tuning::friction()), None)?;
        let w: f64 = nl.contact_weights[0].iter().sum();
        Ok(self.interface_points(&nl, 0).iter().map(|p| p.pressure * p.w).sum::<f64>() / w.max(1e-300))
    }

    /// Elastic analysis of the pin loaded by `case` on the kernel, with the same outputs as the condensed
    /// [`LugModel::solve`](crate::LugModel::solve). Bushing fit and interference, thermal change of the
    /// fit, finite-strain elastic geometry ([`set_geometric_nonlinearity`](Self::set_geometric_nonlinearity))
    /// and friction are all included.
    pub fn analyze(&self, pin: PinSpec, case: LoadCase) -> Result<LugSolution, String> {
        if !(pin.diameter.is_finite() && pin.diameter > 0.0) {
            return Err("pin diameter must be positive".into());
        }
        if !(case.load_lbf.is_finite() && case.load_lbf >= 0.0) {
            return Err("load must be zero or positive".into());
        }
        if self.half && !case.is_axial() {
            return Err("a symmetric (half) model only supports axial loads".into());
        }
        if pin.diameter / 2.0 > 1.2 * self.bore_radius {
            return Err("pin is far larger than the hole".into());
        }
        if matches!(pin.body, PinBody::Elastic(_)) && !self.half && pin.friction > 0.0 {
            // Friction puts a torque on a meshed pin that only the clevis can react; the symmetric half model has none.
            return Err("an elastic pin with friction needs the symmetric (axial) model: the kernel does not model the clevis torque reaction".into());
        }
        let dir = case.direction();
        let t = self.geom.thickness;
        let tune = self.tuning.unwrap_or(if pin.friction > 0.0 || self.bushing.is_some() { Tuning::friction() } else { Tuning::frictionless() });
        let clock = Instant::now();
        // The unloaded fit: the pin's force-free position and the interface pressure the interference leaves.
        let fit_nl = if self.bushing.is_some() { Some(self.fit_only(pin, tune, None)?) } else { None };
        let (fit_unloaded, s_eq) = if let Some(nl0) = &fit_nl {
            let w: f64 = nl0.contact_weights[0].iter().sum();
            let mean = self.interface_points(nl0, 0).iter().map(|p| p.pressure * p.w).sum::<f64>() / w.max(1e-300);
            // The pin is free inside the fitted bore (or gripped, centred): its force-free position is the centre.
            let s_eq = 0.0;
            (mean, s_eq)
        } else {
            (0.0, 0.0)
        };
        // Friction is path dependent: one load step skips the history and lands ~8 % low on the hoop stress, two or
        // more agree (measured at 45 deg, mu 0.15: 2, 4, 16 and 32 steps give the same hoop stress and travel).
        let steps = if pin.friction > 0.0 || self.bushing.is_some_and(|b| b.friction > 0.0) { 3 } else { 1 };
        let start = fit_nl.as_ref().map(Self::fit_start);
        let elastic = match pin.body {
            // Deformable contact needs the tight passes (pointwise pressures scatter on non-matching meshes); pf 100 stalls with friction.
            PinBody::Elastic(m) => Some(self.run_elastic(m, RunSpec { pin, dir, drive: Drive::Force { load: case.load_lbf }, tune: self.tuning.unwrap_or(Tuning::friction()), steps: 4, start: start.as_ref(), plastic: None })?),
            PinBody::Rigid => None,
        };
        let rigid_nl;
        let (nl, stress_model, c) = match &elastic {
            Some(e) => (&e.nl, &e.model, e.centre),
            None => {
                rigid_nl = self.run(RunSpec { pin, dir, drive: Drive::Force { load: case.load_lbf }, tune, steps, start: start.as_ref(), plastic: None }, |_| {})?;
                let t = rigid_nl.rigid_translation[0];
                (&rigid_nl, &self.model, [t[0], t[1]])
            }
        };
        let contact_ms = clock.elapsed().as_secs_f64() * 1e3;

        let t1 = Instant::now();
        let stresses = stress_model.nodal_stresses(&nl.u, 0.0)?;
        let mesh = &self.mesh;
        let row_stress = |row: usize| -> Vec<BoreStress> {
            (0..mesh.n_cols)
                .map(|c| {
                    let th = mesh.theta[c];
                    let s = &stresses[mesh.node_id(c, row)];
                    let (hoop, radial, shear) = plane(s).polar(th);
                    BoreStress { angle_deg: th.to_degrees().rem_euclid(360.0), hoop, radial, shear, von_mises: von_mises(s) }
                })
                .collect()
        };
        let bore = row_stress(0);
        let lug_bore = if mesh.bush_rows > 0 { row_stress(mesh.bush_rows) } else { bore.clone() };
        let peak_of = |v: &[BoreStress]| v.iter().fold((f64::MIN, 0.0), |(h, a), b| if b.hoop > h { (b.hoop, b.angle_deg) } else { (h, a) });
        let (peak_hoop, peak_hoop_angle) = peak_of(&lug_bore);
        let mut in_bushing = vec![false; mesh.nodes.len()];
        for (ei, e) in mesh.elems.iter().enumerate() {
            if mesh.elem_group[ei] == 1 {
                for &n in e {
                    in_bushing[n] = true;
                }
            }
        }
        let (mut peak_vm, mut peak_at, mut bush_vm) = (0.0f64, [0.0, 0.0], 0.0f64);
        let clamp_x = self.clamp_zone_x();
        for (id, s) in stresses.iter().enumerate().take(mesh.nodes.len()) {
            let vm = von_mises(s);
            if in_bushing[id] {
                bush_vm = bush_vm.max(vm);
            } else if mesh.nodes[id][0] > clamp_x {
                continue;
            } else if vm > peak_vm {
                peak_vm = vm;
                peak_at = mesh.nodes[id];
            }
        }
        let points = self.pin_points(nl, pin.friction);
        let iface = if self.bushing.is_some() { self.interface_points(nl, 1) } else { Vec::new() };
        let bushing_result = self.bushing.map(|b| {
            let (bh, ba) = peak_of(&bore);
            let total: f64 = iface.iter().map(|p| p.w).sum::<f64>().max(1e-300);
            let top = iface.iter().fold(0.0f64, |m, q| m.max(q.pressure)).max(1e-300);
            let separated: f64 = iface.iter().filter(|p| p.pressure <= 1e-6 * top).map(|p| p.w).sum();
            let mean = iface.iter().map(|p| p.pressure * p.w).sum::<f64>() / total;
            BushingResult {
                interface: iface.clone(),
                fit_pressure_unloaded: fit_unloaded,
                interface_mean_pressure: mean,
                interface_peak_pressure: iface.iter().fold(0.0f64, |m, p| m.max(p.pressure)),
                separated_fraction: separated / total,
                peak_hoop: bh,
                peak_hoop_angle_deg: ba,
                peak_von_mises: bush_vm,
                bearing_stress: case.load_lbf / (b.inner_dia * t),
            }
        });

        // Contact patch extent about the load direction.
        let dir_deg = dir[1].atan2(dir[0]).to_degrees();
        let peak_pressure = points.iter().fold(0.0f64, |m, p| m.max(p.pressure));
        let (mut lo, mut hi, mut any) = (f64::MAX, f64::MIN, false);
        let mut all_round = true;
        for p in &points {
            if p.pressure > 1e-3 * peak_pressure && peak_pressure > 0.0 {
                let mut d = (p.angle_deg - dir_deg).rem_euclid(360.0);
                if d > 180.0 {
                    d -= 360.0;
                }
                if self.half {
                    d = d.abs();
                    lo = lo.min(-d);
                }
                lo = lo.min(d);
                hi = hi.max(d);
                any = true;
            } else {
                all_round = false;
            }
        }
        let (contact_arc_deg, contact_centre_deg) = if !any {
            (0.0, dir_deg)
        } else if all_round {
            (360.0, dir_deg)
        } else {
            (hi - lo, dir_deg + 0.5 * (hi + lo))
        };

        let pin_centre = c;
        let travel = c[0] * dir[0] + c[1] * dir[1];
        if std::env::var("NL_DEBUG").is_ok() {
            eprintln!("pin travel {travel:.4e}, force-free {s_eq:.4e}");
        }
        let target = if self.half { 0.5 * case.load_lbf } else { case.load_lbf };
        let force = nl.contact.as_ref().map_or([0.0; 3], |s| s.master_force[0]);
        let along = -(force[0] * dir[0] + force[1] * dir[1]);
        let across = if self.half { 0.0 } else { (force[0] * dir[1] - force[1] * dir[0]).abs() };
        let force_balance = if target > 0.0 { (along - target).abs().max(across) / target } else { 0.0 };
        let peak_p = peak_pressure.max(1e-300);
        let max_penetration = nl.contact.as_ref().map_or(0.0, |s| (-s.min_gap).max(0.0)) / self.bore_radius;
        let mu_i = self.bushing.map_or(0.0, |b| b.friction);
        let ex = points.iter().map(|p| (p.shear.abs() - pin.friction * p.pressure) / peak_p).fold(0.0f64, f64::max);
        let ei = iface.iter().map(|p| (p.shear.abs() - mu_i * p.pressure) / peak_p).fold(0.0f64, f64::max);
        let rn = nl.steps.last().and_then(|s| s.residuals.last().copied()).unwrap_or(0.0);
        // The kernel solves in force control: the pin load is an unknown of the Newton system, so the
        // discrete energy identity is replaced by the equilibrium residual over the load.
        let equilibrium = if target > 0.0 { rn / target } else { 0.0 };
        let zone = self.evaluation_zone_x();
        let discretisation_error = match stress_model.zz_error(&nl.u, 0.0) {
            Ok(zz) => {
                let mut e2 = 0.0;
                let mut at = 0usize;
                for (b, blk) in self.model.mesh.blocks.iter().enumerate() {
                    for e in 0..blk.n_elems() {
                        let cx = blk.elem(e).iter().map(|&n| self.model.mesh.nodes[n][0]).sum::<f64>() / blk.elem(e).len() as f64;
                        if cx <= zone {
                            e2 += zz.eta[at + e] * zz.eta[at + e];
                        }
                    }
                    at += blk.n_elems();
                    let _ = b;
                }
                (e2 / (e2 + zz.norm * zz.norm).max(1e-300)).sqrt()
            }
            Err(_) => 0.0,
        };
        let verification = Verification { force_balance, energy_balance: equilibrium, max_penetration, friction_excess: ex.max(ei), tension: 0.0, ground_leak: 0.0, discretisation_error };
        let recover_ms = t1.elapsed().as_secs_f64() * 1e3;
        let net_section_stress = if self.geom.width > self.geom.hole_dia { case.load_lbf / ((self.geom.width - self.geom.hole_dia) * t) } else { 0.0 };
        let kt_net = if net_section_stress > 0.0 { peak_hoop / net_section_stress } else { 0.0 };
        Ok(LugSolution {
            load: [case.load_lbf * dir[0], case.load_lbf * dir[1]],
            pin_centre,
            bearing_deflection: (travel - s_eq).max(0.0),
            contact: points,
            peak_pressure,
            contact_arc_deg,
            contact_centre_deg,
            bore,
            lug_bore,
            bushing: bushing_result,
            peak_hoop,
            peak_hoop_angle_deg: peak_hoop_angle,
            peak_von_mises: peak_vm,
            peak_von_mises_at: peak_at,
            net_section_stress,
            bearing_stress: case.load_lbf / (self.geom.hole_dia * t),
            kt_net,
            mesh: mesh.stats,
            dofs: self.model.mesh.n_dofs(),
            bandwidth: 0,
            contact_iterations: nl.steps.iter().map(|s| s.iterations).sum(),
            timings: Timings { contact_ms, recover_ms, ..Default::default() },
            verification,
        })
    }
}
