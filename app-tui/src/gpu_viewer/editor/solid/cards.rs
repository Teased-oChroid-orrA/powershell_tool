//! Value cards of the solid editor: the small forms that finish a constraint, a load or a load case. A menu entry opens
//! one (`SolidAction::OpenCard`), the window shows it beside the face (`Authoring::value_card`), `Apply` writes the
//! model (one undo step) and `Cancel` leaves it untouched. Nothing is added until `Apply`.
use super::*;
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::EditorAction,
    ui::{CardRow, ValueCard},
};
use fea_problem::{Load, Support};

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

/// Which form a card is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CardForm {
    /// Hold directions of the selected surfaces (a displacement of zero or a prescribed value).
    Hold,
    Pressure,
    /// Total force components on the selected surfaces.
    Force,
    /// Traction (force per area) components.
    Traction,
    /// Pin load on the selected hole.
    Bearing,
    /// Body force per volume (gravity), one for the model.
    Gravity,
    /// Change the directions of support `.0` (its surface keeps the name).
    EditHold(usize),
    /// Change the numbers of load `.0`.
    EditLoad(usize),
    /// Name, supports kept and load factors of load case `.0`.
    Case(usize),
}

#[derive(Clone, Debug)]
pub(super) struct CardState {
    form: CardForm,
    /// The surfaces the card was opened for.
    names: Vec<String>,
    toggles: Vec<bool>,
    fields: Vec<String>,
}

const AXES: [&str; 3] = ["x", "y", "z"];

fn number(text: &str, what: &str) -> Result<f64, String> {
    let v: f64 = text.trim().parse().map_err(|_| format!("{what}: '{}' is not a number", text.trim()))?;
    if v.is_finite() {
        Ok(v)
    } else {
        Err(format!("{what} must be finite"))
    }
}

impl SolidSession {
    /// Degrees of freedom (axes) the model's displacements have.
    fn card_dim(&self) -> usize {
        self.problem.analysis_dim()
    }

    pub(super) fn open_card(&mut self, form: CardForm) {
        let names = self.all_selected();
        let dim = self.card_dim();
        let needs_surface = matches!(form, CardForm::Hold | CardForm::Pressure | CardForm::Force | CardForm::Traction | CardForm::Bearing);
        if needs_surface && names.is_empty() {
            self.status = "Pick a surface in the viewport or the tree first.".into();
            return;
        }
        if form == CardForm::Bearing && !(names.len() == 1 && names[0].starts_with("hole")) {
            self.status = "A pin load acts on one hole: pick a hole.".into();
            return;
        }
        let edit_support = if let CardForm::EditHold(i) = form { self.problem.supports.get(i).cloned() } else { None };
        let edit_load = if let CardForm::EditLoad(i) = form { self.problem.loads.get(i).cloned() } else { None };
        if matches!(form, CardForm::EditHold(_)) && edit_support.is_none() || matches!(form, CardForm::EditLoad(_)) && edit_load.is_none() {
            self.status = "That entry no longer exists.".into();
            return;
        }
        let comps = |v: &[f64]| v.iter().take(dim).map(|x| format!("{x}")).collect::<Vec<_>>();
        let (toggles, fields) = match form {
            CardForm::Hold => (vec![true; dim], vec!["0".to_string(); dim]),
            CardForm::EditHold(_) => {
                let c = edit_support.as_ref().map(Support::comps).unwrap_or([None; 3]);
                ((0..dim).map(|a| c[a].is_some()).collect(), (0..dim).map(|a| format!("{}", c[a].unwrap_or(0.0))).collect())
            }
            CardForm::EditLoad(_) => (Vec::new(), edit_load.as_ref().map(|l| l.params(dim).iter().map(|(_, v)| format!("{v}")).collect()).unwrap_or_default()),
            CardForm::Pressure => (Vec::new(), vec!["1000".into()]),
            CardForm::Force => (Vec::new(), comps(&[100.0, 0.0, 0.0])),
            CardForm::Traction => (Vec::new(), comps(&[1000.0, 0.0, 0.0])),
            CardForm::Bearing => (Vec::new(), vec!["1000".into(), "0".into()]),
            CardForm::Gravity => {
                let current = self.problem.loads.iter().find_map(|l| if let Load::Body { bx, by, bz } = l { Some([*bx, *by, *bz]) } else { None });
                // Weight per volume pulling down: the density is stored as weight density / 386.09 (inch units).
                let down = if self.problem.material.density > 0.0 { self.problem.material.density * 386.089 } else { 0.1 };
                (Vec::new(), comps(&current.unwrap_or([0.0, -down, 0.0])))
            }
            CardForm::Case(i) => {
                let Some(case) = self.problem.cases.get(i) else {
                    self.status = "That load case no longer exists.".into();
                    return;
                };
                let toggles = (0..self.problem.supports.len()).map(|s| case.support_on(s)).collect();
                let mut fields = vec![case.name.clone()];
                fields.extend((0..self.problem.loads.len()).map(|l| format!("{}", case.factor(l))));
                (toggles, fields)
            }
        };
        let names = match (&edit_support, &edit_load) {
            (Some(s), _) => s.edge().map(|e| vec![e.to_string()]).unwrap_or_default(),
            (_, Some(l)) => l.edge().map(|e| vec![e.to_string()]).unwrap_or_default(),
            _ => names,
        };
        self.card = Some(CardState { form, names, toggles, fields });
        self.status.clear();
        self.touch();
    }

