//! Workbenches of the native CAD window. Each bench is one kind of study with its own ribbon,
//! menus and property content (like the SolidWorks Simulation study types), so a bench shows only
//! what applies to it. The model files stay shared: the three solid benches edit the same
//! `fea_problem::Problem` (the Workbench's file format), the member bench edits a frame/truss
//! model. See `docs/adr/ADR-014-native-cad-process-model.md`.
use super::controller::UiAction;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Bench {
    /// Frame / truss members.
    Members,
    /// Linear static stress of a meshed solid or plate.
    Static,
    /// Natural frequencies and mode shapes.
    Modal,
    /// Linear buckling load factors and shapes.
    Buckling,
    /// Free vibration after sudden removal of the load.
    Transient,
}

impl Bench {
    pub const ALL: [Bench; 5] = [Bench::Members, Bench::Static, Bench::Modal, Bench::Buckling, Bench::Transient];

    pub fn label(self) -> &'static str {
        match self {
            Bench::Members => "Frame and truss",
            Bench::Static => "Static stress",
            Bench::Modal => "Modal / vibration",
            Bench::Buckling => "Buckling",
            Bench::Transient => "Transient",
        }
    }
    pub fn summary(self) -> &'static str {
        match self {
            Bench::Members => "nodes, truss and frame members, supports, loads",
            Bench::Static => "sketch, holes, supports, loads, stress and displacement",
            Bench::Modal => "natural frequencies and animated mode shapes",
            Bench::Buckling => "buckling load factors and shapes",
            Bench::Transient => "free vibration after the load is released",
        }
    }
    /// Benches that edit a meshed solid share one model and session.
    pub fn is_solid(self) -> bool {
        self != Bench::Members
    }
    /// Short tag for window titles and file-free state (advisor files are per bench).
    pub fn slug(self) -> &'static str {
        match self {
            Bench::Members => "members",
            Bench::Static => "static",
            Bench::Modal => "modal",
            Bench::Buckling => "buckling",
            Bench::Transient => "transient",
        }
    }
    pub fn from_slug(s: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|b| b.slug() == s)
    }
    pub fn action(self) -> UiAction {
        UiAction::SwitchBench(self)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn slugs_round_trip_and_only_members_is_not_solid() {
        for b in Bench::ALL {
            assert_eq!(Bench::from_slug(b.slug()), Some(b));
        }
        assert_eq!(Bench::ALL.iter().filter(|b| !b.is_solid()).count(), 1);
        assert_eq!(Bench::from_slug("nope"), None);
    }
}
