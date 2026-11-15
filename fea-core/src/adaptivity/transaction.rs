//! Adaptation step with rollback: a candidate state is built from the current one, checked, and either replaces it or is
//! discarded with the reason, leaving the current state untouched. Nothing continues from an unchecked candidate.

/// Outcome of [`attempt`].
#[derive(Debug)]
pub enum Attempt<S> {
    /// The candidate passed the check and is the new current state.
    Committed(S),
    /// The step failed (building or checking the candidate); `kept` is the unchanged previous state.
    RolledBack { kept: S, reason: String },
}

impl<S> Attempt<S> {
    /// The state to continue from, whichever way the step went.
    pub fn state(self) -> S {
        match self {
            Self::Committed(s) | Self::RolledBack { kept: s, .. } => s,
        }
    }

    pub fn committed(&self) -> bool {
        matches!(self, Self::Committed(_))
    }
}

/// Build a candidate from `current` with `build`, accept it with `accept(current, candidate)`; a failure at either step rolls
/// back to `current`.
pub fn attempt<S>(current: S, build: impl FnOnce(&S) -> Result<S, String>, accept: impl FnOnce(&S, &S) -> Result<(), String>) -> Attempt<S> {
    match build(&current) {
        Err(reason) => Attempt::RolledBack { kept: current, reason },
        Ok(candidate) => match accept(&current, &candidate) {
            Ok(()) => Attempt::Committed(candidate),
            Err(reason) => Attempt::RolledBack { kept: current, reason: format!("candidate rejected: {reason}") },
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_rejected_candidate_leaves_the_current_state() {
        let r = attempt(5, |s| Ok(s + 1), |_, c| if *c < 6 { Ok(()) } else { Err("too big".into()) });
        match r {
            Attempt::RolledBack { kept, reason } => {
                assert_eq!(kept, 5);
                assert!(reason.contains("too big"));
            }
            Attempt::Committed(_) => panic!("must roll back"),
        }
        assert_eq!(attempt(5, |s| Ok(s + 1), |_, _| Ok(())).state(), 6);
        let failed = attempt(5, |_| Err("mesher failed".to_string()), |_, _| Ok(()));
        assert!(!failed.committed());
        assert_eq!(failed.state(), 5);
    }
}
