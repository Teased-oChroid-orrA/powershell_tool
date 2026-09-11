//! Stress Solver - PINN structural stress solver toolbox, wired to
//! `pinn-core`/`pinn-solver` from the sibling `NeuralNetwork-Stress-Solver`
//! checkout (a mature, separately-developed, separately-tested project - see
//! that project's own CLAUDE.md for the solver's own architecture/testing).
//! This file is the ONLY new source file this toolbox needed: `ProblemSpec`/
//! `UserDefinedProblem`/`runner::run_training_user_problem`/
//! `user_problem::evaluate_user_vis_grid` are plain library calls with no
//! GUI-framework coupling, reused completely unchanged.
//!
//! Mirrors `pressure_vessel.rs`'s stepper/card/side_by_side/status-rail shape
//! (same design-system primitives, same visual language) - but unlike
//! Pressure Vessel's synchronous per-frame recompute, PINN training is a
//! genuine long-running background job. Training runs via
//! `runtime.spawn_blocking` (NOT `runtime.spawn` - the training loop is a
//! tight synchronous CPU loop with no `.await` points, so it must not run on
//! an async worker thread, which would starve Search's own async work for
//! the whole training duration) driving `run_training_user_problem`'s
//! existing `crossbeam_channel::Sender<TrainingMsg>`/`Receiver<ControlMsg>`
//! API directly - no new channel type, no `Arc<Mutex<_>>` bridge needed
//! (unlike `search.rs`'s pattern): a crossbeam channel is already safely
//! pollable in a plain `try_recv()` from `ui()` each frame.

use std::sync::Arc;

use eframe::egui::{self, Color32, ColorImage, TextureHandle, TextureOptions};
use pinn_core::beam_spec::BeamSpec;
use pinn_core::messages::{ControlMsg, TrainingMsg};
use pinn_core::problem_spec::ProblemSpec;
use pinn_core::units::{self, PhysicalQuantity, UnitSystem};
use pinn_core::user_geometry::HoleBc;

use crate::theme::Tokens;
use crate::widgets::{card, card_title, side_by_side, stepper, MIN_FLEX_COL};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Step {
    #[default]
    LoadSpec,
    Train,
    Results,
}
const STEPS: [(Step, &str); 3] = [(Step::LoadSpec, "Load Spec"), (Step::Train, "Train"), (Step::Results, "Results")];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Status {
    Idle,
    Training,
    Done,
    Error,
}

/// Which spec format was loaded from `spec_path` - a 2D N-hole plate
/// (`ProblemSpec`) or a 1D Euler-Bernoulli beam (`BeamSpec`, `toy_beam`'s
/// own sanity-check problem). `load_spec` tries `BeamSpec` first (it
/// requires `bc`, which no plate spec has) then falls back to `ProblemSpec` -
/// unambiguous without an explicit discriminator tag (see `pinn-core`'s
/// `beam_spec` module doc).
#[derive(Debug, Clone)]
enum LoadedSpec {
    Plate(ProblemSpec),
    Beam(BeamSpec),
    /// `enhancement.txt` items 7/8/17 - "train once across a range, infer instantly" -
    /// distinct from `Plate` (single fixed material/load) - see
    /// `pinn_core::parametric_spec`'s module doc.
    Parametric(pinn_core::parametric_spec::ParametricProblemSpec),
}

/// Graceful-stop-and-resume: a loaded Plate checkpoint awaiting the user's choice between
/// resuming training or just viewing the trained result. See `pending_resume`'s own doc
/// comment on `StressSolverTool`.
struct PendingResume {
    path: std::path::PathBuf,
    spec: ProblemSpec,
    steps_completed: usize,
    /// Only consumed by "Just View Results" - "Resume Training" re-reads `path` instead.
    model: pinn_solver::network::ElasticityNet<pinn_solver::training_core::BInner>,
    /// Editable in the UI, defaults to the original spec's remaining steps
    /// (`spec.training.max_steps - steps_completed`, floored at a small positive default if
    /// that's zero or negative - a checkpoint saved exactly at `max_steps` still has a
    /// reasonable "train more" default instead of "train 0 more steps").
    additional_steps: usize,
}

/// 8-breakpoint piecewise-linear Viridis colormap approximation - ported
/// verbatim from `NeuralNetwork-Stress-Solver/crates/pinn-gui/src/colormap.rs`
/// (a small, theme-independent pure function; not worth a shared crate for
/// ~35 lines).
const VIRIDIS: &[(f32, u8, u8, u8)] = &[
    (0.000, 68, 1, 84),
    (0.143, 72, 40, 120),
    (0.286, 62, 83, 160),
    (0.429, 49, 104, 142),
    (0.571, 53, 183, 121),
    (0.714, 109, 205, 89),
    (0.857, 180, 222, 44),
    (1.000, 253, 231, 37),
];

fn viridis(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let i = VIRIDIS.partition_point(|&(x, ..)| x <= t).saturating_sub(1).min(VIRIDIS.len() - 2);
    let (t0, r0, g0, b0) = VIRIDIS[i];
    let (t1, r1, g1, b1) = VIRIDIS[i + 1];
    let s = if (t1 - t0).abs() < 1e-6 { 0.0 } else { (t - t0) / (t1 - t0) };
    let lerp = |a: u8, b: u8| (a as f32 + s * (b as f32 - a as f32)) as u8;
    Color32::from_rgb(lerp(r0, r1), lerp(g0, g1), lerp(b0, b1))
}

/// Bilinearly interpolates `disp_u`/`disp_v` at a physical point `(x, y)` - used by
/// `deformed_shape_card` to shift the outer-boundary outline, since (unlike the hole rings,
/// which have exact per-point displacement from `HoleAnalysis::profile`) no exact probe
/// exists for the outer boundary; the existing visualization grid is close enough for a
/// qualitative deformed-shape picture. Grid layout matches `evaluate_user_vis_grid`'s own
/// construction exactly: row `iy`/col `ix`, `x = -half_w + 2*half_w*ix/(nx-1)` (inclusive
/// linspace, so the grid's edge cells sit exactly on the plate boundary - no extrapolation
/// needed for points on the perimeter). Falls back to the nearest in-bounds corner's value
/// when a neighbor is NaN (inside a hole) so the plate's own edges - always outside any
/// hole - never silently return NaN just because a diagonal neighbor happened to graze one.
fn sample_bilinear(disp_u: &ndarray::Array2<f32>, disp_v: &ndarray::Array2<f32>, x: f32, y: f32, half_w: f32, half_h: f32) -> (f32, f32) {
    let (ny, nx) = disp_u.dim();
    if nx < 2 || ny < 2 { return (0.0, 0.0); }
    let fx = ((x / half_w * 0.5 + 0.5) * (nx - 1) as f32).clamp(0.0, (nx - 1) as f32);
    let fy = ((y / half_h * 0.5 + 0.5) * (ny - 1) as f32).clamp(0.0, (ny - 1) as f32);
    let (ix0, iy0) = (fx.floor() as usize, fy.floor() as usize);
    let (ix1, iy1) = ((ix0 + 1).min(nx - 1), (iy0 + 1).min(ny - 1));
    let (tx, ty) = (fx - ix0 as f32, fy - iy0 as f32);
    let sample = |field: &ndarray::Array2<f32>| -> f32 {
        let corners = [
            (field[[iy0, ix0]], (1.0 - tx) * (1.0 - ty)),
            (field[[iy0, ix1]], tx * (1.0 - ty)),
            (field[[iy1, ix0]], (1.0 - tx) * ty),
            (field[[iy1, ix1]], tx * ty),
        ];
        let (sum, weight): (f32, f32) = corners.iter()
            .filter(|(v, _)| v.is_finite())
            .fold((0.0, 0.0), |(s, w), &(v, wt)| (s + v * wt, w + wt));
        if weight > 1e-6 { sum / weight } else { 0.0 }
    };
    (sample(disp_u), sample(disp_v))
}

/// Appends the first point to the end, so a `dashed_line` call over the result visually
/// closes the loop (unlike `Shape::closed_line`, `dashed_line` takes a plain open path).
fn close_loop(pts: &[egui::Pos2]) -> Vec<egui::Pos2> {
    let mut v = pts.to_vec();
    if let Some(&first) = pts.first() { v.push(first); }
    v
}

/// Diverging blue-white-red colormap, zero-centered at `t=0.5` - `enhancement.md` Phase 28's
/// difference-field requirement. `viridis`'s single-hue perceptual ramp has no natural "zero"
/// reference point, which is exactly wrong for a signed New-Original difference field (see
/// `training_vs_new_case_card`'s use of this via `Colormap::Diverging`).
fn diverging(t: f32) -> Color32 {
    let t = t.clamp(0.0, 1.0);
    let blue = (0x21, 0x66, 0xac);
    let white = (0xf7, 0xf7, 0xf7);
    let red = (0xb2, 0x18, 0x2b);
    let lerp3 = |a: (u8, u8, u8), b: (u8, u8, u8), s: f32| -> Color32 {
        let l = |x: u8, y: u8| (x as f32 + s * (y as f32 - x as f32)) as u8;
        Color32::from_rgb(l(a.0, b.0), l(a.1, b.1), l(a.2, b.2))
    };
    if t < 0.5 { lerp3(blue, white, t / 0.5) } else { lerp3(white, red, (t - 0.5) / 0.5) }
}

/// Which colormap `field_to_pixels`/`field_heatmap` uses - `Viridis` (single-hue perceptual,
/// the pre-existing default) for raw fields, `Diverging` (zero-centered blue-white-red) for
/// signed difference fields (Stage B, `enhancement.md` Phase 28).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Colormap {
    #[default]
    Viridis,
    Diverging,
}

/// Color-mapping range vs. the field's TRUE min/max (always reported alongside, never
/// hidden, regardless of `ColorScale` - a real, confirmed review finding: a hole's small
/// high-stress region can visually disappear under a raw linear min-max color scale
/// dominated by unrelated large values elsewhere in the field). `clip_lo`/`clip_hi` are in
/// the COLOR-MAPPING domain (post-log-transform for `ColorScale::Log`), `true_min`/
/// `true_max` are always the real physical values.
struct ColorbarRange {
    clip_lo: f32,
    clip_hi: f32,
    true_min: f32,
    true_max: f32,
}

/// Phase 14 ("Spatial Diagnostic Visualization") color-mapping mode. Default is
/// `PercentileClipped` - the pre-Phase-14 behavior, unchanged (Preservation Rule).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum ColorScale {
    Linear,
    /// Signed-log color mapping (`v.signum() * ln(1 + |v|)`) - handles fields that cross
    /// zero (stress, displacement) without the `ln` domain error a plain log would hit.
    /// Only the COLOR MAPPING is transformed; `true_min`/`true_max` stay the real values.
    Log,
    #[default]
    PercentileClipped,
}

fn field_to_pixels(field: &ndarray::Array2<f32>, scale: ColorScale, colormap: Colormap) -> (Vec<Color32>, ColorbarRange) {
    let true_min = field.iter().copied().filter(|v| v.is_finite()).fold(f32::INFINITY, f32::min);
    let true_max = field.iter().copied().filter(|v| v.is_finite()).fold(f32::NEG_INFINITY, f32::max);
    let signed_log = |v: f32| v.signum() * (1.0 + v.abs()).ln();
    let mut vals: Vec<f32> = field.iter().copied().filter(|v| v.is_finite())
        .map(|v| if scale == ColorScale::Log { signed_log(v) } else { v }).collect();
    vals.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let percentile = |p: f32| -> f32 {
        if vals.is_empty() { return f32::NAN; }
        vals[(((vals.len() - 1) as f32 * p).round() as usize).min(vals.len() - 1)]
    };
    let (mut clip_lo, mut clip_hi) = match scale {
        ColorScale::PercentileClipped => (percentile(0.02), percentile(0.98)),
        ColorScale::Linear | ColorScale::Log =>
            (vals.first().copied().unwrap_or(0.0), vals.last().copied().unwrap_or(1.0)),
    };
    if !clip_lo.is_finite() { clip_lo = 0.0; }
    if !clip_hi.is_finite() { clip_hi = 1.0; }
    if colormap == Colormap::Diverging {
        // Zero-centered, symmetric range - NOT the percentile/linear clip above, which would
        // put zero at some arbitrary off-center position and break the diverging colormap's
        // whole point (`enhancement.md` Phase 28: "blue <- decrease, white <- unchanged, red
        // <- increase"). Uses the TRUE (unclipped) extrema, not `vals`' percentile-filtered
        // range, so a difference field's real magnitude is never visually understated.
        let m = true_min.abs().max(true_max.abs()).max(1e-10);
        let m = if m.is_finite() { m } else { 1.0 };
        clip_lo = -m;
        clip_hi = m;
    }
    let range = (clip_hi - clip_lo).max(1e-10);
    let (ny, nx) = field.dim();
    let mut pixels = Vec::with_capacity(nx * ny);
    let color_fn: fn(f32) -> Color32 = if colormap == Colormap::Diverging { diverging } else { viridis };
    for i in 0..ny {
        let src_row = ny - 1 - i; // physical y=0 at the bottom of the rendered image
        for j in 0..nx {
            let v = field[[src_row, j]];
            let mapped = if scale == ColorScale::Log { signed_log(v) } else { v };
            // Colormap functions clamp their input to [0,1] internally, so values outside the
            // clip window still render (pinned to the colormap's extreme colors) rather than
            // being lost - only the color MAPPING is clipped, no data is discarded.
            pixels.push(if v.is_nan() { Color32::from_gray(30) } else { color_fn((mapped - clip_lo) / range) });
        }
    }
    let colorbar = ColorbarRange {
        clip_lo, clip_hi,
        true_min: if true_min.is_finite() { true_min } else { 0.0 },
        true_max: if true_max.is_finite() { true_max } else { 1.0 },
    };
    (pixels, colorbar)
}

/// Phase 14 field selector - every spatially-resolved quantity the solver can currently
/// produce. `BoundaryResidual` is listed (matching the epic's own Phase 14 list) but has no
/// backing data: this DEM-based solver has no separate rasterized boundary-residual field
/// (see `CLAUDE.md`'s Phase 9/5 audit) - selecting it shows an explanation instead of a
/// heatmap, rather than silently fabricating a field or hiding the option.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpatialField {
    DisplacementMag,
    Ux,
    Uy,
    EpsXx,
    EpsYy,
    GammaXy,
    SigmaXx,
    SigmaYy,
    TauXy,
    VonMises,
    PdeResidual,
    BoundaryResidual,
    AmrScore,
    CollocationDensity,
}

impl SpatialField {
    const ALL: [SpatialField; 13] = [
        SpatialField::DisplacementMag, SpatialField::Ux, SpatialField::Uy,
        SpatialField::EpsXx, SpatialField::EpsYy, SpatialField::GammaXy,
        SpatialField::SigmaXx, SpatialField::SigmaYy, SpatialField::TauXy,
        SpatialField::VonMises, SpatialField::PdeResidual, SpatialField::AmrScore,
        SpatialField::CollocationDensity,
    ];

    fn label(self) -> &'static str {
        match self {
            SpatialField::DisplacementMag => "Displacement |u|",
            SpatialField::Ux => "Ux",
            SpatialField::Uy => "Uy",
            SpatialField::EpsXx => "\u{03b5}xx",
            SpatialField::EpsYy => "\u{03b5}yy",
            SpatialField::GammaXy => "\u{03b3}xy",
            SpatialField::SigmaXx => "\u{03c3}xx",
            SpatialField::SigmaYy => "\u{03c3}yy",
            SpatialField::TauXy => "\u{03c4}xy",
            SpatialField::VonMises => "Von Mises",
            // Labeled "Constitutive residual" (not "PDE residual") - it measures
            // ‖sigma_direct - Hooke's-law(strain_FD)‖, not an equilibrium/PDE residual, which
            // this solver never computes for the plate path. See the Kt investigation writeup
            // in this repo's CLAUDE.md for why that distinction matters.
            SpatialField::PdeResidual => "Constitutive residual",
            SpatialField::BoundaryResidual => "Boundary residual",
            SpatialField::AmrScore => "AMR score",
            SpatialField::CollocationDensity => "Collocation density",
        }
    }

    /// `None` for `BoundaryResidual` (no backing data - see this enum's doc comment) or for a
    /// shear-strain field the current `VisFields` doesn't compute `\gamma_xy = 2\epsilon_xy`
    /// from (it does: `eps_xy` is the tensor-convention value, so `\gamma_xy` is derived here).
    fn array(self, vis: &pinn_core::messages::VisFields) -> Option<std::borrow::Cow<'_, ndarray::Array2<f32>>> {
        use std::borrow::Cow;
        Some(match self {
            SpatialField::DisplacementMag =>
                Cow::Owned((&vis.disp_u * &vis.disp_u + &vis.disp_v * &vis.disp_v).mapv(f32::sqrt)),
            SpatialField::Ux => Cow::Borrowed(&vis.disp_u),
            SpatialField::Uy => Cow::Borrowed(&vis.disp_v),
            SpatialField::EpsXx => Cow::Borrowed(&vis.eps_xx),
            SpatialField::EpsYy => Cow::Borrowed(&vis.eps_yy),
            // Tensor-convention eps_xy = 1/2 * engineering shear gamma_xy (see
            // NeuralNetwork-Stress-Solver's CLAUDE.md Phase 9 Von Mises audit) - double it
            // back to the engineering convention this label promises.
            SpatialField::GammaXy => Cow::Owned(vis.eps_xy.mapv(|v| 2.0 * v)),
            SpatialField::SigmaXx => Cow::Borrowed(&vis.sigma_xx),
            SpatialField::SigmaYy => Cow::Borrowed(&vis.sigma_yy),
            SpatialField::TauXy => Cow::Borrowed(&vis.sigma_xy),
            SpatialField::VonMises => Cow::Borrowed(&vis.von_mises),
            SpatialField::PdeResidual => Cow::Borrowed(&vis.pde_residual),
            SpatialField::AmrScore => Cow::Borrowed(&vis.amr_score),
            SpatialField::CollocationDensity => Cow::Borrowed(&vis.collocation_density),
            SpatialField::BoundaryResidual => return None,
        })
    }

    /// Physical dimension of this field's values - drives the colorbar's unit label (Stage A,
    /// `enhancement.md` Phase 53). Displacement fields are lengths; stress/Von Mises/principal
    /// are stresses; strain fields get their own conventional µε display; residual/AMR-score/
    /// collocation-density are dimensionless (normalized quantities, not raw physical values).
    fn quantity(self) -> PhysicalQuantity {
        match self {
            SpatialField::DisplacementMag | SpatialField::Ux | SpatialField::Uy => PhysicalQuantity::Length,
            SpatialField::EpsXx | SpatialField::EpsYy | SpatialField::GammaXy => PhysicalQuantity::Strain,
            SpatialField::SigmaXx | SpatialField::SigmaYy | SpatialField::TauXy | SpatialField::VonMises => PhysicalQuantity::Stress,
            SpatialField::PdeResidual | SpatialField::BoundaryResidual
            | SpatialField::AmrScore | SpatialField::CollocationDensity => PhysicalQuantity::Dimensionless,
        }
    }
}

pub struct StressSolverTool {
    runtime: Arc<tokio::runtime::Runtime>,
    step: Step,
    status: Status,
    error_msg: Option<String>,

