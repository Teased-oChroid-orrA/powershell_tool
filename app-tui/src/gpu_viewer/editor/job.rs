//! Background solve for the editor: one job at a time, stale or cancelled results are discarded.
//! The kernel solve itself is not interruptible, so "cancel" means "ignore the answer"; the
//! worker thread finishes and exits on its own. The thread, panic and channel handling is the shared
//! [`Worker`]; this adds the problem snapshot a result belongs to and the one-shot `Cancelled` outcome.
use super::worker::Worker;
use fea_problem::members::{MemberProblem, MemberSolved};
use std::time::Duration;

#[derive(Debug)]
pub enum Outcome {
    /// Accepted result for `problem` (the snapshot the worker solved).
    Finished { problem: Box<MemberProblem>, solved: Box<MemberSolved> },
    /// The solver refused the model or failed its acceptance gate.
    Failed(String),
    /// The job was cancelled; its answer was dropped.
    Cancelled,
}

pub struct Runner {
    worker: Worker<MemberSolved>,
    /// The snapshot of the job in flight (also while a cancelled one winds down).
    problem: Option<MemberProblem>,
    cancelled: bool,
}

impl Default for Runner {
    fn default() -> Self {
        Self { worker: Worker::new("member-solve"), problem: None, cancelled: false }
    }
}

impl Runner {
    pub fn is_running(&self) -> bool {
        self.worker.is_running()
    }
    /// A cancelled job's thread may still be finishing; a new job waits for it to avoid piling up threads.
    pub fn is_busy(&self) -> bool {
        self.worker.is_busy()
    }
    pub fn elapsed(&self) -> Option<Duration> {
        self.worker.elapsed()
    }
    pub fn start(&mut self, problem: MemberProblem) -> Result<(), String> {
        if self.is_busy() {
            return Err("a solve is still running".into());
        }
        let snapshot = problem.clone();
        self.worker.start(0, move || snapshot.solve())?;
        self.problem = Some(problem);
        self.cancelled = false;
        Ok(())
    }
    pub fn cancel(&mut self) -> bool {
        let wanted = self.worker.cancel();
        self.cancelled |= wanted;
        wanted
    }
    /// Non-blocking: `Some` exactly once when the job ends.
    pub fn poll(&mut self) -> Option<Outcome> {
        if let Some((_, answer)) = self.worker.poll() {
            let problem = self.problem.take().expect("a job in flight has its snapshot");
            return Some(match answer {
                Ok(solved) => Outcome::Finished { problem: Box::new(problem), solved: Box::new(solved) },
                Err(e) => Outcome::Failed(e),
            });
        }
        // `Worker::poll` drops a cancelled answer silently; report it once, when the thread is gone.
        if self.cancelled && !self.worker.is_busy() {
            self.cancelled = false;
            self.problem = None;
            return Some(Outcome::Cancelled);
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_problem::members::{templates, Edit};
    use std::time::Instant;

    fn wait(r: &mut Runner) -> Outcome {
        let t = Instant::now();
        loop {
            if let Some(o) = r.poll() {
                return o;
            }
            assert!(t.elapsed() < Duration::from_secs(20), "solve did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
    }

    #[test]
    fn finished_job_returns_the_solved_snapshot_once() {
        let mut r = Runner::default();
        let p = templates().remove(0);
        r.start(p.clone()).unwrap();
        assert!(r.is_running() && r.elapsed().is_some());
        assert!(r.start(p.clone()).is_err(), "one job at a time");
        let Outcome::Finished { problem, solved } = wait(&mut r) else { panic!("expected a result") };
        assert_eq!(*problem, p);
        assert!(solved.acceptance.force_balance < 1e-10);
        assert!(r.poll().is_none() && !r.is_busy());
    }
    #[test]
    fn cancelled_job_drops_its_answer() {
        let mut r = Runner::default();
        r.start(templates().remove(1)).unwrap();
        assert!(r.cancel() && !r.cancel());
        assert!(!r.is_running() && r.is_busy(), "thread still winding down");
        assert!(matches!(wait(&mut r), Outcome::Cancelled));
        r.start(templates().remove(0)).unwrap();
        assert!(matches!(wait(&mut r), Outcome::Finished { .. }));
    }
    #[test]
    fn solver_refusals_are_reported_not_lost() {
        let mut r = Runner::default();
        let mut p = templates().remove(0);
        for s in p.supports.clone() {
            p.apply(Edit::SetSupport { node: s.node, fixed: [false; 6] }).unwrap();
        }
        r.start(p).unwrap();
        let Outcome::Failed(msg) = wait(&mut r) else { panic!("expected a failure") };
        assert!(!msg.is_empty());
        assert!(!r.cancel(), "nothing to cancel afterwards");
    }
}
