//! The solid editor's contribution to the CAD chrome: feature tree, property manager (the shared
//! Workbench rows), ribbon, menus and context menu, plus the `Authoring` impl the window drives.
use super::*;
use crate::{
    gpu_viewer::{
        controller::UiAction,
        editor::{authoring::Authoring, session::EditorAction},
        menu::{Entry, Item, Menu},
        text::wrap,
        ui::{DockItem, RibbonTab, TreeNode},
    },
    toolboxes::fea_workbench::model::row_value,
};
use crate::gpu_viewer::editor::session::Tone;

fn ui(a: SolidAction) -> UiAction {
    UiAction::Editor(EditorAction::Solid(a))
}
fn entry(label: &str, shortcut: &str, a: SolidAction) -> Entry {
    Entry::new(label, shortcut, ui(a))
}
fn item(e: Entry) -> Item {
    Item::Entry(e)
}
fn clip(s: &str, n: usize) -> String {
    s.chars().take(n).collect()
}

/// Tree ids.
pub(super) const T_ROOT: u32 = 0;
pub(super) const T_GEOMETRY: u32 = 1;
pub(super) const T_SUPPORTS: u32 = 2;
pub(super) const T_LOADS: u32 = 3;
pub(super) const T_BUSHINGS: u32 = 4;
pub(super) const T_SURFACES: u32 = 5;
pub(super) const T_HEAT_SUPPORTS: u32 = 6;
pub(super) const T_HEAT_LOADS: u32 = 7;
pub(super) const T_FEATURE_BASE: u32 = 100;
pub(super) const T_CASES: u32 = 8;
pub(super) const T_CASE_BASE: u32 = T_FEATURE_BASE + 6_000;
pub(super) const T_SURFACE_BASE: u32 = 10_000;

