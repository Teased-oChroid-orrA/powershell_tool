//! The model registry. To add a model: implement [`crate::model::EdgeModel`]
//! in a new file here and add it to [`default_models`]. To remove one:
//! delete its line (and, if wanted, its file).

pub mod allowable;
pub mod analytic_model;
pub mod fem_model;
pub mod plastic_model;

use crate::model::EdgeModel;
use crate::runner::EdgeConfig;

/// The models run, in report order. The first is the primary result; the
/// rest are cross-checks.
pub fn default_models(cfg: &EdgeConfig) -> Vec<Box<dyn EdgeModel>> {
    let mut models: Vec<Box<dyn EdgeModel>> = vec![
        Box::new(analytic_model::AnalyticModel),
        Box::new(fem_model::FemModel::default()),
        Box::new(allowable::AllowableModel { fbru_e15: cfg.fbru_e15 }),
    ];
    if cfg.include_plastic {
        models.push(Box::new(plastic_model::PlasticModel::default()));
    }
    models
}
