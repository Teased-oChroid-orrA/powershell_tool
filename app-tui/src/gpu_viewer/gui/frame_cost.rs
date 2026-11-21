//! Per-frame CPU cost of the egui chrome (no GPU): `Gui::run` (layout and widgets) plus `Context::tessellate`
//! (shapes to triangles), the two parts the window does on every redraw before the renderer paints.
//!
//! `cargo test -p app-tui --release --lib frame_cost -- --ignored --nocapture`
//!
//! Timings, not assertions (ignored by default). Each case runs warm-up frames (a scroll area or a floating card
//! takes the size of what it held the pass before), then `BATCHES` batches of `FRAMES` identical frames, and prints the
//! median batch's per-frame cost. Results and what they mean: `docs/performance-profile-2026-11.md`.
use super::Gui;
use crate::gpu_viewer::{
    controller::ViewState,
    dashboard::{unix_now, Dashboard, Recents},
    editor::authoring::Authoring,
    results::GuiInput,
    Scene,
};
use std::time::Instant;

const FRAMES: usize = 100;
const BATCHES: usize = 7;
const SIZES: [(f32, f32); 2] = [(1360.0, 860.0), (780.0, 900.0)];

fn raw(w: f32, h: f32) -> egui::RawInput {
    egui::RawInput { screen_rect: Some(egui::Rect::from_min_size(egui::Pos2::ZERO, egui::vec2(w, h))), ..Default::default() }
}

fn scene() -> Scene {
    Scene {
        label: "d".into(),
        positions: vec![[0.0; 3]; 3],
        triangles: vec![0, 1, 2],
        samples: vec![[0.0; 4]; 3],
        times: vec![0.0],
        range: [0.0, 1.0],
        gain: 1.0,
        harmonic_period: None,
        field_label: String::new(),
        fields: Vec::new(),
    }
}

/// Median over batches of (run ms, tessellate ms, triangles) per frame of `frame`.
fn measure(ctx: &egui::Context, mut frame: impl FnMut() -> egui::FullOutput) -> (f64, f64, usize) {
    for _ in 0..5 {
        let out = frame();
        std::hint::black_box(ctx.tessellate(out.shapes, out.pixels_per_point));
    }
    let (mut run, mut tess, mut tris) = (Vec::new(), Vec::new(), 0);
    for _ in 0..BATCHES {
        let (mut r, mut t) = (0.0, 0.0);
        for _ in 0..FRAMES {
            let a = Instant::now();
            let out = frame();
            r += a.elapsed().as_secs_f64();
            let b = Instant::now();
            let prims = ctx.tessellate(out.shapes, out.pixels_per_point);
            t += b.elapsed().as_secs_f64();
            tris = prims
                .iter()
                .map(|p| match &p.primitive {
                    egui::epaint::Primitive::Mesh(m) => m.indices.len() / 3,
                    _ => 0,
                })
                .sum();
            std::hint::black_box(prims);
        }
        run.push(r * 1e3 / FRAMES as f64);
        tess.push(t * 1e3 / FRAMES as f64);
    }
    let med = |v: &mut Vec<f64>| {
        v.sort_by(|a, b| a.total_cmp(b));
        v[v.len() / 2]
    };
    (med(&mut run), med(&mut tess), tris)
}

fn report(name: &str, (w, h): (f32, f32), (run, tess, tris): (f64, f64, usize)) {
    eprintln!("{name:<22} {w:>5.0}x{h:<4.0} run {run:>7.3} ms  tessellate {tess:>7.3} ms  total {:>7.3} ms  {tris} triangles", run + tess);
}

/// One frame of `editor` in the window's chrome.
fn editor_case(name: &str, size: (f32, f32), editor: &dyn Authoring, scenes: &[Scene], view: &ViewState) {
    let mut gui = Gui::new();
    let ctx = gui.ctx().clone();
    let input = GuiInput { view, scenes, selected: 0, editor: Some(editor), error: None, box_drag: None, overlay: &[] };
    let cost = measure(&ctx, || gui.run(raw(size.0, size.1), &input).0);
    report(name, size, cost);
}

#[test]
#[ignore = "timing harness: run with --ignored --nocapture"]
fn frame_cost() {
    use crate::gpu_viewer::editor::{search::SearchSession, stackup::StackupSession};
    let only = std::env::var("CASE").unwrap_or_default();
    let want = |n: &str| n.contains(&only);
    let scene = scene();
    let view = ViewState::new(&scene);
    let scenes = [scene.clone()];
    super::theme::set_hour(Some(12.0));

    let search = SearchSession::in_memory();
    let stackup = StackupSession::open(std::path::Path::new("/tmp/frame-cost-stackup-unused.json")).unwrap();
    for size in SIZES {
        if want("dashboard") {
            let mut recents = Recents::default();
            recents.record("fea", Some("/work/bracket.json".into()), unix_now() - 7_200);
            let mut dash = Dashboard::new(recents, None);
            let mut gui = Gui::new();
            let ctx = gui.ctx().clone();
            let cost = measure(&ctx, || gui.run_dashboard(raw(size.0, size.1), &mut dash).0);
            report("dashboard", size, cost);
        }
        if want("search") {
            editor_case("search (empty)", size, &search, &scenes, &view);
        }
        if want("stackup") {
            editor_case("stackup", size, &stackup, &scenes, &view);
        }
    }
}
