//! One background job at a time with a caller-chosen tag. The computation is not interruptible, so
//! cancelling (or starting a newer job's tag) means the answer is ignored when it arrives; the thread
//! finishes and exits on its own. A new job waits until the previous thread has finished, so threads
//! never pile up.
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::time::{Duration, Instant};

struct Running<T> {
    tag: u64,
    started: Instant,
    rx: Receiver<Result<T, String>>,
    cancelled: bool,
}

pub struct Worker<T> {
    name: &'static str,
    running: Option<Running<T>>,
}

impl<T: Send + 'static> Worker<T> {
    pub fn new(name: &'static str) -> Self {
        Self { name, running: None }
    }
    /// A job whose answer is still wanted.
    pub fn is_running(&self) -> bool {
        self.running.as_ref().is_some_and(|r| !r.cancelled)
    }
    /// A thread is still alive (possibly cancelled): no new job can start yet.
    pub fn is_busy(&self) -> bool {
        self.running.is_some()
    }
    pub fn tag(&self) -> Option<u64> {
        self.running.as_ref().filter(|r| !r.cancelled).map(|r| r.tag)
    }
    pub fn elapsed(&self) -> Option<Duration> {
        self.running.as_ref().map(|r| r.started.elapsed())
    }
    pub fn start(&mut self, tag: u64, job: impl FnOnce() -> Result<T, String> + Send + 'static) -> Result<(), String> {
        if self.is_busy() {
            return Err("a computation is still running".into());
        }
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name(self.name.into())
            .spawn(move || {
                let answer = std::panic::catch_unwind(std::panic::AssertUnwindSafe(job)).unwrap_or_else(|_| Err("the solver stopped unexpectedly".into()));
                let _ = tx.send(answer);
            })
            .map_err(|e| format!("cannot start the {} thread: {e}", self.name))?;
        self.running = Some(Running { tag, started: Instant::now(), rx, cancelled: false });
        Ok(())
    }
    /// Ignore the running job's answer. Returns true when a wanted job was running.
    pub fn cancel(&mut self) -> bool {
        match &mut self.running {
            Some(r) if !r.cancelled => {
                r.cancelled = true;
                true
            }
            _ => false,
        }
    }
    /// The finished job's `(tag, answer)`, once. Cancelled answers are dropped (`None`).
    pub fn poll(&mut self) -> Option<(u64, Result<T, String>)> {
        let r = self.running.as_ref()?;
        match r.rx.try_recv() {
            Ok(answer) => {
                let r = self.running.take()?;
                (!r.cancelled).then_some((r.tag, answer))
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                let r = self.running.take()?;
                (!r.cancelled).then(|| (r.tag, Err("the solver stopped unexpectedly".into())))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wait<T: Send + 'static>(w: &mut Worker<T>) -> Option<(u64, Result<T, String>)> {
        let t = Instant::now();
        while w.is_busy() {
            if let Some(a) = w.poll() {
                return Some(a);
            }
            assert!(t.elapsed().as_secs() < 10, "worker did not finish");
            std::thread::sleep(Duration::from_millis(2));
        }
        None
    }

    #[test]
    fn delivers_the_answer_once_with_its_tag() {
        let mut w = Worker::new("test");
        w.start(7, || Ok(41 + 1)).unwrap();
        assert_eq!(wait(&mut w), Some((7, Ok(42))));
        assert!(!w.is_busy());
        assert_eq!(w.poll(), None);
    }

    #[test]
    fn cancelled_answers_are_dropped_and_a_new_job_waits_for_the_thread() {
        let mut w = Worker::new("test");
        w.start(1, || {
            std::thread::sleep(Duration::from_millis(50));
            Ok(1)
        })
        .unwrap();
        assert!(w.is_running());
        assert!(w.cancel());
        assert!(!w.is_running() && w.is_busy());
        assert!(w.start(2, || Ok(2)).is_err(), "a cancelled thread still occupies the worker");
        assert_eq!(wait(&mut w), None, "the cancelled answer is dropped");
        w.start(3, || Ok(3)).unwrap();
        assert_eq!(wait(&mut w), Some((3, Ok(3))));
    }

    #[test]
    fn a_panicking_job_reports_an_error_instead_of_hanging() {
        let mut w: Worker<u8> = Worker::new("test");
        w.start(9, || panic!("boom")).unwrap();
        let (tag, answer) = wait(&mut w).unwrap();
        assert_eq!((tag, answer), (9, Err("the solver stopped unexpectedly".to_string())));
    }
}
