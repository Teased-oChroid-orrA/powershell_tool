//! Saved Bushing ID and drill catalog filtering for the native window.
use super::bushing_id_persistence::{self, PersistedBushingId};
use super::model::BushingModel;
use crate::library::LibraryItem;
use bushing_solver::drills::{self, DrillEntry};

/// One selectable row: a user-saved size or a standard drill.
#[derive(Clone, Copy)]
pub(crate) enum IdRow<'a> {
    User(&'a LibraryItem<PersistedBushingId>),
    Drill(&'static DrillEntry),
}

impl IdRow<'_> {
    /// What the row is called: the saved label or the drill's.
    pub(crate) fn label(&self) -> String {
        match self {
            IdRow::User(li) => li.item.label.clone(),
            IdRow::Drill(d) => d.label.clone(),
        }
    }
    /// A short note under the label: a saved size's tags, or the drill's kind.
    pub(crate) fn detail(&self) -> String {
        match self {
            IdRow::User(li) => format!(
                "saved{}",
                if li.labels.is_empty() {
                    String::new()
                } else {
                    format!(", {}", li.labels.join(", "))
                }
            ),
            IdRow::Drill(d) => format!(
                "{} drill{}",
                d.kind.name(),
                if d.common { ", common" } else { "" }
            ),
        }
    }
    pub(crate) fn id_in(&self) -> f64 {
        match self {
            IdRow::User(li) => li.item.id_in,
            IdRow::Drill(d) => d.nominal_in,
        }
    }

    fn search_text(&self) -> String {
        match self {
            IdRow::User(li) => {
                format!("{} saved {}", li.item.label, li.labels.join(" ")).to_lowercase()
            }
            IdRow::Drill(d) => format!(
                "{} {} drill {}",
                d.label,
                d.kind.name(),
                if d.common { "common" } else { "" }
            )
            .to_lowercase(),
        }
    }
}

pub struct BushingIdPickerState {
    pub cursor: usize,
    pub filter_text: String,
    pub library: Vec<LibraryItem<PersistedBushingId>>,
}

impl BushingIdPickerState {
    pub fn open_near(model: &BushingModel) -> Self {
        let mut state = Self {
            cursor: 0,
            filter_text: String::new(),
            library: bushing_id_persistence::load(),
        };
        let target = model.id_bushing;
        state.cursor = state
            .visible()
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.id_in() - target)
                    .abs()
                    .total_cmp(&(b.id_in() - target).abs())
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        state
    }

    pub(crate) fn visible(&self) -> Vec<IdRow<'_>> {
        let needle = self.filter_text.trim().to_lowercase();
        let user = self.library.iter().map(IdRow::User);
        let drills = drills::all_drills().into_iter().map(IdRow::Drill);
        let numeric = super::size_filter::is_numeric(&needle);
        let all = user.chain(drills);
        if numeric {
            super::size_filter::select_numeric(all.collect(), &needle, |r| r.id_in())
        } else {
            all.filter(|r| needle.is_empty() || r.search_text().contains(&needle))
                .collect()
        }
    }
}
