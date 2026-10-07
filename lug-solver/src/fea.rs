//! The lug on the general finite-element kernel (`fea-core`): the lug's own Q9 mesh handed to the
//! kernel, a rigid analytic pin as a force-controlled master whose centre is an unknown, and the
//! kernel's contact (augmented Lagrangian, Coulomb) and nonlinear material. It is the independent
//! route the condensed solver is cross-checked against (`tests/fea_bridge.rs`); the condensed model
//! stays the interactive default because it is much faster (`docs/fea-core.md`).

use fea_core::kernel::von_mises;
use crate::fe::Material;
use crate::geometry::LugGeometry;
use crate::mesh::{Mesh, MeshSpec};
use crate::plastic::{Hardening, PlaneMode};
use crate::solve::{BushingSpec, LoadCase, PinSpec};
use fea_core::contact::{ContactSpec, RigidMaster, RigidShape};
use fea_core::material::J2;
use fea_core::{Elastic, ElementKind, Loads, Model, NlOptions, NlSolution, Physics, Start};

/// A lug mesh in the kernel.
pub struct FeaLug {
    pub model: Model,
    /// The lug's own mesh (node numbering shared with `model`).
    pub mesh: Mesh,
    pub geom: LugGeometry,
    pub half: bool,
    pub mode: PlaneMode,
    /// Radius of the surface the pin bears on (the bushing's bore when there is a bushing).
    pub bore_radius: f64,
    /// Pin-contact edges in the kernel's order `[corner, corner, midside]`, the contact slave surface.
    pub bore_faces: Vec<Vec<usize>>,
    pub bushing: Option<BushingSpec>,
    /// Conforming bushing / lug interface, `(bushing outer edge, lug bore edge)` in kernel order: the
    /// bushing's runs counter-clockwise (its body on the left), the lug's clockwise.
    pub iface: Vec<(Vec<usize>, Vec<usize>)>,
    /// Overrides the contact-iteration controls of every run (see [`Tuning`]); `None` picks the one that
    /// fits the analysis.
    pub tuning: Option<Tuning>,
    pub(crate) material: Material,
    /// The lug is a finite-strain elastic body (geometric nonlinearity), see [`FeaLug::set_geometric_nonlinearity`].
    pub(crate) finite_elastic: bool,
}

pub use fea_core::fit::Tuning;

/// What moves the rigid pin.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Drive {
    /// A force (lbf, whole lug) along the load direction; the pin finds its own position.
    Force { load: f64 },
    /// A prescribed travel (in) along the load direction; the pin's load is the result.
    Travel { cap: f64 },
}

/// Everything one run of the rigid pin on the kernel needs.
#[derive(Clone, Copy)]
pub(crate) struct RunSpec<'a> {
    pub pin: PinSpec,
    /// Load direction (the pin moves, or is pulled, along it).
    pub dir: [f64; 2],
    pub drive: Drive,
    pub tune: Tuning,
    pub steps: usize,
    /// A converged state to continue from (the interference fit).
    pub start: Option<&'a Start>,
    /// Give the lug von Mises plasticity.
    pub plastic: Option<J2>,
}

/// Result of a pin-load solve on the kernel.
pub struct FeaSolution {
    pub nl: NlSolution,
    /// Pin centre relative to the hole centre.
    pub pin_centre: [f64; 2],
    /// Pin centre displacement along the load direction (the condensed solver's `bearing_deflection`).
    pub bearing_deflection: f64,
    pub peak_pressure: f64,
    pub contact_arc_deg: f64,
    pub peak_hoop: f64,
    pub peak_von_mises: f64,
    pub factorisations: usize,
}

/// Discretisation and clevis geometry of [`FeaLug::solve_double_shear`].
#[derive(Debug, Clone, Copy)]
pub struct DoubleShearSpec {
    /// Distance from the lug face to the ear's load centroid; the ear thickness is twice this.
    pub offset: f64,
    /// Element layers across the lug's half thickness.
    pub lug_layers: usize,
    /// Element layers across the ear (`0`: no ears, the pin is loaded uniformly through the lug).
    pub ear_layers: usize,
    /// Radial element rings in the pin.
    pub pin_rings: usize,
    pub steps: usize,
}

/// Through-thickness bearing load of the 3D double-shear model.
pub struct DoubleShear {
    /// Layer centres from the lug mid-plane outward (`z >= 0`), in.
    pub z: Vec<f64>,
    /// Bearing load per unit thickness (lbf/in) in every layer.
    pub slice_load: Vec<f64>,
    /// Sum over the lug thickness (should equal the applied load).
    pub total_load: f64,
    pub factorisations: usize,
    pub dofs: usize,
}

