//! The solid editor's inspector and tree menus. The old property manager listed every row of the whole problem in a
//! permanent dock; this shows only what the selection owns: the focused feature's value fields (numbers, components,
//! text), plus cards for the current step (mesh density, recommendations, results). Choices and actions (change the
//! analysis, a support's kind or edge, add, remove) are what a right click on the feature-tree row offers, so they
//! are not repeated as rows.
use super::*;
use crate::{
    gpu_viewer::{
        controller::UiAction,
        editor::{
            authoring::{cards_from, Card, Relevance},
            session::{EditorAction, Tone},
        },
        menu::{Entry, Item},
        text::wrap,
        ui::DockItem,
    },
    toolboxes::fea_workbench::model::{edit_kind, field_hint, row_label, row_value, EditKind},
};

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}

/// The feature a feature-tree row stands for (`None` for result, surface and history rows).
pub(super) fn node_feature(node: u32) -> Option<Feature> {
    use super::ui::{T_BUSHINGS, T_CONTACTS, T_FEATURE_BASE, T_GEOMETRY, T_HEAT_LOADS, T_HEAT_SUPPORTS, T_LOADS, T_ROOT, T_SUPPORTS};
    let rel = |base: u32| node.checked_sub(T_FEATURE_BASE + base).map(|n| n as usize);
    Some(match node {
        T_ROOT => Feature::Problem,
        T_GEOMETRY => Feature::Geometry,
        T_SUPPORTS => Feature::Support(usize::MAX),
        T_LOADS => Feature::Load(usize::MAX),
        T_BUSHINGS => Feature::Bushing(usize::MAX),
        T_CONTACTS => Feature::Contact(usize::MAX),
        T_HEAT_SUPPORTS => Feature::HeatSupport(usize::MAX),
        T_HEAT_LOADS => Feature::HeatLoad(usize::MAX),
        n if n == T_FEATURE_BASE + 4 => Feature::Thermal,
        n if n == T_FEATURE_BASE => Feature::Problem,
        n if n == T_FEATURE_BASE + 1 => Feature::Material,
        n if n == T_FEATURE_BASE + 2 => Feature::Geometry,
        n if n == T_FEATURE_BASE + 3 => Feature::Mesh,
        n if (T_FEATURE_BASE + 10..T_FEATURE_BASE + 1_000).contains(&n) => Feature::Hole(rel(10)?),
        n if (T_FEATURE_BASE + 1_000..T_FEATURE_BASE + 2_000).contains(&n) => Feature::Support(rel(1_000)?),
        n if (T_FEATURE_BASE + 2_000..T_FEATURE_BASE + 3_000).contains(&n) => Feature::Load(rel(2_000)?),
        n if (T_FEATURE_BASE + 3_000..T_FEATURE_BASE + 4_000).contains(&n) => Feature::Bushing(rel(3_000)?),
        n if (T_FEATURE_BASE + 7_000..T_FEATURE_BASE + 8_000).contains(&n) => Feature::Contact(rel(7_000)?),
        n if (T_FEATURE_BASE + 4_000..T_FEATURE_BASE + 5_000).contains(&n) => Feature::HeatSupport(rel(4_000)?),
        n if (T_FEATURE_BASE + 5_000..T_FEATURE_BASE + 6_000).contains(&n) => Feature::HeatLoad(rel(5_000)?),
        _ => return None,
    })
}

impl SolidSession {
    fn feature_title(&self, f: Feature) -> String {
        match f {
            Feature::Problem => "Problem".into(),
            Feature::Material => "Material".into(),
            Feature::Geometry => "Geometry".into(),
            Feature::Mesh => "Mesh".into(),
            Feature::Hole(h) => format!("Hole {}", h + 1),
            Feature::Support(s) if s != usize::MAX => format!("Support {}", s + 1),
            Feature::Load(l) if l != usize::MAX => format!("Load {}", l + 1),
            Feature::Bushing(b) if b != usize::MAX => format!("Bushing {}", b + 1),
            Feature::Contact(c) if c != usize::MAX => format!("Contact {}", c + 1),
            Feature::Contact(_) => "Contacts".into(),
            Feature::Thermal => "Thermal".into(),
            Feature::HeatSupport(s) if s != usize::MAX => format!("Fixed temperature {}", s + 1),
            Feature::HeatLoad(l) if l != usize::MAX => format!("Heat load {}", l + 1),
            Feature::HeatSupport(_) => "Fixed temperatures".into(),
            Feature::HeatLoad(_) => "Heat loads".into(),
            Feature::Support(_) => "Supports".into(),
            Feature::Load(_) => "Loads".into(),
            Feature::Bushing(_) => "Bushings".into(),
        }
    }