    pub(super) fn card_toggle(&mut self, i: usize) {
        if let Some(c) = self.card.as_mut() {
            if let Some(t) = c.toggles.get_mut(i) {
                *t = !*t;
                self.touch();
            }
        }
    }

    pub(super) fn card_set_field(&mut self, i: usize, text: String) {
        if let Some(c) = self.card.as_mut() {
            if let Some(f) = c.fields.get_mut(i) {
                *f = text;
                self.touch();
            }
        }
    }

    pub(super) fn card_cancel(&mut self) {
        if self.card.take().is_some() {
            self.status.clear();
            self.touch();
        }
    }

    /// The card's contents for the window.
    pub(super) fn card_view(&self) -> Option<ValueCard> {
        let c = self.card.as_ref()?;
        let dim = self.card_dim();
        let target = match c.names.len() {
            0 => String::new(),
            1 => format!(" on '{}'", c.names[0]),
            n => format!(" on {n} surfaces"),
        };
        let field = |label: String, index: usize, unit: &str| CardRow::Field { label, text: c.fields[index].clone(), index, unit: unit.to_string() };
        let cancel = ui(SolidAction::CardCancel);
        let apply = ui(SolidAction::CardApply);
        let mut rows = Vec::new();
        let (title, tag, apply_label) = match c.form {
            CardForm::Hold | CardForm::EditHold(_) => {
                for a in 0..dim {
                    rows.push(CardRow::Toggle { label: format!("Hold {}", AXES[a]), on: c.toggles[a], action: ui(SolidAction::CardToggle(a)) });
                    if c.toggles[a] {
                        rows.push(field(format!("{} displacement", AXES[a]), a, ""));
                    }
                }
                if dim == 3 {
                    rows.push(CardRow::Text("A solid element has no rotations: holding all three is a fixed face.".into()));
                } else {
                    rows.push(CardRow::Text("Hold one direction and the face slides in the other (a roller).".into()));
                }
                let held: String = (0..dim).filter(|a| c.toggles[*a]).map(|a| char::from(b'1' + a as u8)).collect();
                (format!("{}{target}", if matches!(c.form, CardForm::EditHold(_)) { "Change directions" } else { "Hold directions" }), format!("SPC1 {}", if held.is_empty() { "-".to_string() } else { held }), "Apply".to_string())
            }
            CardForm::Pressure => {
                rows.push(field("Pressure".into(), 0, "force / area"));
                rows.push(CardRow::Text("Positive pushes into the face.".into()));
                (format!("Pressure{target}"), "PLOAD4".into(), "Apply".into())
            }
            CardForm::Force => {
                for a in 0..dim {
                    rows.push(field(format!("Force {}", AXES[a]), a, "force"));
                }
                (format!("Force{target}"), "FORCE".into(), "Apply".into())
            }
            CardForm::Traction => {
                for a in 0..dim {
                    rows.push(field(format!("Traction {}", AXES[a]), a, "force / area"));
                }
                (format!("Traction{target}"), "PLOAD4 dir".into(), "Apply".into())
            }
            CardForm::Bearing => {
                rows.push(field("Force x".into(), 0, "force"));
                rows.push(field("Force y".into(), 1, "force"));
                (format!("Pin load{target}"), "bearing".into(), "Apply".into())
            }
            CardForm::EditLoad(i) => {
                let params = self.problem.loads.get(i).map(|l| l.params(dim)).unwrap_or_default();
                for (k, (label, _)) in params.iter().enumerate() {
                    rows.push(field((*label).to_string(), k, ""));
                }
                (format!("Edit load{target}"), "load".into(), "Apply".into())
            }
            CardForm::Gravity => {
                for a in 0..dim {
                    rows.push(field(format!("Body force {}", AXES[a]), a, "force / volume"));
                }
                rows.push(CardRow::Text("Weight density pulls down with a negative y. Zero removes it.".into()));
                ("Gravity / body force".into(), "GRAV".into(), "Apply".into())
            }
            CardForm::Case(i) => {
                rows.push(CardRow::Field { label: "Name".into(), text: c.fields[0].clone(), index: 0, unit: String::new() });
                for (s, on) in c.toggles.iter().enumerate() {
                    let label = self.problem.supports.get(s).map_or("support".to_string(), |sup| format!("Support on {}", sup.edge().unwrap_or("point")));
                    rows.push(CardRow::Toggle { label, on: *on, action: ui(SolidAction::CardToggle(s)) });
                }
                for l in 0..self.problem.loads.len() {
                    let label = self.problem.loads.get(l).map_or("load".to_string(), |load| format!("{} on {}", load.label(), load.edge().unwrap_or("point")));
                    rows.push(CardRow::Field { label, text: c.fields[l + 1].clone(), index: l + 1, unit: "factor".into() });
                }
                rows.push(CardRow::Text("A factor of 0 switches a load off; 2 doubles it.".into()));
                (format!("Load case {}", i + 1), "LOAD / SPCADD".into(), "Apply".into())
            }
        };
        Some(ValueCard { title, tag, rows, apply_label, apply, cancel })
    }