/// Load-travel curve of a displacement-driven rigid pin.
pub struct FeaLimit {
    /// `(pin travel from first touch, pin load in lbf)` after every converged step.
    pub curve: Vec<(f64, f64)>,
    /// Largest load on the curve.
    pub limit_load_lbf: f64,
    pub max_equivalent_plastic_strain: f64,
    pub factorisations: usize,
}

impl FeaLug {
    /// Convert the lug's own mesh (no bushing).
    pub fn build(geom: &LugGeometry, material: Material, spec: MeshSpec, half: bool, mode: PlaneMode) -> Result<Self, String> {
        Self::build_bushed(geom, material, spec, half, mode, None)
    }

    /// As [`build`](Self::build), optionally with a bushing pressed into the hole.
    pub fn build_bushed(geom: &LugGeometry, material: Material, spec: MeshSpec, half: bool, mode: PlaneMode, bushing: Option<BushingSpec>) -> Result<Self, String> {
        let bm = bushing.as_ref().map(|b| crate::solve::bushing_mesh(geom, spec, b)).transpose()?;
        let mesh = Mesh::build_with(geom, spec, half, bm)?;
        Self::from_mesh(&mesh, geom, material, half, mode, bushing)
    }

    pub fn from_mesh(mesh: &Mesh, geom: &LugGeometry, material: Material, half: bool, mode: PlaneMode, bushing: Option<BushingSpec>) -> Result<Self, String> {
        let physics = match mode {
            PlaneMode::Stress => Physics::PlaneStress { thickness: geom.thickness },
            PlaneMode::Strain => Physics::PlaneStrain { thickness: geom.thickness },
        };
        let mut fm = fea_core::Mesh::new(physics);
        for p in &mesh.nodes {
            fm.add_node([p[0], p[1], 0.0]);
        }
        // Lattice position (angular i, radial j) of every Q9 local node `3*i + j`, in the library's VTK order.
        const VTK: [(usize, usize); 9] = [(0, 0), (2, 0), (2, 2), (0, 2), (1, 0), (2, 1), (1, 2), (0, 1), (1, 1)];
        let mut conn = [Vec::with_capacity(mesh.elems.len() * 9), Vec::new()];
        for (ei, e) in mesh.elems.iter().enumerate() {
            let at = |i: usize, j: usize| e[3 * i + j];
            let (p0, p1, p3) = (mesh.nodes[at(0, 0)], mesh.nodes[at(2, 0)], mesh.nodes[at(0, 2)]);
            let orient = (p1[0] - p0[0]) * (p3[1] - p0[1]) - (p1[1] - p0[1]) * (p3[0] - p0[0]);
            let group = usize::from(mesh.elem_group[ei] == 1);
            for (i, j) in VTK {
                conn[group].push(if orient > 0.0 { at(i, j) } else { at(j, i) });
            }
        }
        let [lug_conn, bush_conn] = conn;
        fm.add_block(ElementKind::Quad9, lug_conn, Elastic::new(material.e_psi, material.nu), "lug")?;
        if let Some(b) = &bushing {
            if bush_conn.is_empty() {
                return Err("the mesh has no bushing elements".into());
            }
            fm.add_block(ElementKind::Quad9, bush_conn, Elastic::new(b.material.e_psi, b.material.nu), "bushing")?;
        }
        let far = geom.length - 1e-9;
        fm.select_nodes("far", |p| p[0] >= far);
        fm.select_nodes("symmetry", |p| p[1].abs() < 1e-12);
        let bore_faces: Vec<Vec<usize>> = mesh.bore_edges.iter().map(|e| vec![e[0], e[2], e[1]]).collect(); // the kernel orders an edge corner, corner, midside
        fm.surfaces.insert("bore".into(), bore_faces.clone());
        // Interface edges are listed `[corner, mid, corner]`; orient the bushing's by increasing angle.
        let ang = |n: usize| mesh.nodes[n][1].atan2(mesh.nodes[n][0]);
        let iface = mesh
            .interface_edges
            .iter()
            .map(|(b, l)| {
                let mut d = ang(b[2]) - ang(b[0]);
                while d > std::f64::consts::PI {
                    d -= std::f64::consts::TAU;
                }
                while d < -std::f64::consts::PI {
                    d += std::f64::consts::TAU;
                }
                let ccw = d > 0.0;
                let (bf, lf) = if ccw { (vec![b[0], b[2], b[1]], vec![l[2], l[0], l[1]]) } else { (vec![b[2], b[0], b[1]], vec![l[0], l[2], l[1]]) };
                (bf, lf)
            })
            .collect();
        let model = Model::new(fm)?;
        Ok(Self { model, mesh: mesh.clone(), geom: *geom, half, mode, bore_radius: mesh.bore_radius, bore_faces, bushing, iface, tuning: None, material, finite_elastic: false })
    }

