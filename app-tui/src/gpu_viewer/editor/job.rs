//! Background solve for the editor: one job at a time, stale or cancelled results are discarded.
//! The kernel solve itself is not interruptible, so "cancel" means "ignore the answer"; the
//! worker thread finishes and exits on its own.
use fea_problem::members::{MemberProblem, MemberSolved};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

type Answer = Result<MemberSolved, String>;

struct Running {
    problem: MemberProblem,
    started: Instant,
    rx: Receiver<Answer>,
    cancelled: bool,
}

#[derive(Debug)]
pub enum Outcome {
    /// Accepted result for `problem` (the snapshot the worker solved).
    Finished { problem: Box<MemberProblem>, solved: Box<MemberSolved> },
    /// The solver refused the model or failed its acceptance gate.
    Failed(String),
    /// The job was cancelled; its answer was dropped.
    Cancelled,
}

#[derive(Default)]
pub struct Runner {
    running: Option<Running>,
}

impl Runner {
    pub fn is_running(&self) -> bool {
        self.running.as_ref().is_some_and(|r| !r.cancelled)
    }
    /// A cancelled job's thread may still be finishing; a new job waits for it to avoid piling up threads.
    pub fn is_busy(&self) -> bool {
        self.running.is_some()
    }
    pub fn elapsed(&self) -> Option<Duration> {
        self.running.as_ref().map(|r| r.started.elapsed())
    }
    pub fn start(&mut self, problem: MemberProblem) -> Result<(), String> {
        if self.is_busy() {
            return Err("a solve is still running".into());
        }
        let (tx, rx) = channel();
        let snapshot = problem.clone();
        std::thread::Builder::new()
            .name("member-solve".into())
            .spawn(move || {
                let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| snapshot.solve()))
                    .unwrap_or_else(|_| Err("the solver stopped unexpectedly".into()));
                let _ = tx.send(answer);
            })
            .map_err(|e| format!("cannot start the solver thread: {e}"))?;
        self.running = Some(Running { problem, started: Instant::now(), rx, cancelled: false });
        Ok(())
    }
    pub fn cancel(&mut self) -> bool {
        match &mut self.running {
            Some(r) if !r.cancelled => {
                r.cancelled = true;
                true
            }
            _ => false,
        }
    }
    /// Non-blocking: `Some` exactly once when the job ends.
    pub fn poll(&mut self) -> Option<Outcome> {
        let r = self.running.as_ref()?;
        let answer = match r.rx.try_recv() {
            Ok(a) => a,
            Err(TryRecvError::Empty) => return None,
            Err(TryRecvError::Disconnected) => Err("the solver stopped unexpectedly".into()),
        };
        let r = self.running.take().expect("checked above");
        Some(match (r.cancelled, answer) {
            (true, _) => Outcome::Cancelled,
            (false, Ok(solved)) => Outcome::Finished { problem: Box::new(r.problem), solved: Box::new(solved) },
            (false, Err(e)) => Outcome::Failed(e),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fea_problem::members::{templates, Edit};

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