    spec_path: String,
    spec: Option<LoadedSpec>,
    load_error: Option<String>,

    /// Graceful-stop-and-resume: set by `load_checkpoint` for a Plate checkpoint instead of
    /// immediately spawning a serve-only thread, so the user can choose "Resume Training" or
    /// "Just View Results" first. `None` the rest of the time. The already-loaded `model` is
    /// only used by the "Just View Results" branch - "Resume Training" re-reads `path` itself
    /// via `run_training_user_problem_resume` (needs a TRAINABLE model, a different backend
    /// than this inference-only one - simplest to just read the small weights file twice than
    /// convert between backends in memory).
    pending_resume: Option<PendingResume>,

    tx_control: Option<crossbeam_channel::Sender<ControlMsg>>,
    rx_train: Option<crossbeam_channel::Receiver<TrainingMsg>>,

    /// Set on `start_training`, read (never cleared) so the rail can show
    /// how long a run has taken so far even between data updates - training
    /// on a heavy spec can go tens of seconds between chart-visible changes,
    /// and a static number otherwise reads as "frozen" rather than "slow".
    train_started_at: Option<std::time::Instant>,
    /// Set once training actually stops (`Done`/`Error`/`ParametricReady`) -
    /// a real, reported bug fix: `elapsed_secs` used to always compute
    /// `train_started_at.elapsed()` against the live wall clock, so the
    /// displayed timer kept counting up forever after training finished
    /// (every repaint - e.g. just moving the mouse over an already-`Done`
    /// window - read a larger `now - train_started_at`). Once this is
    /// `Some`, elapsed time is computed against THIS frozen instant instead,
    /// so the displayed/report duration matches how long training actually
    /// ran, not how long the window has been open since.
    training_finished_at: Option<std::time::Instant>,
    step_num: usize,
    max_steps: usize,
    total_loss: Vec<f32>,
    energy_loss: Vec<f32>,
    neumann_loss: Vec<f32>,
    lr_history: Vec<f32>,
    vis: Option<pinn_core::messages::VisFields>,
    texture: Option<TextureHandle>,
    colorbar_range: ColorbarRange,
    /// Phase 14 ("Spatial Diagnostic Visualization") field selector + color-scale mode.
    /// Deliberately NOT reset by `start_training` - a user comparing runs likely wants to
    /// keep looking at the same field/scale across a re-run, not have it silently snap back.
    selected_field: SpatialField,
    color_scale: ColorScale,
    show_extrema_markers: bool,

    // `BeamSpec`-only telemetry - a 1D beam has no energy/boundary split
    // (one combined potential-energy scalar) and no spatial field, so it
    // doesn't reuse any of the Plate fields above.
    beam_loss: Vec<f32>,
    beam_max_abs_error: f64,
    beam_max_abs_deflection: f64,
    beam_eval_points: Vec<(f64, f64, f64)>,

    // Phases 13/15 (Neural-Network-Wide Adaptive Collocation epic) - AMR observability.
    // Every `AmrSweepReport` received, in order (one entry per sweep event, across every
    // domain that swept) - drives both the "Adaptive Refinement" status card (latest entry)
    // and the loss-chart's AMR event markers (every entry's `step`).
    amr_sweeps: Vec<pinn_core::messages::AmrSweepReport>,
    /// Parallel to `amr_sweeps` - the `total_loss` chart-index each report was received at
    /// (see the push site's own comment for why this differs from `report.step`).
    amr_marker_positions: Vec<usize>,

    // Smart adaptive architecture - every `ArchitectureEvent` received, in order, plus which
    // `total_loss` chart-index each fired at (same `amr_sweeps`/`amr_marker_positions` pairing
    // convention above, same reason: the training->UI channel drops steps under contention).
    architecture_events: Vec<pinn_core::messages::ArchitectureEvent>,
    architecture_event_marker_positions: Vec<usize>,

    // Phase 16 (Neural-Network-Wide Adaptive Collocation epic) - real per-hole stress
    // analysis (nominal/max stress, Kt, full angular profile), latest received.
    hole_analyses: Vec<pinn_core::messages::HoleAnalysis>,

    // Parametric PINN (`enhancement.txt` items 7/8/17-18) - only meaningful for
    // `LoadedSpec::Parametric`. `parametric_ready` flips true on `TrainingMsg::
    // ParametricReady`, after which the training thread stays alive answering
    // `ControlMsg::ParametricInfer` requests (see `parametric_problem`'s module doc).
    parametric_ready: bool,
    /// `(step, e, nu, px)` for every `ParametricUpdate` received - real-time evidence the
    /// network is seeing the full trained range, plotted as a coverage scatter.
    parametric_param_history: Vec<(usize, f64, f64, f64)>,
    /// Slider state for the "New Problem" instant-inference panel - initialized to the
    /// trained ranges' midpoints once a `ParametricProblemSpec` loads.
    infer_e: f64,
    infer_nu: f64,
    infer_px: f64,
    infer_result: Option<pinn_core::messages::ParametricInferenceResult>,
    /// Snapshot of the training-time result (last `vis`/hole analyses/params received) kept
    /// distinct from `infer_result` specifically so `enhancement.txt` item 16 ("Training
    /// Case vs New Case") has a real ORIGINAL to compare an instant-inference result against
    /// - not re-derived from whatever `self.vis` happens to hold when the comparison card
    /// renders (which could already have been overwritten by a later training update).
    training_case_snapshot: Option<TrainingCaseSnapshot>,

    // `enhancement.txt` item B ("Gradient Norm") / items 4/C ("BC residual RMS/max") - latest
    // values from either `TrainingMsg::Update` or `TrainingMsg::ParametricUpdate`.
    grad_norm_history: Vec<f32>,
    bc_residual_rms: f64,
    bc_residual_max: f64,
    /// Stage C (`enhancement.md` Phase 9) - latest real reaction-force/equilibrium-error
    /// reading, same "only updated on the vis cadence, `None` between updates is a real
    /// absence not a sentinel" convention as `bc_residual_rms`/`_max` above.
    reaction_force: Option<pinn_core::messages::ReactionForce>,
    /// Stage D (`enhancement.md` Phase 10) - latest real internal-energy-vs-external-work
    /// reading. DISTINCT from `energy_loss` (the raw optimizer loss term) - see
    /// `pinn_core::messages::EnergyBalance`'s doc comment.
    energy_balance: Option<pinn_core::messages::EnergyBalance>,

    /// Stage H (model checkpoint save/load) - result of the last `ControlMsg::SaveCheckpoint`
    /// request (`Ok(path)`/`Err(message)`), shown inline next to the Save button.
    checkpoint_status: Option<Result<String, String>>,

    /// Stage F (exportable analysis report) - result of the last export attempt, shown inline
    /// next to the Export button. Kept separate from `checkpoint_status` (a different action).
    export_status: Option<Result<String, String>>,

    /// Stage I (live network-evolution visualization) - latest per-layer weight-magnitude
    /// snapshot, same "only updated on the vis cadence" convention as `reaction_force`/
    /// `energy_balance` above. Read ONLY from `model_val` at the existing vis-cadence
    /// throttle - see `pinn_solver::network::network_snapshot`'s doc comment for why this adds
    /// zero cost to the training hot loop.
    network_snapshot: Option<pinn_core::messages::NetworkSnapshot>,

    /// General-PINN architecture recommendations §15/§30 (Kt investigation follow-up) - which
    /// active loss term's gradient dominates optimization, and which are functionally inert.
    /// Same "only updated on the vis cadence, `None` between updates is a real absence" rule
    /// as `bc_residual_rms`/`reaction_force` above - see `pinn_core::messages::
    /// GradientShareSummary`'s doc comment.
    gradient_share_report: Option<pinn_core::messages::GradientShareSummary>,

    /// General-PINN architecture recommendations §17 (Priority 4, "gradient conflict
    /// diagnostics") - pairwise gradient cosine similarity between active loss terms. Same
    /// vis-cadence-gated convention as `gradient_share_report` immediately above.
    gradient_conflict_report: Option<pinn_core::messages::GradientConflictSummary>,

    /// General-PINN architecture recommendations §4 (Priority 1, "physics dependency graph") -
    /// which stress representation each active loss term reads, see `pinn_core::messages::
    /// TrainingUpdate::stress_source_report`'s doc comment. Static per problem, so this is
    /// never gated/reset to empty mid-run the way `gradient_share_report` is - overwritten
    /// wholesale on every update, always reflecting the current run's real terms.
    stress_source_report: Vec<(&'static str, &'static str)>,

    /// General-PINN architecture recommendations §13 (Priority 5, "generic boundary operator
    /// system") - which classical PDE boundary-condition family each active loss term
    /// enforces, see `pinn_core::messages::TrainingUpdate::boundary_operator_report`'s doc
    /// comment. Same "static per problem, never gated" treatment as `stress_source_report`.
    boundary_operator_report: Vec<(&'static str, &'static str)>,

    /// Stage A (`enhancement.md` Phases 43-65) - global USCS/SI display toggle, persisted via
    /// `PersistedState.unit_system` (read/written directly by `main.rs`, mirroring
    /// `pv.outer_diameter`'s own `pub` field convention for cross-module persistence access).
    /// Purely a display/input-layer concern - never mutates `self.spec`/`self.vis`/anything
    /// feeding training or inference (see `units.rs`'s module doc for why the canonical SI
    /// storage this reads from is untouched by this toggle).
    pub unit_system: UnitSystem,
}

/// A `(E, nu, Px, vis, hole_analyses)` snapshot - training-time value comparable against a
/// later `ParametricInferenceResult`. See `training_case_snapshot`'s doc comment.
#[derive(Clone)]
struct TrainingCaseSnapshot {
    e: f64,
    nu: f64,
    px: f64,
    vis: pinn_core::messages::VisFields,
    hole_analyses: Vec<pinn_core::messages::HoleAnalysis>,
}

/// `enhancement.txt` item 12 - the three failure levels a validity check should report, not
/// just a binary in-range/out-of-range.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ValidityTier { Green, Yellow, Red }

/// Result of `StressSolverTool::classify_infer_result` - the tier plus the evidence behind
/// it, so the UI can show real numbers next to the verdict instead of just a colored dot.
struct Verdict {
    tier: ValidityTier,
    reasons: Vec<String>,
    pde_baseline: f64,
    pde_query: f64,
    bc_baseline: f64,
}

impl StressSolverTool {
    pub fn new(runtime: Arc<tokio::runtime::Runtime>) -> Self {
        Self {
            runtime,
            step: Step::default(),
            status: Status::Idle,
            error_msg: None,
            spec_path: "examples/problems/notched_plate.toml".to_string(),
            spec: None,
            load_error: None,
            pending_resume: None,
            tx_control: None,
            rx_train: None,
            train_started_at: None,
            training_finished_at: None,
            step_num: 0,
            max_steps: 0,
            total_loss: Vec::new(),
            energy_loss: Vec::new(),
            neumann_loss: Vec::new(),
            lr_history: Vec::new(),
            vis: None,
            texture: None,
            colorbar_range: ColorbarRange { clip_lo: 0.0, clip_hi: 1.0, true_min: 0.0, true_max: 1.0 },
            selected_field: SpatialField::VonMises,
            color_scale: ColorScale::default(),
            show_extrema_markers: true,
            beam_loss: Vec::new(),
            beam_max_abs_error: 0.0,
            beam_max_abs_deflection: 0.0,
            beam_eval_points: Vec::new(),
            amr_sweeps: Vec::new(),
            amr_marker_positions: Vec::new(),
            architecture_events: Vec::new(),
            architecture_event_marker_positions: Vec::new(),
            hole_analyses: Vec::new(),
            parametric_ready: false,
            parametric_param_history: Vec::new(),
            infer_e: 0.0,
            infer_nu: 0.0,
            infer_px: 0.0,
            infer_result: None,
            training_case_snapshot: None,
            grad_norm_history: Vec::new(),
            bc_residual_rms: 0.0,
            bc_residual_max: 0.0,
            reaction_force: None,
            energy_balance: None,
            checkpoint_status: None,
            export_status: None,
            network_snapshot: None,
            gradient_share_report: None,
            gradient_conflict_report: None,
            stress_source_report: Vec::new(),
            boundary_operator_report: Vec::new(),
            unit_system: UnitSystem::default(),
        }
    }

    /// Formats an SI value using the currently-selected display unit system - the shared
    /// entry point every result/summary card should use instead of a hardcoded `"{:.3e} Pa"`-
    /// style literal (Stage A).
    fn fmt(&self, value_si: f64, qty: PhysicalQuantity) -> String {
        units::format_value(value_si, qty, self.unit_system)
    }

    /// Seconds since training started - frozen at `training_finished_at` once training has
    /// actually stopped, rather than always reading the live wall clock (see
    /// `training_finished_at`'s doc comment for the bug this fixes).
    fn elapsed_secs(&self) -> Option<f32> {
        let started = self.train_started_at?;
        let end = self.training_finished_at.unwrap_or_else(std::time::Instant::now);
        Some(end.duration_since(started).as_secs_f32())
    }

    /// Tries `BeamSpec` first (requires `bc`, which no plate spec carries), then
    /// `ParametricProblemSpec` (requires `e_range`/`nu_range`/`load_range`, which no plain
    /// plate spec carries), then falls back to `ProblemSpec` - see `LoadedSpec`'s doc comment.
    fn load_spec(&mut self) {
        let contents = match std::fs::read_to_string(&self.spec_path) {
            Ok(c) => c,
            Err(e) => {
                self.load_error = Some(format!("read error: {e}"));
                return;
            }
        };
        if let Ok(beam) = toml::from_str::<BeamSpec>(&contents) {
            self.max_steps = beam.training.steps;
            self.spec = Some(LoadedSpec::Beam(beam));
            self.load_error = None;
            return;
        }
        if let Ok(spec) = toml::from_str::<pinn_core::parametric_spec::ParametricProblemSpec>(&contents) {
            self.max_steps = spec.training.max_steps;
            self.infer_e = spec.e_range.mid();
            self.infer_nu = spec.nu_range.mid();
            self.infer_px = spec.load_range.mid();
            self.spec = Some(LoadedSpec::Parametric(spec));
            self.load_error = None;
            return;
        }
        match toml::from_str::<ProblemSpec>(&contents) {
            Ok(spec) => {
                self.max_steps = spec.training.max_steps;
                self.spec = Some(LoadedSpec::Plate(spec));
                self.load_error = None;
            }
            Err(e) => self.load_error = Some(format!("parse error: {e}")),
        }
    }

    fn start_training(&mut self) {
        let Some(spec) = self.spec.clone() else { return };
        self.status = Status::Training;
        self.error_msg = None;
        self.train_started_at = Some(std::time::Instant::now());
        self.training_finished_at = None;
        self.step_num = 0;
        self.total_loss.clear();
        self.energy_loss.clear();
        self.neumann_loss.clear();
        self.lr_history.clear();
        self.vis = None;
        self.beam_loss.clear();
        self.beam_max_abs_error = 0.0;
        self.beam_max_abs_deflection = 0.0;
        self.beam_eval_points.clear();
        self.amr_sweeps.clear();
        self.amr_marker_positions.clear();
        self.architecture_events.clear();
        self.architecture_event_marker_positions.clear();
        self.hole_analyses.clear();
        self.parametric_ready = false;
        self.parametric_param_history.clear();
        self.infer_result = None;
        self.training_case_snapshot = None;
        self.grad_norm_history.clear();
        self.bc_residual_rms = 0.0;
        self.bc_residual_max = 0.0;
        self.reaction_force = None;
        self.energy_balance = None;
        self.checkpoint_status = None;
        self.export_status = None;
        self.network_snapshot = None;
        self.gradient_share_report = None;
        self.gradient_conflict_report = None;
        self.stress_source_report = Vec::new();
        self.boundary_operator_report = Vec::new();

        let (tx_train, rx_train) = crossbeam_channel::bounded(1); // latest-value channel, matches pinn-gui's own convention
        let (tx_ctrl, rx_ctrl) = crossbeam_channel::unbounded();
        self.rx_train = Some(rx_train);
        self.tx_control = Some(tx_ctrl);

        // `spawn_blocking`, not `spawn` - see this module's doc comment for
        // why: the training loop is synchronous CPU work with no `.await`
        // points, so it must run on tokio's dedicated blocking-thread pool,
        // not steal an async worker thread for the whole run.
        match spec {
            LoadedSpec::Plate(spec) => {
                self.runtime.spawn_blocking(move || {
                    pinn_solver::runner::run_training_user_problem(spec, tx_train, rx_ctrl);
                });
            }
            LoadedSpec::Beam(spec) => {
                self.runtime.spawn_blocking(move || {
                    pinn_solver::toy_beam::run_training_beam_streaming(spec, tx_train, rx_ctrl);
                });
            }
            LoadedSpec::Parametric(spec) => {
                // Deliberately NOT dropped/replaced after `training.max_steps` - the thread
                // stays alive answering `ControlMsg::ParametricInfer` (see
                // `parametric_problem`'s module doc). `stop_training`/leaving the tool sends
                // `Stop`, which ends it like any other path.
                self.runtime.spawn_blocking(move || {
                    pinn_solver::parametric_problem::run_training_parametric(spec, tx_train, rx_ctrl);
                });
            }
        }
    }

    fn stop_training(&mut self) {
        if let Some(tx) = &self.tx_control {
            let _ = tx.send(ControlMsg::Stop);
        }
    }

    /// Stage H (model checkpoint save/load) - only meaningful once a model has reached a
    /// stable, evaluated state (`Status::Done`) - the training/serving thread stays alive at
    /// that point (mirrors the existing `ParametricInfer` precedent) specifically so this can
    /// be requested at any later moment, not only right at the instant training finishes.
    fn save_checkpoint(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("model")
            .add_filter("Model checkpoint", &["gz"])
            .save_file()
        else { return };
        let saved_at_unix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        self.checkpoint_status = None;
        if let Some(tx) = &self.tx_control {
            let _ = tx.send(ControlMsg::SaveCheckpoint { path, saved_at_unix });
        }
    }