    /// Give the lug von Mises plasticity with this true stress - plastic strain curve: small strain, or
    /// total-Lagrangian finite (logarithmic) strain when `finite` is set.
    pub fn set_plasticity(&mut self, hardening: Hardening, finite: bool) -> Result<(), String> {
        self.model.mesh.set_plasticity(0, if finite { J2::finite(hardening) } else { J2::small(hardening) })
    }

    /// Radius of the rigid pin and the bushing's interference (radial overlap) after the temperature change:
    /// free thermal expansion of the lug, the bushing and the pin changes only the fit.
    pub(crate) fn fit(&self, pin: PinSpec) -> (f64, f64) {
        let radius = pin.thermal.pin_diameter(pin.diameter, self.bushing.is_some()) / 2.0;
        let overlap = self.bushing.map_or(0.0, |b| pin.thermal.interference(b.interference_dia, self.geom.hole_dia) / 2.0);
        (radius, overlap)
    }

    /// The contacts of an analysis: the rigid pin on the bore, and with a bushing the interference-fit
    /// interface between bushing and lug (two-pass, Coulomb friction).
    pub(crate) fn contacts(&self, pin: PinSpec, master: Option<RigidMaster>, overlap: f64, tune: Tuning) -> Vec<ContactSpec> {
        let eps_n = tune.eps_n(self.material.e_psi, self.bore_radius);
        let mut specs = Vec::new();
        if let Some(master) = master {
            let mut pin_spec = ContactSpec::rigid("pin", self.bore_faces.clone(), master, eps_n).with_rule(fea_core::contact::ContactRule::Reduced);
            if pin.friction > 0.0 {
                // The multiplier passes make the answer independent of this penalty (checked 0.01 - 0.1); a soft one keeps
                // Newton well conditioned and the passes few.
                pin_spec = pin_spec.with_friction(pin.friction, 0.03 * eps_n);
            }
            specs.push(pin_spec);
        }
        if let Some(b) = &self.bushing {
            let bush: Vec<Vec<usize>> = self.iface.iter().map(|(b, _)| b.clone()).collect();
            let lug: Vec<Vec<usize>> = self.iface.iter().map(|(_, l)| l.clone()).collect();
            // The interface may slide several elements under a collapse load: the matrix pattern is built from the
            // initial proximity, so give it a generous margin (a few face lengths).
            let face = lug.iter().map(|f| { let (p, q) = (self.mesh.nodes[f[0]], self.mesh.nodes[f[1]]); (p[0] - q[0]).hypot(p[1] - q[1]) }).fold(0.0f64, f64::max);
            for spec in fea_core::fit::interference_contacts("fit", lug, bush, eps_n, b.friction, overlap, 6.0 * face) {
                specs.push(spec);
            }
        }
        specs
    }

    /// Weak grounding springs on every bushing dof: the bushing touches the lug only through the interface, so a
    /// frictionless or lost fit would leave it free to translate and rotate. `1e-9 E t` per node dof leaks a
    /// negligible force (`Verification::ground_leak` in the condensed solver measured the same choice).
    pub(crate) fn bushing_ground(&self) -> Vec<(usize, f64)> {
        let Some(b) = &self.bushing else { return Vec::new() };
        let k = 1e-9 * b.material.e_psi * self.geom.thickness;
        let mut seen = vec![false; self.mesh.nodes.len()];
        for (ei, e) in self.mesh.elems.iter().enumerate() {
            if self.mesh.elem_group[ei] == 1 {
                for &n in e {
                    seen[n] = true;
                }
            }
        }
        (0..seen.len()).filter(|&n| seen[n]).flat_map(|n| [(2 * n, k), (2 * n + 1, k)]).collect()
    }

    /// Boundary conditions: the far end clamped, the symmetry plane of a half model.
    pub(crate) fn boundary(&self, model: &Model) -> Result<fea_core::Dirichlet, String> {
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("far")? {
            bc.fix_node(n);
        }
        if self.half {
            for &n in model.mesh.node_set("symmetry")? {
                bc.fix(n, 1, 0.0);
            }
        }
        Ok(bc)
    }