    /// The value rows (not the choices) of `feature`, as inspector fields.
    fn value_fields(&self, feature: Feature) -> Vec<DockItem> {
        let names = self.names();
        let mut out = Vec::new();
        for (i, row) in self.rows().into_iter().enumerate() {
            if matches!(row, FieldRow::Header(_)) || feature_of(row) != feature || edit_kind(row) == EditKind::Action {
                continue;
            }
            let focused = i == self.focus;
            let editing = self.prompt.as_ref().filter(|p| focused && p.row == Some(row));
            let value = match editing {
                Some(p) => format!("[{}_]", p.text),
                None => row_value(&self.problem, &names, self.import_len(), row),
            };
            let value = if editing.is_none() {
                let unit = match row {
                    FieldRow::Young | FieldRow::Yield | FieldRow::HardeningRow => self.problem.units.stress(),
                    FieldRow::Density => self.problem.units.density(),
                    FieldRow::Alpha => if self.problem.units == fea_problem::UnitSystem::InchPsi { "1/deg F" } else { "1/deg C" },
                    _ => "",
                };
                if unit.is_empty() { value } else { format!("{value} {unit}") }
            } else { value };
            let tone = if editing.is_some() { Tone::Warn } else if focused { Tone::On } else { Tone::Normal };
            out.push(DockItem::field(row_label(&self.problem, row), value, tone, Some(ui(SolidAction::Focus(i))), focused));
        }
        out
    }

    pub(super) fn inspector_cards(&self) -> Vec<Card> {
        if self.sketching.is_some() {
            return cards_from(self.sketch_properties());
        }
        let mut cards = Vec::new();
        if let Some((label, text)) = self.prompt_text().filter(|_| self.prompt.as_ref().is_some_and(|p| p.row.is_none())) {
            let title = if self.prompt.as_ref().is_some_and(|p| p.open) { "Open file" } else { "Material search" };
            cards.push(Card::new("prompt", title, Relevance::Primary, vec![DockItem::row(format!("{label}: [{text}_]"), Tone::Warn, None, true)]));
        }
        let focused = self.focused_row();
        let feature = focused.map(feature_of);
        if let Some(f) = feature {
            let mut items = self.value_fields(f);
            if f == Feature::Material && matches!(self.problem.geometry, Geometry::Imported { .. }) {
                items.push(DockItem::text("Default material: file-defined section materials override these values and convert with the selected unit system.".to_string(), Tone::Dim));
            }
            if !items.is_empty() {
                if let Some(row) = focused.filter(|r| edit_kind(*r) != EditKind::Action) {
                    items.push(DockItem::text(if self.prompting() { "type a value, Enter applies, Esc cancels".to_string() } else { field_hint(row).to_string() }, Tone::Dim));
                }
                cards.push(Card::new(format!("feature-{}", self.feature_title(f).to_lowercase().split(' ').next().unwrap_or("")), self.feature_title(f), Relevance::Primary, items));
            }
        }
        if let Some(c) = self.guidance_card() {
            cards.push(c);
        }
        let mut one = |id: &str, items: Vec<DockItem>, relevance: Relevance, steps: &[&'static str]| {
            if let Some(mut c) = cards_from(items).into_iter().next() {
                c.id = id.into();
                c.relevance = relevance;
                c.steps = steps.to_vec();
                cards.push(c);
            }
        };
        one("history-feature", self.feature_panel(), Relevance::Primary, &[]);
        // Mesh density follows the Mesh step, and the Mesh row when that is what is selected.
        one("mesh-density", self.refine_panel(), Relevance::Context, if feature == Some(Feature::Mesh) { &[] } else { &["Mesh"] });
        one("recommendations", self.advice_panel(), Relevance::Context, &[]);
        let mut study = Vec::new();
        if let Some(st) = self.current_study() {
            study.push(DockItem::Heading(st.kind.label().into()));
            study.push(DockItem::text(format!("result {} of {}{}", self.study_index.min(st.scenes.len() - 1) + 1, st.scenes.len(), if self.show_study { "" } else { " (hidden)" }), Tone::Normal));
            study.push(DockItem::text("[ ] cycle, Simulation > Show study", Tone::Dim));
        }
        one("study", study, Relevance::Info, &["Simulation"]);
        let mut result = vec![DockItem::Heading("Result".into())];
        if let Some(sol) = self.current_solution() {
            let s = &sol.summary;
            result.push(DockItem::text(format!("{} nodes, {} elements", s.nodes, s.elements), Tone::Dim));
            if let Some(h) = &s.heat {
                result.push(DockItem::text(format!("temperature {:.4} to {:.4}", h.t_min.value, h.t_max.value), Tone::Normal));
                result.push(DockItem::text(format!("heat balance error {:.1e}", h.balance_error), Tone::Dim));
            }
            if sol.problem.study.is_structural() {
                result.push(DockItem::text(format!("max von Mises {:.4e}", s.max_von_mises.value), Tone::Normal));
                let size = {
                    let (mut lo, mut hi) = ([f64::INFINITY; 3], [f64::NEG_INFINITY; 3]);
                    for n in &sol.model.mesh.nodes {
                        for k in 0..3 {
                            lo[k] = lo[k].min(n[k]);
                            hi[k] = hi[k].max(n[k]);
                        }
                    }
                    (0..3).map(|k| hi[k] - lo[k]).fold(0.0f64, f64::max)
                };
                let share = if size > 0.0 { format!(" ({:.3} % of the part's size)", 100.0 * s.max_displacement.value / size) } else { String::new() };
                result.push(DockItem::text(format!("max displacement {:.4e}{share}", s.max_displacement.value), Tone::Normal));
                if let Some(m) = s.margin {
                    result.push(DockItem::text(format!("yield margin {m:+.3}"), if m < 0.0 { Tone::Warn } else { Tone::On }));
                }
                for c in &s.contacts {
                    result.push(DockItem::text(format!("contact {} / {}: {:.1} normal, {:.1} shear", c.a, c.b, c.normal_force, c.tangential_force), Tone::Normal));
                    result.push(DockItem::text(format!("  {:.0} % of it touching, {:.0} % at the friction limit", 100.0 * c.contact_share, 100.0 * c.slip_share), if c.contact_share < 0.999 { Tone::Warn } else { Tone::Dim }));
                }
                result.push(DockItem::text(format!("equilibrium error {:.1e}", s.equilibrium_error), Tone::Dim));
            }
            result.extend(self.adaptive_lines(sol));
            for n in &s.notes {
                for l in wrap(n, 44) {
                    result.push(DockItem::text(l, Tone::Warn));
                }
            }
        } else if self.solved.is_some() {
            result.push(DockItem::text("Result is stale: the model changed since the last solve", Tone::Warn));
        } else {
            result.clear();
        }
        one("result", result, Relevance::Info, &["Simulation"]);
        cards
    }

