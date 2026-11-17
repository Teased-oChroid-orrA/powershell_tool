//! The model's side of "Minimum wall by requirement" and the supported line run: the material class and allowables the rows
//! use, the analysis at the wall entered, the walls (inline for the cylinder, a job for the line run), and what the line
//! run's drawing and reactions card need. Every number is `pressure-vessel-solver`'s.
use super::model::PressureVesselModel;
use super::requirements::{self, MaterialKind, Req};
use super::walls::{self, LineWallJob, WallOutcome, WallRow, Walls};
use mechanics_core::materials::Material;
use pressure_vessel_solver::criterion_thickness::{margin_at_geometry, solve_per_criterion, CriterionRequirement, PerCriterionInputs};
use pressure_vessel_solver::line_run::{analyze, analyze_segment, analyze_segment_bounds, Bound, CaseResult, Criterion, LineResult, MaterialClass, Reaction, Tube};

/// The lowest margin of `criterion` over every case of a line result.
fn lowest_margin(res: &LineResult, criterion: Criterion) -> Option<f64> {
    res.cases.iter().flat_map(|c| c.margins.iter()).filter(|m| m.criterion == criterion).map(|m| m.margin).min_by(f64::total_cmp)
}

/// One requirement's factored margin with the segment held pinned and held clamped, and the worse of the two.
#[derive(Clone, Debug, PartialEq)]
pub struct BoundRow {
    pub label: String,
    pub pinned: f64,
    pub clamped: f64,
    pub bound: Bound,
    pub required_ms: f64,
}

impl BoundRow {
    pub fn worse(&self) -> f64 {
        self.pinned.min(self.clamped)
    }
}

/// A load balance: what the supports react against what is applied, per axis.
#[derive(Clone, Debug, PartialEq)]
pub struct Equilibrium {
    pub applied: [f64; 3],
    pub reacted: [f64; 3],
    /// The solver's own equilibrium and backward-error residual for the case.
    pub residual: f64,
}

impl Equilibrium {
    /// The reactions are the applied load reversed, to a part in a million of the largest force.
    pub fn balanced(&self) -> bool {
        let scale = self.applied.iter().chain(&self.reacted).fold(1e-12_f64, |m, v| m.max(v.abs()));
        (0..3).all(|k| (self.applied[k] + self.reacted[k]).abs() <= 1e-6 * scale)
    }
}

impl PressureVesselModel {
    /// The class the criteria follow: the person's choice, else the catalog record's.
    pub fn kind(&self) -> MaterialKind {
        self.kind_override.unwrap_or_else(|| MaterialKind::default_for(self.material()))
    }

    /// The material with the person's allowable overrides applied; every check and the walls read this one.
    pub fn effective_material(&self) -> Material {
        let mut m = *self.material();
        if let Some(sy) = self.allowable_override[0] {
            m.sy_ksi = sy;
        }
        if let Some(ftu) = self.allowable_override[1] {
            m.ftu_ksi = ftu;
        }
        m
    }

    /// Ultimate compression strength (ksi) for a brittle line run: the override, else the ultimate tension. The window says so.
    pub fn compression_ultimate_ksi(&self) -> f64 {
        self.allowable_override[2].unwrap_or(self.effective_material().ftu_ksi)
    }

    /// A new material: its class and allowables are its own, the rows the person has not edited start over.
    pub(super) fn material_changed(&mut self) {
        self.kind_override = None;
        self.allowable_override = [None; 3];
        let kind = self.kind();
        requirements::reseed(&mut self.requirements, kind);
        self.recompute();
    }

    pub fn toggle_kind(&mut self) {
        self.kind_override = Some(self.kind().toggled());
        let kind = self.kind();
        requirements::reseed(&mut self.requirements, kind);
        self.recompute();
    }

