//! Bushing model shared by the native workbench and Eccentric Bushing.

pub mod advice;
pub mod bushing_id_persistence;
pub mod bushing_id_picker;
pub mod edge_check;
pub mod material_persistence;
pub mod material_picker;
pub mod model;
pub mod reamer_persistence;
pub mod reamer_picker;
pub mod report;
pub mod size_filter;

use model::{BushingModel, FieldRow};

pub struct BushingState {
    pub model: BushingModel,
    pub rec_selected: usize,
    pub last_applied: Option<String>,
    pub input_error: Option<String>,
    pub edge_check: Option<edge_check::EdgeCheckRun>,
    pub edge_settings: edge_check::EdgeSettings,
    pub edge_job: Option<EdgeJob>,
    next_edge_job: u64,
    edge_deep_pref: bool,
    edge_seen: Option<(edge_check::EdgeSig, std::time::Instant)>,
    edge_armed: bool,
}

#[derive(Debug)]
pub struct EdgeRequest {
    pub id: u64,
    pub input: ::edge_check::runner::EdgeInput,
    pub cfg: ::edge_check::runner::EdgeConfig,
}

#[derive(Debug, Clone)]
pub struct EdgeJob {
    pub id: u64,
    pub deep: bool,
    pub auto: bool,
    pub started: std::time::Instant,
    sig: edge_check::EdgeSig,
}

pub const EDGE_AUTO_DEBOUNCE_MS: u64 = 400;

impl Default for BushingState {
    fn default() -> Self {
        Self {
            model: BushingModel::default(),
            rec_selected: 0,
            last_applied: None,
            input_error: None,
            edge_check: None,
            edge_settings: Default::default(),
            edge_job: None,
            next_edge_job: 1,
            edge_deep_pref: false,
            edge_seen: None,
            edge_armed: false,
        }
    }
}

impl BushingState {
    pub(crate) fn activate(&mut self, row: FieldRow) {
        self.last_applied = None;
        self.input_error = None;
        match row {
            FieldRow::ToggleFitType => self.model.toggle_fit_type(),
            FieldRow::ToggleToleranceMode => self.model.toggle_tolerance_mode(),
            FieldRow::ToggleBushingType => self.model.toggle_bushing_type(),
            FieldRow::ToggleIdType => self.model.toggle_id_type(),
            FieldRow::ToggleEndConstraint => self.model.toggle_end_constraint(),
            FieldRow::ToggleCsMode => self.model.toggle_cs_mode(),
            FieldRow::ToggleExtCsMode => self.model.toggle_ext_cs_mode(),
            FieldRow::ToggleAssemblyThermalEnabled => self.model.toggle_assembly_thermal_enabled(),
            FieldRow::AdvancedSection => self.model.advanced_open = !self.model.advanced_open,
            FieldRow::Header(_)
            | FieldRow::Number(_)
            | FieldRow::Tol(_)
            | FieldRow::OpenHousingMaterialPicker
            | FieldRow::OpenBushingMaterialPicker => return,
        }
    }

    pub fn start_edge_check(&mut self, deep: bool) -> Vec<EdgeRequest> {
        if self.edge_job.is_some() {
            return Vec::new();
        }
        self.edge_deep_pref = deep;
        match self.launch_edge_check(deep, false) {
            Ok(requests) => requests,
            Err(why) => {
                self.input_error = Some(format!("edge-distance check not run: {why}"));
                Vec::new()
            }
        }
    }

    fn launch_edge_check(&mut self, deep: bool, auto: bool) -> Result<Vec<EdgeRequest>, String> {
        let (input, cfg) = edge_check::prepare_with(&self.model, &self.edge_settings, deep)?;
        let id = self.next_edge_job;
        self.next_edge_job += 1;
        self.edge_job = Some(EdgeJob {
            id,
            deep,
            auto,
            started: std::time::Instant::now(),
            sig: (input.clone(), cfg.bushing, self.edge_settings.clone()),
        });
        if !auto {
            self.last_applied = None;
            self.input_error = None;
        }
        Ok(vec![EdgeRequest { id, input, cfg }])
    }

    pub fn auto_edge_check(&mut self) -> Vec<EdgeRequest> {
        let Some(sig) =
            edge_check::live_signature_with(&self.model, &self.edge_settings, self.edge_deep_pref)
        else {
            return Vec::new();
        };
        match &self.edge_seen {
            None => self.edge_seen = Some((sig.clone(), std::time::Instant::now())),
            Some((seen, _)) if *seen != sig => {
                self.edge_seen = Some((sig.clone(), std::time::Instant::now()));
                self.edge_armed = true;
                if self.edge_job.as_ref().is_some_and(|j| j.sig != sig) {
                    self.edge_job = None;
                }
            }
            _ => {}
        }
        let settled = self
            .edge_seen
            .as_ref()
            .is_some_and(|(_, at)| at.elapsed().as_millis() as u64 >= EDGE_AUTO_DEBOUNCE_MS);
        let armed = self.edge_armed || self.edge_check.is_some();
        let up_to_date = self
            .edge_check
            .as_ref()
            .is_some_and(|run| (run.input.clone(), run.bushing, run.settings.clone()) == sig);
        if !settled || !armed || up_to_date || self.edge_job.is_some() {
            return Vec::new();
        }
        self.launch_edge_check(self.edge_deep_pref, true)
            .unwrap_or_default()
    }

    pub fn finish_edge_check(&mut self, id: u64, run: edge_check::EdgeCheckRun) {
        if self.edge_job.take_if(|j| j.id == id).is_some() {
            self.edge_check = Some(run);
        }
    }

    pub fn cancel_edge_check(&mut self) -> bool {
        self.edge_job.take().is_some()
    }

    pub fn set_edge_cv(&mut self, id: Option<&'static str>, percent: f64) -> Result<bool, String> {
        let current = self
            .edge_settings
            .rows(&self.model)
            .into_iter()
            .find(|(row, _, _)| *row == id)
            .map(|(_, _, cv)| cv);
        if current == Some(percent / 100.0) {
            return Ok(false);
        }
        let changed = self.edge_settings.set_percent(id, percent)?;
        if changed {
            self.edge_armed = true;
            self.edge_job = None;
            self.edge_seen = edge_check::live_signature_with(
                &self.model,
                &self.edge_settings,
                self.edge_deep_pref,
            )
            .map(|sig| (sig, std::time::Instant::now()));
        }
        Ok(changed)
    }

    pub fn report_text(&self) -> String {
        let mut text = report::build_report_text(&self.model);
        if let Some(run) = self
            .edge_check
            .as_ref()
            .filter(|run| !edge_check::is_stale_with(&self.model, &self.edge_settings, run))
        {
            text.push_str(&edge_check::report_text(run));
        }
        text
    }
}
