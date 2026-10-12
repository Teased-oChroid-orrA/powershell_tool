//! Deterministic operation availability and next-step guidance. Every rule is an explicit check on
//! the editor state, so the same state always yields the same answer and the same explanation.
//! This is rule-based filtering, not machine learning.
use super::camera::Hit;
use super::state::EditorState;

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, serde::Serialize, serde::Deserialize)]
pub enum Operation {
    Undo,
    Redo,
    Delete,
    Solve,
    CancelSolve,
    Save,
    ConvertToFrame,
    ConvertToTruss,
    ApplyMoment,
    FixRotations,
    ShowResults,
    TogglePlanar,
}
impl Operation {
    pub const ALL: [Operation; 12] = [
        Operation::Solve,
        Operation::CancelSolve,
        Operation::ShowResults,
        Operation::Undo,
        Operation::Redo,
        Operation::Delete,
        Operation::Save,
        Operation::ConvertToFrame,
        Operation::ConvertToTruss,
        Operation::ApplyMoment,
        Operation::FixRotations,
        Operation::TogglePlanar,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Operation::Undo => "Undo (Ctrl/Cmd+Z)",
            Operation::Redo => "Redo (Ctrl/Cmd+Shift+Z)",
            Operation::Delete => "Delete selection (Del)",
            Operation::Solve => "Solve (Enter)",
            Operation::CancelSolve => "Cancel solve (Esc)",
            Operation::Save => "Save (Ctrl/Cmd+S)",
            Operation::ConvertToFrame => "Make member a frame (B)",
            Operation::ConvertToTruss => "Make member a truss (T)",
            Operation::ApplyMoment => "Apply a moment",
            Operation::FixRotations => "Fix rotations",
            Operation::ShowResults => "Show results (V)",
            Operation::TogglePlanar => "Planar / 3D analysis (K)",
        }
    }
}

/// `Ok` when the operation can run now; `Err` carries the reason it cannot.
pub fn availability(state: &EditorState, solving: bool, op: Operation) -> Result<(), String> {
    let p = &state.problem;
    let node_with_frame = match state.selection {
        Some(Hit::Node(id)) => Some(p.has_frame(id)),
        _ => None,
    };
    match op {
        Operation::Undo => state.can_undo().then_some(()).ok_or_else(|| "nothing to undo".into()),
        Operation::Redo => state.can_redo().then_some(()).ok_or_else(|| "nothing to redo".into()),
        Operation::Delete => state.selection.map(|_| ()).ok_or_else(|| "select a node or member first".into()),
        Operation::Save => if state.dirty { Ok(()) } else { Err("no changes since the last save".into()) },
        Operation::Solve => {
            if solving {
                Err("a solve is already running (Esc cancels it)".into())
            } else if p.members.is_empty() {
                Err("add at least one member (tools 3 or 4)".into())
            } else if p.supports.is_empty() {
                Err("add a support (tool 5) so the structure cannot move freely".into())
            } else if p.loads.is_empty() && !p.members.iter().any(|m| matches!(m.kind, fea_problem::members::MemberKind::Frame { uniform_load, .. } if uniform_load.iter().any(|q| *q != 0.0))) {
                Err("add a load (tool 6) or a distributed load; with none every result is zero".into())
            } else if state.current_solution().is_some() {
                Err("results are already current for this model".into())
            } else {
                Ok(())
            }
        }
        Operation::TogglePlanar => {
            let target = if p.plane.is_some() { None } else { Some(state.plane_axis) };
            p.clone().apply(fea_problem::members::Edit::SetPlane { axis: target }).map(|_| ())
        }
        Operation::CancelSolve => if solving { Ok(()) } else { Err("no solve is running".into()) },
        Operation::ShowResults => if state.current_solution().is_some() {
            Ok(())
        } else if state.solution_is_stale() {
            Err("the model changed after the last solve; solve again".into())
        } else {
            Err("solve the model first".into())
        },
        Operation::ConvertToFrame | Operation::ConvertToTruss => {
            let want_frame = op == Operation::ConvertToFrame;
            match state.selection {
                Some(Hit::Member(id)) => match p.member(id).map(|m| m.kind.is_frame()) {
                    Some(is) if is == want_frame => Err(format!("the selected member is already a {}", if is { "frame" } else { "truss" })),
                    Some(_) => Ok(()),
                    None => Err("select a member first".into()),
                },
                _ => Err("select a member first".into()),
            }
        }
        Operation::ApplyMoment | Operation::FixRotations => match node_with_frame {
            Some(true) => Ok(()),
            Some(false) => Err("this node has only truss members, which carry no rotation: attach a frame member".into()),
            None => Err("select a node first".into()),
        },
    }
}

