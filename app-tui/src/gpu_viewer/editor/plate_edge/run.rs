//! The calculations of the Plate Edge window, on a worker thread: the allowables (`edge_check::modes`) and the FE bracket
//! (`lug_solver::bracket`, 2D plane strain and plane stress, and 3D). A job reports each mesh as it finishes, can be cancelled,
//! and ends with an [`Outcome`] tagged with the inputs it ran for, so a result for changed inputs is recognised as stale.
use super::inputs::{MeshMode, PlateEdgeInputs};
use edge_check::modes::{self, ModesResult};
use lug_solver::bracket::{self, Case, Fields, Idealisation, Refinement};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, TryRecvError};
use std::sync::Arc;
use std::time::Instant;

/// What a run computes.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Kind {
    /// The closed forms and the elastic stress field (about 0.3 s): what updates while typing.
    Quick,
    /// Also the contact FE and the 2D bracket (a few seconds).
    TwoD,
    /// And the 3D solid (minutes).
    ThreeD,
}

/// One FE idealisation's result: every mesh it ran on, the last being the answer.
pub struct FeLine {
    pub label: &'static str,
    pub meshes: Vec<Refinement>,
    pub converged: bool,
    /// The deformed model of the last mesh, for the picture.
    pub fields: Option<Fields>,
}

impl FeLine {
    pub fn last(&self) -> &Refinement {
        self.meshes.last().expect("a line has a mesh")
    }
}

/// A finished run.
pub struct Outcome {
    pub kind: Kind,
    /// The inputs it ran for.
    pub inputs: PlateEdgeInputs,
    pub modes: Result<ModesResult, String>,
    pub lines: Vec<FeLine>,
    /// Why the FE did not run, if it did not (a fit the bracket cannot model, a failed solve).
    pub fe_note: Option<String>,
    pub secs: f64,
}

pub enum Msg {
    /// A mesh finished: the idealisation and its label for the status line.
    Mesh(&'static str, Refinement),
    Done(Outcome),
}

pub struct Job {
    pub kind: Kind,
    pub started: Instant,
    rx: Receiver<Msg>,
    cancel: Arc<AtomicBool>,
    /// What has finished so far, for the status line.
    pub progress: Vec<(&'static str, Refinement)>,
}

impl Job {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Relaxed);
    }

    /// Drains what the worker has sent: `Some(outcome)` when it is done, `Some(None)` if the worker died.
    pub fn poll(&mut self) -> Option<Option<Outcome>> {
        loop {
            match self.rx.try_recv() {
                Ok(Msg::Mesh(label, r)) => self.progress.push((label, r)),
                Ok(Msg::Done(o)) => return Some(Some(o)),
                Err(TryRecvError::Empty) => return None,
                Err(TryRecvError::Disconnected) => return Some(None),
            }
        }
    }
}

/// Validates the inputs and starts the job.
pub fn start(inputs: &PlateEdgeInputs, kind: Kind) -> Result<Job, String> {
    // The allowables' input is checked here so a bad case is refused before a thread starts.
    inputs.modes_input(false)?;
    if kind == Kind::ThreeD && inputs.bracket_refusal().is_none() {
        inputs.bracket_case()?;
    }
    let (tx, rx) = channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let (inp, flag) = (inputs.clone(), cancel.clone());
    std::thread::Builder::new()
        .name("plate-edge".into())
        .spawn(move || {
            let t0 = Instant::now();
            let out = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| work(&inp, kind, &flag, &tx)));
            let out = out.unwrap_or_else(|_| Outcome { kind, inputs: inp.clone(), modes: Err("the calculation failed".into()), lines: Vec::new(), fe_note: Some("the calculation failed".into()), secs: t0.elapsed().as_secs_f64() });
            let _ = tx.send(Msg::Done(out));
        })
        .map_err(|e| format!("could not start the calculation: {e}"))?;
    Ok(Job { kind, started: Instant::now(), rx, cancel, progress: Vec::new() })
}

/// The meshes of one idealisation: one mesh, or finer ones until the loads stop moving.
fn line(label: &'static str, inp: &PlateEdgeInputs, case: Case, ideal: Idealisation, cancel: &Arc<AtomicBool>, tx: &std::sync::mpsc::Sender<Msg>) -> Result<FeLine, String> {
    let (case, ideal, max) = match inp.mesh {
        MeshMode::Graded => (case, ideal, 1),
        MeshMode::Converge => (case, ideal, if matches!(ideal, Idealisation::Solid { .. }) { 3 } else { 4 }),
        MeshMode::Fine => {
            let ideal = match ideal {
                Idealisation::Solid { lock_z, .. } => Idealisation::Solid { layers: 4, lock_z },
                other => other,
            };
            (Case { around: 72, ..case }, ideal, 1)
        }
    };
    let mut last_fields: Option<Fields> = None;
    let r = bracket::converge_with(case, ideal, inp.tol_percent / 100.0, max, Some(cancel.clone()), &mut |r, fields| {
        let _ = tx.send(Msg::Mesh(label, r.clone()));
        last_fields = fields;
    })?;
    Ok(FeLine { label, converged: r.converged, meshes: r.steps, fields: last_fields })
}

fn work(inp: &PlateEdgeInputs, kind: Kind, cancel: &Arc<AtomicBool>, tx: &std::sync::mpsc::Sender<Msg>) -> Outcome {
    let t0 = Instant::now();
    let modes = inp.modes_input(kind != Kind::Quick).and_then(|i| modes::analyse(&i));
    let mut lines = Vec::new();
    let mut fe_note = None;
    if kind != Kind::Quick {
        match inp.bracket_case() {
            Err(why) => fe_note = Some(why),
            Ok(case) => {
                let lines_ref = &mut lines;
                let mut run = |label: &'static str, ideal: Idealisation| -> Option<String> {
                    match line(label, inp, case, ideal, cancel, tx) {
                        Ok(l) => {
                            lines_ref.push(l);
                            None
                        }
                        Err(e) => Some(format!("{label}: {e}")),
                    }
                };
                // Each idealisation stands alone: one that stalls (plane stress of a yielded wall can) does not stop the others.
                let mut notes: Vec<String> = [run("2D plane strain", Idealisation::Strain), run("2D plane stress", Idealisation::Stress)].into_iter().flatten().collect();
                if kind == Kind::ThreeD && !cancel.load(Ordering::Relaxed) {
                    notes.extend(run("3D solid", inp.solid()));
                }
                fe_note = (!notes.is_empty()).then(|| notes.join("; "));
            }
        }
    }
    Outcome { kind, inputs: inp.clone(), modes, lines, fe_note, secs: t0.elapsed().as_secs_f64() }
}