    /// The kernel model with the lug's blocks switched to finite-strain elastic bodies when
    /// [`set_geometric_nonlinearity`](Self::set_geometric_nonlinearity) asked for it.
    fn analysis_model(&self, rotation: Option<f64>, plastic: Option<J2>) -> Result<Model, String> {
        let mut mesh = self.model.mesh.clone();
        if let Some(j2) = plastic {
            mesh.set_plasticity(0, j2)?;
        }
        if let Some(angle) = rotation {
            let (s, c) = angle.sin_cos();
            for p in mesh.nodes.iter_mut() {
                let (x, y) = (p[0], p[1]);
                *p = [c * x - s * y, s * x + c * y, 0.0];
            }
        }
        if self.finite_elastic {
            for b in 0..mesh.blocks.len() {
                if mesh.blocks[b].plasticity.is_none() {
                    mesh.set_plasticity(b, J2::elastic_finite())?;
                }
            }
        }
        Model::new(mesh)
    }

    /// Include geometric nonlinearity (the second-order effect: tension stiffens the lug, compression
    /// softens it) by treating every elastic block as a finite-strain elastic body.
    pub fn set_geometric_nonlinearity(&mut self, on: bool) {
        self.finite_elastic = on;
    }

    /// Run the rigid pin on the kernel. `Drive::Force` loads the pin with `load` along `dir` (it is free to
    /// move); `Drive::Travel` drives it `cap` along `dir` (a full model keeps the sideways translation free
    /// and force-free, by rotating the lug so that `dir` is the global `-x` axis).
    pub(crate) fn run(&self, spec: RunSpec, tweak: impl Fn(&mut NlOptions)) -> Result<NlSolution, String> {
        let RunSpec { pin, dir, drive, tune, steps, start, plastic } = spec;
        let (r_pin, overlap) = self.fit(pin);
        let clear = self.bore_radius - r_pin;
        let share = if self.half { 0.5 } else { 1.0 };
        let mut master = RigidMaster::new(RigidShape::Circle { c: [0.0; 2], r: r_pin });
        let (model, master) = match drive {
            Drive::Force { load } => {
                master.shift = [clear * dir[0], clear * dir[1], 0.0];
                (self.analysis_model(None, plastic)?, master.with_free_translation([true, !self.half, false], [share * load * dir[0], share * load * dir[1], 0.0]))
            }
            Drive::Travel { cap } => {
                // Rotate by `phi` so that `dir` becomes -x.
                let phi = if self.half { 0.0 } else { std::f64::consts::PI - dir[1].atan2(dir[0]) };
                master.shift = [-clear, 0.0, 0.0];
                let m = master.with_travel([-cap, 0.0, 0.0]);
                (self.analysis_model(Some(phi), plastic)?, if self.half { m } else { m.with_free_translation([false, true, false], [0.0; 3]) })
            }
        };
        let bc = self.boundary(&model)?;
        let mut opt = tune.options(steps);
        tweak(&mut opt);
        let nl = model.solve_nonlinear_contact_from(&Loads { ground: self.bushing_ground(), ..Default::default() }, &bc, self.contacts(pin, Some(master), overlap, tune), &opt, start)?;
        if !nl.complete() {
            return Err(format!("the analysis stopped early: {:?}", nl.stop));
        }
        Ok(nl)
    }

    /// The bushed lug with the interference fit and no pin: the fit state the pin is then loaded in.
    /// The solution holds the interface contacts as interfaces `0` (lug slave) and `1` (bushing slave).
    pub(crate) fn fit_only(&self, pin: PinSpec, tune: Tuning, plastic: Option<J2>) -> Result<NlSolution, String> {
        let (_, overlap) = self.fit(pin);
        let model = self.analysis_model(None, plastic)?;
        let bc = self.boundary(&model)?;
        let nl = model.solve_nonlinear_contact(&Loads { ground: self.bushing_ground(), ..Default::default() }, &bc, self.contacts(pin, None, overlap, tune), &tune.options(1))?;
        if !nl.complete() {
            return Err(format!("the interference fit could not be solved: {:?}", nl.stop));
        }
        Ok(nl)
    }

    /// The interference-fit state as the start of a run that adds the pin: displacements and the interface
    /// contacts (the pin's contact is new).
    pub(crate) fn fit_start(fit: &NlSolution) -> Start {
        fea_core::fit::start_after_fit(fit, 1)
    }