pub fn all(state: &EditorState, solving: bool) -> Vec<(Operation, Result<(), String>)> {
    Operation::ALL.iter().map(|op| (*op, availability(state, solving, *op))).collect()
}

/// The single most useful next action for the current state.
pub fn next_step(state: &EditorState, solving: bool) -> String {
    let p = &state.problem;
    if solving {
        "Solving... edits are allowed; the finished result is discarded if the model changed.".into()
    } else if p.nodes.is_empty() {
        "Press 2, then click the view to place nodes (grid-snapped).".into()
    } else if p.members.is_empty() {
        "Press 3 (truss) or 4 (frame), then click two nodes to connect them.".into()
    } else if p.supports.is_empty() {
        "Press 5 and click a node to support it (click again to cycle fixed / pinned).".into()
    } else if p.loads.is_empty() {
        "Press 6 and click a node to apply the default load; edit values with Enter.".into()
    } else if state.current_solution().is_some() {
        "Result shown: F cycles stress / displacement / moment fields; edit the model to continue.".into()
    } else {
        "Press Enter to solve.".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::editor::state::Tool;
    use fea_problem::members::{templates, MemberProblem};

    fn ok(s: &EditorState, op: Operation) -> bool {
        availability(s, false, op).is_ok()
    }

    #[test]
    fn solve_requires_members_supports_and_loads_with_explanations() {
        let mut s = EditorState::new(MemberProblem::new("e"));
        assert!(availability(&s, false, Operation::Solve).unwrap_err().contains("member"));
        s.set_tool(Tool::Truss);
        s.click(None, Some([0.0, 0.0, 0.0])).unwrap();
        s.click(None, Some([50.0, 0.0, 0.0])).unwrap();
        assert!(availability(&s, false, Operation::Solve).unwrap_err().contains("support"));
        s.set_tool(Tool::Support);
        s.click(Some(Hit::Node(1)), None).unwrap();
        s.apply_support(1, crate::gpu_viewer::editor::state::SupportPreset::Fixed).unwrap();
        assert!(availability(&s, false, Operation::Solve).unwrap_err().contains("load"));
        s.set_tool(Tool::Load);
        s.click(Some(Hit::Node(2)), None).unwrap();
        s.apply_default_load(2).unwrap();
        assert!(ok(&s, Operation::Solve));
        assert!(availability(&s, true, Operation::Solve).unwrap_err().contains("already running"));
        assert!(ok(&s, Operation::Undo) && !ok(&s, Operation::Redo));
        assert!(ok(&s, Operation::Save), "dirty after edits");
    }
    #[test]
    fn solved_model_reports_current_stale_and_available_results() {
        let mut s = EditorState::new(templates().remove(0));
        assert!(availability(&s, false, Operation::ShowResults).unwrap_err().contains("solve"));
        let solved = s.problem.solve().unwrap();
        s.accept_solution(solved, s.problem.clone());
        assert!(ok(&s, Operation::ShowResults));
        assert!(availability(&s, false, Operation::Solve).unwrap_err().contains("current"));
        s.select(Some(Hit::Node(4)));
        s.set_property(crate::gpu_viewer::editor::state::PropertyKind::Force(2), -5.0).unwrap();
        assert!(availability(&s, false, Operation::ShowResults).unwrap_err().contains("changed"));
        assert!(ok(&s, Operation::Solve));
    }
    #[test]
    fn rotational_operations_depend_on_frame_members() {
        let mut t = EditorState::new(templates().remove(0));
        assert!(availability(&t, false, Operation::ApplyMoment).unwrap_err().contains("select a node"));
        t.select(Some(Hit::Node(4)));
        assert!(availability(&t, false, Operation::ApplyMoment).unwrap_err().contains("truss"));
        assert!(!ok(&t, Operation::FixRotations));
        let mut f = EditorState::new(templates().remove(1));
        f.select(Some(Hit::Node(2)));
        assert!(ok(&f, Operation::ApplyMoment) && ok(&f, Operation::FixRotations));
        // The capability rule agrees with what the problem actually accepts.
        assert!(f.set_property(crate::gpu_viewer::editor::state::PropertyKind::Moment(2), 5.0).is_ok());
        assert!(t.set_property(crate::gpu_viewer::editor::state::PropertyKind::Moment(2), 5.0).is_err());
    }
    #[test]
    fn conversion_and_delete_follow_the_selection() {
        let mut s = EditorState::new(templates().remove(1));
        assert!(!ok(&s, Operation::Delete) && !ok(&s, Operation::ConvertToTruss));
        s.select(Some(Hit::Member(3)));
        assert!(ok(&s, Operation::Delete) && ok(&s, Operation::ConvertToTruss));
        assert!(availability(&s, false, Operation::ConvertToFrame).unwrap_err().contains("already a frame"));
        s.convert_selected(false).unwrap();
        assert!(ok(&s, Operation::ConvertToFrame) && !ok(&s, Operation::ConvertToTruss));
    }
    #[test]
    fn every_available_operation_is_actually_executable() {
        // Consistency between the filter and the state machine for the edit operations.
        let mut s = EditorState::new(templates().remove(1));
        s.select(Some(Hit::Member(3)));
        for (op, a) in all(&s, false) {
            match (op, a.is_ok()) {
                (Operation::Delete, true) => {
                    let mut c = EditorState::new(s.problem.clone());
                    c.select(s.selection);
                    assert!(c.delete_selected().is_ok());
                }
                (Operation::ConvertToTruss, true) => {
                    let mut c = EditorState::new(s.problem.clone());
                    c.select(s.selection);
                    assert!(c.convert_selected(false).is_ok());
                }
                (Operation::ConvertToFrame, false) => {
                    let mut c = EditorState::new(s.problem.clone());
                    c.select(s.selection);
                    assert!(c.convert_selected(true).is_err());
                }
                _ => {}
            }
        }
    }
    #[test]
    fn next_step_walks_the_authoring_workflow() {
        let mut s = EditorState::new(MemberProblem::new("e"));
        assert!(next_step(&s, false).contains("place nodes"));
        s.set_tool(Tool::Truss);
        s.click(None, Some([0.0, 0.0, 0.0])).unwrap();
        s.click(None, Some([50.0, 0.0, 0.0])).unwrap();
        assert!(next_step(&s, false).contains("5"));
        s.set_tool(Tool::Support);
        s.click(Some(Hit::Node(1)), None).unwrap();
        s.apply_support(1, crate::gpu_viewer::editor::state::SupportPreset::Fixed).unwrap();
        assert!(next_step(&s, false).contains("load"));
        s.set_tool(Tool::Load);
        s.click(Some(Hit::Node(2)), None).unwrap();
        s.apply_default_load(2).unwrap();
        assert_eq!(next_step(&s, false), "Press Enter to solve.");
        assert!(next_step(&s, true).starts_with("Solving"));
        let n = Operation::ALL.len();
        assert_eq!(all(&s, false).len(), n);
    }
}
