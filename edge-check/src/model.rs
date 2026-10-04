//! The two traits every cross-check implements. Adding a model means
//! implementing [`EdgeModel`] and listing it in
//! [`crate::models::default_models`]; removing one means deleting that line.

use crate::types::{Geometry, Loads, ModeMargin, Strengths};

/// A model's response at one fixed geometry: cheap to evaluate for any
/// number of load combinations (the expensive work - meshing, solving,
/// fitting - happens once, in [`EdgeModel::respond`]).
pub trait Response {
    /// Margins `allowable/applied - 1` per failure mode this model
    /// evaluates. `strength_scale` multiplies every strength (Monte-Carlo
    /// material scatter; 1.0 for the deterministic result).
    fn margins(&self, loads: &Loads, strength_scale: f64) -> Vec<ModeMargin>;

    /// Whether the interference fit keeps the bushing in contact all round
    /// under these loads, if the model has a contact notion.
    fn contact_retained(&self, _loads: &Loads) -> Option<bool> {
        None
    }

    /// Model-specific remarks for the report (assumptions that bind at this
    /// geometry, missing data).
    fn notes(&self) -> Vec<String> {
        Vec::new()
    }
}

pub trait EdgeModel {
    fn id(&self) -> &'static str;
    fn label(&self) -> &'static str;

    /// Builds the response for `geom`. An `Err` means the model cannot
    /// evaluate this geometry (reported, never silently dropped).
    fn respond(&self, geom: &Geometry, mat: &Strengths, _fit_pressure: f64) -> Result<Box<dyn Response>, String>;

    /// Response used while searching for the minimum edge distance, where
    /// many geometries are evaluated: a model may use a cheaper
    /// discretisation here. Defaults to [`respond`](Self::respond).
    fn respond_for_search(&self, geom: &Geometry, mat: &Strengths, fit_pressure: f64) -> Result<Box<dyn Response>, String> {
        self.respond(geom, mat, fit_pressure)
    }

    /// Whether this model solves the same stress problem as the others (so
    /// its results can be compared quantitatively with theirs) rather than
    /// applying a different, table-based criterion.
    fn is_field_model(&self) -> bool {
        true
    }

    /// Coefficient of variation of this model's own prediction error (its
    /// capacity as a multiplicative random factor), from validation against
    /// test data where there is some, else an engineering judgement stated
    /// where it is set. `0` for a check that already is a statistical
    /// allowable.
    fn model_cv(&self) -> f64 {
        0.0
    }

    /// Whether the margin is a smooth monotone function of the edge
    /// distance, so the runner may search for the minimum edge distance.
    /// Models with a stepped allowable (the tabulated-`Fbru` check) say no.
    fn supports_edge_search(&self) -> bool {
        true
    }
}