    /// Solve the rigid pin loaded by `case` (force on the pin, applied as its free translation).
    pub fn solve(&self, pin: PinSpec, case: LoadCase, steps: usize) -> Result<FeaSolution, String> {
        let dir = case.direction();
        if self.half && !case.is_axial() {
            return Err("a half model is valid only for axial loads".into());
        }
        let tune = self.tuning.unwrap_or(if pin.friction > 0.0 || self.bushing.is_some() { Tuning::friction() } else { Tuning::frictionless() });
        let fit = if self.bushing.is_some() { Some(Self::fit_start(&self.fit_only(pin, tune, None)?)) } else { None };
        let nl = self.run(RunSpec { pin, dir, drive: Drive::Force { load: case.load_lbf }, tune, steps, start: fit.as_ref(), plastic: None }, |_| {})?;
        self.postprocess(nl, dir)
    }

    /// Plastic collapse of an axially loaded lug (half model): the pin is driven along the lug axis by
    /// `travel_cap_over_a` bore radii in `steps` increments with the lug's plasticity (set with
    /// [`set_plasticity`](Self::set_plasticity)), and the pin load is read at every step. A load-travel curve
    /// that flattens is the collapse load; with hardening it keeps rising, so read the curve at the strain of interest.
    pub fn limit_load_axial(&self, pin: PinSpec, travel_cap_over_a: f64, steps: usize) -> Result<FeaLimit, String> {
        if !self.half {
            return Err("the displacement-driven limit load is implemented for axial loads on the half model".into());
        }
        let tune = self.tuning.unwrap_or(Tuning::collapse());
        let fit = if self.bushing.is_some() { Some(Self::fit_start(&self.fit_only(pin, tune, None)?)) } else { None };
        let nl = self.run(RunSpec { pin, dir: [-1.0, 0.0], drive: Drive::Travel { cap: travel_cap_over_a * self.bore_radius }, tune, steps, start: fit.as_ref(), plastic: None }, |_| {})?;
        let cap = travel_cap_over_a * self.bore_radius;
        let curve: Vec<(f64, f64)> = nl.steps.iter().map(|s| (s.lambda * cap, 2.0 * s.master_force.first().map_or(0.0, |f| f[0]))).collect();
        let limit_load_lbf = curve.iter().fold(0.0f64, |m, c| m.max(c.1));
        Ok(FeaLimit { curve, limit_load_lbf, max_equivalent_plastic_strain: nl.state.max_ep(), factorisations: nl.factorisations })
    }

    /// Ring of Quad9 elements for the pin, from `r_core` to `r_pin`, matching the lug's angular divisions, in
    /// the lug's plane idealisation: the half ring `y >= 0` (node set `symmetry`: its two ends) or, when `full`,
    /// the whole ring (which needs weak grounding, see `Loads::ground`). Surface `rim` holds the outer edges, counter-clockwise, in kernel order `[end, end, midside]`.
    pub(crate) fn ring_mesh(&self, r_pin: f64, r_core: f64, layers: usize, pin_material: Material, full: bool) -> Result<fea_core::Mesh, String> {
        let n_around = self.bore_faces.len();
        let mut pm = fea_core::Mesh::new(self.model.mesh.physics);
        let nr = 2 * layers + 1;
        let nc = if full { 2 * n_around } else { 2 * n_around + 1 };
        let span = if full { std::f64::consts::TAU } else { std::f64::consts::PI };
        let id = |c: usize, r: usize| (c % nc) * nr + r;
        for c in 0..nc {
            let th = span * c as f64 / (if full { nc } else { nc - 1 }) as f64;
            for r in 0..nr {
                let rho = r_core + (r_pin - r_core) * r as f64 / (nr - 1) as f64;
                pm.add_node([rho * th.cos(), rho * th.sin(), 0.0]);
            }
        }
        let mut conn = Vec::new();
        for e in 0..n_around {
            for j in 0..layers {
                for (dc, dr) in [(0, 0), (0, 2), (2, 2), (2, 0), (0, 1), (1, 2), (2, 1), (1, 0), (1, 1)] {
                    conn.push(id(2 * e + dc, 2 * j + dr));
                }
            }
        }
        pm.add_block(ElementKind::Quad9, conn, Elastic::new(pin_material.e_psi, pin_material.nu), "pin")?;
        let faces: Vec<Vec<usize>> = (0..n_around).map(|e| vec![id(2 * e, nr - 1), id(2 * e + 2, nr - 1), id(2 * e + 1, nr - 1)]).collect();
        if !full {
            let sym: Vec<usize> = (0..nr).flat_map(|r| [id(0, r), id(nc - 1, r)]).collect();
            pm.node_sets.insert("symmetry".into(), sym);
        }
        pm.surfaces.insert("rim".into(), faces);
        Ok(pm)
    }