impl SolidSession {
    /// Simulation entries of the current bench (studies of other benches are not offered).
    fn simulation_entries(&self) -> Vec<Entry> {
        let cancel = entry(if Self::bench_study(self.bench).is_some() { "Cancel study" } else { "Cancel solve" }, "Esc", SolidAction::CancelSolve).when(if self.solving() { Ok(()) } else { Err("no solve is running".into()) });
        match Self::bench_study(self.bench) {
            None => {
                let cases = self.problem.cases.len();
                let active = self.problem.active_case_index();
                let case_only = |what: &str| if cases > 0 { Ok(()) } else { Err(format!("{what}: add a second load case first (Simulation > New load case)")) };
                vec![
                    entry("Solve", "Ctrl/Cmd+Enter", SolidAction::Solve).when(self.solve_availability()),
                    cancel,
                    entry("Show results", "V", SolidAction::ShowResults).when(self.results_availability()).checked(self.show_results),
                    entry("Gravity / body force...", "", SolidAction::OpenCard(CardForm::Gravity)),
                    entry("New load case", "", SolidAction::NewCase),
                    entry("Edit load case...", "", SolidAction::OpenCard(CardForm::Case(active))).when(case_only("there is only the model's own set of supports and loads")),
                    entry("Next load case", "", SolidAction::SetCase((active + 1) % cases.max(1))).when(if cases > 1 { Ok(()) } else { Err("there is one load case at most".to_string()) }),
                ]
            }
            Some(kind) => vec![
                entry("Run study", "Ctrl/Cmd+Enter", SolidAction::RunStudy(kind)).when(self.study_availability(kind)),
                cancel,
                entry("Show study", "V", SolidAction::ShowStudy).when(self.show_study_availability()).checked(self.show_study),
                entry("Previous mode", "[", SolidAction::StudyStep(false)).when(self.show_study_availability()),
                entry("Next mode", "]", SolidAction::StudyStep(true)).when(self.show_study_availability()),
            ],
        }
    }
    /// What kind of analysis the window is set up for: the benches (static, frequencies, buckling, transient) and the
    /// physics of the model (structural, thermal, thermal stress). One click each, in the Simulation tab and menu.
    fn analysis_entries(&self) -> Vec<Entry> {
        let mut v: Vec<Entry> = [Bench::Static, Bench::Modal, Bench::Buckling, Bench::Transient].into_iter().map(|b| Entry::new(b.label(), "", b.action()).checked(self.bench == b)).collect();
        v.extend(fea_problem::Study::ALL.into_iter().map(|st| entry(st.label(), "", SolidAction::SetStudy(st)).checked(self.problem.study == st)));
        v
    }
    fn simulation_items(&self) -> Vec<Item> {
        let mut items: Vec<Item> = vec![Item::Heading("Analysis".into())];
        items.extend(self.analysis_entries().into_iter().map(Item::Entry));
        items.push(Item::Separator);
        items.extend(self.simulation_entries().into_iter().map(Item::Entry));
        items
    }
    #[cfg(test)]
    pub(super) fn simulation_items_for_test(&self) -> Vec<Item> {
        self.simulation_items()
    }
    /// `Ok` when `row` exists for the current analysis / geometry source.
    pub(super) fn row_available(&self, row: FieldRow) -> Result<(), String> {
        if self.rows().contains(&row) {
            Ok(())
        } else {
            Err(match row {
                FieldRow::HoleAdd | FieldRow::Element if self.problem.history.is_some() => "this model is built from a feature history: edit its sketch (Features > Edit selected sketch)".to_string(),
                FieldRow::HoleAdd | FieldRow::Element | FieldRow::MeshStrategyRow | FieldRow::AdaptModeRow | FieldRow::PredictRow => "only sketch geometry has this (switch the source to Sketch)".to_string(),
                FieldRow::BushingAdd => "bushings need a plane analysis with a circular hole".to_string(),
                _ => "not available for this model".to_string(),
            })
        }
    }
    pub(super) fn selected_ok(&self) -> Result<(), String> {
        (self.selected.as_ref().map(|_| ()).or(self.multi.iter().next().map(|_| ()))).ok_or_else(|| "pick a surface in the viewport or the tree first".to_string())
    }
    fn row_value(&self, row: FieldRow) -> String {
        row_value(&self.problem, &self.names(), self.import_len(), row)
    }
    fn feature_nodes(&self) -> Vec<TreeNode> {
        let p = &self.problem;
        let focus_feature = self.focused_row().map(feature_of);
        let node = |id: u32, depth: u8, glyph: char, label: String, f: Feature| {
            TreeNode::new(id, depth, glyph, label).selected(focus_feature == Some(f)).on_click(ui(SolidAction::FocusFeature(f)))
        };
        let mut out = vec![TreeNode::new(T_ROOT, 0, 'M', format!("{}{}", p.name, if self.dirty { " *" } else { "" })).tone(Tone::Title).selected(focus_feature == Some(Feature::Problem)).on_click(ui(SolidAction::FocusFeature(Feature::Problem)))];
        out.push(node(T_FEATURE_BASE, 1, 'A', format!("Analysis: {}", self.row_value(FieldRow::Analysis)), Feature::Problem));
        out.push(node(T_FEATURE_BASE + 1, 1, 'm', format!("Material: {}", p.material.name), Feature::Material));
        out.push(TreeNode::new(T_FEATURE_BASE + 5, 1, 'u', format!("Units: {}", p.units.label())).on_click(ui(SolidAction::OpenCard(CardForm::Units))));
        out.push(TreeNode::new(T_GEOMETRY, 1, '+', "Geometry").folder());
        if p.history.is_none() {
            out.push(node(T_FEATURE_BASE + 2, 2, 'o', match &p.geometry {
                Geometry::Sketch { .. } => format!("Outline: {}", self.row_value(FieldRow::OuterKind)),
                Geometry::Imported { .. } => "Imported mesh".to_string(),
                Geometry::Solid { cache, .. } => format!("STEP solid ({} faces)", cache.as_ref().map_or(0, |c| c.face_names().len())),
            }, Feature::Geometry));
        }
        if let (Geometry::Sketch { holes, .. }, None) = (&p.geometry, &p.history) {
            for h in 0..holes.len() {
                out.push(node(T_FEATURE_BASE + 10 + h as u32, 2, 'O', format!("Hole {} ({})", h + 1, self.row_value(FieldRow::HoleKind(h))), Feature::Hole(h)));
            }
        }
        out.extend(self.history_nodes());
        let mesh_label = match self.preview_summary() {
            Ok((n, e)) => format!("Mesh: {n} nodes, {e} elements"),
            Err(m) => format!("Mesh: {}", clip(&m, 28)),
        };
        out.push(node(T_FEATURE_BASE + 3, 1, '#', mesh_label, Feature::Mesh));
        if p.study.is_structural() {
            out.push(TreeNode::new(T_SUPPORTS, 1, '+', format!("Supports ({})", p.supports.len())).folder());
            for (s, sup) in p.supports.iter().enumerate() {
                let target = sup.edge().map_or("point".to_string(), str::to_string);
                out.push(node(T_FEATURE_BASE + 1_000 + s as u32, 2, '^', format!("{} on {target}", self.row_value(FieldRow::SupportKind(s))), Feature::Support(s)));
            }
            out.push(TreeNode::new(T_LOADS, 1, '+', format!("Loads ({})", p.loads.len())).folder());
            for (l, load) in p.loads.iter().enumerate() {
                let target = load.edge().map_or("point".to_string(), str::to_string);
                out.push(node(T_FEATURE_BASE + 2_000 + l as u32, 2, '>', format!("{} on {target}", self.row_value(FieldRow::LoadKind(l))), Feature::Load(l)));
            }
        }
        if p.study.is_structural() && !p.cases.is_empty() {
            out.push(TreeNode::new(T_CASES, 1, '+', format!("Load cases ({})", p.cases.len())).folder());
            for (i, case) in p.cases.iter().enumerate() {
                let active = i == p.active_case_index();
                out.push(TreeNode::new(T_CASE_BASE + i as u32, 2, 'c', format!("{}{}", case.name, if active { " (active)" } else { "" })).selected(active).on_click(ui(SolidAction::SetCase(i))));
            }
        }
        if p.study.is_thermal() {
            out.push(node(T_FEATURE_BASE + 4, 1, 'T', format!("Thermal: k = {}", crate::toolboxes::fea_workbench::model::format_number(p.thermal.conductivity)), Feature::Thermal));
            out.push(TreeNode::new(T_HEAT_SUPPORTS, 1, '+', format!("Fixed temperatures ({})", p.thermal.supports.len())).folder());
            for (s, sup) in p.thermal.supports.iter().enumerate() {
                let target = sup.edge().map_or("point".to_string(), str::to_string);
                out.push(node(T_FEATURE_BASE + 4_000 + s as u32, 2, '^', format!("{} on {target}", crate::toolboxes::fea_workbench::model::format_number(sup.temperature())), Feature::HeatSupport(s)));
            }
            out.push(TreeNode::new(T_HEAT_LOADS, 1, '+', format!("Heat loads ({})", p.thermal.loads.len())).folder());
            for (l, load) in p.thermal.loads.iter().enumerate() {
                let target = load.edge().map_or("point".to_string(), str::to_string);
                out.push(node(T_FEATURE_BASE + 5_000 + l as u32, 2, '>', format!("{} on {target}", load.label()), Feature::HeatLoad(l)));
            }
        }
        if !p.bushings.is_empty() {
            out.push(TreeNode::new(T_BUSHINGS, 1, '+', format!("Bushings ({})", p.bushings.len())).folder());
            for b in 0..p.bushings.len() {
                out.push(node(T_FEATURE_BASE + 3_000 + b as u32, 2, 'B', format!("Bushing {}", b + 1), Feature::Bushing(b)));
            }
        }
        let names = self.names();
        if !names.is_empty() {
            out.push(TreeNode::new(T_SURFACES, 1, '+', format!("Surfaces ({})", names.len())).folder());
            for (i, n) in names.iter().enumerate() {
                out.push(TreeNode::new(T_SURFACE_BASE + i as u32, 2, 's', n.clone()).selected(self.selected.as_deref() == Some(n.as_str()) || self.multi.contains(n)).on_click(ui(SolidAction::SelectSurface(i))));
            }
        }
        out
    }
}