    /// Write the card into the model. A bad number keeps the card open and says what is wrong.
    pub(super) fn card_apply(&mut self) {
        let Some(c) = self.card.clone() else { return };
        let dim = self.card_dim();
        let parsed: Result<Vec<f64>, String> = match c.form {
            CardForm::Case(_) => c.fields.iter().skip(1).enumerate().map(|(l, t)| number(t, &format!("Factor of load {}", l + 1))).collect(),
            _ => c.fields.iter().enumerate().map(|(i, t)| number(t, &format!("{}", ["First value", "Second value", "Third value"][i.min(2)]))).collect(),
        };
        let values = match parsed {
            Ok(v) => v,
            Err(e) => {
                self.status = e;
                return;
            }
        };
        let names = c.names.clone();
        let label = match names.len() {
            1 => format!("'{}'", names[0]),
            n => format!("{n} surfaces"),
        };
        match c.form {
            CardForm::Hold | CardForm::EditHold(_) => {
                if !c.toggles.iter().any(|t| *t) {
                    self.status = "Hold at least one direction.".into();
                    return;
                }
                let comp = |a: usize| if c.toggles.get(a).copied().unwrap_or(false) { Some(values[a]) } else { None };
                let (ux, uy, uz) = (comp(0), comp(1), if dim == 3 { comp(2) } else { None });
                let form = c.form;
                self.edit(|p| match form {
                    CardForm::EditHold(i) => {
                        if let Some(s) = p.supports.get_mut(i) {
                            for (a, v) in [ux, uy, uz].into_iter().enumerate() {
                                s.set_comp(a, v);
                            }
                        }
                    }
                    _ => {
                        for n in &names {
                            p.supports.push(Support::Edge { edge: n.clone(), ux, uy, uz });
                        }
                    }
                });
                self.status = format!("Holding {} on {label}.", (0..dim).filter(|a| c.toggles[*a]).map(|a| AXES[a]).collect::<Vec<_>>().join(" "));
            }
            CardForm::Pressure => {
                let mut next = self.problem.clone();
                for n in &names {
                    next.loads.push(Load::Pressure { edge: n.clone(), p: values[0] });
                }
                self.edit(|p| *p = next);
                self.record_boundary_choice(fea_problem::advice::BcKind::Pressure);
                self.status = format!("Pressure {} on {label}.", values[0]);
            }
            CardForm::Force | CardForm::Traction => {
                let v = |a: usize| if a < dim { values[a] } else { 0.0 };
                let form = c.form;
                self.edit(|p| {
                    for n in &names {
                        p.loads.push(if form == CardForm::Force { Load::Force { edge: n.clone(), fx: v(0), fy: v(1), fz: v(2) } } else { Load::Traction { edge: n.clone(), tx: v(0), ty: v(1), tz: v(2) } });
                    }
                });
                self.status = format!("{} on {label}.", if form == CardForm::Force { "Force" } else { "Traction" });
            }
            CardForm::Bearing => {
                let mut next = self.problem.clone();
                if let Err(e) = fea_problem::advice::add_boundary(&mut next, &names[0], fea_problem::advice::BcKind::Bearing) {
                    self.status = e;
                    return;
                }
                if let Some(Load::Bearing { fx, fy, .. }) = next.loads.last_mut() {
                    (*fx, *fy) = (values[0], values[1]);
                }
                self.edit(|p| *p = next);
                self.record_boundary_choice(fea_problem::advice::BcKind::Bearing);
                self.status = format!("Pin load on {label}.");
            }
            CardForm::EditLoad(i) => {
                self.edit(|p| {
                    if let Some(l) = p.loads.get_mut(i) {
                        values.iter().enumerate().for_each(|(k, v)| l.set_param(dim, k, *v));
                    }
                });
                self.status = format!("Load on {label} updated.");
            }
            CardForm::Gravity => {
                let v = |a: usize| if a < dim { values[a] } else { 0.0 };
                let body = Load::Body { bx: v(0), by: v(1), bz: v(2) };
                self.edit(|p| {
                    let none = v(0) == 0.0 && v(1) == 0.0 && v(2) == 0.0;
                    match p.loads.iter().position(|l| matches!(l, Load::Body { .. })) {
                        Some(i) if none => p.remove_load(i),
                        Some(i) => p.loads[i] = body.clone(),
                        None if !none => p.loads.push(body.clone()),
                        None => {}
                    }
                });
                self.status = "Gravity / body force set.".into();
            }
            CardForm::Case(i) => {
                let name = c.fields[0].trim().to_string();
                if name.is_empty() {
                    self.status = "Give the load case a name.".into();
                    return;
                }
                self.edit(|p| {
                    if let Some(case) = p.cases.get_mut(i) {
                        case.name = name.clone();
                        for (s, on) in c.toggles.iter().enumerate() {
                            case.set_support(s, *on);
                        }
                        for (l, f) in values.iter().enumerate() {
                            case.set_factor(l, *f);
                        }
                    }
                });
                self.status = format!("Load case '{name}' updated.");
            }
        }
        self.card = None;
        self.touch();
    }

    // ------------------------------------------------------------------ load cases

    pub(super) fn new_case(&mut self) {
        let n = self.problem.cases.len().max(1) + 1;
        self.edit(|p| {
            p.add_case(&format!("Case {n}"));
        });
        self.open_card(CardForm::Case(self.problem.active_case_index()));
        self.status = format!("Case {n} is active: choose what acts in it, then Apply.");
    }

    pub(super) fn set_case(&mut self, i: usize) {
        if i < self.problem.cases.len() && i != self.problem.active_case_index() {
            self.edit(|p| p.active_case = i);
            self.status = format!("Load case '{}' is active (solve to see its result).", self.problem.cases[i].name);
        }
    }

    pub(super) fn remove_case(&mut self, i: usize) {
        if i < self.problem.cases.len() {
            let name = self.problem.cases[i].name.clone();
            self.edit(|p| p.remove_case(i));
            self.status = format!("Removed load case '{name}'.");
        }
    }
}
