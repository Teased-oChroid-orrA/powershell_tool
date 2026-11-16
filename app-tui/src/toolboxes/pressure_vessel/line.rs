//! The inputs of a supported line run (the tube between supports, its point loads and load cases) and their conversion to
//! `pressure_vessel_solver::line_run`. Numbers only: the stations, margins and reactions are the solver's. The tube itself
//! is the window's vessel (outer diameter, remaining wall, material); this adds what a cylinder analysis has no use for.
use pressure_vessel_solver::line_run::{LineRun, LoadCase, PointLoad, Support, Tube, DEFAULT_SUBDIVISIONS};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct SupportIn {
    pub x: f64,
    /// Also holds the line axially (a thermal anchor).
    #[serde(default)]
    pub axial_anchor: bool,
    /// Also holds the slope (a built-in end).
    #[serde(default)]
    pub rotation_fixed: bool,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LoadIn {
    pub label: String,
    pub x: f64,
    /// Force along x, y, z (lb).
    pub force: [f64; 3],
    /// Weight (lb), scaled by the case's load factors like the tube's own weight.
    pub weight: f64,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CaseIn {
    pub name: String,
    pub internal_pressure: f64,
    pub external_pressure: f64,
    /// The pressure also loads the ends (a closed run), else it acts on the wall only.
    pub closed_ends: bool,
    pub delta_t: f64,
    /// Multiplies the weights along x, y, z (a gravity case is `[0, 0, -1]`).
    pub load_factors: [f64; 3],
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct LineRunInputs {
    /// The window analyses the supported line instead of the plain cylinder.
    pub enabled: bool,
    pub length: f64,
    /// Weight of what the tube carries (lb per in).
    pub contents_weight: f64,
    /// Tube material weight density (lb per in^3); the catalog's density when it has one, never invented.
    pub density: f64,
    pub supports: Vec<SupportIn>,
    pub loads: Vec<LoadIn>,
    pub cases: Vec<CaseIn>,
    /// The case the drawing and the reactions show.
    pub selected_case: usize,
}

impl LineRunInputs {
    /// A two-support run with a gravity case at the vessel's own pressure.
    pub fn new(internal_pressure: f64, external_pressure: f64, closed_ends: bool, delta_t: f64) -> Self {
        let length = 60.0;
        Self {
            enabled: false,
            length,
            contents_weight: 0.0,
            density: 0.0,
            supports: vec![SupportIn { x: 0.0, axial_anchor: true, rotation_fixed: false }, SupportIn { x: length, axial_anchor: false, rotation_fixed: false }],
            loads: Vec::new(),
            cases: vec![CaseIn { name: "Case 1".into(), internal_pressure, external_pressure, closed_ends, delta_t, load_factors: [0.0, 0.0, -1.0] }],
            selected_case: 0,
        }
    }

    pub fn selected(&self) -> usize {
        self.selected_case.min(self.cases.len().saturating_sub(1))
    }

    /// The tube between the supports as the solver wants it; `tube.wall` and `outer_diameter` are the vessel's remaining wall
    /// and diameter.
    pub fn solver_run(&self, tube: Tube) -> LineRun {
        LineRun {
            tube,
            x_start: 0.0,
            x_end: self.length,
            supports: self.supports.iter().map(|s| Support { x: s.x, axial_anchor: s.axial_anchor, rotation_fixed: s.rotation_fixed, rotation_stiffness: None }).collect(),
            contents_weight_per_length: self.contents_weight,
            insulation_weight_per_length: 0.0,
            point_loads: self.loads.iter().map(|l| PointLoad { x: l.x, force: l.force, weight: l.weight, ..PointLoad::default() }).collect(),
            subdivisions: DEFAULT_SUBDIVISIONS,
        }
    }

    pub fn solver_cases(&self) -> Vec<LoadCase> {
        self.cases
            .iter()
            .map(|c| LoadCase {
                name: c.name.clone(),
                internal_pressure: c.internal_pressure,
                external_pressure: c.external_pressure,
                pressure_end_load: c.closed_ends,
                delta_t: c.delta_t,
                load_factors: c.load_factors,
                point_loads: Vec::new(),
            })
            .collect()
    }

    /// The total applied load of a case along x, y, z: point forces, point weights and the tube's own and its contents'
    /// weight, which `tube_weight_per_length` gives. What the reactions must balance.
    pub fn applied_total(&self, case: &CaseIn, tube_weight_per_length: f64) -> [f64; 3] {
        let n = case.load_factors;
        let w = tube_weight_per_length + self.contents_weight;
        let mut t = [0.0; 3];
        for k in 0..3 {
            t[k] = w * self.length * n[k] + self.loads.iter().map(|l| l.force[k] + l.weight * n[k]).sum::<f64>();
        }
        t
    }

    pub fn add_support(&mut self) {
        let x = self.supports.iter().map(|s| s.x).fold(0.0_f64, f64::max);
        let x = (x + self.length / 4.0).min(self.length);
        self.supports.push(SupportIn { x, axial_anchor: false, rotation_fixed: false });
    }
    pub fn add_load(&mut self) {
        let n = self.loads.len() + 1;
        self.loads.push(LoadIn { label: format!("Load {n}"), x: self.length / 2.0, force: [0.0; 3], weight: 0.0 });
    }
    pub fn add_case(&mut self) {
        let mut c = self.cases.last().cloned().unwrap_or_else(|| CaseIn { name: String::new(), internal_pressure: 0.0, external_pressure: 0.0, closed_ends: true, delta_t: 0.0, load_factors: [0.0, 0.0, -1.0] });
        c.name = format!("Case {}", self.cases.len() + 1);
        self.cases.push(c);
        self.selected_case = self.cases.len() - 1;
    }
    pub fn remove_support(&mut self, i: usize) {
        if i < self.supports.len() {
            self.supports.remove(i);
        }
    }
    pub fn remove_load(&mut self, i: usize) {
        if i < self.loads.len() {
            self.loads.remove(i);
        }
    }
    /// The last case stays: a run has at least one.
    pub fn remove_case(&mut self, i: usize) {
        if self.cases.len() > 1 && i < self.cases.len() {
            self.cases.remove(i);
            self.selected_case = self.selected_case.min(self.cases.len() - 1);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tube() -> Tube {
        Tube { outer_diameter: 2.0, wall: 0.1, e: 10.3e6, alpha: 12.8e-6, weight_density: 0.1 }
    }

    #[test]
    fn the_default_run_is_two_supports_and_one_gravity_case_and_solves() {
        let run = LineRunInputs::new(1000.0, 0.0, true, 0.0);
        let res = pressure_vessel_solver::line_run::analyze(&run.solver_run(tube()), &pressure_vessel_solver::line_run::MaterialClass::Ductile { yield_strength: 70e3, ultimate_strength: 77e3, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.0 }, &run.solver_cases()).unwrap();
        assert_eq!(res.cases.len(), 1);
        assert_eq!(res.cases[0].reactions.len(), 2);
    }

    #[test]
    fn the_applied_total_is_what_the_reactions_balance() {
        let mut run = LineRunInputs::new(0.0, 0.0, false, 0.0);
        run.contents_weight = 0.5;
        run.add_load();
        run.loads[0].force = [0.0, 0.0, -10.0];
        run.loads[0].weight = 4.0;
        let t = tube();
        let w = t.weight_density * t.area();
        let applied = run.applied_total(&run.cases[0], w);
        assert!((applied[2] - (-10.0 - 4.0 - (w + 0.5) * 60.0)).abs() < 1e-9, "{applied:?}");
        let res = pressure_vessel_solver::line_run::analyze(&run.solver_run(t), &pressure_vessel_solver::line_run::MaterialClass::Ductile { yield_strength: 70e3, ultimate_strength: 77e3, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.0 }, &run.solver_cases()).unwrap();
        let sum: f64 = res.cases[0].reactions.iter().map(|r| r.force[2]).sum();
        assert!((sum + applied[2]).abs() < 1e-8 * applied[2].abs(), "{sum} vs {}", applied[2]);
    }

    #[test]
    fn the_last_case_cannot_be_removed_and_the_selection_follows() {
        let mut run = LineRunInputs::new(0.0, 0.0, true, 0.0);
        run.add_case();
        assert_eq!((run.cases.len(), run.selected_case, run.cases[1].name.as_str()), (2, 1, "Case 2"));
        run.remove_case(1);
        assert_eq!((run.cases.len(), run.selected()), (1, 0));
        run.remove_case(0);
        assert_eq!(run.cases.len(), 1);
    }
}