impl Authoring for SolidSession {
    fn bench(&self) -> Bench {
        self.bench
    }
    fn path(&self) -> &std::path::Path {
        &self.path
    }
    fn switch_bench(&mut self, bench: Bench) -> bool {
        if !bench.is_solid() {
            return false;
        }
        self.set_bench(bench);
        true
    }
    fn act(&mut self, action: EditorAction) {
        if let EditorAction::Solid(a) = action {
            self.do_action(a);
        }
    }
    fn key(&mut self, key: Key, mods: Mods) -> KeyResult {
        self.handle_key(key, mods)
    }
    fn click(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        SolidSession::click(self, px, size, cam)
    }
    fn click_with(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) -> bool {
        SolidSession::click_with(self, px, size, cam, additive)
    }
    fn box_select(&mut self, a: [f32; 2], b: [f32; 2], size: [f32; 2], cam: &Camera, additive: bool) {
        SolidSession::box_select(self, a, b, size, cam, additive)
    }
    fn take_popup(&mut self) -> Option<Vec<Item>> {
        if !self.take_popup_flag() {
            return None;
        }
        let items = self.boundary_items();
        Some(items)
    }
    fn select_at(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        SolidSession::select_at(self, px, size, cam)
    }
    fn hover(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        self.sketching.is_some() && self.sketch_hover(px, size, cam)
    }
    fn grab(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) -> bool {
        self.sketch_grab(px, size, cam)
    }
    fn drag(&mut self, px: [f32; 2], size: [f32; 2], cam: &Camera) {
        self.sketch_drag(px, size, cam)
    }
    fn release_grab(&mut self) {
        self.sketch_release()
    }
    fn take_view(&mut self) -> Option<crate::gpu_viewer::controller::StandardView> {
        self.want_view.take()
    }
    fn scene(&mut self) -> Result<Scene, String> {
        self.build_scene()
    }
    fn needs_scene(&self) -> bool {
        SolidSession::needs_scene(self)
    }
    fn value_card(&self) -> Option<crate::gpu_viewer::ui::ValueCard> {
        self.card_view()
    }
    fn card_field(&mut self, index: usize, text: String) {
        self.card_set_field(index, text);
    }
    fn poll(&mut self) -> bool {
        SolidSession::poll(self)
    }
    fn prompting(&self) -> bool {
        SolidSession::prompting(self)
    }
    fn solving(&self) -> bool {
        SolidSession::solving(self)
    }
    fn status(&self) -> String {
        self.status.clone()
    }
    fn set_status(&mut self, text: String) {
        self.status = text;
    }
    fn status_fix(&self) -> Option<(String, UiAction)> {
        self.status_fix_action()
    }
    fn title(&self) -> String {
        format!("{} | {}{}", self.bench.label(), self.problem.name, if self.dirty { " *" } else { "" })
    }
    fn dirty(&self) -> bool {
        self.dirty
    }
    fn default_mesh_lines(&self) -> bool {
        true
    }
    fn geometry_view(&self) -> bool {
        self.showing_geometry
    }
    fn shows_result(&self) -> bool {
        self.showing_result
    }
    fn field_unit(&self, label: &str) -> String {
        let u = self.problem.units;
        let l = label.to_lowercase();
        if l.contains("strain") {
            "strain".into()
        } else if l.contains("stress") {
            u.stress().into()
        } else if l.contains("displacement") && !l.contains("arbitrary") {
            u.length().into()
        } else if l.contains("temperature") {
            u.temperature().into()
        } else {
            String::new()
        }
    }
    fn shows_body_only(&self) -> bool {
        self.showing_body
    }
    fn initial_view(&self) -> Option<crate::gpu_viewer::controller::StandardView> {
        (self.problem.analysis == fea_problem::Analysis::Solid).then_some(crate::gpu_viewer::controller::StandardView::Isometric)
    }