    /// Stage H - picks a saved `.mpk.gz` checkpoint and its `.meta.json` sidecar, reconstructs
    /// the matching architecture, and spawns the appropriate "no training, serve immediately"
    /// thread (`pinn_solver::runner::serve_loaded_plate_checkpoint`/`parametric_problem::
    /// serve_loaded_checkpoint`) in place of `start_training` - reuses every existing
    /// `TrainingMsg`-driven UI path (`ParametricReady`/`Done`/`Update`/`ParametricInferResult`)
    /// unchanged, since a loaded checkpoint answers with the exact same message shapes a real
    /// training run would.
    fn load_checkpoint(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .add_filter("Model checkpoint", &["gz"])
            .pick_file()
        else { return };
        let device = pinn_solver::training_core::BDevice::default();
        match pinn_solver::checkpoint::load_checkpoint(&path, &device) {
            Ok((model, meta)) => {
                self.status = Status::Training;
                self.error_msg = None;
                self.load_error = None;
                self.train_started_at = Some(std::time::Instant::now());
                self.training_finished_at = None;
                self.step_num = meta.steps_completed;
                self.max_steps = meta.steps_completed;
                self.total_loss.clear();
                self.energy_loss.clear();
                self.neumann_loss.clear();
                self.lr_history.clear();
                self.vis = None;
                self.amr_sweeps.clear();
                self.amr_marker_positions.clear();
                self.architecture_events.clear();
                self.architecture_event_marker_positions.clear();
                self.hole_analyses.clear();
                self.parametric_ready = false;
                self.parametric_param_history.clear();
                self.infer_result = None;
                self.training_case_snapshot = None;
                self.grad_norm_history.clear();
                self.bc_residual_rms = 0.0;
                self.bc_residual_max = 0.0;
                self.reaction_force = None;
                self.energy_balance = None;
                self.checkpoint_status = None;
                self.network_snapshot = None;
                self.gradient_share_report = None;
                self.gradient_conflict_report = None;
                self.stress_source_report = Vec::new();
                self.boundary_operator_report = Vec::new();

                let (tx_train, rx_train) = crossbeam_channel::bounded(1);
                let (tx_ctrl, rx_ctrl) = crossbeam_channel::unbounded();
                self.rx_train = Some(rx_train);
                self.tx_control = Some(tx_ctrl);

                match meta.spec {
                    pinn_solver::checkpoint::CheckpointSpec::Plate(spec) => {
                        // Graceful-stop-and-resume: don't spawn anything yet - `self.pending_
                        // resume` drives an inline prompt (rendered in `step_content`) that lets
                        // the user pick "Resume Training" or "Just View Results" first. Undo the
                        // `Status::Training`/channel setup above, which assumed an immediate spawn.
                        self.status = Status::Idle;
                        self.spec = Some(LoadedSpec::Plate(spec.clone()));
                        let additional_steps =
                            spec.training.max_steps.saturating_sub(meta.steps_completed).max(1000);
                        self.pending_resume = Some(PendingResume {
                            path,
                            spec,
                            steps_completed: meta.steps_completed,
                            model,
                            additional_steps,
                        });
                    }
                    pinn_solver::checkpoint::CheckpointSpec::Parametric(spec) => {
                        self.infer_e = spec.e_range.mid();
                        self.infer_nu = spec.nu_range.mid();
                        self.infer_px = spec.load_range.mid();
                        self.spec = Some(LoadedSpec::Parametric(spec.clone()));
                        self.runtime.spawn_blocking(move || {
                            pinn_solver::parametric_problem::serve_loaded_checkpoint(spec, model, tx_train, rx_ctrl);
                        });
                    }
                }
            }
            Err(e) => self.load_error = Some(format!("checkpoint load error: {e}")),
        }
    }

    /// Graceful-stop-and-resume "Just View Results" branch - serves the already-loaded
    /// inference model exactly the way `load_checkpoint` used to unconditionally, before the
    /// resume prompt existed.
    fn view_loaded_checkpoint(&mut self) {
        let Some(pending) = self.pending_resume.take() else { return };
        self.status = Status::Training;
        self.train_started_at = Some(std::time::Instant::now());
        self.training_finished_at = None;
        self.step_num = pending.steps_completed;
        self.max_steps = pending.steps_completed;

        let (tx_train, rx_train) = crossbeam_channel::bounded(1);
        let (tx_ctrl, rx_ctrl) = crossbeam_channel::unbounded();
        self.rx_train = Some(rx_train);
        self.tx_control = Some(tx_ctrl);

        let spec = pending.spec;
        let model = pending.model;
        self.runtime.spawn_blocking(move || {
            pinn_solver::runner::serve_loaded_plate_checkpoint(spec, model, tx_train, rx_ctrl);
        });
    }

    /// Graceful-stop-and-resume "Resume Training" branch. Discards the already-loaded
    /// inference model (unused here - `run_training_user_problem_resume` needs a TRAINABLE
    /// model on a different backend, so it re-reads `pending.path` itself; see
    /// `PendingResume`'s doc comment for why reading the small weights file twice is simpler
    /// than converting between backends in memory) and spawns the solver's resume entry point
    /// for `pending.additional_steps` more steps beyond `pending.steps_completed`.
    fn resume_training(&mut self) {
        let Some(pending) = self.pending_resume.take() else { return };
        self.status = Status::Training;
        self.error_msg = None;
        self.train_started_at = Some(std::time::Instant::now());
        self.training_finished_at = None;
        self.step_num = pending.steps_completed;
        self.max_steps = pending.steps_completed.saturating_add(pending.additional_steps);
        self.total_loss.clear();
        self.energy_loss.clear();
        self.neumann_loss.clear();
        self.lr_history.clear();
        self.vis = None;
        self.amr_sweeps.clear();
        self.amr_marker_positions.clear();
        self.architecture_events.clear();
        self.architecture_event_marker_positions.clear();
        self.hole_analyses.clear();
        self.grad_norm_history.clear();
        self.bc_residual_rms = 0.0;
        self.bc_residual_max = 0.0;
        self.reaction_force = None;
        self.energy_balance = None;
        self.checkpoint_status = None;
        self.network_snapshot = None;
        self.gradient_share_report = None;
        self.gradient_conflict_report = None;
        self.stress_source_report = Vec::new();
        self.boundary_operator_report = Vec::new();

        let (tx_train, rx_train) = crossbeam_channel::bounded(1);
        let (tx_ctrl, rx_ctrl) = crossbeam_channel::unbounded();
        self.rx_train = Some(rx_train);
        self.tx_control = Some(tx_ctrl);

        let path = pending.path;
        let additional_steps = pending.additional_steps;
        self.runtime.spawn_blocking(move || {
            pinn_solver::runner::run_training_user_problem_resume(path, additional_steps, tx_train, rx_ctrl);
        });
    }

    /// Stage F (exportable analysis report, `enhancement.md` Phase 40) - assembles a single
    /// JSON document from data ALREADY flowing through `self.*` fields (no new solver-side
    /// computation - pure UI-side assembly, matching the plan's own scope) and writes it to a
    /// user-chosen path. Deliberately excludes raw field grids (`VisFields`'s 2D arrays) to
    /// keep the file small - engineering scalars/summaries only, consistent with this pass's
    /// "keep generated file size to a minimum" instruction.
    fn export_analysis_report(&mut self) {
        let Some(path) = rfd::FileDialog::new()
            .set_file_name("stress_solver_report.json")
            .add_filter("Analysis report", &["json"])
            .save_file()
        else { return };
        let report = self.build_analysis_report();
        self.export_status = match serde_json::to_string_pretty(&report) {
            Ok(json) => match std::fs::write(&path, json) {
                Ok(()) => Some(Ok(path.display().to_string())),
                Err(e) => Some(Err(format!("write failed: {e}"))),
            },
            Err(e) => Some(Err(format!("serialization failed: {e}"))),
        };
    }

    fn build_analysis_report(&self) -> serde_json::Value {
        use serde_json::json;

        let problem = match &self.spec {
            Some(LoadedSpec::Plate(spec)) => json!({
                "kind": "Plate",
                "geometry": {
                    "half_w_m": spec.geometry.half_w, "half_h_m": spec.geometry.half_h, "thickness_m": spec.geometry.thickness,
                    "holes": spec.geometry.holes.iter().enumerate().map(|(i, h)| json!({
                        "index": i, "radius_m": h.radius, "center_m": h.center, "bc": format!("{:?}", h.bc)
                    })).collect::<Vec<_>>(),
                },
                "material": {"e_pa": spec.material.e, "nu": spec.material.nu, "density_kg_m3": spec.material.density, "ultimate_strength_pa": spec.material.ultimate_strength_pa},
                "load": {"px_pa": spec.load.px, "py_pa": spec.load.py},
                "network": {"hidden_dim": spec.network.hidden_dim, "n_hidden": spec.network.n_hidden},
            }),
            Some(LoadedSpec::Parametric(spec)) => json!({
                "kind": "Parametric",
                "geometry": {
                    "half_w_m": spec.geometry.half_w, "half_h_m": spec.geometry.half_h, "thickness_m": spec.geometry.thickness,
                    "holes": spec.geometry.holes.iter().enumerate().map(|(i, h)| json!({
                        "index": i, "radius_m": h.radius, "center_m": h.center, "bc": format!("{:?}", h.bc)
                    })).collect::<Vec<_>>(),
                },
                "e_range_pa": [spec.e_range.min, spec.e_range.max],
                "nu_range": [spec.nu_range.min, spec.nu_range.max],
                "load_range_pa": [spec.load_range.min, spec.load_range.max],
                "network": {"hidden_dim": spec.network.hidden_dim, "n_hidden": spec.network.n_hidden},
            }),
            Some(LoadedSpec::Beam(_)) => json!({"kind": "Beam"}),
            None => serde_json::Value::Null,
        };

        // Mirrors `solution_summary_card`'s own aggregate computation - a pure, cheap
        // re-derivation from `self.vis`, not a new solver-side computation.
        let (disp_max, disp_min, vm_max, vm_avg, principal_max, pde_rms, pde_max, pde_p95) = if let Some(vis) = &self.vis {
            let finite = |a: &ndarray::Array2<f32>| -> Vec<f32> { a.iter().copied().filter(|v| v.is_finite()).collect() };
            let max_of = |v: &[f32]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let min_of = |v: &[f32]| v.iter().copied().fold(f32::INFINITY, f32::min);
            let mean_of = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };
            let disp_mag: Vec<f32> = vis.disp_u.iter().zip(vis.disp_v.iter())
                .filter(|(&u, &v)| u.is_finite() && v.is_finite())
                .map(|(&u, &v)| (u * u + v * v).sqrt()).collect();
            let vm = finite(&vis.von_mises);
            let principal: Vec<f32> = vis.sigma_xx.iter().zip(vis.sigma_yy.iter()).zip(vis.sigma_xy.iter())
                .filter(|((&sxx, &syy), &sxy)| sxx.is_finite() && syy.is_finite() && sxy.is_finite())
                .map(|((&sxx, &syy), &sxy)| { let avg = (sxx + syy) * 0.5; let r = (((sxx - syy) * 0.5).powi(2) + sxy * sxy).sqrt(); avg + r }).collect();
            let pde = finite(&vis.pde_residual);
            let pde_rms = if pde.is_empty() { 0.0 } else { (pde.iter().map(|v| v * v).sum::<f32>() / pde.len() as f32).sqrt() };
            (
                if disp_mag.is_empty() { 0.0 } else { max_of(&disp_mag) } as f64,
                if disp_mag.is_empty() { 0.0 } else { min_of(&disp_mag) } as f64,
                if vm.is_empty() { 0.0 } else { max_of(&vm) } as f64,
                mean_of(&vm) as f64,
                if principal.is_empty() { 0.0 } else { max_of(&principal) } as f64,
                pde_rms as f64,
                if pde.is_empty() { 0.0 } else { max_of(&pde) } as f64,
                Self::percentile(&pde, 0.95) as f64,
            )
        } else { (0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.0) };

