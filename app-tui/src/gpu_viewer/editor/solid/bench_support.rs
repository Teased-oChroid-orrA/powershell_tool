//! The solid editor's benches: static stress, modal, buckling and transient share one model and
//! differ in which study the Simulation menu and ribbon offer.
use super::*;

impl SolidSession {
    /// The dynamic study a bench runs (`None` for the static bench).
    pub fn bench_study(bench: Bench) -> Option<StudyKind> {
        match bench {
            Bench::Modal => Some(StudyKind::Modal),
            Bench::Buckling => Some(StudyKind::Buckling),
            Bench::Transient => Some(StudyKind::Transient),
            Bench::Static | Bench::Members => None,
        }
    }
    /// Switch the bench; the displayed study is hidden when it belongs to another bench.
    pub fn set_bench(&mut self, bench: Bench) {
        debug_assert!(bench.is_solid());
        self.bench = bench;
        self.tool = SolidTool::Select;
        let keep = Self::bench_study(bench).is_some_and(|k| self.current_study().is_some_and(|s| s.kind == k));
        self.show_study = keep && self.show_study;
        if bench != Bench::Static {
            self.show_results = false;
        }
        self.touch();
    }
}