    fn tree(&self) -> Vec<TreeNode> {
        self.feature_nodes()
    }

    fn properties(&self) -> Vec<DockItem> {
        crate::gpu_viewer::editor::authoring::flatten(self.inspector_cards())
    }

    fn inspector(&self) -> Vec<crate::gpu_viewer::editor::authoring::Card> {
        self.inspector_cards()
    }

    fn tree_menu(&self, node: u32) -> Vec<Item> {
        self.node_menu(node)
    }

    fn set_view(&mut self, cam: &Camera, size: [f32; 2]) {
        self.set_sketch_pixel(cam, size);
    }

    fn overlay(&self, cam: &Camera, size: [f32; 2]) -> Vec<crate::gpu_viewer::editor::authoring::Overlay> {
        if self.sketching.is_some() {
            self.sketch_overlay(cam, size)
        } else {
            self.bc_overlay(cam, size)
        }
    }

    fn focus_step(&self) -> Option<&'static str> {
        if self.sketching.is_some() {
            return None;
        }
        self.focused_row().map(|r| if feature_of(r) == Feature::Mesh { "Mesh" } else { "Model" })
    }

    fn solving_for(&self) -> Option<std::time::Duration> {
        self.solve_elapsed()
    }

    fn ribbon(&self) -> Vec<RibbonTab> {
        if self.sketching.is_some() {
            return self.sketch_ribbon();
        }
        use FieldRow::*;
        let act = |label: &str, shortcut: &str, row: FieldRow| entry(label, shortcut, SolidAction::Activate(row)).when(self.row_available(row));
        let imported = matches!(self.problem.geometry, Geometry::Imported { .. });
        vec![
            RibbonTab::new("Model")
                .group("Problem", vec![entry("New part", "Ctrl/Cmd+N", SolidAction::NewPart).when(self.replace_availability()), entry("Open file", "Ctrl/Cmd+O", SolidAction::OpenFile).when(self.replace_availability()), entry("Next template", "", SolidAction::NextTemplate), act("Analysis type", "", Analysis), entry("Material library", "", SolidAction::OpenCard(CardForm::Materials)), entry("Units", "", SolidAction::OpenCard(CardForm::Units))])
                .group("Send", vec![entry("To Bushing", "", SolidAction::SendTo(crate::gpu_viewer::EditorKind::Bushing)).when(self.send_availability(crate::gpu_viewer::EditorKind::Bushing)), entry("To Eccentric", "", SolidAction::SendTo(crate::gpu_viewer::EditorKind::Eccentric)).when(self.send_availability(crate::gpu_viewer::EditorKind::Eccentric))])
                .group(
                    "Geometry",
                    vec![
                        entry("Hole tool", "", SolidAction::SetTool(SolidTool::Hole)).when(self.hole_availability()).checked(self.tool == SolidTool::Hole),
                        act("Add hole", "", HoleAdd),
                        entry("Sketch / import", "", SolidAction::Activate(Source)),
                        act("Import mesh", "", ImportPath).when(if imported { Ok(()) } else { Err("switch the geometry source to Imported first".to_string()) }),
                    ],
                )
                .group(
                    "Boundary",
                    vec![
                        act("Add support", "", SupportAdd),
                        act("Add load", "", LoadAdd),
                        act("Add bushing", "", BushingAdd),
                        entry("Support on surface", "", SolidAction::SupportOnSelected).when(self.selected_ok()),
                        entry("Load on surface", "", SolidAction::LoadOnSelected).when(self.selected_ok()),
                        entry("Assign surface", "", SolidAction::AssignSelection).when(self.assign_availability()),
                    ],
                )
                .group(
                    "Edit",
                    vec![
                        entry("Undo", "Ctrl/Cmd+Z", SolidAction::Undo).when(if self.can_undo() { Ok(()) } else { Err("nothing to undo".into()) }),
                        entry("Redo", "Ctrl/Cmd+Shift+Z", SolidAction::Redo).when(if self.can_redo() { Ok(()) } else { Err("nothing to redo".into()) }),
                        entry("Remove entry", "Del", SolidAction::RemoveFocused).when(self.remove_availability()),
                        entry("Save", "Ctrl/Cmd+S", SolidAction::Save).when(self.save_availability()),
                    ],
                ),
            self.features_ribbon(),
            RibbonTab::new("Mesh").group(
                "Mesh",
                vec![act("Element type", "", Element), act("Element size", "", MeshSize), act("Adaptive passes", "", AdaptPasses), act("Target error", "", TargetError), entry("Refine where recommended", "", SolidAction::RefineAll).when(self.refine_availability()), entry("Show refinement hints", "", SolidAction::ToggleRefineShow).checked(self.show_refine), entry("Generate mesh", "", SolidAction::GenerateMesh).when(self.mesh_availability())],
            ),
            RibbonTab::new("Simulation").group("Analysis", self.analysis_entries()).group("Study", self.simulation_entries()),
        ]
    }

    fn start_options(&self) -> Vec<Entry> {
        self.start_entries()
    }

    fn step_state(&self, tab: &str) -> Option<crate::gpu_viewer::editor::authoring::StepState> {
        use crate::gpu_viewer::editor::authoring::StepState::{Done, Todo};
        let done = match tab {
            "Model" => self.problem.validate_geometry().is_ok(),
            "Mesh" => self.preview_current(),
            "Simulation" => self.current_solution().is_some() || self.current_study().is_some(),
            _ => return None,
        };
        Some(if done { Done } else { Todo })
    }

    fn menus(&self) -> Vec<Menu> {
        use FieldRow::*;
        let act = |label: &str, row: FieldRow| item(entry(label, "", SolidAction::Activate(row)).when(self.row_available(row)));
        vec![
            Menu {
                title: "File".into(),
                items: [
                    self.model_items(),
                    vec![
                        Item::Separator,
                        item(entry("Save", "Ctrl/Cmd+S", SolidAction::Save).when(self.save_availability())),
                        item(entry("Reload saved file", "", SolidAction::Load)),
                        Item::Separator,
                        item(Entry::new("Quit", "", UiAction::Quit)),
                    ],
                ]
                .concat(),
            },
            Menu {
                title: "Edit".into(),
                items: vec![
                    item(entry("Undo", "Ctrl/Cmd+Z", SolidAction::Undo).when(if self.can_undo() { Ok(()) } else { Err("nothing to undo".into()) })),
                    item(entry("Redo", "Ctrl/Cmd+Shift+Z", SolidAction::Redo).when(if self.can_redo() { Ok(()) } else { Err("nothing to redo".into()) })),
                    item(entry("Remove entry", "Del", SolidAction::RemoveFocused).when(self.remove_availability())),
                    Item::Separator,
                    item(entry("Assign surface to focused row", "", SolidAction::AssignSelection).when(self.assign_availability())),
                    item(entry("Clear surface selection", "Esc", SolidAction::ClearSelection).when(self.selected_ok())),
                ],
            },
            Menu {
                title: "Insert".into(),
                items: self.insert_items(vec![
                    item(entry(SolidTool::Hole.label(), "", SolidAction::SetTool(SolidTool::Hole)).when(self.hole_availability()).checked(self.tool == SolidTool::Hole)),
                    act("Hole (default position)", HoleAdd),
                    act("Support", SupportAdd),
                    act("Load", LoadAdd),
                    act("Bushing", BushingAdd),
                    Item::Separator,
                    item(entry("Support on selected surface", "", SolidAction::SupportOnSelected).when(self.selected_ok())),
                    item(entry("Load on selected surface", "", SolidAction::LoadOnSelected).when(self.selected_ok())),
                    Item::Separator,
                    item(entry("Fillet all edges...", "", SolidAction::SolidOp(stepops::PromptOp::Fillet)).when(self.solid_op_availability(stepops::PromptOp::Fillet))),
                    item(entry("Hole in selected face...", "", SolidAction::SolidOp(stepops::PromptOp::Hole)).when(self.solid_op_availability(stepops::PromptOp::Hole))),
                    item(entry("Boss on selected face...", "", SolidAction::SolidOp(stepops::PromptOp::Boss)).when(self.solid_op_availability(stepops::PromptOp::Boss))),
                ]),
            },
            Menu {
                title: "Tools".into(),
                items: vec![item(entry("Send to the Bushing workbench", "", SolidAction::SendTo(crate::gpu_viewer::EditorKind::Bushing)).when(self.send_availability(crate::gpu_viewer::EditorKind::Bushing))), item(entry("Send to the Eccentric Bushing workbench", "", SolidAction::SendTo(crate::gpu_viewer::EditorKind::Eccentric)).when(self.send_availability(crate::gpu_viewer::EditorKind::Eccentric))), Item::Separator, act("Analysis type", Analysis), item(entry("Material library...", "", SolidAction::OpenCard(CardForm::Materials))), item(entry("Units...", "", SolidAction::OpenCard(CardForm::Units))), act("Element type", Element), act("Adaptive passes", AdaptPasses), item(entry("Refine where recommended", "", SolidAction::RefineAll).when(self.refine_availability())), item(entry("Show refinement hints", "", SolidAction::ToggleRefineShow).checked(self.show_refine)), item(entry("Generate mesh", "", SolidAction::GenerateMesh).when(self.mesh_availability()))],
            },
            Menu { title: "Simulation".into(), items: self.simulation_items() },
            Menu { title: "Advisor".into(), items: self.advisor_items() },
        ]
    }

    fn context_menu(&self) -> Vec<Item> {
        if self.sketching.is_some() {
            return self.sketch_context();
        }
        let mut items = Vec::new();
        if self.selected.is_some() {
            items.extend(self.surface_menu());
            items.push(Item::Separator);
            items.push(item(entry("Assign to focused row", "", SolidAction::AssignSelection).when(self.assign_availability())));
            items.push(item(entry("Clear selection", "Esc", SolidAction::ClearSelection)));
            items.push(Item::Separator);
        }
        if self.selected.is_none() && self.problem.study.is_structural() {
            items.push(item(entry("Gravity / body force...", "", SolidAction::OpenCard(CardForm::Gravity))));
            items.push(Item::Separator);
        }
        items.push(item(entry("Solve", "Ctrl/Cmd+Enter", SolidAction::Solve).when(self.solve_availability())));
        items.push(item(entry("Show results", "V", SolidAction::ShowResults).when(self.results_availability())));
        items.push(item(entry("Undo", "Ctrl/Cmd+Z", SolidAction::Undo).when(if self.can_undo() { Ok(()) } else { Err("nothing to undo".into()) })));
        items
    }
}