    pub fn add_requirement(&mut self) {
        let n = self.requirements.len() + 1;
        let criterion = Req::choices(self.kind()).first().copied().unwrap_or(Req::Buckling);
        let mut r = requirements::Requirement::neutral(&format!("Requirement {n}"), criterion);
        r.edited = true;
        self.requirements.push(r);
        self.recompute();
    }
    pub fn remove_requirement(&mut self, i: usize) {
        if i < self.requirements.len() {
            self.requirements.remove(i);
            self.recompute();
        }
    }
    pub fn cycle_requirement(&mut self, i: usize) {
        let kind = self.kind();
        if let Some(r) = self.requirements.get_mut(i) {
            r.criterion = r.criterion.next(kind);
            r.edited = true;
            self.recompute();
        }
    }

    // ------------------------------------------------------------------ analysis

    /// The tube of the line run: the vessel's diameter and remaining wall, the material's stiffness and expansion.
    fn line_tube(&self, m: &Material) -> Option<Tube> {
        let g = self.geometry.ok()?;
        Some(Tube { outer_diameter: 2.0 * g.outer_radius, wall: g.wall_thickness(), e: m.e_ksi * 1000.0, alpha: m.alpha_u_f * 1e-6, weight_density: self.line.density })
    }

    /// The material class the line run evaluates. Its own safety factors are 1: the requirement rows carry the factors.
    fn line_class(&self, m: &Material) -> MaterialClass {
        match self.kind() {
            MaterialKind::Ductile => MaterialClass::Ductile { yield_strength: m.sy_ksi * 1000.0, ultimate_strength: m.ftu_ksi * 1000.0, yield_factor_of_safety: 1.0, ultimate_factor_of_safety: 1.0 },
            MaterialKind::Brittle => MaterialClass::Brittle { ultimate_tension: m.ftu_ksi * 1000.0, ultimate_compression: self.compression_ultimate_ksi() * 1000.0, factor_of_safety: 1.0 },
            MaterialKind::Composite => MaterialClass::Composite,
        }
    }

    /// Recomputes the line analysis and, for the cylinder, the walls; a line run's walls wait for `set_line_walls`.
    pub(super) fn recompute_walls(&mut self) {
        self.walls_revision += 1;
        let material = self.effective_material();
        if self.line.enabled {
            let (result, bounds) = match self.line_tube(&material) {
                Some(tube) => {
                    let (result, bounds) = self.analyze_line(tube, &material);
                    (Some(result), bounds)
                }
                None => (None, None),
            };
            self.line_result = result;
            self.bounds = bounds;
            self.walls = None;
        } else {
            self.line_result = None;
            self.bounds = None;
            self.walls = self.cylinder_walls(&material);
        }
    }

    /// The line run, the segment, or the segment's two bounds. With the bound, the result is the bound with the lowest
    /// margin (the one the picture, reactions and curve follow) and the bounds come with it.
    fn analyze_line(&self, tube: Tube, m: &Material) -> (Result<LineResult, String>, Option<pressure_vessel_solver::line_run::SegmentBounds>) {
        let (run, class, cases) = (self.line.solver_run(tube), self.line_class(m), self.line.solver_cases());
        let seg = &self.line.segment;
        if !seg.enabled {
            return (analyze(&run, &class, &cases).map_err(|e| e.to_string()), None);
        }
        if !seg.bound {
            return (analyze_segment(&run, self.line.segment_ends(), &class, &cases).map_err(|e| e.to_string()), None);
        }
        if seg.ends.iter().any(|e| e.fixity == super::line::FixityIn::Free) {
            return (Err(super::line::BOUND_NEEDS_HELD_ENDS.into()), None);
        }
        match analyze_segment_bounds(&run, self.line.segment_ends(), &class, &cases) {
            Ok(b) => {
                let lowest = b.worst.iter().min_by(|x, y| x.margin.margin.total_cmp(&y.margin.margin)).map_or(Bound::Pinned, |w| w.bound);
                (Ok(if lowest == Bound::Clamped { b.clamped.clone() } else { b.pinned.clone() }), Some(b))
            }
            Err(e) => (Err(e.to_string()), None),
        }
    }

    /// Which bound `line_result` is (when the bound is on).
    pub fn governing_bound(&self) -> Option<Bound> {
        let b = self.bounds.as_ref()?;
        let Some(Ok(res)) = &self.line_result else { return None };
        Some(if *res == b.clamped && *res != b.pinned { Bound::Clamped } else { Bound::Pinned })
    }

