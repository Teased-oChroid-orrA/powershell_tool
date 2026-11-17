//! Typed edits of the fields that are not a fixed row of the form: the requirement rows' label, factor and required margin,
//! the allowable overrides, and the line run's lists (supports, loads, load cases). One address (`Edit`), one way to read
//! the text an edit starts from and one way to apply what was typed, so the window's prompt and the undo history treat
//! them like any other number.
use super::model::{NumberTarget, PressureVesselModel};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ReqField {
    Label,
    Factor,
    RequiredMs,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Allowable {
    Yield,
    Ultimate,
    UltimateCompression,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LineField {
    Length,
    Contents,
    Density,
    SupportX(usize),
    LoadX(usize),
    /// Force of a load along x (0), y (1) or z (2).
    LoadForce(usize, usize),
    LoadWeight(usize),
    CaseInternal(usize),
    CaseExternal(usize),
    CaseDeltaT(usize),
    /// Load factor of a case along x (0), y (1) or z (2).
    CaseFactor(usize, usize),
    /// Rotational stiffness of a segment end's clip (lbf-in per radian), end 0 left and 1 right.
    EndSpring(usize),
}

/// What a typed edit changes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Edit {
    Number(NumberTarget),
    Req(usize, ReqField),
    Allowable(Allowable),
    Line(LineField),
    CaseName(usize),
    LoadLabel(usize),
}

impl Edit {
    /// Typed as text, not as a number.
    pub fn is_text(self) -> bool {
        matches!(self, Edit::Req(_, ReqField::Label) | Edit::CaseName(_) | Edit::LoadLabel(_))
    }

    /// The name a status message uses.
    pub fn label(self) -> String {
        match self {
            Edit::Number(t) => t.label().to_string(),
            Edit::Req(_, ReqField::Label) => "Requirement label".into(),
            Edit::Req(_, ReqField::Factor) => "Factor".into(),
            Edit::Req(_, ReqField::RequiredMs) => "Required MS".into(),
            Edit::Allowable(Allowable::Yield) => "Yield strength".into(),
            Edit::Allowable(Allowable::Ultimate) => "Ultimate strength".into(),
            Edit::Allowable(Allowable::UltimateCompression) => "Ultimate compression".into(),
            Edit::Line(LineField::Length) => "Run length".into(),
            Edit::Line(LineField::Contents) => "Contents weight".into(),
            Edit::Line(LineField::Density) => "Tube weight density".into(),
            Edit::Line(LineField::SupportX(_)) => "Support position".into(),
            Edit::Line(LineField::LoadX(_)) => "Load position".into(),
            Edit::Line(LineField::LoadForce(_, k)) => format!("Load force {}", ["x", "y", "z"][k.min(2)]),
            Edit::Line(LineField::LoadWeight(_)) => "Load weight".into(),
            Edit::Line(LineField::CaseInternal(_)) => "Internal pressure".into(),
            Edit::Line(LineField::CaseExternal(_)) => "External pressure".into(),
            Edit::Line(LineField::CaseDeltaT(_)) => "Temperature differential".into(),
            Edit::Line(LineField::CaseFactor(_, k)) => format!("Load factor {}", ["x", "y", "z"][k.min(2)]),
            Edit::Line(LineField::EndSpring(_)) => "Clip stiffness".into(),
            Edit::CaseName(_) => "Case name".into(),
            Edit::LoadLabel(_) => "Load label".into(),
        }
    }
}

fn trimmed(v: f64) -> String {
    super::model::format_for_edit(v)
}

impl PressureVesselModel {
    /// The current value as the edit buffer starts: a number trimmed of trailing zeros, or the text itself. `None` when the
    /// address no longer exists (a row was removed).
    pub fn edit_text(&self, e: Edit) -> Option<String> {
        Some(match e {
            Edit::Number(t) => trimmed(self.number_value(t)),
            Edit::Req(i, f) => {
                let r = self.requirements.get(i)?;
                match f {
                    ReqField::Label => r.label.clone(),
                    ReqField::Factor => trimmed(r.factor),
                    ReqField::RequiredMs => trimmed(r.required_ms),
                }
            }
            Edit::Allowable(a) => {
                let m = self.effective_material();
                trimmed(match a {
                    Allowable::Yield => m.sy_ksi,
                    Allowable::Ultimate => m.ftu_ksi,
                    Allowable::UltimateCompression => self.compression_ultimate_ksi(),
                })
            }
            Edit::Line(f) => trimmed(self.line_value(f)?),
            Edit::CaseName(i) => self.line.cases.get(i)?.name.clone(),
            Edit::LoadLabel(i) => self.line.loads.get(i)?.label.clone(),
        })
    }

    fn line_value(&self, f: LineField) -> Option<f64> {
        let l = &self.line;
        Some(match f {
            LineField::Length => l.length,
            LineField::Contents => l.contents_weight,
            LineField::Density => l.density,
            LineField::SupportX(i) => l.supports.get(i)?.x,
            LineField::LoadX(i) => l.loads.get(i)?.x,
            LineField::LoadForce(i, k) => *l.loads.get(i)?.force.get(k)?,
            LineField::LoadWeight(i) => l.loads.get(i)?.weight,
            LineField::CaseInternal(i) => l.cases.get(i)?.internal_pressure,
            LineField::CaseExternal(i) => l.cases.get(i)?.external_pressure,
            LineField::CaseDeltaT(i) => l.cases.get(i)?.delta_t,
            LineField::CaseFactor(i, k) => *l.cases.get(i)?.load_factors.get(k)?,
            LineField::EndSpring(i) => l.segment.ends.get(i)?.spring,
        })
    }

    /// Applies typed text. `Ok(true)` changed something, `Ok(false)` typed what was already there, `Err` says why nothing
    /// changed. Every applied edit recomputes at once.
    pub fn apply_edit(&mut self, e: Edit, text: &str) -> Result<bool, String> {
        let text = text.trim();
        let current = self.edit_text(e).ok_or_else(|| format!("{} no longer exists.", e.label()))?;
        if e.is_text() {
            if text.is_empty() {
                return Err(format!("{} cannot be empty.", e.label()));
            }
            if text == current {
                return Ok(false);
            }
            match e {
                Edit::Req(i, _) => {
                    self.requirements[i].label = text.to_string();
                    self.requirements[i].edited = true;
                }
                Edit::CaseName(i) => self.line.cases[i].name = text.to_string(),
                Edit::LoadLabel(i) => self.line.loads[i].label = text.to_string(),
                _ => unreachable!("text edits"),
            }
            return Ok(true);
        }
        let v: f64 = text.parse().ok().filter(|v: &f64| v.is_finite()).ok_or_else(|| format!("{} not changed: {text:?} is not a number.", e.label()))?;
        if let Edit::Number(t) = e {
            let changed = v != self.number_value(t);
            if changed {
                self.commit_number(t, v);
            }
            return Ok(changed);
        }
        if v == self.line_or_req_value(e) {
            return Ok(false);
        }
        self.validate_edit(e, v)?;
        match e {
            Edit::Req(i, ReqField::Factor) => {
                self.requirements[i].factor = v;
                self.requirements[i].edited = true;
            }
            Edit::Req(i, ReqField::RequiredMs) => {
                self.requirements[i].required_ms = v;
                self.requirements[i].edited = true;
            }
            Edit::Allowable(a) => {
                let catalog = self.material();
                let (slot, base) = match a {
                    Allowable::Yield => (0, catalog.sy_ksi),
                    Allowable::Ultimate => (1, catalog.ftu_ksi),
                    Allowable::UltimateCompression => (2, f64::NAN),
                };
                self.allowable_override[slot] = (v != base).then_some(v);
            }
            Edit::Line(f) => self.set_line(f, v),
            _ => unreachable!("numeric edits"),
        }
        self.recompute();
        Ok(true)
    }

    fn line_or_req_value(&self, e: Edit) -> f64 {
        match e {
            Edit::Req(i, ReqField::Factor) => self.requirements[i].factor,
            Edit::Req(i, ReqField::RequiredMs) => self.requirements[i].required_ms,
            Edit::Allowable(_) => self.edit_text(e).and_then(|t| t.parse().ok()).unwrap_or(f64::NAN),
            Edit::Line(f) => self.line_value(f).unwrap_or(f64::NAN),
            _ => f64::NAN,
        }
    }

    /// The limits a typed value must keep: what the solver would refuse is refused here, with the reason.
    fn validate_edit(&self, e: Edit, v: f64) -> Result<(), String> {
        let positive = |what: &str| if v > 0.0 { Ok(()) } else { Err(format!("{what} must be greater than zero.")) };
        let nonneg = |what: &str| if v >= 0.0 { Ok(()) } else { Err(format!("{what} must be zero or more.")) };
        match e {
            Edit::Req(_, ReqField::Factor) => positive("Factor"),
            Edit::Allowable(_) => positive(&e.label()),
            Edit::Line(LineField::Length) => positive("Run length"),
            Edit::Line(LineField::Contents | LineField::Density | LineField::EndSpring(_)) => nonneg(&e.label()),
            Edit::Line(LineField::CaseInternal(_) | LineField::CaseExternal(_)) => nonneg(&e.label()),
            Edit::Line(LineField::SupportX(_) | LineField::LoadX(_)) => {
                if (0.0..=self.line.length).contains(&v) { Ok(()) } else { Err(format!("{} must lie within the run, 0 to {} in.", e.label(), trimmed(self.line.length))) }
            }
            _ => Ok(()),
        }
    }

    fn set_line(&mut self, f: LineField, v: f64) {
        let l = &mut self.line;
        match f {
            LineField::Length => l.length = v,
            LineField::Contents => l.contents_weight = v,
            LineField::Density => l.density = v,
            LineField::SupportX(i) => l.supports[i].x = v,
            LineField::LoadX(i) => l.loads[i].x = v,
            LineField::LoadForce(i, k) => l.loads[i].force[k] = v,
            LineField::LoadWeight(i) => l.loads[i].weight = v,
            LineField::CaseInternal(i) => l.cases[i].internal_pressure = v,
            LineField::CaseExternal(i) => l.cases[i].external_pressure = v,
            LineField::CaseDeltaT(i) => l.cases[i].delta_t = v,
            LineField::CaseFactor(i, k) => l.cases[i].load_factors[k] = v,
            LineField::EndSpring(i) => l.segment.ends[i].spring = v,
        }
    }

    // ------------------------------------------------------------------ switches and list edits (one undo step each)

    pub fn toggle_line_run(&mut self) {
        self.line.enabled = !self.line.enabled;
        self.recompute();
    }
    pub fn toggle_support(&mut self, i: usize, rotation: bool) {
        if let Some(s) = self.line.supports.get_mut(i) {
            if rotation {
                s.rotation_fixed = !s.rotation_fixed;
            } else {
                s.axial_anchor = !s.axial_anchor;
            }
            self.recompute();
        }
    }
    /// Segment mode on or off (the run's supports at its two stations are superseded while it is on).
    pub fn toggle_segment(&mut self) {
        self.line.segment.enabled = !self.line.segment.enabled;
        self.recompute();
    }
    /// The bound: run the segment pinned and clamped and take the worse margin.
    pub fn toggle_bound(&mut self) {
        self.line.segment.bound = !self.line.segment.bound;
        self.recompute();
    }
    /// The next condition of an end; a free end holds nothing, so its axial anchor goes.
    pub fn cycle_fixity(&mut self, end: usize) {
        if let Some(e) = self.line.segment.ends.get_mut(end) {
            e.fixity = e.fixity.next();
            if e.fixity == super::line::FixityIn::Free {
                e.axial_anchor = false;
            }
            self.recompute();
        }
    }
    pub fn toggle_end_anchor(&mut self, end: usize) {
        if let Some(e) = self.line.segment.ends.get_mut(end).filter(|e| e.fixity != super::line::FixityIn::Free) {
            e.axial_anchor = !e.axial_anchor;
            self.recompute();
        }
    }
    pub fn add_end_load(&mut self, end: usize) {
        self.line.add_end_load(end);
        self.recompute();
    }
    pub fn toggle_case_ends(&mut self, i: usize) {
        if let Some(c) = self.line.cases.get_mut(i) {
            c.closed_ends = !c.closed_ends;
            self.recompute();
        }
    }
    pub fn select_case(&mut self, i: usize) {
        if i < self.line.cases.len() {
            self.line.selected_case = i;
            // Only the picture follows the case; the analysis is the same.
        }
    }
    pub fn add_support(&mut self) {
        self.line.add_support();
        self.recompute();
    }
    pub fn add_load(&mut self) {
        self.line.add_load();
        self.recompute();
    }
    pub fn add_case(&mut self) {
        self.line.add_case();
        self.recompute();
    }
    pub fn remove_support(&mut self, i: usize) {
        self.line.remove_support(i);
        self.recompute();
    }
    pub fn remove_load(&mut self, i: usize) {
        self.line.remove_load(i);
        self.recompute();
    }
    pub fn remove_case(&mut self, i: usize) {
        self.line.remove_case(i);
        self.recompute();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn model() -> PressureVesselModel {
        let mut m = PressureVesselModel::default();
        m.custom_materials.clear();
        m
    }

    #[test]
    fn a_requirement_edit_marks_the_row_edited_and_recomputes() {
        let mut m = model();
        assert_eq!(m.apply_edit(Edit::Req(0, ReqField::Factor), "1.5"), Ok(true));
        assert!(m.requirements[0].edited && m.requirements[0].factor == 1.5);
        assert_eq!(m.apply_edit(Edit::Req(0, ReqField::Factor), "1.5"), Ok(false), "the same value is not an edit");
        assert!(m.apply_edit(Edit::Req(0, ReqField::Factor), "0").unwrap_err().contains("greater than zero"));
        assert!(m.apply_edit(Edit::Req(0, ReqField::Factor), "abc").unwrap_err().contains("not a number"));
        assert_eq!(m.apply_edit(Edit::Req(1, ReqField::Label), "Proof"), Ok(true));
        assert_eq!(m.requirements[1].label, "Proof");
        assert!(m.apply_edit(Edit::Req(1, ReqField::Label), "  ").is_err());
        assert!(m.apply_edit(Edit::Req(9, ReqField::Factor), "2").unwrap_err().contains("no longer exists"));
    }

    #[test]
    fn an_allowable_override_is_the_catalog_value_again_when_typed_back() {
        let mut m = model();
        let sy = m.material().sy_ksi;
        assert_eq!(m.apply_edit(Edit::Allowable(Allowable::Yield), "50"), Ok(true));
        assert_eq!(m.allowable_override[0], Some(50.0));
        assert_eq!(m.edit_text(Edit::Allowable(Allowable::Yield)).unwrap(), "50");
        assert_eq!(m.apply_edit(Edit::Allowable(Allowable::Yield), &sy.to_string()), Ok(true));
        assert_eq!(m.allowable_override[0], None);
        assert_eq!(m.apply_edit(Edit::Allowable(Allowable::Yield), "-1").is_err(), true);
    }

    #[test]
    fn line_lists_validate_positions_and_reach_the_solver() {
        let mut m = model();
        m.toggle_line_run();
        assert_eq!(m.apply_edit(Edit::Line(LineField::SupportX(1)), "40"), Ok(true));
        assert!(m.apply_edit(Edit::Line(LineField::SupportX(1)), "61").unwrap_err().contains("within the run"));
        m.add_load();
        assert_eq!(m.apply_edit(Edit::LoadLabel(0), "Valve"), Ok(true));
        assert_eq!(m.apply_edit(Edit::Line(LineField::LoadForce(0, 2)), "-25"), Ok(true));
        assert_eq!(m.line.loads[0].force, [0.0, 0.0, -25.0]);
        assert_eq!(m.apply_edit(Edit::CaseName(0), "Limit"), Ok(true));
        assert!(matches!(m.line_result, Some(Ok(_))), "{:?}", m.line_result);
        m.remove_load(0);
        assert!(m.line.loads.is_empty());
        assert!(m.apply_edit(Edit::LoadLabel(0), "x").is_err());
    }
}
