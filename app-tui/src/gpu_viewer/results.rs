//! What the viewer itself contributes to the window besides the editor's own tree and property rows: the loaded
//! results (a tree folder, the active result's fields) and the result display section (field, legend, playback,
//! errors). Pure data, so it is tested without a window.
use super::{
    controller::{Command, UiAction, ViewState},
    editor::{
        authoring::{Authoring, Card, Relevance},
        session::Tone,
    },
    text::{wrap, HELP},
    ui::{DockItem, TreeNode},
    Scene,
};

/// Everything a frame of the front end reads.
pub struct GuiInput<'a> {
    pub view: &'a ViewState,
    pub scenes: &'a [Scene],
    pub selected: usize,
    pub editor: Option<&'a dyn Authoring>,
    pub error: Option<&'a str>,
    /// The selection box being dragged, in window pixels (press point, cursor).
    pub box_drag: Option<((f64, f64), (f64, f64))>,
    /// Labels the editor wants on the model (viewport pixels).
    pub overlay: &'a [crate::gpu_viewer::editor::authoring::Overlay],
}

impl GuiInput<'_> {
    pub fn scene(&self) -> &Scene {
        &self.scenes[self.selected.min(self.scenes.len().saturating_sub(1))]
    }
}

const RESULTS_FOLDER: u32 = 9_000;
const RESULT_BASE: u32 = 9_100;
const FIELD_BASE: u32 = 9_500;

fn clip(text: &str, n: usize) -> String {
    text.chars().take(n).collect()
}

/// The viewer's own tree part: loaded results and the fields of the active one.
pub fn results_tree(input: &GuiInput) -> Vec<TreeNode> {
    let scene = input.scene();
    if input.scenes.len() <= 1 && scene.fields.is_empty() {
        return Vec::new();
    }
    let mut out = vec![TreeNode::new(RESULTS_FOLDER, 0, '+', format!("Results ({})", input.scenes.len())).folder()];
    for (i, s) in input.scenes.iter().enumerate() {
        let active = i == input.selected;
        out.push(TreeNode::new(RESULT_BASE + i as u32, 1, 'R', clip(&s.label, 60)).selected(active).on_click(UiAction::SelectResult(i)));
        if active {
            let nfields = 1 + s.fields.len();
            for k in 0..nfields {
                let name = if k == 0 {
                    if s.field_label.is_empty() { "result value".to_string() } else { s.field_label.clone() }
                } else {
                    s.fields[k - 1].label.clone()
                };
                out.push(TreeNode::new(FIELD_BASE + k as u32, 2, 'f', clip(&name, 50)).selected(input.view.field.min(nfields - 1) == k).on_click(UiAction::Command(Command::SetField(k))));
            }
        }
    }
    out
}

/// The result display card: the contour legend, playback, deformation gain, errors and the key help. Which result and
/// field is shown is the tree's business (and `F`), so it is not repeated here.
pub fn result_card(input: &GuiInput) -> Card {
    let scene = input.scene();
    let view = input.view;
    let mut out = Vec::new();
    let [lo, hi] = view.range(scene);
    out.push(DockItem::Legend { lo, hi });
    out.push(DockItem::text(format!("{}  |  units consistent with the model inputs", view.field_name(scene)), Tone::Dim));
    if scene.harmonic_period.is_some() {
        out.push(DockItem::text("modal amplitude is arbitrary (mass-normalized)", Tone::Dim));
    }
    if scene.duration() > 0.0 {
        out.push(DockItem::row(
            format!("{}  t={:.4}s of {:.4}s", if view.paused { "Paused" } else { "Playing" }, view.time, scene.duration()),
            Tone::Normal,
            Some(UiAction::Command(Command::TogglePause)),
            false,
        ));
        out.push(DockItem::text(format!("speed x{:.3}", view.speed), Tone::Dim));
    }
    out.push(DockItem::text(format!("gain x{:.3}{}", view.gain, if view.deformed { "" } else { " (deformation off)" }), Tone::Dim));
    if let Some(e) = input.error {
        for l in wrap(e, 48) {
            out.push(DockItem::text(l, Tone::Warn));
        }
    }
    if view.show_help {
        out.push(DockItem::text("Keys", Tone::Title));
        for l in HELP {
            out.push(DockItem::text(l, Tone::Dim));
        }
    }
    Card::new("display", format!("Result {}/{}", input.selected + 1, input.scenes.len()), Relevance::Info, out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gpu_viewer::scene::ScalarField;

    pub(crate) fn scene(label: &str, fields: usize) -> Scene {
        Scene {
            label: label.into(),
            positions: vec![[0.0; 3]; 3],
            triangles: vec![0, 1, 2],
            samples: vec![[0.0; 4]; 3],
            times: vec![0.0],
            range: [0.0, 250.0],
            gain: 1.0,
            harmonic_period: None,
            field_label: "von Mises stress".into(),
            fields: (0..fields).map(|k| ScalarField { label: format!("Field {k}"), range: [0.0, 1.0], values: vec![0.0; 3] }).collect(),
        }
    }
    fn input<'a>(view: &'a ViewState, scenes: &'a [Scene]) -> GuiInput<'a> {
        GuiInput { view, scenes, selected: 0, editor: None, error: None, box_drag: None, overlay: &[] }
    }

    #[test]
    fn results_appear_in_the_tree_and_only_the_active_one_lists_its_fields() {
        let scenes = [scene("first result", 2), scene("second result", 0)];
        let view = ViewState::new(&scenes[0]);
        let tree = results_tree(&input(&view, &scenes));
        assert_eq!(tree[0].label, "Results (2)");
        assert!(tree.iter().any(|n| n.label == "second result" && n.action == Some(UiAction::SelectResult(1))));
        assert_eq!(tree.iter().filter(|n| n.glyph == 'f').count(), 3);
        assert!(tree.iter().any(|n| n.action == Some(UiAction::Command(Command::SetField(2)))));
    }

    #[test]
    fn a_single_plain_result_has_no_results_folder() {
        let (view, scenes) = (ViewState::new(&scene("a", 0)), [scene("a", 0)]);
        assert!(results_tree(&input(&view, &scenes)).is_empty());
    }

    #[test]
    fn the_result_card_has_legend_playback_and_help_but_not_the_field_the_tree_already_names() {
        let mut animated = scene("modal", 1);
        animated.times = vec![0.0, 1.0];
        let view = ViewState::new(&animated);
        let scenes = [animated];
        let card = result_card(&input(&view, &scenes));
        assert_eq!(card.title, "Result 1/1");
        assert!(card.items.iter().any(|i| matches!(i, DockItem::Legend { .. })));
        assert!(card.items.iter().any(|i| matches!(i, DockItem::Line { text, .. } if text.starts_with("Playing"))));
        assert!(!card.items.iter().any(|i| matches!(i, DockItem::Line { text, .. } if text.starts_with("Field "))), "the field is a tree row");
        let help = ViewState { show_help: true, ..view };
        assert!(result_card(&input(&help, &scenes)).items.iter().any(|i| matches!(i, DockItem::Line { text, .. } if text == "Keys")));
    }
}