    fn cylinder_walls(&self, material: &Material) -> Option<Result<Walls, String>> {
        let (Ok(g), Ok(pressure)) = (self.geometry, self.pressure) else { return None };
        if self.requirements.is_empty() {
            return None;
        }
        let mut reqs = Vec::new();
        let mut slot = Vec::new();
        for r in &self.requirements {
            match (walls::invalid(r), r.criterion.cylinder()) {
                (None, Some(criterion)) => {
                    slot.push(Some(reqs.len()));
                    reqs.push(CriterionRequirement { label: r.label.clone(), criterion, factor: r.factor, required_ms: r.required_ms });
                }
                _ => slot.push(None),
            }
        }
        let inputs = PerCriterionInputs { inner_radius: g.inner_radius, pressure, material: *material, unsupported_length: Some(self.unsupported_length), requirements: reqs.clone() };
        let solved = if reqs.is_empty() {
            Vec::new()
        } else {
            match solve_per_criterion(&inputs, 200, 1e-9) {
                Ok(s) => s.rows.into_iter().map(|r| (r.requirement, r.outcome)).collect(),
                Err(e) => return Some(Err(format!("{e:?}"))),
            }
        };
        let outcomes = walls::cylinder_outcome(&self.requirements, &slot, &solved);
        let rows = outcomes.into_iter().zip(&slot).map(|(outcome, s)| WallRow { outcome, margin_at_wall: s.and_then(|i| margin_at_geometry(&inputs, &reqs[i], &g)) }).collect();
        Some(Ok(Walls { rows, current_wall: g.wall_thickness() }))
    }

    /// The line run's wall solve as an owned job (and the revision its answer belongs to); `None` when there is nothing to solve.
    pub fn line_wall_job(&self) -> Option<(u64, LineWallJob)> {
        if !self.line.enabled || self.requirements.is_empty() {
            return None;
        }
        let m = self.effective_material();
        let tube = self.line_tube(&m)?;
        Some((self.walls_revision, LineWallJob { runs: self.line.wall_runs(tube), class: self.line_class(&m), cases: self.line.solver_cases(), rows: self.requirements.clone() }))
    }

    /// Takes the answer of a line wall solve; an answer to an older revision is dropped (false).
    pub fn set_line_walls(&mut self, revision: u64, outcomes: Result<Vec<WallOutcome>, String>) -> bool {
        if revision != self.walls_revision {
            return false;
        }
        let Some(g) = self.geometry.ok() else { return false };
        self.walls = Some(outcomes.map(|outcomes| {
            let rows = outcomes.into_iter().zip(&self.requirements).map(|(outcome, r)| WallRow { outcome, margin_at_wall: self.line_margin_at_wall(r) }).collect();
            Walls { rows, current_wall: g.wall_thickness() }
        }));
        true
    }

    /// A row's factored margin at the wall entered: the lowest of its criterion over every case.
    fn line_margin_at_wall(&self, r: &requirements::Requirement) -> Option<f64> {
        let criterion = r.criterion.line().filter(|_| walls::invalid(r).is_none())?;
        let lowest = match &self.bounds {
            Some(b) => b.worst.iter().find(|w| w.margin.criterion == criterion).map(|w| w.margin.margin)?,
            None => {
                let Some(Ok(res)) = &self.line_result else { return None };
                lowest_margin(res, criterion)?
            }
        };
        Some(walls::factored(lowest, r.factor))
    }

    /// With the bound on, each requirement row's factored margin at both bounds, the worse of them and which bound that is.
    pub fn bound_rows(&self) -> Vec<BoundRow> {
        let Some(b) = &self.bounds else { return Vec::new() };
        self.requirements
            .iter()
            .filter_map(|r| {
                let c = r.criterion.line().filter(|_| walls::invalid(r).is_none())?;
                let at = |res: &LineResult| lowest_margin(res, c).map(|m| walls::factored(m, r.factor));
                let (pinned, clamped) = (at(&b.pinned)?, at(&b.clamped)?);
                let bound = if clamped < pinned { Bound::Clamped } else { Bound::Pinned };
                Some(BoundRow { label: r.label.clone(), pinned, clamped, bound, required_ms: r.required_ms })
            })
            .collect()
    }