        json!({
            "generated_at_unix": std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0),
            "unit_system": match self.unit_system { UnitSystem::Uscs => "USCS", UnitSystem::Si => "SI" },
            "spec_path": self.spec_path,
            "problem": problem,
            "training": {
                "step": self.step_num,
                "max_steps": self.max_steps,
                "elapsed_secs": self.elapsed_secs().map(|s| s as f64),
                "final_total_loss": self.total_loss.last().copied().unwrap_or(0.0),
                "final_optimization_loss_energy_term": self.energy_loss.last().copied().unwrap_or(0.0),
                "final_grad_norm": self.grad_norm_history.last().copied().unwrap_or(0.0),
            },
            "engineering_results": {
                "max_displacement_m": disp_max, "min_displacement_m": disp_min,
                "max_von_mises_pa": vm_max, "avg_von_mises_pa": vm_avg,
                "max_principal_stress_pa": principal_max,
            },
            "physics_validation": {
                "pde_residual_rms": pde_rms, "pde_residual_max": pde_max, "pde_residual_p95": pde_p95,
                "bc_residual_rms": self.bc_residual_rms, "bc_residual_max": self.bc_residual_max,
                "reaction_force": self.reaction_force.map(|rf| json!({
                    "net_fx_n": rf.net_fx, "net_fy_n": rf.net_fy, "reference_force_n": rf.reference_force, "equilibrium_error": rf.equilibrium_error
                })),
                "energy_balance": self.energy_balance.map(|eb| json!({
                    "internal_energy_j": eb.internal_energy, "external_work_j": eb.external_work, "energy_balance_error": eb.energy_balance_error
                })),
            },
            "amr": {
                "sweep_count": self.amr_sweeps.len(),
                "last_sweep": self.amr_sweeps.last().map(|s| json!({
                    "step": s.step, "points_before": s.points_before, "points_after": s.points_after,
                    "residual_rms_before": s.residual_rms_before, "residual_rms_after": s.residual_rms_after,
                })),
            },
            "hole_analyses": self.hole_analyses.iter().map(|h| json!({
                "hole_index": h.hole_index,
                "nominal_stress_pa": h.concentration.nominal_stress,
                "max_von_mises_pa": h.concentration.max_von_mises,
                "kt": h.concentration.kt,
                "peak_theta_deg": h.concentration.max_theta_deg,
            })).collect::<Vec<_>>(),
            "model_validity": self.infer_result.as_ref().map(|r| {
                let verdict = self.classify_infer_result(r);
                json!({
                    "query_e_pa": r.e, "query_nu": r.nu, "query_px_pa": r.px,
                    "in_range": r.in_range,
                    "tier": match verdict.tier { ValidityTier::Green => "GREEN", ValidityTier::Yellow => "YELLOW", ValidityTier::Red => "RED" },
                    "reasons": verdict.reasons,
                    "bc_residual_rms": r.bc_residual_rms,
                    "equilibrium_error": r.reaction_force.equilibrium_error,
                    "energy_balance_error": r.energy_balance.energy_balance_error,
                    "nearest_sample_distance": r.nearest_sample_distance,
                    "typical_sample_spacing": r.typical_sample_spacing,
                })
            }),
        })
    }

    /// Sends a `ControlMsg::ParametricInfer` request at the current slider values - only
    /// meaningful once `parametric_ready` (the training thread is alive and waiting for
    /// exactly this).
    fn run_instant_inference(&mut self) {
        if let Some(tx) = &self.tx_control {
            let _ = tx.send(ControlMsg::ParametricInfer { e: self.infer_e, nu: self.infer_nu, px: self.infer_px });
        }
    }

    /// Drains the training channel, keeping only the latest message (mirrors
    /// `pinn-gui/src/app.rs::drain_channel`'s own "drain, keep latest"
    /// pattern) - training streams far faster than the UI needs to render.
    ///
    /// Real bug fixed here: the training closure's `Sender` disconnects the
    /// instant it returns, which happens right after its final blocking
    /// `tx.send(TrainingMsg::Done)` succeeds - so a normal, successful finish
    /// can legitimately observe `Ok(Done)` immediately followed by
    /// `Err(Disconnected)` within the SAME drain call. The disconnect branch
    /// used to fire before `last` (which already held `Done`) was applied,
    /// so `self.status` was still `Training` at that check and got
    /// overwritten to a spurious `Error` - only for `apply_msg(Done)` to then
    /// flip `status` back to `Done` right after, leaving the correct status
    /// but a stale, wrong `error_msg` still displayed underneath it. Fixed by
    /// applying `last` first, then only treating a disconnect as a genuine
    /// failure if `status` is STILL `Training` afterward (i.e. the sender
    /// vanished without ever delivering a terminal `Done`/`Error` message -
    /// e.g. the training closure actually panicked).
    fn drain_channel(&mut self) {
        let Some(rx) = &self.rx_train else { return };
        let mut last = None;
        let mut disconnected = false;
        loop {
            match rx.try_recv() {
                Ok(msg) => last = Some(msg),
                Err(crossbeam_channel::TryRecvError::Empty) => break,
                Err(crossbeam_channel::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        if let Some(msg) = last {
            self.apply_msg(msg);
        }
        if disconnected {
            if self.status == Status::Training {
                self.status = Status::Error;
                self.error_msg = Some("Training task ended unexpectedly".to_string());
            }
            self.rx_train = None;
            self.tx_control = None;
        }
    }

    fn apply_msg(&mut self, msg: TrainingMsg) {
        match msg {
            TrainingMsg::Update(upd) => {
                self.step_num = upd.step;
                self.total_loss.push(upd.total_loss);
                self.energy_loss.push(upd.energy_loss);
                self.neumann_loss.push(upd.neumann_loss);
                self.lr_history.push(upd.lr);
                // `bc_residual_rms`/`_max` are only meaningful on the same cadence `vis`
                // itself is sent on (see `TrainingUpdate::bc_residual_rms`'s doc comment) -
                // captured BEFORE `upd.vis` is moved out, so the displayed value holds the
                // last REAL reading between vis-cadence updates instead of flickering to the
                // `0.0` sentinel every non-vis step.
                let had_vis = upd.vis.is_some();
                // Static per problem, sent on every update (never vis-cadence-gated) - see
                // `TrainingUpdate::stress_source_report`'s doc comment.
                self.stress_source_report = upd.stress_source_report;
                self.boundary_operator_report = upd.boundary_operator_report;
                if let Some(vis) = upd.vis {
                    self.vis = Some(vis);
                }
                if !upd.hole_analyses.is_empty() {
                    self.hole_analyses = upd.hole_analyses;
                }
                if let Some(report) = upd.amr_sweep {
                    // Chart marker position uses the just-pushed `total_loss` INDEX, not
                    // `report.step` - the training→UI channel is bounded(1)/`try_send`
                    // (drops intermediate steps under UI-poll contention), so the loss
                    // chart's x-axis is "i-th message actually received", not the real step
                    // number. Using the real step here would misalign the marker against
                    // the loss line whenever a step got dropped. `report.step` (the true
                    // training step) is still shown in the status card's own text.
                    self.amr_marker_positions.push(self.total_loss.len() - 1);
                    self.amr_sweeps.push(report);
                }
                if let Some(event) = upd.architecture_event {
                    self.architecture_event_marker_positions.push(self.total_loss.len() - 1);
                    self.architecture_events.push(event);
                }
                if let Some(g) = upd.grad_norm { self.grad_norm_history.push(g); }
                if had_vis {
                    self.bc_residual_rms = upd.bc_residual_rms;
                    self.bc_residual_max = upd.bc_residual_max;
                    self.reaction_force = upd.reaction_force;
                    self.energy_balance = upd.energy_balance;
                    self.network_snapshot = upd.network_snapshot;
                    self.gradient_share_report = upd.gradient_share_report;
                    self.gradient_conflict_report = upd.gradient_conflict_report;
                }
            }
            TrainingMsg::BeamUpdate(upd) => {
                self.step_num = upd.step;
                self.beam_loss.push(upd.loss);
                self.beam_max_abs_error = upd.max_abs_error;
                self.beam_max_abs_deflection = upd.max_abs_deflection;
                self.beam_eval_points = upd.eval_points;
            }
            TrainingMsg::ParametricUpdate(upd) => {
                self.step_num = upd.step;
                self.total_loss.push(upd.total_loss);
                self.energy_loss.push(upd.energy_loss);
                self.neumann_loss.push(upd.boundary_loss);
                self.lr_history.push(upd.lr);
                self.parametric_param_history.push((upd.step, upd.e_this_step, upd.nu_this_step, upd.load_this_step));
                self.grad_norm_history.push(upd.grad_norm);
                if let Some(event) = upd.architecture_event {
                    self.architecture_event_marker_positions.push(self.total_loss.len() - 1);
                    self.architecture_events.push(event);
                }
                if let Some(vis) = upd.vis {
                    self.bc_residual_rms = upd.bc_residual_rms;
                    self.bc_residual_max = upd.bc_residual_max;
                    self.reaction_force = upd.reaction_force;
                    self.energy_balance = upd.energy_balance;
                    self.network_snapshot = upd.network_snapshot;
                    // `ParametricTrainingUpdate` has no `gradient_share_report` field -
                    // `step_parametric` is a separate, hand-written step function from
                    // `step_physics_multi` and never computes `term_grad_norms` at all.
                    self.gradient_share_report = None;
                    self.gradient_conflict_report = None;
                    self.stress_source_report = Vec::new();
                    self.boundary_operator_report = Vec::new();
                    // Keep the LATEST training snapshot as the "Training Case" comparison
                    // baseline (item 16) - overwritten every vis-cadence update rather than
                    // frozen at the first one, so a comparison always reads against what the
                    // model was actually trained to right before the user asked for an
                    // instant-inference result.
                    self.training_case_snapshot = Some(TrainingCaseSnapshot {
                        e: upd.e_this_step, nu: upd.nu_this_step, px: upd.load_this_step,
                        vis: vis.clone(), hole_analyses: upd.hole_analyses.clone(),
                    });
                    self.vis = Some(vis);
                }
                if !upd.hole_analyses.is_empty() {
                    self.hole_analyses = upd.hole_analyses;
                }
            }
            TrainingMsg::ParametricReady => {
                // Training itself is done (matches `Done`'s status transition), but the
                // solver thread stays alive - `parametric_ready` gates the "New Problem"
                // instant-inference panel, distinct from an ordinary finished run.
                self.status = Status::Done;
                self.parametric_ready = true;
                self.training_finished_at.get_or_insert_with(std::time::Instant::now);
            }
            TrainingMsg::ParametricInferResult(result) => {
                self.infer_result = Some(*result);
            }
            TrainingMsg::Done => {
                // The solver thread deliberately stays alive after `Done` to serve
                // `ControlMsg::SaveCheckpoint` (mirrors the `ParametricReady` precedent right
                // above, which never touches `tx_control`/`rx_train` either) - a real,
                // reported bug: this used to null both out immediately here, which broke
                // "Save Trained Model" silently (`save_checkpoint`'s `if let Some(tx) = &self.
                // tx_control` found `None` and skipped sending entirely, and even had it sent,
                // `drain_channel`'s `let Some(rx) = &self.rx_train else { return }` would never
                // see the `CheckpointSaved` reply). The channel is correctly torn down only once
                // it ACTUALLY disconnects (`drain_channel`'s own `disconnected` branch below),
                // e.g. after the user clicks Stop or the process exits.
                self.status = Status::Done;
                self.training_finished_at.get_or_insert_with(std::time::Instant::now);
            }
            TrainingMsg::Error(e) => {
                self.status = Status::Error;
                self.error_msg = Some(e);
                self.tx_control = None;
                self.rx_train = None;
                self.training_finished_at.get_or_insert_with(std::time::Instant::now);
            }
            // Pin-lug-only variants on the shared TrainingMsg enum - never
            // sent by run_training_user_problem (single-domain, no contact
            // export path).
            TrainingMsg::PinLugUpdate(_) | TrainingMsg::ExportComplete(_) => {}
            TrainingMsg::CheckpointSaved(result) => {
                self.checkpoint_status = Some(result);
            }
        }
    }

    pub fn ui(&mut self, ui: &mut egui::Ui, tokens: &Tokens, ctx: &egui::Context) {
        self.drain_channel();
        if self.status == Status::Training {
            ctx.request_repaint_after(std::time::Duration::from_millis(80));
        }

        // Real, measured finding (not speculation): training this toolbox's physics loop in
        // a debug build is ~20-30x slower than a release build - far more than any
        // algorithmic fix here can close (debug: ~2.2s/step measured on `single_hole_plate`-
        // shaped config; release: ~93ms/step, at/below the documented GPU baseline). A user
        // running `cargo run -p app-egui` (no `--release`) would otherwise have no way to
        // know a "frozen-looking" run is a build-mode problem, not a bug - surface it
        // directly instead of leaving it as tribal knowledge in a doc comment.
        if cfg!(debug_assertions) {
            egui::Frame::default()
                .fill(tokens.warning_bg)
                .stroke(egui::Stroke::new(1.0, tokens.warning))
                .rounding(crate::design::radii::md())
                .inner_margin(10.0)
                .show(ui, |ui| {
                    ui.colored_label(tokens.warning, egui::RichText::new(
                        "\u{26a0} Debug build - PINN training runs ~20-30\u{d7} slower here than in a release build. Use `cargo run --release -p app-egui` for real training runs."
                    ).size(11.5));
                });
            ui.add_space(8.0);
        }

        stepper(ui, tokens, &STEPS, &mut self.step);
        ui.add_space(10.0);

        let step = self.step;
        // `status_rail` takes a plain snapshot, not `&self` - mirrors
        // `pressure_vessel.rs`'s own `side_by_side` usage (its `status_rail`
        // is a free function over pre-extracted data). Two closures both
        // capturing `self` (one mutably, via `step_content`) in the same
        // `side_by_side` call is a real borrow-checker conflict, not a style
        // choice - the left/right closures aren't guaranteed non-overlapping
        // from the borrow checker's view even though `side_by_side` runs
        // them strictly sequentially.
        let telemetry = match &self.spec {
            Some(LoadedSpec::Beam(_)) => self.beam_loss.last().map(|&loss| RailTelemetry::Beam {
                loss,
                max_abs_error: self.beam_max_abs_error,
                max_abs_deflection: self.beam_max_abs_deflection,
            }),
            _ => self.total_loss.last().map(|&total_loss| RailTelemetry::Plate {
                total_loss,
                energy_loss: self.energy_loss.last().copied().unwrap_or(0.0),
                neumann_loss: self.neumann_loss.last().copied().unwrap_or(0.0),
                lr: self.lr_history.last().copied().unwrap_or(0.0),
            }),
        };
        let rail = RailSnapshot {
            status: self.status,
            step_num: self.step_num,
            max_steps: self.max_steps,
            error_msg: self.error_msg.clone(),
            elapsed_secs: self.elapsed_secs(),
            telemetry,
        };
        // The content column (Stress Field card, field selector, Adaptive Refinement card,
        // Hole Stress Analysis card on Results) genuinely overflows a normal window height
        // once every Phase 8/13/14/16 card is stacked - without a `ScrollArea` here, anything
        // past the visible fold (a real, confirmed user report: "can't scroll to see
        // 'Adaptive Refinement'") was simply unreachable, not just visually cut off.
        // `auto_shrink([false, false])` - NOT the default - forces the area to actually fill
        // the available height and show a scrollbar on overflow, rather than shrinking itself
        // to fit its content (which would silently un-do the fix).
        side_by_side(
            ui, 232.0, MIN_FLEX_COL,
            |ui| {
                egui::ScrollArea::vertical()
                    .id_salt("stress_solver_content_scroll")
                    .auto_shrink([false, false])
                    .show(ui, |ui| self.step_content(ui, tokens, step));
            },
            |ui| status_rail(ui, tokens, &rail),
        );
    }

    fn step_content(&mut self, ui: &mut egui::Ui, tokens: &Tokens, step: Step) {
        ui.horizontal(|ui| {
            ui.colored_label(tokens.fg_muted, egui::RichText::new("UNITS").size(10.0).strong());
            ui.add_space(6.0);
            crate::design::components::segmented(ui, tokens, &mut self.unit_system, &[(UnitSystem::Uscs, "USCS"), (UnitSystem::Si, "SI")]);
        });
        ui.add_space(8.0);
        card(ui, tokens, |ui| {
            card_title(ui, "Problem Spec");
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.colored_label(tokens.fg_muted, egui::RichText::new("Spec path").size(11.5));
                    ui.add_space(3.0);
                    ui.add(egui::TextEdit::singleline(&mut self.spec_path).desired_width(300.0));
                });
                ui.add_space(8.0);
                let running = self.status == Status::Training;
                // Also locked while a resume prompt is pending - loading a second checkpoint
                // or a fresh spec mid-prompt would silently orphan the first choice.
                let locked = running || self.pending_resume.is_some();
                if ui.add_enabled(!locked, egui::Button::new("Browse\u{2026}")).clicked() {
                    if let Some(path) = rfd::FileDialog::new().add_filter("TOML spec", &["toml"]).pick_file() {
                        self.spec_path = path.display().to_string();
                    }
                }
                ui.add_space(6.0);
                if ui.add_enabled(!locked, egui::Button::new("Load")).clicked() {
                    self.load_spec();
                }
                ui.add_space(14.0);
                ui.separator();
                ui.add_space(14.0);
                // Stage H (model checkpoint save/load) - skips training entirely: reconstructs
                // the saved architecture and serves it immediately (see `load_checkpoint`'s
                // doc comment).
                if ui.add_enabled(!locked, egui::Button::new("Load Trained Model\u{2026}")).clicked() {
                    self.load_checkpoint();
                }
            });
            if let Some(err) = &self.load_error {
                ui.colored_label(tokens.danger, err.as_str());
            }
            // Stage G (`enhancement.md` items 43-65's "unit-aware inputs" + the user's own
            // explicit follow-up ask: edit problem inputs in-app, no TOML round-trip needed).
            // Editable only while `Idle` - training/a loaded checkpoint has already captured
            // whatever spec it started with, and `run_training_*`/checkpoint-load only ever
            // read the spec once at thread-spawn time, so edits made mid-run would silently
            // do nothing if allowed - disabling them here is honest, not just cosmetic.
            let unit_system = self.unit_system;
            let editable = self.status == Status::Idle;
            let fmt = |v: f64, qty: PhysicalQuantity| units::format_value(v, qty, unit_system);
            let unit_drag = |ui: &mut egui::Ui, value: &mut f64, qty: PhysicalQuantity| {
                let unit = units::pick_unit(*value, qty, unit_system);
                let mut disp = units::to_unit(*value, qty, unit);
                let speed = (disp.abs() * 0.01).max(1e-6);
                let resp = ui.add(egui::DragValue::new(&mut disp).speed(speed)
                    .suffix(if unit.is_empty() { String::new() } else { format!(" {unit}") }));
                if resp.changed() { *value = units::convert_from_display(disp, qty, unit); }
            };
            match &mut self.spec {
                Some(LoadedSpec::Plate(spec)) => {
                    ui.separator();
                    if editable {
                        ui.horizontal(|ui| {
                            ui.label("Width"); let mut w = 2.0 * spec.geometry.half_w; unit_drag(ui, &mut w, PhysicalQuantity::Length); spec.geometry.half_w = w / 2.0;
                            ui.add_space(10.0);
                            ui.label("Height"); let mut h = 2.0 * spec.geometry.half_h; unit_drag(ui, &mut h, PhysicalQuantity::Length); spec.geometry.half_h = h / 2.0;
                            ui.add_space(10.0);
                            ui.label("Thickness"); unit_drag(ui, &mut spec.geometry.thickness, PhysicalQuantity::Length);
                        });
                        ui.horizontal(|ui| {
                            ui.label("E"); unit_drag(ui, &mut spec.material.e, PhysicalQuantity::Stress);
                            ui.add_space(10.0);
                            ui.label("\u{3bd}"); ui.add(egui::DragValue::new(&mut spec.material.nu).speed(0.001).range(0.0..=0.5));
                            ui.add_space(10.0);
                            ui.label("Load Px"); unit_drag(ui, &mut spec.load.px, PhysicalQuantity::Stress);
                            ui.add_space(10.0);
                            ui.label("Load Py"); unit_drag(ui, &mut spec.load.py, PhysicalQuantity::Stress);
                        });
                        for (i, hole) in spec.geometry.holes.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let (label, color) = match hole.bc { HoleBc::Free => ("free", tokens.good), HoleBc::Fixed => ("fixed", tokens.danger) };
                                ui.colored_label(color, format!("Hole {i} ({label})"));
                                ui.label("r"); unit_drag(ui, &mut hole.radius, PhysicalQuantity::Length);
                                ui.label("cx"); unit_drag(ui, &mut hole.center[0], PhysicalQuantity::Length);
                                ui.label("cy"); unit_drag(ui, &mut hole.center[1], PhysicalQuantity::Length);
                            });
                        }
                        adaptive_architecture_controls(ui, &mut spec.network);
                    } else {
                        ui.label(format!(
                            "Plate: {} \u{d7} {}   Material E = {}, \u{3bd} = {:.3}   Load Px = {}",
                            fmt(2.0 * spec.geometry.half_w, PhysicalQuantity::Length), fmt(2.0 * spec.geometry.half_h, PhysicalQuantity::Length),
                            fmt(spec.material.e, PhysicalQuantity::Stress), spec.material.nu, fmt(spec.load.px, PhysicalQuantity::Stress)
                        ));
                        ui.horizontal(|ui| {
                            ui.label("Holes:");
                            for hole in &spec.geometry.holes {
                                let (label, color) = match hole.bc { HoleBc::Free => ("free", tokens.good), HoleBc::Fixed => ("fixed", tokens.danger) };
                                ui.colored_label(color, format!("\u{25cf} {label} r={}", fmt(hole.radius, PhysicalQuantity::Length)));
                            }
                        });
                    }
                }
                Some(LoadedSpec::Beam(spec)) => {
                    ui.separator();
                    let bc_label = match spec.bc {
                        pinn_core::beam_spec::BeamBcSpec::Cantilever => "Cantilever (clamped-free)",
                        pinn_core::beam_spec::BeamBcSpec::SimplySupported => "Simply supported (pinned-pinned)",
                    };
                    ui.label(format!(
                        "1D Euler-Bernoulli beam: {bc_label}   hidden_dim={} n_hidden={}   steps={} n_points={}",
                        spec.network.hidden_dim, spec.network.n_hidden, spec.training.steps, spec.training.n_points
                    ));
                }
                Some(LoadedSpec::Parametric(spec)) => {
                    ui.separator();
                    if editable {
                        ui.horizontal(|ui| {
                            ui.label("Width"); let mut w = 2.0 * spec.geometry.half_w; unit_drag(ui, &mut w, PhysicalQuantity::Length); spec.geometry.half_w = w / 2.0;
                            ui.add_space(10.0);
                            ui.label("Height"); let mut h = 2.0 * spec.geometry.half_h; unit_drag(ui, &mut h, PhysicalQuantity::Length); spec.geometry.half_h = h / 2.0;
                            ui.add_space(10.0);
                            ui.label("Thickness"); unit_drag(ui, &mut spec.geometry.thickness, PhysicalQuantity::Length);
                        });
                        ui.horizontal(|ui| {
                            ui.label("E range"); unit_drag(ui, &mut spec.e_range.min, PhysicalQuantity::Stress); ui.label("\u{2013}"); unit_drag(ui, &mut spec.e_range.max, PhysicalQuantity::Stress);
                        });
                        ui.horizontal(|ui| {
                            ui.label("\u{3bd} range");
                            ui.add(egui::DragValue::new(&mut spec.nu_range.min).speed(0.001).range(0.0..=0.5));
                            ui.label("\u{2013}");
                            ui.add(egui::DragValue::new(&mut spec.nu_range.max).speed(0.001).range(0.0..=0.5));
                        });
                        ui.horizontal(|ui| {
                            ui.label("Load range"); unit_drag(ui, &mut spec.load_range.min, PhysicalQuantity::Stress); ui.label("\u{2013}"); unit_drag(ui, &mut spec.load_range.max, PhysicalQuantity::Stress);
                        });
                        for (i, hole) in spec.geometry.holes.iter_mut().enumerate() {
                            ui.horizontal(|ui| {
                                let (label, color) = match hole.bc { HoleBc::Free => ("free", tokens.good), HoleBc::Fixed => ("fixed", tokens.danger) };
                                ui.colored_label(color, format!("Hole {i} ({label})"));
                                ui.label("r"); unit_drag(ui, &mut hole.radius, PhysicalQuantity::Length);
                                ui.label("cx"); unit_drag(ui, &mut hole.center[0], PhysicalQuantity::Length);
                                ui.label("cy"); unit_drag(ui, &mut hole.center[1], PhysicalQuantity::Length);
                            });
                        }
                        adaptive_architecture_controls(ui, &mut spec.network);
                    } else {
                        ui.label(format!(
                            "Parametric plate: {} \u{d7} {} (fixed geometry)   {} hole(s)",
                            fmt(2.0 * spec.geometry.half_w, PhysicalQuantity::Length), fmt(2.0 * spec.geometry.half_h, PhysicalQuantity::Length),
                            spec.geometry.holes.len()
                        ));
                        ui.label(format!(
                            "Trained across: E {}\u{2013}{}   \u{3bd} {:.3}\u{2013}{:.3}   Load {}\u{2013}{}",
                            fmt(spec.e_range.min, PhysicalQuantity::Stress), fmt(spec.e_range.max, PhysicalQuantity::Stress),
                            spec.nu_range.min, spec.nu_range.max,
                            fmt(spec.load_range.min, PhysicalQuantity::Stress), fmt(spec.load_range.max, PhysicalQuantity::Stress)
                        ));
                    }
                    ui.colored_label(tokens.fg_muted, egui::RichText::new(
                        "One (E, \u{3bd}, Px) sample per training step - see the Results step's Model Validity Envelope card for instant inference once training completes."
                    ).size(10.5));
                }
                None => {}
            }
        });

        ui.add_space(10.0);

        let running = self.status == Status::Training;
        let can_start = self.spec.is_some() && !running && self.pending_resume.is_none();
        ui.horizontal(|ui| {
            if crate::design::components::button(ui, tokens, crate::design::components::ButtonVariant::Primary, "\u{25b6} Start Training", can_start).clicked() {
                self.start_training();
                self.step = Step::Train;
            }
            if ui.add_enabled(running, egui::Button::new("\u{23f9} Stop")).clicked() {
                self.stop_training();
            }
            ui.add_space(14.0);
            ui.separator();
            ui.add_space(14.0);
            // Stage H - enabled once a model is in a stable, evaluated state (a real run
            // finished, or a checkpoint was loaded - both reach `Status::Done`).
            if ui.add_enabled(self.status == Status::Done, egui::Button::new("\u{1F4be} Save Trained Model\u{2026}")).clicked() {
                self.save_checkpoint();
            }
            if let Some(result) = &self.checkpoint_status {
                match result {
                    Ok(path) => { ui.colored_label(tokens.good, format!("Saved to {path}")); }
                    Err(e) => { ui.colored_label(tokens.danger, format!("Save failed: {e}")); }
                }
            }
        });

        ui.add_space(10.0);

        // Graceful-stop-and-resume: shown whenever a loaded Plate checkpoint is awaiting the
        // user's choice - see `pending_resume`'s doc comment. Click outcomes are captured as
        // plain bools and acted on AFTER this borrow of `self.pending_resume` ends, since both
        // handlers need `&mut self`.
        let mut resume_clicked = false;
        let mut view_clicked = false;
        if let Some(pending) = &mut self.pending_resume {
            card(ui, tokens, |ui| {
                card_title(ui, "Resume Training?");
                ui.label(format!(
                    "Loaded checkpoint at step {} (original spec calls for {} total steps). \
                     Optimizer momentum, SAW-BRDR weights, LR schedule, and AMR grid were not \
                     saved - resuming restarts them fresh, the same warmup cost any new run \
                     pays.",
                    pending.steps_completed, pending.spec.training.max_steps
                ));
                ui.horizontal(|ui| {
                    ui.label("Additional steps:");
                    ui.add(egui::DragValue::new(&mut pending.additional_steps).range(1..=1_000_000));
                });
                ui.horizontal(|ui| {
                    if crate::design::components::button(ui, tokens, crate::design::components::ButtonVariant::Primary, "\u{25b6} Resume Training", true).clicked() {
                        resume_clicked = true;
                    }
                    if ui.button("Just View Results").clicked() {
                        view_clicked = true;
                    }
                });
            });
            ui.add_space(10.0);
        }
        if resume_clicked {
            self.resume_training();
        }
        if view_clicked {
            self.view_loaded_checkpoint();
        }

        let is_beam = matches!(self.spec, Some(LoadedSpec::Beam(_)));
        let loss_title = match step {
            Step::Results => "Loss Trajectory \u{2014} final",
            _ => "Loss Trajectory",
        };
        let loss_empty_hint = match step {
            Step::Results => "Run training first (Train step)",
            _ => "Load a spec, then start training to populate this chart",
        };
        let has_loss = if is_beam { self.beam_loss.len() > 1 } else { self.total_loss.len() > 1 };
        card(ui, tokens, |ui| {
            card_title(ui, loss_title);
            if has_loss {
                if is_beam { self.beam_loss_plot(ui); } else { self.loss_plot(ui); }
            } else {
                crate::design::components::empty_state(ui, tokens, "\u{1F4C8}", "No data yet", loss_empty_hint);
            }
        });
        ui.add_space(10.0);

        if is_beam {
            let final_suffix = if step == Step::Results { " (final)" } else { "" };
            card(ui, tokens, |ui| {
                card_title(ui, &format!("Deflection w(x) \u{2014} network vs. exact{final_suffix}"));
                if self.beam_eval_points.len() > 1 {
                    self.beam_deflection_plot(ui);
                    ui.add_space(4.0);
                    ui.colored_label(tokens.fg_muted, format!(
                        "max |error| = {:.3e}   max |deflection| = {:.3e}",
                        self.beam_max_abs_error, self.beam_max_abs_deflection
                    ));
                } else {
                    crate::design::components::empty_state(ui, tokens, "\u{223f}", "No data yet", "Comparison renders once training produces its first sample");
                }
            });
        } else {
            let field_title = if step == Step::Results {
                format!("Stress Field \u{2014} {} (final)", self.selected_field.label())
            } else {
                format!("Stress Field \u{2014} {}", self.selected_field.label())
            };
            let field_empty_hint = if step == Step::Results { "Run training first (Train step)" } else { "Field renders once training produces its first sample" };
            card(ui, tokens, |ui| {
                card_title(ui, &field_title);
                if let Some(vis) = self.vis.clone() {
                    self.field_selector(ui);
                    ui.add_space(4.0);
                    let hole_analyses = self.hole_analyses.clone();
                    self.field_heatmap(ui, tokens, &vis, &hole_analyses, Colormap::Viridis);
                } else {
                    self.field_selector(ui);
                    crate::design::components::empty_state(ui, tokens, "\u{25a3}", "No data yet", field_empty_hint);
                }
            });
            ui.add_space(10.0);
            self.amr_status_card(ui, tokens);
            ui.add_space(10.0);
            self.network_architecture_card(ui, tokens);
            if step == Step::Results {
                ui.add_space(10.0);
                self.solution_summary_card(ui, tokens);
                ui.add_space(10.0);
                self.hole_stress_analysis_card(ui, tokens);
                ui.add_space(10.0);
                self.deformed_shape_card(ui, tokens);
                if self.parametric_ready {
                    ui.add_space(10.0);
                    self.parametric_envelope_card(ui, tokens);
                } else if matches!(&self.spec, Some(LoadedSpec::Plate(_))) {
                    ui.add_space(10.0);
                    self.model_contract_card(ui, tokens);
                }
                ui.add_space(10.0);
                card(ui, tokens, |ui| {
                    ui.horizontal(|ui| {
                        if ui.button("\u{1F4c4} Export Analysis Report\u{2026}").clicked() {
                            self.export_analysis_report();
                        }
                        if let Some(result) = &self.export_status {
                            match result {
                                Ok(path) => { ui.colored_label(tokens.good, format!("Exported to {path}")); }
                                Err(e) => { ui.colored_label(tokens.danger, format!("Export failed: {e}")); }
                            }
                        }
                    });
                });
            }
        }
    }

    /// `enhancement.txt` items 6/B/C - "Solution Summary" / engineering validation numbers.
    /// Displacement/stress/PDE-residual values are computed directly from the current
    /// `VisFields` grid; BC residual, gradient norm, and reaction force/equilibrium error come
    /// from `pinn_solver::user_problem::{probe_boundary_residuals,probe_reaction_force}`/
    /// `training_core::StepOutput::grad_norm` (real solver-side telemetry, not derived here -
    /// see `enhancement.md` Phase 9 for the reaction-force check's design). Max principal
    /// stress uses the standard 2D formula `(sxx+syy)/2 + sqrt(((sxx-syy)/2)^2 + sxy^2)`.
    fn solution_summary_card(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        let Some(vis) = &self.vis else { return };
        let finite = |a: &ndarray::Array2<f32>| -> Vec<f32> { a.iter().copied().filter(|v| v.is_finite()).collect() };
        let max_of = |v: &[f32]| v.iter().copied().fold(f32::NEG_INFINITY, f32::max);
        let min_of = |v: &[f32]| v.iter().copied().fold(f32::INFINITY, f32::min);
        let mean_of = |v: &[f32]| if v.is_empty() { 0.0 } else { v.iter().sum::<f32>() / v.len() as f32 };

        let disp_mag: Vec<f32> = vis.disp_u.iter().zip(vis.disp_v.iter())
            .filter(|(&u, &v)| u.is_finite() && v.is_finite())
            .map(|(&u, &v)| (u * u + v * v).sqrt()).collect();
        let vm = finite(&vis.von_mises);
        let principal: Vec<f32> = vis.sigma_xx.iter().zip(vis.sigma_yy.iter()).zip(vis.sigma_xy.iter())
            .filter(|((&sxx, &syy), &sxy)| sxx.is_finite() && syy.is_finite() && sxy.is_finite())
            .map(|((&sxx, &syy), &sxy)| {
                let avg = (sxx + syy) * 0.5;
                let r = (((sxx - syy) * 0.5).powi(2) + sxy * sxy).sqrt();
                avg + r
            }).collect();
        let pde = finite(&vis.pde_residual);
        let pde_rms = if pde.is_empty() { 0.0 } else { (pde.iter().map(|v| v * v).sum::<f32>() / pde.len() as f32).sqrt() };

        card(ui, tokens, |ui| {
            card_title(ui, "Solution Summary");
            ui.add_space(6.0);
            egui::Grid::new("stress_solver_solution_summary_grid").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                let row = |ui: &mut egui::Ui, label: &str, value: String| { ui.label(label); ui.strong(value); ui.end_row(); };
                row(ui, "Max displacement", self.fmt(max_of(&disp_mag) as f64, PhysicalQuantity::Length));
                row(ui, "Min displacement", self.fmt(if disp_mag.is_empty() { 0.0 } else { min_of(&disp_mag) as f64 }, PhysicalQuantity::Length));
                row(ui, "Max \u{3c3}VM", self.fmt(if vm.is_empty() { 0.0 } else { max_of(&vm) as f64 }, PhysicalQuantity::Stress));
                row(ui, "Avg \u{3c3}VM", self.fmt(mean_of(&vm) as f64, PhysicalQuantity::Stress));
                row(ui, "Max principal stress", self.fmt(if principal.is_empty() { 0.0 } else { max_of(&principal) as f64 }, PhysicalQuantity::Stress));
                row(ui, "Optimization loss (energy term)", format!("{:.4e}", self.energy_loss.last().copied().unwrap_or(0.0)));
                row(ui, "Constitutive residual RMS", format!("{:.4e}", pde_rms));
                row(ui, "Constitutive residual max", format!("{:.4e}", if pde.is_empty() { 0.0 } else { max_of(&pde) }));
                row(ui, "Constitutive residual P95", format!("{:.4e}", Self::percentile(&pde, 0.95)));
                row(ui, "BC residual RMS", format!("{:.4e}", self.bc_residual_rms));
                row(ui, "BC residual max", format!("{:.4e}", self.bc_residual_max));
                row(ui, "Gradient norm (last step)", format!("{:.4e}", self.grad_norm_history.last().copied().unwrap_or(0.0)));
                if let Some(rf) = &self.reaction_force {
                    row(ui, "Net boundary force", format!(
                        "{}, {}", self.fmt(rf.net_fx, PhysicalQuantity::Force), self.fmt(rf.net_fy, PhysicalQuantity::Force)
                    ));
                    row(ui, "Force-equilibrium error", format!("{:.2}%", rf.equilibrium_error * 100.0));
                }
                if let Some(eb) = &self.energy_balance {
                    row(ui, "Internal energy", self.fmt(eb.internal_energy, PhysicalQuantity::Energy));
                    row(ui, "External work", self.fmt(eb.external_work, PhysicalQuantity::Energy));
                    row(ui, "Energy-balance error", format!("{:.2}%", eb.energy_balance_error * 100.0));
                }
            });
            if self.reaction_force.is_none() {
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                    "Reaction force / force-equilibrium error appear once training reaches its first vis-cadence update."
                ).size(10.0));
            }
            // General-PINN architecture recommendations §15/§30 - which active loss term's
            // gradient actually dominates optimization, and which are functionally inert. Same
            // "term is registered ≠ term is exerting real optimization pressure" question this
            // session's own Kt investigation had to answer by hand before this existed.
            if let Some(report) = &self.gradient_share_report {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.strong("Gradient share by term");
                ui.add_space(4.0);
                let mut shares = report.shares.clone();
                shares.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
                egui::Grid::new("stress_solver_gradient_share_grid").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                    for (name, share) in &shares {
                        let flag = if report.dominant == Some(*name) { " (dominant)" }
                            else if report.inert.contains(name) { " (inert)" }
                            else { "" };
                        ui.label(format!("{name}{flag}"));
                        ui.strong(format!("{:.1}%", share * 100.0));
                        ui.end_row();
                    }
                });
            }
            // General-PINN architecture recommendations §17 (Priority 4, "gradient conflict
            // diagnostics") - pairwise cosine similarity between active terms' own gradients.
            // A negative reading means the two terms are, to first order, actively working
            // against each other this step (reducing one increases the other) - §17's own
            // worked example ("traction gradient · energy gradient < 0").
            if let Some(report) = &self.gradient_conflict_report {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.strong("Gradient conflict between terms");
                ui.add_space(4.0);
                if let Some((a, b, cos)) = report.most_conflicting {
                    ui.colored_label(tokens.danger, format!(
                        "Most conflicting: {a} vs {b} (cosine = {cos:.2}) - these objectives are actively competing"
                    ));
                    ui.add_space(4.0);
                } else {
                    ui.colored_label(tokens.good, "No conflicting term pairs this step - every active term's gradient at least weakly agrees.");
                    ui.add_space(4.0);
                }
                let mut pairs = report.pairs.clone();
                pairs.sort_by(|a, b| a.2.partial_cmp(&b.2).unwrap_or(std::cmp::Ordering::Equal));
                egui::Grid::new("stress_solver_gradient_conflict_grid").num_columns(3).spacing([20.0, 4.0]).show(ui, |ui| {
                    for (term_a, term_b, cosine) in &pairs {
                        ui.label(*term_a);
                        ui.label(*term_b);
                        let color = if *cosine < 0.0 { tokens.danger } else { tokens.fg };
                        ui.colored_label(color, format!("{cosine:.2}"));
                        ui.end_row();
                    }
                });
            }
            // General-PINN architecture recommendations §4 (Priority 1, "physics dependency
            // graph") - which stress representation each active term actually reads, the
            // generalized answer to a question this session's Kt investigation had to resolve
            // by hand repeatedly (is a term using the network's direct output, or stress
            // derived from strain?). Empty for Kirsch's own path (documented, not a bug).
            if !self.stress_source_report.is_empty() {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.strong("Stress source by term");
                ui.add_space(4.0);
                egui::Grid::new("stress_solver_stress_source_grid").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                    for (name, source) in &self.stress_source_report {
                        ui.label(*name);
                        ui.strong(*source);
                        ui.end_row();
                    }
                });
            }
            // General-PINN architecture recommendations §13 (Priority 5, "generic boundary
            // operator system") - which classical PDE boundary-condition family each active
            // term enforces (Dirichlet/Neumann/Interface, etc.). Static per problem, same
            // "always shown once known, never gated" treatment as "Stress source by term"
            // immediately above.
            if !self.boundary_operator_report.is_empty() {
                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);
                ui.strong("Boundary condition by term");
                ui.add_space(4.0);
                egui::Grid::new("stress_solver_boundary_operator_grid").num_columns(2).spacing([20.0, 4.0]).show(ui, |ui| {
                    for (name, kind) in &self.boundary_operator_report {
                        ui.label(*name);
                        ui.strong(*kind);
                        ui.end_row();
                    }
                });
            }
        });
    }

    /// P95 of a (not-necessarily-sorted) sample - `enhancement.txt`'s own physics-residual
    /// mockup ("RMS / MAX / P95"). Empty input returns `0.0`.
    fn percentile(values: &[f32], p: f32) -> f32 {
        if values.is_empty() { return 0.0; }
        let mut sorted: Vec<f32> = values.to_vec();
        sorted.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let idx = ((sorted.len() - 1) as f32 * p).round() as usize;
        sorted[idx.min(sorted.len() - 1)]
    }

    /// `enhancement.txt` item 14 - "Model Contract" - the honest, single-instance-model
    /// version: this model is a solution to exactly ONE `ProblemSpec`, not a range (see
    /// `pinn_core::inference_envelope`'s doc comment for the verified finding behind that -
    /// the network's input is spatial coordinates only, never material/load/geometry). Lists
    /// the exact trained values and, per parameter, what `pinn_core::classify_inference`
    /// would say about changing it - real classification output, not a hand-written summary.
    fn model_contract_card(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        let Some(LoadedSpec::Plate(spec)) = &self.spec else { return };
        card(ui, tokens, |ui| {
            card_title(ui, "Model Contract");
            ui.add_space(4.0);
            ui.colored_label(tokens.fg_muted, egui::RichText::new(
                "This model solves exactly the values below - it is not a parametric surrogate \
                 (see the Parametric spec format for that workflow). Any change to a parameter \
                 classified below as anything other than \u{2018}safe for inference\u{2019} requires retraining."
            ).size(10.5));
            ui.add_space(8.0);
            egui::Grid::new("stress_solver_model_contract_grid").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
                ui.colored_label(tokens.fg_muted, egui::RichText::new("PARAMETER").size(10.0).strong());
                ui.colored_label(tokens.fg_muted, egui::RichText::new("TRAINED VALUE").size(10.0).strong());
                ui.colored_label(tokens.fg_muted, egui::RichText::new("IF CHANGED").size(10.0).strong());
                ui.end_row();

                let row = |ui: &mut egui::Ui, name: &str, value: String, class: pinn_core::ParameterClass| {
                    ui.label(name);
                    ui.label(value);
                    let (label, color) = match class {
                        pinn_core::ParameterClass::SafeForInference => ("safe for inference", tokens.good),
                        pinn_core::ParameterClass::RequiresRetraining => ("requires retraining", tokens.warning),
                        pinn_core::ParameterClass::RequiresGeometryRegeneration => ("requires geometry regen", tokens.danger),
                        pinn_core::ParameterClass::RequiresNewBoundaryConditionTraining => ("requires new BC training", tokens.danger),
                        pinn_core::ParameterClass::Unsupported => ("unsupported (new model)", tokens.danger),
                    };
                    ui.colored_label(color, label);
                    ui.end_row();
                };
                row(ui, "E", self.fmt(spec.material.e, PhysicalQuantity::Stress), pinn_core::ParameterClass::RequiresRetraining);
                row(ui, "\u{3bd}", format!("{:.4}", spec.material.nu), pinn_core::ParameterClass::RequiresRetraining);
                row(ui, "Load (Px)", self.fmt(spec.load.px, PhysicalQuantity::Stress), pinn_core::ParameterClass::RequiresRetraining);
                row(ui, "Geometry", format!("{}\u{d7}{}, {} hole(s)", self.fmt(2.0 * spec.geometry.half_w, PhysicalQuantity::Length), self.fmt(2.0 * spec.geometry.half_h, PhysicalQuantity::Length), spec.geometry.holes.len()), pinn_core::ParameterClass::RequiresGeometryRegeneration);
                row(ui, "Network", format!("hidden_dim={} n_hidden={}", spec.network.hidden_dim, spec.network.n_hidden), pinn_core::ParameterClass::Unsupported);
                row(ui, "Query point (x, y)", "any point in the trained domain".to_string(), pinn_core::ParameterClass::SafeForInference);
            });
            ui.add_space(6.0);
            ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                "Want to change E/\u{3bd}/Load and get a new solution without retraining? Use a Parametric spec \
                 (e_range/nu_range/load_range) instead - see examples/problems/parametric_single_hole_plate.toml."
            ).size(10.0));
        });
    }

    /// `enhancement.txt` items 7-12 - "Model Validity Envelope" / "New Problem" instant
    /// inference. Only shown once `parametric_ready` (the training thread is alive and
    /// waiting for exactly this - see `run_instant_inference`). Sliders are bounded to the
    /// trained ranges but can be dragged past them (via `clamp_to_range: false`) specifically
    /// so the user CAN ask for an out-of-range value and see the honest RED result - hiding
    /// that possibility would defeat the point of this card.
    fn parametric_envelope_card(&mut self, ui: &mut egui::Ui, tokens: &Tokens) {
        let Some(LoadedSpec::Parametric(spec)) = &self.spec else { return };
        let (e_range, nu_range, load_range) = (spec.e_range, spec.nu_range, spec.load_range);

        card(ui, tokens, |ui| {
            card_title(ui, "Model Validity Envelope \u{2014} New Problem");
            ui.add_space(4.0);
            ui.colored_label(tokens.fg_muted, egui::RichText::new(
                "Trained across the ranges below - drag past an endpoint to see how an out-of-range request is flagged."
            ).size(10.5));
            ui.add_space(8.0);

            // Stage A: the slider operates on `*value` (always canonical SI) but is DISPLAYED
            // and DRAGGED in the currently-selected unit system - a single unit is picked from
            // the range's own upper bound and reused for lo/hi/value together (`pick_unit`/
            // `to_unit`), never independently per-value, so this row never mixes e.g. ksi and
            // psi against each other. Writes back through `units::convert_from_display` only
            // when the slider itself reports a change, so external re-renders (a different
            // `ParametricInferenceResult` arriving) don't churn `*value` via a stale unit.
            let unit_system = self.unit_system;
            let slider = |ui: &mut egui::Ui, label: &str, value: &mut f64, lo: f64, hi: f64, qty: PhysicalQuantity| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(label).size(11.5));
                    ui.add_space(6.0);
                    let margin = (hi - lo).max(1e-12) * 0.25; // allow dragging 25% past either end
                    let unit = units::pick_unit((hi.abs()).max(lo.abs()).max(1e-300), qty, unit_system);
                    let mut disp = units::to_unit(*value, qty, unit);
                    let (lo_d, hi_d) = (units::to_unit(lo - margin, qty, unit), units::to_unit(hi + margin, qty, unit));
                    let (range_lo, range_hi) = if lo_d <= hi_d { (lo_d, hi_d) } else { (hi_d, lo_d) };
                    let resp = ui.add(egui::Slider::new(&mut disp, range_lo..=range_hi)
                        .clamping(egui::SliderClamping::Never)
                        .suffix(if unit.is_empty() { String::new() } else { format!(" {unit}") }));
                    if resp.changed() {
                        *value = units::convert_from_display(disp, qty, unit);
                    }
                    ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                        format!("(trained {:.3e} \u{2013} {:.3e}{})", units::to_unit(lo, qty, unit), units::to_unit(hi, qty, unit),
                            if unit.is_empty() { String::new() } else { format!(" {unit}") })
                    ).size(10.0));
                });
            };
            slider(ui, "E", &mut self.infer_e, e_range.min, e_range.max, PhysicalQuantity::Stress);
            slider(ui, "\u{3bd}", &mut self.infer_nu, nu_range.min, nu_range.max, PhysicalQuantity::Dimensionless);
            slider(ui, "Load (Px)", &mut self.infer_px, load_range.min, load_range.max, PhysicalQuantity::Stress);

            ui.add_space(8.0);
            if ui.button("Calculate New Solution").clicked() {
                self.run_instant_inference();
            }

            let Some(result) = self.infer_result.clone() else {
                ui.add_space(6.0);
                crate::design::components::empty_state(ui, tokens, "\u{223f}", "No query yet", "Set E/\u{3bd}/Load above and click Calculate");
                return;
            };

            ui.add_space(10.0);
            let verdict = self.classify_infer_result(&result);
            let (dot, label) = match verdict.tier {
                ValidityTier::Green => (tokens.good, "VALID"),
                ValidityTier::Yellow => (tokens.warning, "WARNING \u{2014} elevated residual"),
                ValidityTier::Red => (tokens.danger, "INVALID"),
            };
            ui.horizontal(|ui| {
                let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, dot);
                ui.colored_label(dot, egui::RichText::new(label).strong().size(12.0));
            });
            ui.colored_label(tokens.fg_subtle, egui::RichText::new(format!(
                "E={}   \u{3bd}={:.3}   Load={}", self.fmt(result.e, PhysicalQuantity::Stress), result.nu, self.fmt(result.px, PhysicalQuantity::Stress)
            )).size(10.5));
            ui.colored_label(tokens.fg_subtle, egui::RichText::new(format!(
                "BC residual RMS={:.3e} (trained baseline {:.3e})  Constitutive residual RMS={:.3e} (trained baseline {:.3e})",
                result.bc_residual_rms, verdict.bc_baseline, verdict.pde_query, verdict.pde_baseline,
            )).size(10.0));
            ui.colored_label(tokens.fg_subtle, egui::RichText::new(format!(
                "Force equilibrium error: {:.2}%   Energy-balance error: {:.2}%",
                result.reaction_force.equilibrium_error * 100.0, result.energy_balance.energy_balance_error * 100.0
            )).size(10.0));
            for line in &verdict.reasons {
                ui.colored_label(dot, egui::RichText::new(format!("\u{2022} {line}")).size(10.5));
            }

            if let Some(analysis) = result.hole_analyses.first() {
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_muted, egui::RichText::new(format!(
                    "Hole 0: nominal={}  max \u{3c3}VM={}  Kt={:.3}",
                    self.fmt(analysis.concentration.nominal_stress, PhysicalQuantity::Stress),
                    self.fmt(analysis.concentration.max_von_mises, PhysicalQuantity::Stress),
                    analysis.concentration.kt,
                )).size(10.5));
            }

            ui.add_space(8.0);
            self.field_heatmap(ui, tokens, &result.vis, &result.hole_analyses, Colormap::Viridis);

            ui.add_space(10.0);
            self.training_vs_new_case_card(ui, tokens, &result);
        });
    }

    /// `enhancement.txt` items 11/12 - real GREEN/YELLOW/RED classification, not just the
    /// `in_range` min/max check. RED whenever `!in_range` (outside the trained parameter
    /// box, unconditionally - a parameter-space extrapolation is always flagged regardless
    /// of how the residual happens to look at this one query point). Otherwise compares the
    /// query's PDE/BC residual against THIS RUN's OWN trained-baseline residual (the last
    /// real training-time reading, `self.bc_residual_rms`/the last `vis.pde_residual`'s RMS)
    /// - a ratio-based threshold derived from the model's own observed behavior, not an
    /// arbitrary hardcoded absolute constant (per `enhancement.txt` item 10's own explicit
    /// caution against unearned precision). >10x baseline -> RED even though in-range (the
    /// physics-based safety check items 11/12 ask for, independent of the range check);
    /// >3x baseline -> YELLOW; otherwise GREEN.
    fn classify_infer_result(&self, result: &pinn_core::messages::ParametricInferenceResult) -> Verdict {
        const YELLOW_FACTOR: f64 = 3.0;
        const RED_FACTOR: f64 = 10.0;

        let pde_baseline = self.training_case_snapshot.as_ref()
            .map(|s| Self::rms(&s.vis.pde_residual))
            .unwrap_or(0.0);
        let pde_query = Self::rms(&result.vis.pde_residual);
        let bc_baseline = self.bc_residual_rms;

        let mut reasons = Vec::new();
        let mut tier = ValidityTier::Green;

        if !result.in_range {
            tier = ValidityTier::Red;
            reasons.push("Outside the trained parameter range (E/\u{3bd}/Load) - the network is extrapolating.".to_string());
        }

        let ratio_of = |query: f64, baseline: f64| -> f64 {
            if baseline > 1e-300 { query / baseline } else if query > 1e-300 { f64::INFINITY } else { 1.0 }
        };
        let pde_ratio = ratio_of(pde_query as f64, pde_baseline as f64);
        let bc_ratio = ratio_of(result.bc_residual_rms, bc_baseline);
        let worst_ratio = pde_ratio.max(bc_ratio);

        if worst_ratio > RED_FACTOR {
            tier = ValidityTier::Red;
            reasons.push(format!("Residual is {worst_ratio:.1}x the trained baseline - physics validation failed independent of the range check."));
        } else if worst_ratio > YELLOW_FACTOR && tier == ValidityTier::Green {
            tier = ValidityTier::Yellow;
            reasons.push(format!("Residual is {worst_ratio:.1}x the trained baseline - moderately elevated, proceed with caution."));
        }

        // `enhancement.md` Phase 21 ("Do Not Rely Only on Min/Max") - a real coverage-density
        // signal ADDITIONAL to the rectangle check above, not a replacement for it. Ratio
        // against the training run's OWN median sample spacing (`typical_sample_spacing`),
        // same "compare against this run's own observed baseline" convention the PDE/BC
        // residual ratios above already use - not an arbitrary absolute distance constant.
        const SPARSE_COVERAGE_FACTOR: f64 = 3.0;
        if result.typical_sample_spacing > 1e-12 {
            let coverage_ratio = result.nearest_sample_distance / result.typical_sample_spacing;
            if coverage_ratio > SPARSE_COVERAGE_FACTOR {
                if tier == ValidityTier::Green { tier = ValidityTier::Yellow; }
                reasons.push(format!(
                    "Sparse training coverage near this parameter combination ({coverage_ratio:.1}x the typical sample spacing)."
                ));
            }
        }

        if reasons.is_empty() {
            reasons.push("Inside the trained range and residual is close to the trained baseline.".to_string());
        }

        Verdict { tier, reasons, pde_baseline: pde_baseline as f64, pde_query: pde_query as f64, bc_baseline }
    }

    fn rms(field: &ndarray::Array2<f32>) -> f32 {
        let vals: Vec<f32> = field.iter().copied().filter(|v| v.is_finite()).collect();
        if vals.is_empty() { return 0.0; }
        (vals.iter().map(|v| v * v).sum::<f32>() / vals.len() as f32).sqrt()
    }

    /// `enhancement.txt` item 16 - "Training Case vs New Case". Compares the last real
    /// training-time snapshot against an instant-inference result: key metrics side by side
    /// plus a NEW-ORIGINAL difference field (reusing the same field-selector/heatmap
    /// machinery, just fed a computed `VisFields` instead of one the solver sent directly).
    fn training_vs_new_case_card(&mut self, ui: &mut egui::Ui, tokens: &Tokens, new: &pinn_core::messages::ParametricInferenceResult) {
        let Some(baseline) = self.training_case_snapshot.clone() else {
            ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                "No training-case snapshot yet to compare against (wait for at least one training update)."
            ).size(10.5));
            return;
        };

        card(ui, tokens, |ui| {
            card_title(ui, "Training Case vs New Case");
            ui.add_space(6.0);
            egui::Grid::new("stress_solver_case_comparison_grid").num_columns(3).spacing([16.0, 4.0]).show(ui, |ui| {
                ui.colored_label(tokens.fg_muted, egui::RichText::new("").size(10.0));
                ui.colored_label(tokens.fg_muted, egui::RichText::new("ORIGINAL (trained)").size(10.0).strong());
                ui.colored_label(tokens.fg_muted, egui::RichText::new("NEW (instant inference)").size(10.0).strong());
                ui.end_row();
                ui.label("E");
                ui.label(self.fmt(baseline.e, PhysicalQuantity::Stress));
                ui.label(self.fmt(new.e, PhysicalQuantity::Stress));
                ui.end_row();
                ui.label("\u{3bd}");
                ui.label(format!("{:.3}", baseline.nu));
                ui.label(format!("{:.3}", new.nu));
                ui.end_row();
                ui.label("Load (Px)");
                ui.label(self.fmt(baseline.px, PhysicalQuantity::Stress));
                ui.label(self.fmt(new.px, PhysicalQuantity::Stress));
                ui.end_row();

                let max_vm = |v: &pinn_core::messages::VisFields| -> f32 {
                    v.von_mises.iter().copied().filter(|x| x.is_finite()).fold(f32::NEG_INFINITY, f32::max)
                };
                let (vm0, vm1) = (max_vm(&baseline.vis), max_vm(&new.vis));
                ui.label("Max \u{3c3}VM");
                ui.label(self.fmt(vm0 as f64, PhysicalQuantity::Stress));
                ui.label(self.fmt(vm1 as f64, PhysicalQuantity::Stress));
                ui.end_row();

                if let (Some(a0), Some(a1)) = (baseline.hole_analyses.first(), new.hole_analyses.first()) {
                    ui.label("Kt (hole 0)");
                    ui.label(format!("{:.3}", a0.concentration.kt));
                    ui.label(format!("{:.3}", a1.concentration.kt));
                    ui.end_row();
                }
            });

            let pct = |old: f32, new_v: f32| -> f32 { if old.abs() > 1e-30 { (new_v - old) / old * 100.0 } else { 0.0 } };
            let max_vm = |v: &pinn_core::messages::VisFields| -> f32 {
                v.von_mises.iter().copied().filter(|x| x.is_finite()).fold(f32::NEG_INFINITY, f32::max)
            };
            ui.add_space(6.0);
            ui.colored_label(tokens.fg_muted, egui::RichText::new(format!(
                "\u{394} Max \u{3c3}VM: {:+.1}%", pct(max_vm(&baseline.vis), max_vm(&new.vis))
            )).size(11.0));

            ui.add_space(8.0);
            ui.label(egui::RichText::new("Difference field (New \u{2212} Original)").size(11.5).strong());
            ui.add_space(4.0);
            let diff = Self::difference_vis(&baseline.vis, &new.vis);
            // No hole-boundary overlay here - `HoleAnalysis::profile`'s von_mises values are
            // absolute stresses, not deltas, so overlaying them on a DIFFERENCE field would
            // mix two different quantities on one color scale.
            self.field_heatmap(ui, tokens, &diff, &[], Colormap::Diverging);
        });
    }

    /// Elementwise `new - original` for every field, NaN where either side is NaN (outside
    /// either run's domain mask - the two masks are identical in practice since geometry is
    /// fixed in v1, but this stays correct even if that ever changes).
    fn difference_vis(a: &pinn_core::messages::VisFields, b: &pinn_core::messages::VisFields) -> pinn_core::messages::VisFields {
        let d = |x: &ndarray::Array2<f32>, y: &ndarray::Array2<f32>| -> ndarray::Array2<f32> {
            ndarray::Zip::from(x).and(y).map_collect(|&xv, &yv| if xv.is_finite() && yv.is_finite() { yv - xv } else { f32::NAN })
        };
        pinn_core::messages::VisFields {
            von_mises: d(&a.von_mises, &b.von_mises),
            sigma_xx: d(&a.sigma_xx, &b.sigma_xx),
            sigma_yy: d(&a.sigma_yy, &b.sigma_yy),
            sigma_xy: d(&a.sigma_xy, &b.sigma_xy),
            disp_u: d(&a.disp_u, &b.disp_u),
            disp_v: d(&a.disp_v, &b.disp_v),
            eps_xx: d(&a.eps_xx, &b.eps_xx),
            eps_yy: d(&a.eps_yy, &b.eps_yy),
            eps_xy: d(&a.eps_xy, &b.eps_xy),
            pde_residual: d(&a.pde_residual, &b.pde_residual),
            amr_score: d(&a.amr_score, &b.amr_score),
            collocation_density: d(&a.collocation_density, &b.collocation_density),
        }
    }

    /// Phase 16 (Neural-Network-Wide Adaptive Collocation epic) - "Hole Stress Analysis".
    /// Every number is real, computed from the actual trained network via
    /// `pinn_solver::user_problem::probe_hole_boundary_profile`/`stress_concentration_from_
    /// profile` - not a hardcoded Kt=3 (that's the idealized-infinite-plate textbook value;
    /// this plate is finite and may carry biaxial load or have other holes, so a real
    /// discrepancy is expected). This is the concrete, numerical answer to the original
    /// question that started this whole investigation: does the PINN actually resolve the
    /// stress concentration at the hole, or does it just look smooth?
    fn hole_stress_analysis_card(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        card(ui, tokens, |ui| {
            card_title(ui, "Hole Stress Analysis");
            ui.add_space(4.0);
            if self.hole_analyses.is_empty() {
                crate::design::components::empty_state(
                    ui, tokens, "\u{25cb}", "No data yet",
                    "Populates once training produces its first stress-field sample",
                );
                return;
            }
            for analysis in &self.hole_analyses {
                if analysis.hole_index > 0 {
                    ui.add_space(12.0);
                    ui.separator();
                    ui.add_space(8.0);
                }
                ui.colored_label(tokens.fg_muted, egui::RichText::new(format!("HOLE {}", analysis.hole_index + 1)).size(10.5).strong());
                ui.add_space(4.0);
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.colored_label(tokens.fg_muted, egui::RichText::new("NOMINAL STRESS").size(10.5));
                        ui.colored_label(tokens.fg, self.fmt(analysis.concentration.nominal_stress, PhysicalQuantity::Stress));
                    });
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        ui.colored_label(tokens.fg_muted, egui::RichText::new("MAX \u{3c3}\u{1d65}\u{2098} (hole)").size(10.5));
                        ui.colored_label(tokens.fg, self.fmt(analysis.concentration.max_von_mises, PhysicalQuantity::Stress));
                    });
                    ui.add_space(20.0);
                    ui.vertical(|ui| {
                        ui.colored_label(tokens.fg_muted, egui::RichText::new("Kt").size(10.5));
                        ui.colored_label(tokens.accent_strong, egui::RichText::new(format!("{:.2}", analysis.concentration.kt)).strong());
                    });
                });
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                    format!("peak at \u{3b8}={:.0}\u{b0}", analysis.concentration.max_theta_deg)
                ).size(10.5));
                ui.add_space(8.0);
                self.hole_profile_plot(ui, analysis);
            }
        });
    }

    /// Deformed-shape visualization (Results step) - the plate's outer boundary and every
    /// hole, drawn both at their original (undeformed) position and shifted by the trained
    /// model's own displacement field. Real displacements here are tiny relative to the plate
    /// (e.g. ~1e-5 m against a ~0.2 m plate) so, matching standard FEA post-processor
    /// convention, the shift is auto-exaggerated by a scale factor chosen so the largest
    /// displacement reads as a fixed fraction of the plate's half-size - the factor itself is
    /// shown so the drawing is never mistaken for true scale.
    ///
    /// Reuses data already flowing through the app: `vis.disp_u`/`disp_v` (the existing
    /// visualization grid) for the outer boundary via bilinear interpolation, and each hole's
    /// own `HoleAnalysis::profile` (`ux`/`uy`, already computed exactly at the hole ring - no
    /// interpolation needed) for the hole outlines. No new solver-side computation.
    fn deformed_shape_card(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        card(ui, tokens, |ui| {
            card_title(ui, "Deformed Shape");
            ui.add_space(4.0);
            let Some(vis) = &self.vis else {
                crate::design::components::empty_state(ui, tokens, "\u{25a1}", "No data yet", "Run training first (Train step)");
                return;
            };
            let geometry = match &self.spec {
                Some(LoadedSpec::Plate(spec)) => &spec.geometry,
                Some(LoadedSpec::Parametric(spec)) => &spec.geometry,
                _ => {
                    crate::design::components::empty_state(ui, tokens, "\u{25a1}", "Not available", "Deformed shape is only shown for plate geometries");
                    return;
                }
            };
            let half_w = geometry.half_w as f32;
            let half_h = geometry.half_h as f32;

            // Auto-exaggeration: the largest displacement magnitude seen anywhere (outer
            // boundary samples + every hole ring point) maps to `TARGET_FRACTION` of the
            // plate's smaller half-dimension. Falls back to 1.0 (no exaggeration) if every
            // displacement is exactly zero (an untrained/degenerate model) rather than
            // dividing by zero.
            const N_EDGE_SAMPLES: usize = 40;
            const TARGET_FRACTION: f32 = 0.15;
            let edge_points = Self::rectangle_boundary_points(half_w, half_h, N_EDGE_SAMPLES);
            let edge_disp: Vec<(f32, f32)> = edge_points.iter()
                .map(|&(x, y)| sample_bilinear(&vis.disp_u, &vis.disp_v, x, y, half_w, half_h))
                .collect();
            let mut max_disp = edge_disp.iter().fold(0.0_f32, |m, &(u, v)| m.max((u * u + v * v).sqrt()));
            for analysis in &self.hole_analyses {
                for p in &analysis.profile {
                    max_disp = max_disp.max((p.ux * p.ux + p.uy * p.uy).sqrt());
                }
            }
            let scale = if max_disp > 1e-12 { TARGET_FRACTION * half_w.min(half_h) / max_disp } else { 1.0 };

            ui.colored_label(tokens.fg_muted, egui::RichText::new(
                format!("Displacement exaggerated {scale:.0}\u{d7} for visibility - not true scale")
            ).size(10.5));
            ui.add_space(6.0);

            // Same `available_size().y`-collapses-inside-ScrollArea class of bug as
            // `field_heatmap` (see that fn's doc comment) - avoided here on principle even
            // though the `.max(240.0)` floor on the old upper clamp happened to keep this
            // particular card visible today. Fixed height cap instead, like every other
            // chart in this file.
            let aspect = (half_w / half_h).max(1e-3);
            const MAX_H: f32 = 320.0;
            let w = ui.available_width().max(1.0);
            let h = (w / aspect).clamp(120.0, MAX_H);
            let (rect, _resp) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
            let painter = ui.painter_at(rect);
            painter.rect_filled(rect, crate::design::radii::sm(), tokens.bg_sunken);

            // A small margin so a fully-exaggerated shape at the plate's own edge doesn't
            // clip against the card's border.
            let margin = 0.12;
            let to_screen = |x: f32, y: f32| -> egui::Pos2 {
                egui::pos2(
                    rect.min.x + (x / half_w * 0.5 * (1.0 - margin) + 0.5) * rect.width(),
                    rect.max.y - (y / half_h * 0.5 * (1.0 - margin) + 0.5) * rect.height(),
                )
            };

            // Undeformed outline - dashed, muted.
            let undeformed: Vec<egui::Pos2> = edge_points.iter().map(|&(x, y)| to_screen(x, y)).collect();
            painter.extend(egui::Shape::dashed_line(&close_loop(&undeformed), egui::Stroke::new(1.0, tokens.fg_subtle), 4.0, 3.0));

            // Deformed outline - solid, accent.
            let deformed: Vec<egui::Pos2> = edge_points.iter().zip(edge_disp.iter())
                .map(|(&(x, y), &(u, v))| to_screen(x + u * scale, y + v * scale))
                .collect();
            painter.add(egui::Shape::closed_line(deformed, egui::Stroke::new(2.0, tokens.accent_strong)));

            // Every hole: undeformed circle (dashed, muted) + deformed outline from the real
            // per-ring-point displacement (solid, same color convention `field_heatmap` uses
            // for Free/Fixed).
            for (hole_index, hole) in geometry.holes.iter().enumerate() {
                let color = match hole.bc { HoleBc::Free => tokens.good, HoleBc::Fixed => tokens.danger };
                let cx = hole.center[0] as f32;
                let cy = hole.center[1] as f32;
                let r = hole.radius as f32;
                let n_ring = 48;
                let undeformed_ring: Vec<egui::Pos2> = (0..=n_ring).map(|i| {
                    let theta = std::f32::consts::TAU * i as f32 / n_ring as f32;
                    to_screen(cx + r * theta.cos(), cy + r * theta.sin())
                }).collect();
                painter.extend(egui::Shape::dashed_line(&undeformed_ring, egui::Stroke::new(1.0, tokens.fg_subtle), 3.0, 2.0));

                if let Some(analysis) = self.hole_analyses.iter().find(|a| a.hole_index == hole_index) {
                    if !analysis.profile.is_empty() {
                        let mut deformed_ring: Vec<egui::Pos2> = analysis.profile.iter().map(|p| {
                            to_screen(p.x as f32 + p.ux * scale, p.y as f32 + p.uy * scale)
                        }).collect();
                        if let Some(&first) = deformed_ring.first() { deformed_ring.push(first); }
                        painter.add(egui::Shape::line(deformed_ring, egui::Stroke::new(2.0, color)));
                    }
                }
            }
        });
    }

    /// Evenly-spaced points around the outer rectangle's perimeter, `total` points spread
    /// proportionally to each edge's length. Used by `deformed_shape_card` for both the
    /// undeformed outline and (after bilinear-sampling displacement at each point) the
    /// deformed one.
    fn rectangle_boundary_points(half_w: f32, half_h: f32, total: usize) -> Vec<(f32, f32)> {
        let perimeter = 4.0 * (half_w + half_h);
        let n_w = ((half_w / perimeter * 2.0 * total as f32).round() as usize).max(2);
        let n_h = ((half_h / perimeter * 2.0 * total as f32).round() as usize).max(2);
        let mut pts = Vec::with_capacity(2 * (n_w + n_h));
        for i in 0..n_w { let t = i as f32 / n_w as f32; pts.push((-half_w + 2.0 * half_w * t, half_h)); }
        for i in 0..n_h { let t = i as f32 / n_h as f32; pts.push((half_w, half_h - 2.0 * half_h * t)); }
        for i in 0..n_w { let t = i as f32 / n_w as f32; pts.push((half_w - 2.0 * half_w * t, -half_h)); }
        for i in 0..n_h { let t = i as f32 / n_h as f32; pts.push((-half_w, -half_h + 2.0 * half_h * t)); }
        pts
    }

    fn hole_profile_plot(&self, ui: &mut egui::Ui, analysis: &pinn_core::messages::HoleAnalysis) {
        use egui_plot::{Line, Plot, PlotPoints};
        let id = format!("stress_solver_hole_profile_{}", analysis.hole_index);
        Plot::new(id).height(140.0).y_axis_label("Von Mises \u{3c3} (Pa)").x_axis_label("\u{3b8} (deg)").show(ui, |pui| {
            // Wrap the first point to the end so the plotted curve closes the circle visually.
            let mut pts: Vec<[f64; 2]> = analysis.profile.iter().map(|p| [p.theta_deg, p.von_mises as f64]).collect();
            if let Some(&first) = pts.first() {
                pts.push([360.0, first[1]]);
            }
            let series: PlotPoints = pts.into_iter().collect();
            pui.line(Line::new(series).name("Von Mises").width(2.0).color(Color32::from_rgb(0x6a, 0xd2, 0xf2)));
        });
    }

    /// Phase 13 (Neural-Network-Wide Adaptive Collocation epic) - "Adaptive Refinement"
    /// status card. Every number here comes from a real `AmrSweepReport` the training thread
    /// actually sent - nothing here is illustrative/placeholder (see that struct's own doc
    /// comment in `pinn_core::messages`).
    fn amr_status_card(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        card(ui, tokens, |ui| {
            card_title(ui, "Adaptive Refinement");
            ui.add_space(4.0);
            let Some(last) = self.amr_sweeps.last() else {
                let hint = if self.status == Status::Training {
                    "No sweep yet - AMR warms up before its first evaluation"
                } else {
                    "Runs automatically during training once warmup completes"
                };
                crate::design::components::empty_state(ui, tokens, "\u{25a6}", "No sweep yet", hint);
                return;
            };
            ui.horizontal(|ui| {
                let (dot, label) = if self.status == Status::Training {
                    (tokens.accent, "ACTIVE")
                } else {
                    (tokens.fg_subtle, "IDLE")
                };
                let (rect, _) = ui.allocate_exact_size(egui::vec2(9.0, 9.0), egui::Sense::hover());
                ui.painter().circle_filled(rect.center(), 4.5, dot);
                ui.colored_label(dot, egui::RichText::new(label).strong().size(11.5));
                ui.add_space(8.0);
                ui.colored_label(tokens.fg_subtle, format!("last sweep: step {}", last.step));
            });
            ui.add_space(10.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.colored_label(tokens.fg_muted, egui::RichText::new("COLLOCATION").size(10.5));
                    ui.colored_label(tokens.fg, format!("{} \u{2192} {}", last.points_before, last.points_after));
                });
                ui.add_space(24.0);
                ui.vertical(|ui| {
                    ui.colored_label(tokens.fg_muted, egui::RichText::new("SWEEP COST").size(10.5));
                    ui.colored_label(tokens.fg, format!("{:.1} ms", last.sweep_duration_ms));
                });
            });
            ui.add_space(8.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.colored_label(tokens.fg_muted, egui::RichText::new("RESIDUAL RMS").size(10.5));
                    ui.colored_label(tokens.fg, format!("{:.3e} \u{2192} {:.3e}", last.residual_rms_before, last.residual_rms_after));
                });
            });
            let improvement = if last.residual_rms_before.abs() > 1e-300 {
                (last.residual_rms_before - last.residual_rms_after) / last.residual_rms_before * 100.0
            } else {
                0.0
            };
            let improvement_color = if improvement > 0.0 { tokens.good } else { tokens.danger };
            let arrow = if improvement > 0.0 { "\u{2193}" } else { "\u{2191}" };
            ui.add_space(6.0);
            ui.colored_label(improvement_color, format!("{arrow} {:.1}% PDE RMS change this sweep", improvement.abs()));
            // Phase 8 ("Plate-With-Hole Physics Validation") diagnostic - the concrete,
            // numeric answer to "does AMR actually identify the hole region as needing
            // resolution", not just a plausible-looking picture. `0.0`/`0.0` (ratio 1.0) for a
            // feature-less geometry - there is no hole to concentrate near, which IS the
            // correct answer there.
            if last.domain_mean_density_after > 0.0 {
                let ratio_before = if last.domain_mean_density_before > 0.0 {
                    last.hole_zone_density_before / last.domain_mean_density_before
                } else { 0.0 };
                let ratio_after = last.hole_zone_density_after / last.domain_mean_density_after;
                ui.add_space(4.0);
                ui.colored_label(tokens.fg_muted, egui::RichText::new(
                    format!("Hole-zone density: {ratio_before:.1}x \u{2192} {ratio_after:.1}x domain average")
                ).size(10.5));
            }
            if self.amr_sweeps.len() > 1 {
                ui.add_space(4.0);
                ui.colored_label(tokens.fg_subtle, egui::RichText::new(format!("{} sweeps so far", self.amr_sweeps.len())).size(10.5));
            }
        });
    }

    /// Split into two independently-auto-scaled plots (a real bug fix, not a style choice):
    /// `energy_loss` and `neumann_loss`/`total_loss` routinely sit 3-5 decades apart (e.g. a
    /// plate-with-hole run's boundary term dominates total_loss almost completely while the
    /// energy term stays tiny in comparison) - egui_plot auto-fits ONE shared y-axis to the
    /// union of every line drawn in a single `Plot`, so the tiny series stretches the axis
    /// until the dominant series' real, substantial movement (a run observed to roughly halve
    /// total_loss over 600 steps) gets compressed into a visually flat-looking sliver. Giving
    /// `total_loss` its own plot means its own real range always fills the chart.
    fn loss_plot(&self, ui: &mut egui::Ui) {
        use egui_plot::{Line, Plot, PlotPoints, VLine};
        let series = |hist: &[f32]| -> PlotPoints {
            hist.iter().enumerate().map(|(i, &v)| [i as f64, v.max(1e-12).log10() as f64]).collect()
        };
        Plot::new("stress_solver_total_loss").height(150.0).y_axis_label("log10(total loss)").x_axis_label("step").show(ui, |pui| {
            pui.line(Line::new(series(&self.total_loss)).name("Total").width(2.0).color(Color32::from_rgb(0x6a, 0xd2, 0xf2)));
            // Phase 15 (Neural-Network-Wide Adaptive Collocation epic): mark every AMR sweep
            // directly on the training timeline, so the causal link (sweep -> refinement ->
            // physics change) is visually inspectable, not just numbers in a card.
            for &pos in &self.amr_marker_positions {
                pui.vline(VLine::new(pos as f64).color(Color32::from_rgba_unmultiplied(0xe0, 0xb3, 0x55, 130)).width(1.0));
            }
            // Smart adaptive architecture - mark every grow/shrink/prune/revert event on the
            // same timeline, in a distinct color from AMR's own markers above.
            for &pos in &self.architecture_event_marker_positions {
                pui.vline(VLine::new(pos as f64).color(Color32::from_rgba_unmultiplied(0xb0, 0x8c, 0xf0, 130)).width(1.0));
            }
        });
        ui.add_space(4.0);
        ui.colored_label(ui.visuals().weak_text_color(), egui::RichText::new(
            "Loss components (own scale - Boundary and Energy can differ by orders of magnitude)"
        ).size(10.0));
        Plot::new("stress_solver_loss_components").height(90.0).y_axis_label("log10(loss)").x_axis_label("step").show(ui, |pui| {
            pui.line(Line::new(series(&self.neumann_loss)).name("Boundary").width(1.3).color(Color32::from_rgb(0xe0, 0xb3, 0x55)));
            pui.line(Line::new(series(&self.energy_loss)).name("Energy").width(1.3).color(Color32::from_rgb(0x3f, 0xbf, 0xe8)));
        });
    }

    fn beam_loss_plot(&self, ui: &mut egui::Ui) {
        use egui_plot::{Line, Plot, PlotPoints};
        Plot::new("stress_solver_beam_loss").height(190.0).y_axis_label("log10(loss)").x_axis_label("sample").show(ui, |pui| {
            let series: PlotPoints = self.beam_loss.iter().enumerate()
                .map(|(i, &v)| [i as f64, v.max(1e-12).log10() as f64]).collect();
            pui.line(Line::new(series).name("Loss").width(2.0).color(Color32::from_rgb(0x6a, 0xd2, 0xf2)));
        });
    }

    fn beam_deflection_plot(&self, ui: &mut egui::Ui) {
        use egui_plot::{Line, Plot, PlotPoints};
        Plot::new("stress_solver_beam_deflection").height(190.0).y_axis_label("w(x)").x_axis_label("x").show(ui, |pui| {
            let net: PlotPoints = self.beam_eval_points.iter().map(|&(x, w, _)| [x, w]).collect();
            let exact: PlotPoints = self.beam_eval_points.iter().map(|&(x, _, e)| [x, e]).collect();
            pui.line(Line::new(exact).name("Exact").width(1.5).color(Color32::from_rgb(0xe0, 0xb3, 0x55)));
            pui.line(Line::new(net).name("Network").width(2.0).color(Color32::from_rgb(0x6a, 0xd2, 0xf2)));
        });
    }

    /// Phase 14 field/overlay/scale selector row - above the heatmap so the picker is always
    /// visible regardless of image aspect ratio.
    fn field_selector(&mut self, ui: &mut egui::Ui) {
        ui.horizontal_wrapped(|ui| {
            egui::ComboBox::from_id_salt("stress_solver_field_select")
                .selected_text(self.selected_field.label())
                .show_ui(ui, |ui| {
                    for f in SpatialField::ALL {
                        ui.selectable_value(&mut self.selected_field, f, f.label());
                    }
                    ui.selectable_value(&mut self.selected_field, SpatialField::BoundaryResidual, SpatialField::BoundaryResidual.label());
                });
            ui.add_space(10.0);
            ui.radio_value(&mut self.color_scale, ColorScale::PercentileClipped, "P2/P98");
            ui.radio_value(&mut self.color_scale, ColorScale::Linear, "Linear");
            ui.radio_value(&mut self.color_scale, ColorScale::Log, "Log");
            ui.add_space(10.0);
            ui.checkbox(&mut self.show_extrema_markers, "Max/min markers");
        });
    }

    /// `vis` is an explicit parameter (not read from `self.vis`) so this same renderer serves
    /// both a live training snapshot (`self.vis`) and a parametric instant-inference result
    /// (`self.infer_result`'s own `vis`) without duplicating this whole function. `colormap`
    /// defaults to `Viridis` for every raw-field caller; `training_vs_new_case_card` passes
    /// `Diverging` for its New-Original difference field (Stage B).
    fn field_heatmap(&mut self, ui: &mut egui::Ui, tokens: &Tokens, vis: &pinn_core::messages::VisFields, hole_analyses: &[pinn_core::messages::HoleAnalysis], colormap: Colormap) {
        let Some(field) = self.selected_field.array(vis) else {
            ui.colored_label(tokens.fg_muted, format!(
                "{} has no backing data on this solver architecture - see CLAUDE.md's Phase 9/5 audit \
                 (this DEM-based codebase never rasterizes a separate boundary-residual field).",
                self.selected_field.label(),
            ));
            return;
        };
        let (pixels, colorbar) = field_to_pixels(&field, self.color_scale, colormap);
        self.colorbar_range = colorbar;
        let (ny, nx) = field.dim();

        // argmax/argmin in the SAME row-flip convention `field_to_pixels` renders with
        // (physical y=0 at the image bottom), so a marker lands on the pixel it labels.
        let mut max_pt: Option<(usize, usize, f32)> = None;
        let mut min_pt: Option<(usize, usize, f32)> = None;
        for ((row, col), &v) in field.indexed_iter() {
            if !v.is_finite() { continue; }
            if max_pt.map_or(true, |(_, _, m)| v > m) { max_pt = Some((row, col, v)); }
            if min_pt.map_or(true, |(_, _, m)| v < m) { min_pt = Some((row, col, v)); }
        }

        let img = ColorImage { size: [nx, ny], pixels };
        let handle = ui.ctx().load_texture("stress_solver_field", img, TextureOptions::LINEAR);
        self.texture = Some(handle);

        // Real bug, confirmed via a user-supplied mid-training screenshot: this card lives
        // inside `step_content`'s `ScrollArea::vertical()`. egui 0.29's `ScrollArea` does NOT
        // give its content `ui` an infinite-height `max_rect` on the scroll axis (see
        // `egui-0.29.1/src/containers/scroll_area.rs`'s `show_viewport_dyn` - the
        // infinite-height branch is dead code behind a literal `if true`; its own comment
        // says the opposite of what you'd expect: "better to... shrink images than show a
        // horizontal scrollbar"). So `ui.available_size().y` here reflects only the space
        // left before the bottom of the CURRENTLY VISIBLE viewport, not the true scrollable
        // extent - once the Loss Trajectory chart above grows past its first data point
        // (`has_loss = total_loss.len() > 1`, true almost immediately after training starts),
        // it permanently eats enough of that viewport budget that `avail.y` here collapses
        // toward zero, flooring `w`/`h` at the `.max(1.0)` 1px clamp - an invisible sliver,
        // not a missing render. Every other chart in this file (`loss_plot`,
        // `beam_deflection_plot`, etc.) already avoids this by using a fixed `.height(...)`
        // instead of `available_size().y` - do the same here: size off `available_width()`
        // (stable regardless of scroll position) with a fixed height cap, never off the
        // scroll-collapsing `available_size().y`.
        let aspect = nx as f32 / ny as f32;
        const MAX_H: f32 = 480.0;
        let w = ui.available_width().min(MAX_H * aspect).max(1.0);
        let h = w / aspect;
        let resp = ui.image((self.texture.as_ref().unwrap().id(), egui::vec2(w, h)));
        let rect = resp.rect;

        // Hole overlay: color-coded Free (good)/Fixed (danger) ring, same semantic colors the
        // rest of this app already uses for pass/fail - no new color introduced. Geometry is
        // read from EITHER spec type (previously `Plate`-only - parametric runs got no hole
        // overlay at all, a real gap fixed here alongside the residual-colored points below).
        let geometry = match &self.spec {
            Some(LoadedSpec::Plate(spec)) => Some(&spec.geometry),
            Some(LoadedSpec::Parametric(spec)) => Some(&spec.geometry),
            _ => None,
        };
        if let Some(geometry) = geometry {
            let painter = ui.painter_at(rect);
            let half_w = geometry.half_w as f32;
            let half_h = geometry.half_h as f32;
            let to_screen = |x: f64, y: f64| -> egui::Pos2 {
                egui::pos2(
                    rect.min.x + (x as f32 / half_w * 0.5 + 0.5) * rect.width(),
                    rect.max.y - (y as f32 / half_h * 0.5 + 0.5) * rect.height(),
                )
            };
            for (hole_index, hole) in geometry.holes.iter().enumerate() {
                let center = to_screen(hole.center[0], hole.center[1]);
                let r_px = (hole.radius as f32 / (2.0 * half_w) * rect.width() + hole.radius as f32 / (2.0 * half_h) * rect.height()) * 0.5;
                let color = match hole.bc {
                    HoleBc::Free => tokens.good,
                    HoleBc::Fixed => tokens.danger,
                };
                painter.circle_stroke(center, r_px, egui::Stroke::new(2.0, color));

                // `enhancement.txt` item 4 ("boundary-condition error visualization") -
                // color-coded points around the hole boundary by the ACTUAL von Mises stress
                // there (`HoleAnalysis::profile`, already computed and already flowing
                // through `TrainingUpdate`/`ParametricInferenceResult` - no new solver-side
                // plumbing needed). Uses this field's own colorbar range so the boundary
                // points are on the same color scale as the heatmap underneath them.
                if let Some(analysis) = hole_analyses.get(hole_index).filter(|a| a.hole_index == hole_index) {
                    // Own min/max, not `self.colorbar_range` - that range belongs to whichever
                    // field is currently selected in the heatmap underneath, which may not be
                    // Von Mises, so reusing it here would mis-scale these points' colors.
                    let vm_vals: Vec<f32> = analysis.profile.iter().map(|p| p.von_mises).collect();
                    let lo = vm_vals.iter().copied().fold(f32::INFINITY, f32::min);
                    let hi = vm_vals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
                    let range = (hi - lo).max(1e-10);
                    for p in &analysis.profile {
                        let t = (p.von_mises - lo) / range;
                        let pos = to_screen(p.x, p.y);
                        painter.circle_filled(pos, 2.2, viridis(t));
                    }
                }
            }
        }

        // Phase 14 "Maximum value"/"Minimum value" overlays - pixel-space markers, exact
        // (not interpolated), on whichever field is currently selected.
        if self.show_extrema_markers {
            let painter = ui.painter_at(rect);
            let to_screen = |row: usize, col: usize| -> egui::Pos2 {
                egui::pos2(
                    rect.min.x + (col as f32 + 0.5) / nx as f32 * rect.width(),
                    rect.min.y + (row as f32 + 0.5) / ny as f32 * rect.height(),
                )
            };
            if let Some((row, col, _)) = max_pt {
                painter.circle_stroke(to_screen(row, col), 6.0, egui::Stroke::new(2.0, Color32::from_rgb(0xff, 0x55, 0x55)));
            }
            if let Some((row, col, _)) = min_pt {
                painter.circle_stroke(to_screen(row, col), 6.0, egui::Stroke::new(2.0, Color32::from_rgb(0x55, 0xaa, 0xff)));
            }
        }

        // Stage A: colorbar labels are unit-aware, tagged by whichever field is selected -
        // previously always bare regardless of dimension (a real, separate gap this closes).
        //
        // Two DISTINCT ranges are shown, on purpose, and were previously unlabeled enough to
        // be confused for each other (a real user-reported clarity gap): `clip_lo`/`clip_hi`
        // is only where the COLOR gradient's two ends are anchored (by default the 2nd/98th
        // percentile - see `ColorScale::PercentileClipped` - so a few extreme pixels don't
        // wash out the color scale for everything else); `true_min`/`true_max` below is the
        // field's REAL, unclipped extreme values, always shown regardless of color scale so a
        // clipped display can never hide how large the true max actually is.
        let qty = self.selected_field.quantity();
        let scale_label = match self.color_scale {
            ColorScale::PercentileClipped => "P2\u{2013}P98 percentile",
            ColorScale::Linear => "linear",
            ColorScale::Log => "signed-log",
        };
        ui.colored_label(tokens.fg_subtle, egui::RichText::new(
            format!("color scale ({scale_label}):")
        ).size(10.0));
        ui.horizontal(|ui| {
            ui.colored_label(tokens.fg_subtle, self.fmt(self.colorbar_range.clip_lo as f64, qty));
            ui.add_space((ui.available_width() - 110.0).max(0.0));
            ui.colored_label(tokens.fg_subtle, self.fmt(self.colorbar_range.clip_hi as f64, qty));
        });
        // The true extremes are always reported here too, numerically, regardless of
        // `ColorScale` - a clipped or log color scale can never hide how large the real max
        // actually is (a real, confirmed review finding - see this fn's doc history).
        ui.colored_label(tokens.fg_muted, egui::RichText::new(format!(
            "true range (unclipped): {} \u{2013} {}", self.fmt(self.colorbar_range.true_min as f64, qty), self.fmt(self.colorbar_range.true_max as f64, qty)
        )).size(10.5));
        // On-canvas overlays legend - three different circle kinds appear on this image and
        // none were previously explained anywhere (a real user-reported clarity gap): the hole
        // boundary ring, the per-point stress dots on it, and the max/min value markers.
        ui.add_space(4.0);
        ui.horizontal_wrapped(|ui| {
            ui.colored_label(tokens.fg_subtle, egui::RichText::new("overlays:").size(10.0));
            ui.colored_label(tokens.good, egui::RichText::new("\u{25cb} hole (free)").size(10.0));
            ui.colored_label(tokens.danger, egui::RichText::new("\u{25cb} hole (fixed)").size(10.0));
            ui.colored_label(tokens.fg_subtle, egui::RichText::new("\u{2022} von Mises on hole boundary").size(10.0));
            if self.show_extrema_markers {
                ui.colored_label(Color32::from_rgb(0xff, 0x55, 0x55), egui::RichText::new("\u{25cb} max value").size(10.0));
                ui.colored_label(Color32::from_rgb(0x55, 0xaa, 0xff), egui::RichText::new("\u{25cb} min value").size(10.0));
            }
        });
    }

    /// Approved network-diagram design (replaces the earlier per-layer bar-chart panel, which
    /// squeezed the heatmap and didn't actually show a network - see the approved artifact
    /// mockup for the full rationale). Full-width card, own row below Adaptive Refinement, so
    /// the layer-by-layer node layout has room to be legible instead of fighting the heatmap
    /// for space.
    fn network_architecture_card(&self, ui: &mut egui::Ui, tokens: &Tokens) {
        card(ui, tokens, |ui| {
            card_title(ui, "Network Architecture");
            ui.add_space(6.0);
            let Some(snap) = &self.network_snapshot else {
                crate::design::components::empty_state(ui, tokens, "\u{1f9e0}", "No data yet", "Populates once training produces its first vis-cadence sample");
                return;
            };
            if snap.layer_weights.is_empty() {
                crate::design::components::empty_state(ui, tokens, "\u{1f9e0}", "No data yet", "No layer weight data in this snapshot");
                return;
            }
            self.draw_network_diagram(ui, tokens, snap);
            // Smart adaptive architecture - latest event, if any, same "status line below the
            // visualization" placement as the Adaptive Refinement card's own sweep-count line.
            if let Some(last) = self.architecture_events.last() {
                ui.add_space(6.0);
                ui.colored_label(tokens.fg_muted, egui::RichText::new(format!(
                    "step {}: {}", last.step, last.description
                )).size(10.5));
                if self.architecture_events.len() > 1 {
                    ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                        format!("{} architecture events so far", self.architecture_events.len())
                    ).size(10.5));
                }
            }
        });
    }

    /// Draws neurons as circles and connections as lines, line width/opacity encoding |weight|
    /// - the approved design. `snap.layer_weights` is the REAL end-to-end matrix chain
    /// (`ElasticityNet::all_weight_matrices`, input layer through the output projection - see
    /// that method's doc comment for why it deliberately includes `out`, unlike `awake_mask`'s
    /// own narrower scope). Wide hidden layers are deterministically sampled down to
    /// `MAX_DISPLAY_NODES` (a fixed stride over the real indices, stable across frames - not
    /// random, so the diagram doesn't jitter between updates) - drawing all of a 64-wide
    /// layer's edges would be an unreadable hairball. Input/output layers get real variable
    /// labels when the dimension matches this toolbox's own known conventions (3/6 in, 5 out);
    /// otherwise a generic index label.
    fn draw_network_diagram(&self, ui: &mut egui::Ui, tokens: &Tokens, snap: &pinn_core::messages::NetworkSnapshot) {
        const MAX_DISPLAY_NODES: usize = 10;
        const HEIGHT: f32 = 260.0;

        let matrices = &snap.layer_weights;
        let n_node_layers = matrices.len() + 1;
        let sizes: Vec<usize> = std::iter::once(matrices[0].nrows())
            .chain(matrices.iter().map(|m| m.ncols()))
            .collect();

        let sample = |count: usize| -> Vec<usize> {
            if count <= MAX_DISPLAY_NODES {
                (0..count).collect()
            } else {
                let step = count as f64 / MAX_DISPLAY_NODES as f64;
                (0..MAX_DISPLAY_NODES).map(|i| ((i as f64 * step) as usize).min(count - 1)).collect()
            }
        };
        let indices: Vec<Vec<usize>> = sizes.iter().map(|&c| sample(c)).collect();

        let in_labels: Vec<String> = match sizes[0] {
            3 => ["x", "y", "z"].iter().map(|s| s.to_string()).collect(),
            6 => ["x", "y", "z", "e\u{2099}", "\u{3bd}\u{2099}", "p\u{2099}"].iter().map(|s| s.to_string()).collect(),
            n => (0..n).map(|i| format!("in{i}")).collect(),
        };
        let out_labels: Vec<String> = match *sizes.last().unwrap() {
            5 => ["u", "v", "\u{3c3}xx", "\u{3c3}yy", "\u{3c3}xy"].iter().map(|s| s.to_string()).collect(),
            3 => ["u", "v", "w"].iter().map(|s| s.to_string()).collect(),
            n => (0..n).map(|i| format!("out{i}")).collect(),
        };

        let (rect, _resp) = ui.allocate_exact_size(egui::vec2(ui.available_width(), HEIGHT), egui::Sense::hover());
        let painter = ui.painter_at(rect);
        painter.rect_filled(rect, 4.0, tokens.bg_sunken);

        let margin_x = 46.0;
        let caption_h = 16.0;
        let last_li = n_node_layers - 1;
        let layer_x: Vec<f32> = (0..n_node_layers).map(|li| {
            rect.min.x + margin_x + (rect.width() - 2.0 * margin_x) * li as f32 / last_li.max(1) as f32
        }).collect();

        let top = rect.min.y + 14.0;
        let bottom = rect.max.y - caption_h - 8.0;
        let node_y = |count: usize, i: usize| -> f32 {
            if count <= 1 { return (top + bottom) * 0.5; }
            top + (bottom - top) * i as f32 / (count - 1) as f32
        };

        let positions: Vec<Vec<egui::Pos2>> = indices.iter().enumerate().map(|(li, idxs)| {
            idxs.iter().enumerate().map(|(vi, _)| egui::pos2(layer_x[li], node_y(idxs.len(), vi))).collect()
        }).collect();

        // Edges first, underneath the nodes. Normalized per-matrix (this layer transition's
        // own min/max |weight| among the DISPLAYED edges) - different layers can sit at very
        // different weight scales, so one global scale would wash out real variation.
        for (mi, m) in matrices.iter().enumerate() {
            let abs_vals: Vec<f32> = indices[mi].iter()
                .flat_map(|&pi| indices[mi + 1].iter().map(move |&ni| m[[pi, ni]].abs()))
                .collect();
            let lo = abs_vals.iter().copied().fold(f32::INFINITY, f32::min);
            let hi = abs_vals.iter().copied().fold(f32::NEG_INFINITY, f32::max);
            let range = (hi - lo).max(1e-10);
            for (a_i, &pi) in indices[mi].iter().enumerate() {
                for (b_i, &ni) in indices[mi + 1].iter().enumerate() {
                    let w = m[[pi, ni]];
                    let mag = ((w.abs() - lo) / range).clamp(0.0, 1.0);
                    let alpha = (0.06 + mag * 0.82) * 255.0;
                    let width = 0.5 + mag * 3.0;
                    let color = Color32::from_rgba_unmultiplied(
                        tokens.accent_strong.r(), tokens.accent_strong.g(), tokens.accent_strong.b(), alpha as u8,
                    );
                    painter.line_segment([positions[mi][a_i], positions[mi + 1][b_i]], egui::Stroke::new(width, color));
                }
            }
        }

        // Nodes + labels + per-layer captions.
        for (li, pts) in positions.iter().enumerate() {
            for (vi, &p) in pts.iter().enumerate() {
                painter.circle_filled(p, 6.0, tokens.bg_raised);
                painter.circle_stroke(p, 6.0, egui::Stroke::new(1.4, tokens.accent_strong));
                let label = if li == 0 { in_labels.get(vi) } else if li == last_li { out_labels.get(vi) } else { None };
                if let Some(text) = label {
                    painter.text(p, egui::Align2::CENTER_CENTER, text, egui::FontId::monospace(8.5), tokens.fg);
                }
            }
            let real_count = sizes[li];
            let shown = indices[li].len();
            let caption = if shown < real_count {
                format!("{shown} of {real_count}")
            } else if li == 0 {
                "Input".to_string()
            } else if li == last_li {
                "Output".to_string()
            } else {
                format!("Hidden ({real_count})")
            };
            painter.text(
                egui::pos2(layer_x[li], rect.max.y - caption_h * 0.5),
                egui::Align2::CENTER_CENTER, caption, egui::FontId::monospace(9.0), tokens.fg_subtle,
            );
        }
    }

}

