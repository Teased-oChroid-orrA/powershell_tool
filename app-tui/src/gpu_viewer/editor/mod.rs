//! Native member editor: pure state, picking, scene building and capability rules. The winit
//! runner (`window.rs`) only forwards input here and draws the result.
pub mod camera;
pub mod scene;
pub mod state;
pub mod caps;
pub mod job;
pub mod session;
pub mod advisory;
pub mod authoring;
pub mod worker;
pub mod learning;
pub mod rank;
pub mod adapt;
pub mod bushing;
pub mod eccentric;
pub mod fastener;
pub mod handoff;
pub mod search;
pub mod stackup;
pub mod stackup_transfer;
pub mod solid;
pub mod sketcher;

use crate::gpu_viewer::bench::Bench;
use authoring::Authoring;
use std::path::{Path, PathBuf};

/// File a bench saves to, beside the current model.
pub fn bench_file(dir: &Path, bench: Bench) -> PathBuf {
    dir.join(if bench.is_solid() { "solid-model.json" } else { "member-model.json" })
}

/// A member session with the advisory environment and the opt-in learning loop attached.
pub fn member_session(problem: fea_problem::members::MemberProblem, path: PathBuf) -> session::Session {
    let mut ed = session::Session::new(problem, path);
    let (advisor, enabled, note) = advisory::from_environment();
    ed.configure_advice(advisor, enabled, note);
    if let Some(store) = learning::Store::for_bench(Bench::Members) {
        let id = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |d| d.as_millis() as u64);
        ed.attach_learning(store, id);
    }
    ed
}

/// Open `bench` on `path`: loads the file when it exists, otherwise starts a blank / template model.
pub fn open(bench: Bench, path: &Path) -> Result<Box<dyn Authoring>, String> {
    if bench == Bench::Members {
        let problem = if path.exists() { state::read_problem(path)? } else { fea_problem::members::blank_planar() };
        problem.validate()?;
        Ok(Box::new(member_session(problem, path.to_path_buf())))
    } else {
        let (problem, destination) = solid::open_path(path)?;
        let mut ed = solid::SolidSession::new(problem, destination);
        ed.set_bench(bench);
        Ok(Box::new(ed))
    }
}