    // ------------------------------------------------------------------ the line run's picture and reactions

    /// The selected case's result when the line run solved (with the bound: the governing bound's).
    pub fn selected_case(&self) -> Option<&CaseResult> {
        let Some(Ok(res)) = &self.line_result else { return None };
        res.cases.get(self.line.selected())
    }

    /// The margin along the line, `(x, margin over the requirement)` for the selected case: at each station the lowest over
    /// the rows of `factored margin - required margin`, so zero is exactly "met" and below it is short. Without rows, the
    /// class's criteria at factor 1 and required margin 0.
    pub fn margin_curve(&self) -> Vec<[f64; 2]> {
        let Some(case) = self.selected_case() else { return Vec::new() };
        let mut rows: Vec<(Criterion, f64, f64)> = self.requirements.iter().filter(|r| walls::invalid(r).is_none()).filter_map(|r| Some((r.criterion.line()?, r.factor, r.required_ms))).collect();
        if rows.is_empty() {
            rows = case.margins.iter().map(|m| (m.criterion, 1.0, 0.0)).collect();
        }
        let curve = |case: &CaseResult| -> Vec<[f64; 2]> {
            let mut pts: Vec<[f64; 2]> = case
                .stations
                .iter()
                .zip(&case.station_margins)
                .filter_map(|(st, ms)| {
                    let v = rows.iter().filter_map(|(c, f, req)| ms.iter().find(|(k, _)| k == c).map(|(_, m)| walls::factored(*m, *f) - req)).fold(f64::INFINITY, f64::min);
                    v.is_finite().then_some([st.x, v])
                })
                .collect();
            pts.sort_by(|a, b| a[0].total_cmp(&b[0]));
            pts
        };
        let mut pts = curve(case);
        // With the bound the curve is the worse of the two at every station (both runs share one station grid).
        if let Some(b) = &self.bounds {
            let sel = self.line.selected();
            for other in [&b.pinned, &b.clamped].into_iter().filter_map(|r| r.cases.get(sel)) {
                let o = curve(other);
                if o.len() == pts.len() && o.iter().zip(&pts).all(|(a, p)| (a[0] - p[0]).abs() < 1e-9) {
                    for (p, a) in pts.iter_mut().zip(&o) {
                        p[1] = p[1].min(a[1]);
                    }
                }
            }
        }
        pts
    }

    /// The lowest point of [`margin_curve`](Self::margin_curve).
    pub fn worst_point(&self) -> Option<[f64; 2]> {
        self.margin_curve().into_iter().min_by(|a, b| a[1].total_cmp(&b[1]))
    }

    /// What the selected case's supports react against what is applied.
    pub fn equilibrium(&self) -> Option<Equilibrium> {
        let case = self.selected_case()?;
        let tube = self.line_tube(&self.effective_material())?;
        let applied = self.line.applied_total(self.line.cases.get(self.line.selected())?, tube.weight_density * tube.area());
        let mut reacted = [0.0; 3];
        for r in &case.reactions {
            for k in 0..3 {
                reacted[k] += r.force[k];
            }
        }
        Some(Equilibrium { applied, reacted, residual: case.residual })
    }