/// Problem-specific stat-rail values - a 1D beam has no energy/boundary
/// split or learning-rate schedule readout, and a plate has no
/// max-abs-error/max-abs-deflection oracle comparison, so these don't share
/// one flat field set.
enum RailTelemetry {
    Plate { total_loss: f32, energy_loss: f32, neumann_loss: f32, lr: f32 },
    Beam { loss: f32, max_abs_error: f64, max_abs_deflection: f64 },
}

/// Plain snapshot of exactly what the status rail needs - see `ui`'s own
/// comment for why this must be a free function over pre-extracted data,
/// not a `&self` method, mirroring `components::status_rail`'s own shape.
struct RailSnapshot {
    status: Status,
    step_num: usize,
    max_steps: usize,
    error_msg: Option<String>,
    /// Wall-clock time since `start_training` - shown so a heavy spec that
    /// goes many seconds between per-step-batch data updates still visibly
    /// ticks, rather than reading as frozen (see `train_started_at`'s doc
    /// comment).
    elapsed_secs: Option<f32>,
    telemetry: Option<RailTelemetry>,
}

/// Stage G (in-app spec editor), smart adaptive architecture controls - shared by the Plate
/// and Parametric editable branches (both use the same `pinn_core::problem_spec::NetworkSpec`).
/// Only rendered while `editable` (the caller already gates its whole surrounding block on
/// `Status::Idle` - see this fn's call sites' own doc comment on why). `max_hidden_dim`/
/// `max_n_hidden` default their DISPLAYED value to a generous multiple of the current
/// hidden_dim/n_hidden when unset, but only WRITE `Some(...)` back once the user actually
/// drags/types a value - rendering the control must never silently set a cap that wasn't
/// there before.
fn adaptive_architecture_controls(ui: &mut egui::Ui, network: &mut pinn_core::problem_spec::NetworkSpec) {
    ui.horizontal(|ui| {
        ui.checkbox(&mut network.adaptive, "Adaptive architecture (grow/shrink during training)");
    });
    if network.adaptive {
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.label("Max hidden_dim");
            let mut max_hd = network.max_hidden_dim.unwrap_or(network.hidden_dim * 4);
            if ui.add(egui::DragValue::new(&mut max_hd).range(network.hidden_dim..=4096)).changed() {
                network.max_hidden_dim = Some(max_hd);
            }
            ui.add_space(10.0);
            ui.label("Max n_hidden");
            let mut max_nh = network.max_n_hidden.unwrap_or(network.n_hidden + 4);
            if ui.add(egui::DragValue::new(&mut max_nh).range(network.n_hidden..=64)).changed() {
                network.max_n_hidden = Some(max_nh);
            }
        });
    }
}

