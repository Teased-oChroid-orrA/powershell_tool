//! Reamer catalog filtering for the native Bushing window.
use super::model::BushingModel;
use super::reamer_persistence::{self, PersistedReamer};
use crate::library::LibraryItem;
use bushing_solver::reamers::{self, ReamerEntry};

pub struct ReamerPickerState {
    pub cursor: usize,
    pub filter_text: String,
    pub library: Vec<LibraryItem<PersistedReamer>>,
}

impl ReamerPickerState {
    pub fn open_near(model: &BushingModel) -> Self {
        let mut state = Self {
            cursor: 0,
            filter_text: String::new(),
            library: reamer_persistence::load(),
        };
        let target = model.bore_dia;
        state.cursor = state
            .visible()
            .iter()
            .enumerate()
            .min_by(|(_, a), (_, b)| {
                (a.0.nominal_in - target)
                    .abs()
                    .total_cmp(&(b.0.nominal_in - target).abs())
            })
            .map(|(i, _)| i)
            .unwrap_or(0);
        state
    }

    pub(crate) fn visible(&self) -> Vec<(ReamerEntry, Vec<String>)> {
        let needle = self.filter_text.trim().to_lowercase();
        let mut all: Vec<(ReamerEntry, Vec<String>)> = reamers::all_reamers()
            .into_iter()
            .map(|e| (e.clone(), Vec::new()))
            .collect();
        for li in &self.library {
            match all
                .iter_mut()
                .find(|(e, _)| PersistedReamer::from_entry(e) == li.item)
            {
                Some((_, labels)) => {
                    for l in &li.labels {
                        if !labels.contains(l) {
                            labels.push(l.clone());
                        }
                    }
                }
                None => all.push((li.item.to_entry(), li.labels.clone())),
            }
        }
        if !needle.is_empty() {
            if super::size_filter::is_numeric(&needle) {
                all = super::size_filter::select_numeric(all, &needle, |(e, _)| e.nominal_in);
            } else {
                all.retain(|(e, _)| e.size_label.to_lowercase().contains(&needle));
            }
        }
        all
    }
}