    pub fn reactions(&self) -> Vec<Reaction> {
        self.selected_case().map(|c| c.reactions.clone()).unwrap_or_default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::toolboxes::pressure_vessel::edit::{Allowable, Edit, LineField, ReqField};
    use crate::toolboxes::pressure_vessel::line::{EndIn, FixityIn};
    use crate::toolboxes::pressure_vessel::model::NumberTarget;
    use crate::toolboxes::pressure_vessel::walls::LineWallJob;

    fn model() -> PressureVesselModel {
        let mut m = PressureVesselModel::default();
        m.custom_materials.clear();
        m
    }

    #[test]
    fn the_cylinder_rows_start_neutral_for_the_materials_class_and_solve() {
        let m = model();
        assert_eq!(m.kind(), MaterialKind::Ductile);
        assert_eq!(m.requirements.len(), 3);
        assert!(m.requirements.iter().all(|r| r.is_neutral() && !r.edited));
        let Some(Ok(w)) = &m.walls else { panic!("walls") };
        assert_eq!(w.rows.len(), 3);
        assert!(w.rows.iter().all(|r| matches!(r.outcome, WallOutcome::Wall { .. }) && r.margin_at_wall.is_some()));
        // Neutral von Mises row is the existing governing-free minimum wall at MS 0 (same solver, the von Mises mode).
        assert!(w.governing().is_some());
    }

    #[test]
    fn a_factor_scales_the_wall_and_the_margin_at_the_wall_is_factored() {
        let mut m = model();
        let base = match &m.walls {
            Some(Ok(w)) => match w.rows[0].outcome {
                WallOutcome::Wall { wall, .. } => wall,
                _ => panic!(),
            },
            _ => panic!(),
        };
        let margin1 = m.walls.as_ref().unwrap().as_ref().unwrap().rows[0].margin_at_wall.unwrap();
        m.requirements[0].factor = 1.5;
        m.requirements[0].edited = true;
        m.recompute();
        let w = m.walls.as_ref().unwrap().as_ref().unwrap();
        let WallOutcome::Wall { wall, .. } = w.rows[0].outcome else { panic!() };
        assert!(wall > base, "{wall} > {base}");
        let want = (1.0 + margin1) / 1.5 - 1.0;
        assert!((w.rows[0].margin_at_wall.unwrap() - want).abs() < 1e-12);
    }

    #[test]
    fn changing_the_material_reseeds_only_unedited_rows_and_resets_overrides() {
        let mut m = model();
        m.requirements[2].factor = 3.0;
        m.requirements[2].edited = true;
        m.allowable_override[0] = Some(10.0);
        let steel = mechanics_core::materials::builtin_catalog().position(|x| x.id == "steel").unwrap();
        m.select_material(steel);
        assert_eq!(m.allowable_override, [None; 3]);
        assert_eq!(m.requirements.iter().filter(|r| r.edited).count(), 1);
        assert_eq!(m.requirements.iter().find(|r| r.edited).unwrap().factor, 3.0);
    }

    #[test]
    fn an_allowable_override_reaches_every_check() {
        let mut m = model();
        let before = m.rows.iter().find(|r| r.name == "Von Mises yield").map(|r| r.margin);
        m.allowable_override[0] = Some(m.material().sy_ksi / 2.0);
        m.recompute();
        let after = m.rows.iter().find(|r| r.name == "Von Mises yield").map(|r| r.margin);
        assert!(after < before, "{after:?} < {before:?}");
        assert_eq!(m.effective_material().sy_ksi, m.material().sy_ksi / 2.0);
    }

    #[test]
    fn a_line_run_solves_with_a_curve_and_balanced_reactions_and_waits_for_its_walls() {
        let mut m = model();
        m.commit_number(NumberTarget::WallThickness, 0.1);
        m.commit_number(NumberTarget::InternalPressure, 500.0);
        m.line.enabled = true;
        m.line.density = 0.1;
        m.recompute();
        assert!(matches!(m.line_result, Some(Ok(_))), "{:?}", m.line_result);
        assert!(m.walls.is_none(), "the walls come from the job");
        let eq = m.equilibrium().unwrap();
        assert!(eq.balanced() && eq.applied[2] < 0.0, "{eq:?}");
        let curve = m.margin_curve();
        assert!(curve.len() > 4 && curve.windows(2).all(|w| w[0][0] <= w[1][0]));
        let worst = m.worst_point().unwrap();
        assert!(curve.iter().all(|p| p[1] >= worst[1]));
        let (rev, job) = m.line_wall_job().unwrap();
        let out = job.run().unwrap();
        assert_eq!(out.len(), 3);
        assert!(m.set_line_walls(rev, Ok(out)));
        let w = m.walls.as_ref().unwrap().as_ref().unwrap();
        assert!(matches!(w.rows[0].outcome, WallOutcome::Wall { .. }));
        // A stale answer is dropped.
        m.recompute();
        assert!(!m.set_line_walls(rev, Ok(Vec::new())));
    }

    #[test]
    fn the_line_runs_minimum_wall_meets_its_requirement_and_a_thinner_wall_fails() {
        let mut m = model();
        m.commit_number(NumberTarget::WallThickness, 0.2);
        m.commit_number(NumberTarget::InternalPressure, 800.0);
        m.line.enabled = true;
        m.requirements[0].factor = 1.5;
        m.requirements[0].required_ms = 0.1;
        m.recompute();
        let (rev, job) = m.line_wall_job().unwrap();
        let out = job.run().unwrap();
        let WallOutcome::Wall { wall, .. } = out[0] else { panic!("{:?}", out[0]) };
        assert!(m.set_line_walls(rev, Ok(out)));
        // At that wall the row's factored margin is the required one; thinner, it is short.
        for (w, ok) in [(wall * 1.0001, true), (wall * 0.999, false)] {
            let od = m.outer_diameter;
            let mut t = model();
            t.commit_number(NumberTarget::InternalPressure, 800.0);
            t.commit_number(NumberTarget::WallThickness, w);
            t.commit_number(NumberTarget::OuterDiameter, od);
            t.line.enabled = true;
            t.requirements[0].factor = 1.5;
            t.requirements[0].required_ms = 0.1;
            t.recompute();
            let got = t.line_margin_at_wall(&t.requirements[0].clone()).unwrap();
            assert_eq!(got >= 0.1, ok, "wall {w}: margin {got}");
        }
    }

    #[test]
    fn rows_a_solver_cannot_take_are_named_not_defaulted() {
        let mut m = model();
        m.requirements[0].factor = 0.0;
        m.recompute();
        let w = m.walls.as_ref().unwrap().as_ref().unwrap();
        assert!(matches!(&w.rows[0].outcome, WallOutcome::NotEvaluable(why) if why.contains("factor")));
        assert!(matches!(w.rows[1].outcome, WallOutcome::Wall { .. }), "the other rows still solve");
    }

    // ------------------------------------------------------------------ rows follow the material (class and allowables)

    #[test]
    fn rows_default_from_the_materials_class_and_catalog_allowables_and_survive_edits() {
        use mechanics_core::materials::builtin_catalog;
        let mut m = model();
        m.line.enabled = true;
        m.recompute();
        let MaterialClass::Ductile { yield_strength, ultimate_strength, .. } = m.line_class(&m.effective_material()) else { panic!("al7075 is ductile") };
        assert_eq!((yield_strength, ultimate_strength), (m.material().sy_ksi * 1000.0, m.material().ftu_ksi * 1000.0), "the solve reads the catalog's allowables");
        assert!(m.requirements.iter().all(|r| Req::choices(MaterialKind::Ductile).contains(&r.criterion) && r.is_neutral()), "neutral factor 1.0 / required MS 0.0 until entered");
        // An edited row sticks through a change of material; the unedited ones follow the new material's class and allowables.
        m.apply_edit(Edit::Req(0, ReqField::Factor), "1.4").unwrap();
        let other = builtin_catalog().position(|x| x.id == "steel").unwrap();
        m.select_material(other);
        assert_eq!(m.requirements.iter().find(|r| r.edited).map(|r| r.factor), Some(1.4));
        let MaterialClass::Ductile { yield_strength, .. } = m.line_class(&m.effective_material()) else { panic!() };
        assert_eq!(yield_strength, m.material().sy_ksi * 1000.0, "the new material's allowable, not the old one's");
        // An allowable the person types wins until the material changes again.
        m.apply_edit(Edit::Allowable(Allowable::Yield), "50").unwrap();
        let MaterialClass::Ductile { yield_strength, .. } = m.line_class(&m.effective_material()) else { panic!() };
        assert_eq!(yield_strength, 50_000.0);
    }

    // ------------------------------------------------------------------ segment mode

    /// A held-held segment with a valve at midspan: the bending is what separates the bounds.
    fn segment_model() -> PressureVesselModel {
        let mut m = model();
        m.commit_number(NumberTarget::WallThickness, 0.1);
        m.commit_number(NumberTarget::InternalPressure, 100.0);
        m.line.enabled = true;
        m.line.density = 0.1;
        m.line.cases[0].load_factors = [0.0, 0.0, -1.0];
        m.toggle_segment();
        m.add_load();
        m.apply_edit(Edit::Line(LineField::LoadWeight(0)), "200").unwrap();
        m
    }

    #[test]
    fn the_stated_ends_are_solved_and_the_supports_at_the_end_stations_are_superseded() {
        let mut m = segment_model();
        // Default supports sit at x = 0 and x = length: pinned-pinned ends, same as the whole-run solve.
        let whole = {
            let mut w = segment_model();
            w.toggle_segment();
            w.selected_case().unwrap().clone()
        };
        let seg = m.selected_case().unwrap().clone();
        assert_eq!(seg.margins, whole.margins);
        // Clamp and valve: clamped root, free tip. One support (the clamp); the stored supports at the ends are not solved.
        m.cycle_fixity(0); // Pinned -> Spring
        m.cycle_fixity(0); // -> Clamped
        for _ in 0..3 {
            m.cycle_fixity(1); // Pinned -> Spring -> Clamped -> Free
        }
        assert_eq!(m.line.segment.ends[1].fixity, FixityIn::Free);
        assert_eq!(m.line.supports.len(), 2, "the list keeps them");
        let c = m.selected_case().unwrap();
        assert_eq!(c.reactions.len(), 1, "only the clamp reacts");
        assert!(c.reactions[0].moment[1].abs() > 0.0);
    }

    #[test]
    fn a_valve_at_a_free_end_loads_the_clamp() {
        let mut m = segment_model();
        m.line.loads.clear();
        m.line.segment.ends = [EndIn { fixity: FixityIn::Clamped, ..EndIn::default() }, EndIn { fixity: FixityIn::Free, ..EndIn::default() }];
        m.line.density = 0.0;
        m.line.cases[0].load_factors = [0.0, 0.0, -1.0];
        m.add_end_load(1);
        assert_eq!((m.line.loads[0].x, m.line.loads[0].label.as_str()), (m.line.length, "Valve 1"));
        m.apply_edit(Edit::Line(LineField::LoadWeight(0)), "30").unwrap();
        let r = &m.selected_case().unwrap().reactions[0];
        // P = 30 lbf at L = 60 in from the clamp: root moment P L, reaction P.
        let moment = (r.moment[0].powi(2) + r.moment[1].powi(2) + r.moment[2].powi(2)).sqrt();
        assert!((moment - 30.0 * 60.0).abs() < 1e-6 * 1800.0, "{moment}");
        assert!((r.force[2] - 30.0).abs() < 1e-8, "{:?}", r.force);
    }

    #[test]
    fn the_bound_gives_both_results_and_the_worse_margin_per_requirement() {
        let mut m = segment_model();
        m.toggle_bound();
        assert!(m.bounds.is_some(), "both held: bounds solved");
        // The pressure dominates this tube, so the bounds differ in the fourth digit and the criteria disagree on which is worse:
        // the governing bound is the one of the lowest worst margin.
        let b = m.bounds.as_ref().unwrap();
        let lowest = b.worst.iter().min_by(|x, y| x.margin.margin.total_cmp(&y.margin.margin)).unwrap();
        assert_eq!(m.governing_bound(), Some(lowest.bound));
        let rows = m.bound_rows();
        assert_eq!(rows.len(), 3);
        for r in &rows {
            assert!(r.pinned != r.clamped, "{r:?}");
            assert_eq!(r.worse(), r.pinned.min(r.clamped));
            assert_eq!(r.bound, if r.clamped < r.pinned { Bound::Clamped } else { Bound::Pinned });
        }
        // The per-row margin at the wall is the worse bound's, the solver's own worst over cases and bounds.
        m.requirements[0].factor = 1.25;
        m.recompute();
        let rows = m.bound_rows();
        let (rev, job) = m.line_wall_job().unwrap();
        assert_eq!(job.runs.as_ref().unwrap().len(), 2, "one run per bound");
        assert!(m.set_line_walls(rev, job.run()));
        let w = m.walls.as_ref().unwrap().as_ref().unwrap();
        assert!((w.rows[0].margin_at_wall.unwrap() - rows[0].worse()).abs() < 1e-12);
        assert_eq!(m.bounds.as_ref().unwrap().worst.len(), 4, "one per criterion of the class");
    }

    #[test]
    fn the_curve_is_the_worse_of_both_bounds_at_every_station() {
        let mut m = segment_model();
        let stated = m.margin_curve();
        m.toggle_bound();
        let bound = m.margin_curve();
        assert_eq!(bound.len(), stated.len());
        assert!(bound.iter().zip(&stated).all(|(b, s)| b[1] <= s[1] + 1e-12), "a bound is never better than the pinned ends");
    }

    #[test]
    fn the_wall_of_the_bound_is_the_thicker_of_the_two_runs() {
        let mut m = segment_model();
        m.toggle_bound();
        m.apply_edit(Edit::Req(0, ReqField::RequiredMs), "0.5").unwrap();
        let wall = |job: LineWallJob| match job.run().unwrap()[0] {
            WallOutcome::Wall { wall, .. } => wall,
            ref o => panic!("{o:?}"),
        };
        let (_, both) = m.line_wall_job().unwrap();
        let runs = both.runs.clone().unwrap();
        let one = |i: usize| {
            let (_, mut j) = m.line_wall_job().unwrap();
            j.runs = Ok(vec![runs[i].clone()]);
            wall(j)
        };
        let (p, c) = (one(0), one(1));
        assert!(p != c, "the bounds need different walls: {p} {c}");
        assert_eq!(wall(both), p.max(c));
    }

    #[test]
    fn a_free_end_has_no_bound_and_says_so() {
        let mut m = segment_model();
        m.line.segment.ends[1].fixity = FixityIn::Free;
        m.toggle_bound();
        assert!(m.bounds.is_none());
        assert_eq!(m.line_result, Some(Err(crate::toolboxes::pressure_vessel::line::BOUND_NEEDS_HELD_ENDS.to_string())));
        let (rev, job) = m.line_wall_job().unwrap();
        assert!(job.runs.is_err());
        let out = job.run();
        assert_eq!(out, Err(crate::toolboxes::pressure_vessel::line::BOUND_NEEDS_HELD_ENDS.to_string()));
        assert!(m.set_line_walls(rev, out));
        // The bound off again: pinned-free is a mechanism, which the solver says (no silent fallback).
        m.toggle_bound();
        assert!(matches!(m.line_result, Some(Err(_))));
    }

    #[test]
    fn a_spring_end_needs_a_stiffness_and_zero_acts_as_pinned() {
        let mut m = segment_model();
        m.cycle_fixity(0);
        assert_eq!(m.line.segment.ends[0].fixity, FixityIn::Spring);
        let pinned = m.selected_case().unwrap().margins.clone();
        assert_eq!(pinned, segment_model().selected_case().unwrap().margins, "stiffness 0 is the pinned end");
        assert!(m.apply_edit(Edit::Line(LineField::EndSpring(0)), "-5").is_err());
        m.apply_edit(Edit::Line(LineField::EndSpring(0)), "1e9").unwrap();
        assert_ne!(m.selected_case().unwrap().margins, pinned, "a stiff clip moves the margins");
    }

    #[test]
    fn a_free_end_holds_no_axial_anchor() {
        let mut m = segment_model();
        assert!(m.line.segment.ends[0].axial_anchor, "the left end anchors by default, as the whole run's first support does");
        m.toggle_end_anchor(0);
        assert!(!m.line.segment.ends[0].axial_anchor);
        m.toggle_end_anchor(0);
        for _ in 0..2 {
            m.cycle_fixity(0); // Pinned -> Spring -> Clamped
        }
        assert_eq!(m.line.segment.ends[0].fixity, FixityIn::Clamped);
        m.cycle_fixity(0);
        assert_eq!((m.line.segment.ends[0].fixity, m.line.segment.ends[0].axial_anchor), (FixityIn::Free, false));
        m.toggle_end_anchor(0);
        assert!(!m.line.segment.ends[0].axial_anchor, "nothing to anchor");
    }
}