impl SolidSession {
    /// What adaptive mesh refinement did for the shown result (it runs inside Solve: solve, estimate the error, refine
    /// where it is large, solve again).
    pub(super) fn adaptive_lines(&self, sol: &Solved) -> Vec<DockItem> {
        let (p, h) = (&sol.problem, &sol.history);
        let pct = |e: f64| format!("{:.1}%", 100.0 * e);
        let text = if p.analysis == fea_problem::Analysis::Solid {
            format!("fixed mesh (no refinement for 3D solids yet); error estimate {}", pct(sol.summary.zz_error))
        } else if matches!(p.geometry, Geometry::Imported { .. }) {
            format!("fixed mesh (imported); error estimate {}", pct(sol.summary.zz_error))
        } else if p.mesh.adapt_passes == 0 {
            format!("adaptive refinement off (Adaptive passes 0); error estimate {}", pct(sol.summary.zz_error))
        } else if let (Some(first), Some(last)) = (h.first(), h.last()) {
            if h.len() > 1 {
                format!("adaptive: {} passes, error {} -> {} (target {}), {} -> {} nodes", h.len() - 1, pct(first.zz_error), pct(last.zz_error), pct(p.mesh.target_error), first.nodes, last.nodes)
            } else {
                format!("adaptive: first mesh already met the target; error {} (target {})", pct(first.zz_error), pct(p.mesh.target_error))
            }
        } else {
            return Vec::new();
        };
        wrap(&text, 38).into_iter().map(|l| DockItem::text(l, Tone::Dim)).collect()
    }
}
