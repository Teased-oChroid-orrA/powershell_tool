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
    /// Search the material library and apply a material to the model.
    Materials,
    /// The unit system: convert the model to another, or declare the one its numbers are written in.
    Units,
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
    /// Material library: the group filter and the sort key (indices).
    group: usize,
    sort: usize,
}

/// Library rows shown at once.
const MATERIAL_ROWS: usize = 12;

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
            CardForm::Materials => (Vec::new(), vec![String::new()]),
            CardForm::Units => (Vec::new(), Vec::new()),
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
        self.card = Some(CardState { form, names, toggles, fields, group: 0, sort: 0 });
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
        let units = self.problem.units;
        let body_unit = format!("{}/{}^3", units.force(), units.length());
        let field = |label: String, index: usize, unit: &str| CardRow::Field { label, text: c.fields[index].clone(), index, unit: unit.to_string(),
        };
        let cancel = ui(SolidAction::CardCancel);
        let apply = ui(SolidAction::CardApply);
        let mut rows = Vec::new();
        let mut width = 270.0;
        let (title, tag, apply_label) = match c.form {
            CardForm::Units => {
                width = 380.0;
                let now = self.problem.units;
                let geometry_ok = self.problem.can_convert_units();
                rows.push(CardRow::Text(format!("The model is written in {} (lengths in {}, forces in {}, stresses in {}).", now.label(), now.length(), now.force(), now.stress())));
                rows.push(CardRow::Text("Convert preserves physical geometry, mechanical and thermal data, including external mesh and CAD coordinates. Declare keeps the numbers and only changes what they mean (use it when they were typed in another system).".into()));
                if geometry_ok.is_ok() {
                    rows.push(CardRow::Select {
                        label: "Unit system".into(),
                        selected: now.label().into(),
                        options: fea_problem::UnitSystem::ALL.into_iter().map(|sys| (sys.label().into(), ui(SolidAction::ConvertUnits(sys))))
                            .collect(),
                    });
                } else {
                    rows.push(CardRow::Text(geometry_ok.unwrap_err()));
                }
                for sys in fea_problem::UnitSystem::ALL.into_iter().filter(|s| *s != now) {
                    rows.push(CardRow::Choice { label: format!("Declare as {}", sys.label()), detail: "keeps the numbers".into(), action: ui(SolidAction::DeclareUnits(sys)), current: false,
                    });
                }
                ("Units".to_string(), now.label().to_string(), "Close".to_string(),
                )
            }
            CardForm::Materials => {
                use crate::toolboxes::material_lookup::model as lib;
                width = 400.0;
                let query = lib::Query { text: c.fields[0].clone(), group: c.group, sort: lib::SortKey::ALL[c.sort % lib::SortKey::ALL.len()], ..Default::default() };
                let hits = lib::filter_sort(&query);
                rows.push(CardRow::Field { label: "Search".into(), text: c.fields[0].clone(), index: 0, unit: String::new(),
                });
                let group = lib::groups().get(c.group).cloned().unwrap_or_default();
                rows.push(CardRow::Choice { label: format!("Group: {group}"), detail: "click for the next group".into(), action: ui(SolidAction::CardMaterialGroup), current: false,
                });
                rows.push(CardRow::Choice { label: format!("Sort by: {}", query.sort.label()), detail: "click for the next key".into(), action: ui(SolidAction::CardMaterialSort), current: false,
                });
                rows.push(CardRow::Text(format!("{} match{} (showing {}). Now: {}.", hits.len(), if hits.len() == 1 { "" } else { "es" }, hits.len().min(MATERIAL_ROWS), self.problem.material.name)));
                let cat = lib::catalog();
                for &i in hits.iter().take(MATERIAL_ROWS) {
                    let m = cat[i];
                    let rho = lib::density(m).map_or(String::new(), |d| format!("  rho {d:.3}"));
                    let detail = format!("E {} ksi  Fty {}{rho}", crate::toolboxes::fea_workbench::model::format_number(m.e_ksi), crate::toolboxes::fea_workbench::model::format_number(m.sy_ksi));
                    rows.push(CardRow::Choice { label: m.name.to_string(), detail, action: ui(SolidAction::ApplyMaterial(i)), current: m.name == self.problem.material.name,
                    });
                }
                ("Material library".to_string(), "MIL-HDBK-5J".to_string(), "Use first match".to_string(),
                )
            }
            CardForm::Hold | CardForm::EditHold(_) => {
                for a in 0..dim {
                    rows.push(CardRow::Toggle { label: format!("Hold {}", AXES[a]), on: c.toggles[a], action: ui(SolidAction::CardToggle(a)),
                    });
                    if c.toggles[a] {
                        rows.push(field(format!("{} displacement", AXES[a]), a, units.length(),
                        ));
                    }
                }
                if dim == 3 {
                    rows.push(CardRow::Text("A solid element has no rotations: holding all three is a fixed face.".into(),
                    ));
                } else {
                    rows.push(CardRow::Text("Hold one direction and the face slides in the other (a roller).".into(),
                    ));
                }
                let held: String = (0..dim).filter(|a| c.toggles[*a]).map(|a| char::from(b'1' + a as u8)).collect();
                (format!("{}{target}", if matches!(c.form, CardForm::EditHold(_)) { "Change directions" } else { "Hold directions" }), format!("SPC1 {}", if held.is_empty() { "-".to_string() } else { held }), "Apply".to_string(),
                )
            }
            CardForm::Pressure => {
                rows.push(field("Pressure".into(), 0, units.stress()));
                rows.push(CardRow::Text("Positive pushes into the face.".into()));
                (format!("Pressure{target}"), "PLOAD4".into(), "Apply".into())
            }
            CardForm::Force => {
                for a in 0..dim {
                    rows.push(field(format!("Force {}", AXES[a]), a, units.force()));
                }
                (format!("Force{target}"), "FORCE".into(), "Apply".into())
            }
            CardForm::Traction => {
                for a in 0..dim {
                    rows.push(field(format!("Traction {}", AXES[a]), a, units.stress()));
                }
                (format!("Traction{target}"), "PLOAD4 dir".into(), "Apply".into(),
                )
            }
            CardForm::Bearing => {
                rows.push(field("Force x".into(), 0, units.force()));
                rows.push(field("Force y".into(), 1, units.force()));
                (format!("Pin load{target}"), "bearing".into(), "Apply".into(),
                )
            }
            CardForm::EditLoad(i) => {
                let params = self.problem.loads.get(i).map(|l| l.params(dim)).unwrap_or_default();
                for (k, (label, _)) in params.iter().enumerate() {
                    let unit = if label.contains("Pressure") || label.contains("Traction") {
                        units.stress().to_string()
                    } else if label.contains("Force") {
                        units.force().to_string()
                    } else if label.contains("Body") {
                        body_unit.clone()
                    } else if matches!(*label, "X" | "Y" | "Z") {
                        units.length().to_string()
                    } else {
                        String::new()
                    };
                    rows.push(field((*label).to_string(), k, &unit));
                }
                (format!("Edit load{target}"), "load".into(), "Apply".into())
            }
            CardForm::Gravity => {
                for a in 0..dim {
                    rows.push(field(format!("Body force {}", AXES[a]), a, &body_unit));
                }
                rows.push(CardRow::Text("Weight density pulls down with a negative y. Zero removes it.".into(),
                ));
                ("Gravity / body force".into(), "GRAV".into(), "Apply".into())
            }
            CardForm::Case(i) => {
                rows.push(CardRow::Field { label: "Name".into(), text: c.fields[0].clone(), index: 0, unit: String::new(),
                });
                for (s, on) in c.toggles.iter().enumerate() {
                    let label = self.problem.supports.get(s).map_or("support".to_string(), |sup| {
                            format!("Support on {}", sup.edge().unwrap_or("point"))
                        });
                    rows.push(CardRow::Toggle { label, on: *on, action: ui(SolidAction::CardToggle(s)),
                    });
                }
                for l in 0..self.problem.loads.len() {
                    let label = self.problem.loads.get(l).map_or("load".to_string(), |load| {
                            format!("{} on {}", load.label(), load.edge().unwrap_or("point"))
                        });
                    rows.push(CardRow::Field { label, text: c.fields[l + 1].clone(), index: l + 1, unit: "factor".into(),
                    });
                }
                rows.push(CardRow::Text("A factor of 0 switches a load off; 2 doubles it.".into(),
                ));
                (format!("Load case {}", i + 1), "LOAD / SPCADD".into(), "Apply".into(),
                )
            }
        };
        Some(ValueCard { title, width, tag, rows, apply_label, apply, cancel,
        })
    }

    /// Write the card into the model. A bad number keeps the card open and says what is wrong.
    pub(super) fn card_apply(&mut self) {
        let Some(c) = self.card.clone() else { return };
        let dim = self.card_dim();
        if c.form == CardForm::Units {
            self.card = None;
            self.touch();
            return;
        }
        if c.form == CardForm::Materials {
            // Enter in the search takes the first match.
            self.card_apply_first_material();
            return;
        }
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
            CardForm::Materials | CardForm::Units => unreachable!("handled before the numbers are read"),
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

    fn card_material_query(&self) -> Option<crate::toolboxes::material_lookup::model::Query> {
        use crate::toolboxes::material_lookup::model as lib;
        let c = self.card.as_ref().filter(|c| c.form == CardForm::Materials)?;
        Some(lib::Query { text: c.fields[0].clone(), group: c.group, sort: lib::SortKey::ALL[c.sort % lib::SortKey::ALL.len()], ..Default::default() })
    }
    fn card_apply_first_material(&mut self) {
        use crate::toolboxes::material_lookup::model as lib;
        let typed = self.card.as_ref().is_some_and(|c| !c.fields[0].trim().is_empty());
        match self.card_material_query().and_then(|q| lib::filter_sort(&q).first().copied()) {
            Some(i) if typed => self.apply_material_index(i),
            _ => {
                self.card = None;
                self.touch();
            }
        }
    }
    /// Apply catalog material `i` to the model and close the library.
    pub(super) fn apply_material_index(&mut self, i: usize) {
        let Some(m) = crate::toolboxes::material_lookup::model::catalog().get(i).copied() else { return };
        self.edit(|p| model::apply_material(p, m));
        let rho = model::mass_density(m, self.problem.units).map_or("density unchanged (not in the library)".to_string(), |d| format!("density {:.3e}", d));
        self.status = format!("Material {} applied: E, nu, expansion, yield, {rho}.", m.name);
        self.card = None;
        self.touch();
    }
    pub(super) fn card_material_cycle(&mut self, sort: bool) {
        use crate::toolboxes::material_lookup::model as lib;
        if let Some(c) = self.card.as_mut().filter(|c| c.form == CardForm::Materials) {
            if sort {
                c.sort = (c.sort + 1) % lib::SortKey::ALL.len();
            } else {
                c.group = (c.group + 1) % lib::groups().len();
            }
            self.touch();
        }
    }

    pub(super) fn convert_units(&mut self, to: fea_problem::UnitSystem) {
        let mut next = self.problem.clone();
        match next.convert_units(to) {
            Ok(()) => {
                self.edit(|p| *p = next);
                self.status = format!("The model is now in {}: every length, force, stress, density and expansion was rescaled including thermal quantities.", to.label());
                self.card = None;
                self.touch();
            }
            Err(e) => self.status = e,
        }
    }
    pub(super) fn declare_units(&mut self, to: fea_problem::UnitSystem) {
        self.edit(|p| p.declare_units(to));
        self.status = format!("The numbers are now read as {}; nothing was rescaled.", to.label());
        self.card = None;
        self.touch();
    }

    /// Measurements of the model's surfaces: from the mesh being shown when there is one, else from the sketch or the STEP tessellation.
    fn surface_facts(&self) -> Option<fea_problem::facts::Facts> {
        self.shown_mesh().and_then(|m| fea_problem::facts::Facts::from_mesh(m, self.problem.thickness)).or_else(|| fea_problem::facts::Facts::of(&self.problem))
    }

    pub(super) fn convert_load(&mut self, i: usize, to: fea_problem::conversion::LoadKind) {
        let Some(facts) = self.surface_facts() else {
            self.status = "The surfaces' sizes are not known yet: generate the mesh first.".into();
            return;
        };
        // A point force has no surface of its own: the picked one, else the nearest.
        let own_surface = self.problem.loads.get(i).is_some_and(|l| l.edge().is_some() || matches!(l, fea_problem::Load::Bearing { .. }));
        let surface = if own_surface { None } else { self.selected.clone() };
        let mut next = self.problem.clone();
        match next.convert_load(i, to, surface.as_deref(), &facts) {
            Ok(notes) => {
                self.edit(|p| *p = next);
                self.status = format!("Converted to a {}{}", to.label().to_lowercase(), if notes.is_empty() { ".".to_string() } else { format!(": {}.", notes.join("; ")) });
            }
            Err(e) => self.status = e,
        }
    }

    pub(super) fn convert_support(&mut self, i: usize, to: fea_problem::conversion::SupportKind) {
        let Some(facts) = self.surface_facts() else {
            self.status = "The surfaces' sizes are not known yet: generate the mesh first.".into();
            return;
        };
        let surface = if self.problem.supports.get(i).is_some_and(|s| s.edge().is_some()) { None } else { self.selected.clone() };
        let mut next = self.problem.clone();
        match next.convert_support(i, to, surface.as_deref(), &facts) {
            Ok(notes) => {
                self.edit(|p| *p = next);
                self.status = format!("Support changed to: {}{}", to.label().to_lowercase(), if notes.is_empty() { ".".to_string() } else { format!(" ({}).", notes.join("; ")) });
            }
            Err(e) => self.status = e,
        }
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