fn status_rail(ui: &mut egui::Ui, tokens: &Tokens, rail: &RailSnapshot) {
    card(ui, tokens, |ui| {
        ui.set_width(208.0);
        let (dot_color, label) = match rail.status {
            Status::Idle => (tokens.fg_subtle, "IDLE"),
            Status::Training => (tokens.accent, "TRAINING"),
            Status::Done => (tokens.good, "DONE"),
            Status::Error => (tokens.danger, "ERROR"),
        };
        ui.horizontal(|ui| {
            let (rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), 5.0, dot_color);
            ui.colored_label(dot_color, egui::RichText::new(label).strong());
            if let Some(secs) = rail.elapsed_secs {
                ui.colored_label(tokens.fg_subtle, format!("\u{00b7} {secs:.0}s"));
            }
        });
        if rail.max_steps > 0 {
            ui.colored_label(tokens.fg_subtle, format!("step {} / {}", rail.step_num, rail.max_steps));
        } else {
            ui.colored_label(tokens.fg_subtle, "no spec loaded");
        }
        if let Some(err) = &rail.error_msg {
            ui.add_space(6.0);
            ui.colored_label(tokens.danger, err.as_str());
        }

        let Some(telemetry) = &rail.telemetry else {
            ui.add_space(10.0);
            ui.colored_label(tokens.fg_subtle, egui::RichText::new(
                "Loss and convergence stats appear here once training starts."
            ).size(11.5));
            return;
        };

        ui.add_space(14.0);
        let stat = |ui: &mut egui::Ui, label: &str, value: String, color: Color32, first: bool| {
            if !first {
                ui.separator();
            }
            ui.colored_label(tokens.fg_muted, egui::RichText::new(label).size(10.5));
            ui.colored_label(color, egui::RichText::new(value).font(egui::FontId::monospace(17.0)).strong());
        };
        match telemetry {
            RailTelemetry::Plate { total_loss, energy_loss, neumann_loss, lr } => {
                stat(ui, "TOTAL LOSS", format!("{total_loss:.3e}"), tokens.accent_strong, true);
                stat(ui, "ENERGY TERM", format!("{energy_loss:.3e}"), tokens.fg, false);
                stat(ui, "BOUNDARY TERM", format!("{neumann_loss:.3e}"), tokens.fg, false);
                stat(ui, "LEARNING RATE", format!("{lr:.3e}"), tokens.fg_muted, false);
            }
            RailTelemetry::Beam { loss, max_abs_error, max_abs_deflection } => {
                stat(ui, "LOSS", format!("{loss:.3e}"), tokens.accent_strong, true);
                stat(ui, "MAX ABS ERROR", format!("{max_abs_error:.3e}"), tokens.fg, false);
                stat(ui, "MAX ABS DEFLECTION", format!("{max_abs_deflection:.3e}"), tokens.fg, false);
            }
        }
    });
}