    /// As [`solve`](Self::solve) for an axial load on the half model with a meshed elastic pin: a ring with a
    /// small core hole (the condensed model's `PinBody::Elastic` disc), loaded through its length by a body
    /// force balancing the contact, in deformable contact with the bore. `layers` radial element layers.
    pub fn solve_elastic_pin(&self, pin: PinSpec, load_lbf: f64, pin_material: Material, layers: usize, steps: usize) -> Result<FeaSolution, String> {
        if !self.half {
            return Err("the meshed elastic pin is implemented for axial loads on the half model".into());
        }
        let r_pin = pin.diameter / 2.0;
        let r_core = 0.1 * r_pin;
        let mut fm = self.model.mesh.clone();
        let pin_nodes = self.ring_mesh(r_pin, r_core, layers, pin_material, false)?;
        let pin_faces = pin_nodes.surfaces["rim"].clone();
        let lug_blocks = fm.blocks.len();
        let off = fm.append(&pin_nodes, "pin/")?;
        let pin_faces: Vec<Vec<usize>> = pin_faces.iter().map(|f| f.iter().map(|n| n + off).collect()).collect();
        let model = Model::new(fm)?;
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("far")? {
            bc.fix_node(n);
        }
        for &n in model.mesh.node_set("symmetry")? {
            bc.fix(n, 1, 0.0);
        }
        for &n in model.mesh.node_set("pin/symmetry")? {
            bc.fix(n, 1, 0.0);
        }
        // The pin sits touching the loaded side: shifted toward -x by the clearance.
        let clear = self.bore_radius - r_pin;
        let mut mesh = model.mesh.clone();
        for p in mesh.nodes[off..].iter_mut() {
            p[0] -= clear;
        }
        let model = Model::new(mesh)?;
        let area = std::f64::consts::PI * (r_pin * r_pin - r_core * r_core) / 2.0;
        // The half model carries half the load; per unit area of the half ring (the kernel multiplies by thickness).
        let body = 0.5 * load_lbf / (area * self.geom.thickness);
        let loads = Loads { block_body: vec![(lug_blocks, [-body, 0.0, 0.0])], ..Default::default() };
        let tune = self.tuning.unwrap_or(Tuning::reference());
        let eps_n = tune.eps_n(self.material.e_psi, self.bore_radius);
        let spec = ContactSpec::deformable("pin", self.bore_faces.clone(), pin_faces, eps_n);
        let nl = model.solve_nonlinear_contact(&loads, &bc, vec![spec], &tune.options(steps))?;
        if !nl.complete() {
            return Err(format!("the analysis stopped early: {:?}", nl.stop));
        }
        let st = &nl.state.contact[0];
        let peak_pressure = st.iter().map(|s| s.p).fold(0.0f64, f64::max);
        let angles: Vec<f64> = st.iter().zip(&nl.contact_points[0]).filter(|(s, _)| s.active).map(|(_, x)| x[1].atan2(x[0])).collect();
        let contact_arc_deg = arc_deg(&angles, [-1.0, 0.0], true);
        let stresses = model.nodal_stresses(&nl.u, 0.0)?;
        let on_bore: std::collections::HashSet<usize> = self.bore_faces.iter().flatten().copied().collect();
        let mut peak_hoop = f64::MIN;
        let mut peak_von_mises = 0.0f64;
        for (i, s) in stresses.iter().enumerate().take(off) {
            peak_von_mises = peak_von_mises.max(von_mises(s));
            if on_bore.contains(&i) {
                let p = model.mesh.nodes[i];
                let (sn, cs) = p[1].atan2(p[0]).sin_cos();
                peak_hoop = peak_hoop.max(s[0] * sn * sn + s[1] * cs * cs - 2.0 * s[3] * sn * cs);
            }
        }
        // Pin centre: the mean displacement of the pin's rim nodes (a ring translates with its centre).
        let rim: Vec<usize> = pin_faces_nodes(&model, off);
        let cx = rim.iter().map(|&n| nl.u[n * 2]).sum::<f64>() / rim.len() as f64;
        let pin_centre = [cx - clear, 0.0];
        let factorisations = nl.factorisations;
        Ok(FeaSolution { nl, pin_centre, bearing_deflection: -(cx - clear), peak_pressure, contact_arc_deg, peak_hoop, peak_von_mises, factorisations })
    }

