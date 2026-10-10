//! Worker progress is stage-based until a stage has measurable completed work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GenerationProgress {
    pub stage: &'static str,
    pub completed: usize,
    pub total: usize,
}
impl GenerationProgress {
    pub fn stage(stage: &'static str) -> Self {
        Self {
            stage,
            completed: 0,
            total: 0,
        }
    }
    pub fn text(&self) -> String {
        if self.total == 0 {
            self.stage.to_string()
        } else {
            format!(
                "{}: {}/{} ({:.0}%)",
                self.stage,
                self.completed.min(self.total),
                self.total,
                100.0 * self.completed.min(self.total) as f64 / self.total as f64
            )
        }
    }
}