    /// Right-click menu of a tree row: the choices and actions of its entity.
    pub(super) fn node_menu(&self, node: u32) -> Vec<Item> {
        use super::ui::T_SURFACE_BASE;
        let entry = |label: String, a: SolidAction| Entry::new(label, "", ui(a));
        let mut out: Vec<Item> = Vec::new();
        if (T_SURFACE_BASE..super::sketching::T_HISTORY_BASE).contains(&node) {
            out.extend(self.surface_menu());
            return out;
        }
        if node >= super::sketching::T_HISTORY_BASE {
            return self.history_node_menu(node);
        }
        if (super::ui::T_CASE_BASE..super::ui::T_CASE_BASE + 1_000).contains(&node) {
            let i = (node - super::ui::T_CASE_BASE) as usize;
            let name = self.problem.cases.get(i).map_or(String::new(), |c| c.name.clone());
            return vec![
                Item::Entry(entry(format!("Make '{name}' active"), SolidAction::SetCase(i))),
                Item::Entry(entry("Edit this load case...".into(), SolidAction::OpenCard(super::CardForm::Case(i)))),
                Item::Entry(entry("Remove this load case".into(), SolidAction::RemoveCase(i))),
                Item::Separator,
                Item::Entry(entry("New load case".into(), SolidAction::NewCase)),
            ];
        }
        let Some(f) = node_feature(node) else { return out };
        for row in self.rows() {
            if matches!(row, FieldRow::Header(_)) || feature_of(row) != f || edit_kind(row) != EditKind::Action {
                continue;
            }
            let label = row_label(&self.problem, row).trim().to_string();
            let value = row_value(&self.problem, &self.names(), self.import_len(), row);
            let text = if value.is_empty() || value == "Enter" { label } else { format!("{label}: {value}") };
            out.push(Item::Entry(entry(text, SolidAction::Activate(row)).when(self.row_available(row))));
        }
        match f {
            Feature::Mesh => {
                out.push(Item::Separator);
                out.push(Item::Entry(Entry::new("Generate mesh", "", ui(SolidAction::GenerateMesh)).when(self.mesh_availability())));
                out.push(Item::Entry(Entry::new("Refine where recommended", "", ui(SolidAction::RefineAll)).when(self.refine_availability())));
            }
            Feature::Support(_) | Feature::Load(_) | Feature::HeatSupport(_) | Feature::HeatLoad(_) => {
                out.push(Item::Separator);
                match f {
                    Feature::Support(i) => {
                        out.push(Item::Entry(entry("Change directions...".into(), SolidAction::OpenCard(super::CardForm::EditHold(i)))));
                        out.extend(self.convert_items(true, i));
                    }
                    Feature::Load(i) => {
                        out.push(Item::Entry(entry("Edit values...".into(), SolidAction::OpenCard(super::CardForm::EditLoad(i)))));
                        out.extend(self.convert_items(false, i));
                    }
                    _ => {}
                }
                out.push(Item::Separator);
                out.push(Item::Entry(entry("Assign to the selected surface".into(), SolidAction::AssignSelection).when(self.assign_availability())));
            }
            _ => {}
        }
        out
    }
}