    /// 3D double-shear lug (one quarter by symmetry: `y >= 0`, `z >= 0` about the lug mid-plane): the lug and a
    /// pin of `pin_material` extruded from the 2D meshes, the pin carrying the clevis ears' bearing load as a
    /// body force over the ear thickness `2 * offset` beyond the lug face, in deformable 3D contact with the
    /// bore. Returns the bearing load of every lug layer (the through-thickness distribution that
    /// [`LugModel::through_thickness`](crate::LugModel::through_thickness) predicts with a beam).
    pub fn solve_double_shear(&self, pin: PinSpec, load_lbf: f64, pin_material: Material, ds: DoubleShearSpec) -> Result<DoubleShear, String> {
        let DoubleShearSpec { offset, lug_layers, ear_layers, pin_rings, steps } = ds;
        use fea_core::sweep::extrude;
        if !self.half {
            return Err("the 3D double-shear model is built on the half (axial) lug".into());
        }
        let t = self.geom.thickness;
        let (r_pin, r_core) = (pin.diameter / 2.0, 0.05 * pin.diameter);
        let half_t = 0.5 * t;
        // `ear_layers == 0`: no ears, the pin is loaded uniformly through the lug thickness (the 3D baseline
        // without pin bending).
        let ear = if ear_layers == 0 { 0.0 } else { 2.0 * offset };
        let lug_levels: Vec<f64> = (0..=lug_layers).map(|i| half_t * i as f64 / lug_layers as f64).collect();
        let mut pin_levels = lug_levels.clone();
        pin_levels.extend((1..=ear_layers).map(|i| half_t + ear * i as f64 / ear_layers as f64));
        let lug3 = extrude(&self.model.mesh, &lug_levels)?;
        let ring = self.ring_mesh(r_pin, r_core, pin_rings, pin_material, false)?;
        let mut pin3 = extrude(&ring, &pin_levels)?;
        // Split the pin into the part inside the lug and the part under the ear (separate body loads).
        let blk = pin3.blocks.remove(0);
        let nn = blk.kind.n_nodes();
        let (mut inner, mut outer) = (Vec::new(), Vec::new());
        for conn in blk.conn.chunks_exact(nn) {
            let zc = conn.iter().map(|&n| pin3.nodes[n][2]).sum::<f64>() / nn as f64;
            if zc < half_t { inner.extend_from_slice(conn) } else { outer.extend_from_slice(conn) }
        }
        pin3.add_block(blk.kind, inner, blk.material, "pin-lug")?;
        if !outer.is_empty() {
            pin3.add_block(blk.kind, outer, blk.material, "pin-ear")?;
        }
        // Master faces: the pin's rim inside the lug thickness.
        let rim: Vec<Vec<usize>> = pin3.surfaces["rim"].iter().filter(|f| f.iter().all(|&n| pin3.nodes[n][2] <= half_t + 1e-9)).cloned().collect();
        let mut mesh = lug3;
        let lug_blocks = mesh.blocks.len();
        let off = mesh.append(&pin3, "pin/")?;
        let rim: Vec<Vec<usize>> = rim.iter().map(|f| f.iter().map(|n| n + off).collect()).collect();
        // The pin touches the loaded side: shifted toward -x by the clearance.
        let clear = self.bore_radius - r_pin;
        for p in mesh.nodes[off..].iter_mut() {
            p[0] -= clear;
        }
        let model = Model::new(mesh)?;
        let mut bc = model.dirichlet();
        for &n in model.mesh.node_set("far")? {
            bc.fix(n, 0, 0.0);
            bc.fix(n, 1, 0.0);
        }
        for (i, p) in model.mesh.nodes.iter().enumerate() {
            if p[1].abs() < 1e-12 {
                bc.fix(i, 1, 0.0);
            }
            if p[2].abs() < 1e-12 {
                bc.fix(i, 2, 0.0);
            }
        }
        // Ear load: P/2 per ear, a quarter of it on this y >= 0, z >= 0 piece, spread over the ear volume.
        let (loaded_block, loaded_depth) = if ear_layers == 0 { (lug_blocks, half_t) } else { (lug_blocks + 1, ear) };
        let ear_vol = std::f64::consts::PI * (r_pin * r_pin - r_core * r_core) / 2.0 * loaded_depth;
        let b = 0.25 * load_lbf / ear_vol;
        let loads = Loads { block_body: vec![(loaded_block, [-b, 0.0, 0.0])], ..Default::default() };
        let tune = self.tuning.unwrap_or(Tuning::reference());
        let eps_n = tune.eps_n(self.material.e_psi, self.bore_radius);
        let spec = ContactSpec::deformable("pin", model.mesh.surfaces["bore"].clone(), rim, eps_n);
        let nl = model.solve_nonlinear_contact(&loads, &bc, vec![spec], &tune.options(steps))?;
        if !nl.complete() {
            return Err(format!("the analysis stopped early: {:?}", nl.stop));
        }
        // Bearing load of every lug layer: force on the lug along the load direction, summed over the layer's points.
        let (st, xs, ws) = (&nl.state.contact[0], &nl.contact_points[0], &nl.contact_weights[0]);
        let mut layer_load = vec![0.0; lug_layers];
        for ((s, x), w) in st.iter().zip(xs).zip(ws) {
            if s.active {
                let layer = ((x[2] / half_t * lug_layers as f64).floor() as usize).min(lug_layers - 1);
                let cos = x[0] / x[0].hypot(x[1]);
                layer_load[layer] += -s.p * w * cos; // force toward -x on the lug (per quarter model)
            }
        }
        // Per unit thickness, whole lug: x4 for the symmetry planes, divided by the layer thickness.
        let dz = half_t / lug_layers as f64;
        let slice_load: Vec<f64> = layer_load.iter().map(|f| 4.0 * f / dz / 2.0).collect();
        let z: Vec<f64> = (0..lug_layers).map(|i| (i as f64 + 0.5) * dz).collect();
        let total: f64 = layer_load.iter().sum::<f64>() * 4.0;
        Ok(DoubleShear { z, slice_load, total_load: total, factorisations: nl.factorisations, dofs: model.mesh.n_dofs() })
    }

    fn postprocess(&self, nl: NlSolution, dir: [f64; 2]) -> Result<FeaSolution, String> {
        let t = nl.rigid_translation[0];
        let pin_centre = [t[0], t[1]];
        let bearing_deflection = t[0] * dir[0] + t[1] * dir[1];
        let st = &nl.state.contact[0];
        let peak_pressure = st.iter().map(|s| s.p).fold(0.0f64, f64::max);
        // Contact patch: angular extent of the loaded Gauss points about the direction of the resultant.
        let angles: Vec<f64> = st.iter().zip(&nl.contact_points[0]).filter(|(s, _)| s.active).map(|(_, x)| x[1].atan2(x[0])).collect();
        let contact_arc_deg = arc_deg(&angles, dir, self.half);
        let stresses = self.model.nodal_stresses(&nl.u, 0.0)?;
        let mut peak_hoop = f64::MIN;
        let mut peak_von_mises = 0.0f64;
        let on_bore: std::collections::HashSet<usize> = self.bore_faces.iter().flatten().copied().collect();
        for (i, s) in stresses.iter().enumerate() {
            peak_von_mises = peak_von_mises.max(von_mises(s));
            if on_bore.contains(&i) {
                let p = self.model.mesh.nodes[i];
                let th = p[1].atan2(p[0]);
                let (sn, cs) = th.sin_cos();
                peak_hoop = peak_hoop.max(s[0] * sn * sn + s[1] * cs * cs - 2.0 * s[3] * sn * cs);
            }
        }
        let factorisations = nl.factorisations;
        Ok(FeaSolution { nl, pin_centre, bearing_deflection, peak_pressure, contact_arc_deg, peak_hoop, peak_von_mises, factorisations })
    }
}

fn wrap_towards(from: f64, to: f64) -> f64 {
    let mut d = to - from;
    while d > std::f64::consts::PI {
        d -= std::f64::consts::TAU;
    }
    while d < -std::f64::consts::PI {
        d += std::f64::consts::TAU;
    }
    from + d
}

/// Angular extent (degrees) of `angles` (radians) about the load direction `dir`; a half model's
/// extent is mirrored about the axis.
fn arc_deg(angles: &[f64], dir: [f64; 2], half: bool) -> f64 {
    if angles.is_empty() {
        return 0.0;
    }
    let centre = dir[1].atan2(dir[0]);
    let rel: Vec<f64> = angles.iter().map(|&a| wrap_towards(0.0, a - centre).to_degrees()).collect();
    if half {
        2.0 * rel.iter().fold(0.0f64, |m, v| m.max(v.abs()))
    } else {
        rel.iter().cloned().fold(f64::MIN, f64::max) - rel.iter().cloned().fold(f64::MAX, f64::min)
    }
}

/// Nodes on the outer rim of the pin appended at node offset `off`.
fn pin_faces_nodes(model: &Model, off: usize) -> Vec<usize> {
    model.mesh.surfaces.get("pin/rim").map_or(Vec::new(), |f| {
        let mut v: Vec<usize> = f.iter().flatten().copied().collect();
        v.sort_unstable();
        v.dedup();
        let _ = off;
        v
    })
}
