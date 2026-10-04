//! Bridge from the bushing model to the independent `edge-check` crate, plus
//! the Results-pane rendering of its report. The crate is a pure function of
//! an [`EdgeInput`]; nothing here feeds back into `bushing_solver::compute`,
//! so the existing margins and governing check are untouched.
//!
//! To remove the feature: delete this module, the `edge_check` field and
//! `EdgeCheck` action in `mod.rs`, the `Edge-Distance Cross-Check` section
//! hook in `view.rs`, and the `edge-check` dependency.

use super::model::BushingModel;
use crate::theme::{StatusTone, Theme};
use edge_check::models::default_models;
use edge_check::runner::{run, Case, EdgeConfig, EdgeInput, EdgeMin, EdgeReport, TargetResult, TARGETS};
use edge_check::types::{BushingSpec, Geometry, Strengths};
use ratatui::text::{Line, Span};

/// What a hover/click tooltip is about: one per check the Results pane shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EdgeTopic {
    Legacy,
    StressSuperposition,
    Allowables,
    /// Dead-load plate model: only shown if the bushing is not known.
    PlasticFe,
    ContactFe,
    /// The recommended (conservative) edge distance block.
    Recommended,
}

impl EdgeTopic {
    pub fn for_model(id: &str) -> Option<EdgeTopic> {
        Some(match id {
            "analytic" => EdgeTopic::StressSuperposition,
            "allowable" => EdgeTopic::Allowables,
            "plastic" => EdgeTopic::PlasticFe,
            "contact" => EdgeTopic::ContactFe,
            _ => return None,
        })
    }
}

/// Tooltip text: a title and `(heading, body)` paragraphs.
pub struct Tip {
    pub title: &'static str,
    pub sections: &'static [(&'static str, &'static str)],
}

pub fn tip(topic: EdgeTopic) -> Tip {
    match topic {
        EdgeTopic::Legacy => Tip {
            title: "Legacy edge-distance check (solve.rs)",
            sections: &[
                ("What it does", "Bearing (the solver calls it sequencing): e/D >= (Fbru + 0.8 p) / (2 Fsu sin(theta)), i.e. the edge must be strong enough that bearing fails first. Strength: e >= Load / (2 t Fsu sin(theta)). Closed form, ported from the TS engine."),
                ("Strengths", "Instant. Differentially tested against the original engine. On the default bushing it is the most conservative of the computed numbers (about 15% above the plastic FE; not guaranteed in general). The Bearing case does not depend on the applied load. Note the + 0.8 p term credits the fit pressure (it adds to the bearing allowable), the opposite of the stress models, which charge the fit against the edge; it has no derivation in this repo."),
                ("Weaknesses", "The 0.8 factor has no derivation in this repo and credits the fit pressure, whereas elastic analysis shows the fit loads the ligament. The sin(theta) treatment of the 40 deg angle is unexplained. Only e/D enters: no real stress field, no plasticity."),
                ("Restrictions", "Nearest free edge only; countersink/flange enter through the effective thickness only; imperial units."),
            ],
        },
        EdgeTopic::StressSuperposition => Tip {
            title: "Stress superposition (primary cross-check)",
            sections: &[
                ("What it does", "Closed-form elastic stress field of a bore near a free edge (Muskhelishvili potentials fitted to zero edge traction), fit pressure + cosine pin load. Checks mean tangent-plane shear vs Fsu, mean ligament hoop tension vs Ftu, and peak von Mises vs Sy. Monte Carlo over the fit-pressure band."),
                ("Strengths", "Exact for the elastic problem (verified against closed forms and an independent FE to 1-5%). Milliseconds. Separates the effect of the interference fit and gives P(fail)."),
                ("Weaknesses", "Elastic. Shear-out assumes full shear redistribution (an upper bound on capacity), so it is optimistic (on the default bushing about 25-30% lower e/D than the plastic FE). First yield is not an ultimate criterion. Load sharing with the back side assumes a rigid, frictionless bushing."),
                ("Restrictions", "Half plane: only the nearest edge, a narrow housing's back edge is ignored. Load acts toward the edge. The half-cosine load kink costs about 0.5% of peak stress."),
            ],
        },
        EdgeTopic::Allowables => Tip {
            title: "Tabulated allowables (test-based)",
            sections: &[
                ("What it does", "Bearing: P/(D t) <= Fbru(e/D), linear between e/D 1.5 and 2.0, Fbru(2.0) above 2.0 (MMPDS rules). Shear-out: P + p D t <= 2 Fsu t (e - D/2 cos 40 deg) (classical inclined-plane rule; the fit pressure p is charged in full against the shear-out)."),
                ("Strengths", "Rooted in test data and the usual certification basis, so no model risk. Conservative against the elastic models."),
                ("Weaknesses", "Needs Fbru at e/D 1.5 per material (enter it in the material library from your MMPDS; blank means bearing needs e/D >= 2). The library's Fbru is a 'typical' value, not an A-basis one. The fit is charged only in shear-out (it confines the bore, so bearing is not reduced); load angle and geometry detail are ignored."),
                ("Restrictions", "Valid for 0.25 <= t/D <= 0.50 and e/D >= 1.5 (below that needs tests). Dry-pin values; the 40 deg rule is a rule of thumb."),
            ],
        },
        EdgeTopic::ContactFe => Tip {
            title: "Contact FE (bushing + housing, elastic-plastic)",
            sections: &[
                ("What it does", "Meshes bushing and housing, installs the interference by contact (with friction), then pushes a rigid pin toward the edge to collapse (housing J2 plane strain, flow stress (Ftu+Fty)/2). Gives the collapse and first-yield loads; solved on an edge-distance grid and interpolated."),
                ("Strengths", "Fewest assumptions: no imposed pin-pressure shape, no rigid bushing, back-side contact loss is a result, the fit is installed first (residual stress, not a dead load). Replaces the shear-out/splitting rules. Fit pressure matches Lame to 0.2%; collapse validated on NACA TN 1503 pin tests (-18/+7%)."),
                ("Weaknesses", "2D plane strain for a triaxial bore; elastic bushing; no hardening, fracture or 3D effects; friction limit lags one load step. Collapse = load plateau (1-2% scatter, ~1% mesh). 1-3 s, so only on C."),
                ("Restrictions", "Load toward the edge; straight bore and bushing; one nearest edge. No test data for the interference fit itself."),
            ],
        },
        EdgeTopic::Recommended => Tip {
            title: "Recommended edge distance (P90 / P95 / P99)",
            sections: &[
                ("What it does", "Re-runs every check over the variability the nominal margin ignores (fit pressure over its tolerance band, material strength scatter, each model's own error, bearing strength) and finds the e/D where the chance of failing is 10 % / 5 % / 1 %. The recommendation is the largest over the models, at P99."),
                ("Strengths", "Covers all the scatters at once, so a pass here means failure is unlikely even at the worst of them. Assumed: strength CV 5 % (typical handbook values, not your data); model error contact FE 9 % (spread of its 12 NACA test points), superposition 15 % (judgement, unvalidated), allowables 0 % (already statistical); fit pressure uniform over the band; normal independent factors clipped at 3 sigma."),
                ("Weaknesses", "The scatter inputs are assumptions, not measurements: other CVs move the numbers. No applied-load variability. 800 Monte-Carlo samples (about 1 % of an e/D unit of noise at P99)."),
                ("Restrictions", "A design guide, not a certification value. The legacy solver check has no variability and is shown for comparison only."),
            ],
        },
        EdgeTopic::PlasticFe => Tip {
            title: "Elastic-plastic FE limit load",
            sections: &[
                ("What it does", "Perfectly plastic (J2) plane-strain plate, flow stress min(Ftu, sqrt(3) Fsu), fit as a dead pressure, pin load raised under displacement control until the load plateaus: the collapse load. Solved on an edge-distance grid in parallel; the minimum edge distance is read off that profile."),
                ("Strengths", "Computes the failure instead of assuming a mechanism, so it tests the full-redistribution assumption (on the default bushing it needs about 30% more e/D than the elastic models). Includes the fit and the geometry."),
                ("Weaknesses", "2D: plane strain stands in for a triaxial bore (plane stress would let bearing crush mask the edge). No hardening, fracture or 3D effects. Collapse load carries about 1-2% solver and about 4% mesh error. Takes 1-2 s, so it only runs on C."),
                ("Restrictions", "Load toward the edge on the loaded half-arc; straight bore; constant fit pressure; single nearest edge. Not validated against edge-distance tests."),
            ],
        },
    }
}

/// One advisory line for the Results check list.
pub struct Advisory {
    pub text: String,
    pub topic: EdgeTopic,
}

/// One advisory (warn, never fail) for the Results check list: the
/// non-stress models - tabulated allowables, plastic FE - that say the actual
/// edge distance is short for the strength or bearing target. At most one
/// line, however many models/targets are involved (the table and the
/// tooltips carry the detail). Empty while the run is stale.
pub fn advisories(run: Option<&EdgeCheckRun>, stale: bool) -> Vec<Advisory> {
    let Some(run) = run.filter(|_| !stale) else { return Vec::new() };
    let r = &run.report;
    let mut who: Vec<(&'static str, EdgeTopic)> = Vec::new();
    let mut worst: f64 = 0.0;
    for m in r.models.iter().filter(|m| !m.field_model && m.error.is_none()) {
        let Some(topic) = EdgeTopic::for_model(m.id) else { continue };
        let mut short = false;
        for t in m.targets.iter().flatten().filter(|t| !t.target.label.starts_with("First yield") && t.margin < 0.0) {
            short = true;
            match t.e_min {
                EdgeMin::Value(v) => worst = worst.max(v),
                EdgeMin::Exceeds(v) => worst = worst.max(v),
                _ => {}
            }
        }
        if short {
            who.push((short_name(m.id), topic));
        }
    }
    let Some(&(_, topic)) = who.last() else { return Vec::new() };
    let names: Vec<&str> = who.iter().map(|(n, _)| *n).collect();
    let need = if worst > 0.0 { format!(" (up to e/D {:.2} vs {:.2} provided)", worst / r.bore_diameter, r.edge / r.bore_diameter) } else { String::new() };
    vec![Advisory { text: format!("Edge advisory: {} need more edge distance{need} - see the cross-check below", names.join(" and ")), topic }]
}

fn short_name(id: &str) -> &'static str {
    match id {
        "analytic" => "Superposition",
        "allowable" => "Allowables",
        "plastic" => "Plastic FE",
        "contact" => "Contact FE",
        _ => "",
    }
}

/// The last run and the exact input it was run for - the Results pane marks
/// it stale as soon as the live input differs.
#[derive(Debug, Clone)]
pub struct EdgeCheckRun {
    pub input: EdgeInput,
    pub report: EdgeReport,
}

/// Builds the cross-check input from the live model, or says why it can't.
pub fn build_input(model: &BushingModel) -> Result<EdgeInput, String> {
    let out = &model.output;
    let a = out.bore_tol.nominal / 2.0;
    let e = model.edge_dist;
    let t = if out.t_eff_seq > 0.0 { out.t_eff_seq } else { model.housing_len };
    if a.is_nan() || a <= 0.0 || !out.pressure.is_finite() || !model.load.is_finite() {
        return Err("bore diameter, contact pressure and load must be finite and positive".to_string());
    }
    if e.is_nan() || e <= a {
        return Err("edge distance must exceed the bore radius".to_string());
    }
    if t.is_nan() || t <= 0.0 {
        return Err("housing length must be positive".to_string());
    }
    // The stress models assume a plate wide enough that its other edges do
    // not matter (the half-plane analytic model has none at all); the FE
    // plate is sized to match: 3 e or 10 bore radii, whichever is larger.
    let reach = (3.0 * e).max(10.0 * a);
    let (p_lo, p_hi) = (out.pressure_range.min.max(0.0), out.pressure_range.max.max(0.0));
    Ok(EdgeInput {
        geom: Geometry { bore_radius: a, edge: e, thickness: t, plate_far: reach, plate_half_height: reach, plane_angle_deg: model.edge_load_angle_deg },
        strengths: Strengths::from_material(model.housing_material()),
        applied_load: model.load,
        fit_pressure: out.pressure.max(0.0),
        fit_pressure_min: p_lo,
        fit_pressure_max: p_hi,
    })
}

/// The bushing as the contact model needs it, or `None` if the inputs do not
/// describe a bushing pressed into the bore (the plate model runs instead).
fn bushing_spec(model: &BushingModel) -> Option<BushingSpec> {
    let m = model.bushing_material();
    let (ri, a) = (model.id_bushing / 2.0, model.output.bore_tol.nominal / 2.0);
    let delta = model.output.delta_total.max(0.0) / 2.0;
    (ri > 0.0 && ri < a && delta.is_finite() && m.e_ksi > 0.0).then_some(BushingSpec { inner_radius: ri, interference: delta, e: m.e_ksi * 1000.0, nu: m.nu, friction: model.friction.max(0.0) })
}

/// The input and configuration of a run, or why the live inputs cannot be
/// checked. Cheap: the heavy part is [`execute`], which the TUI runs off the
/// UI thread.
pub fn prepare(model: &BushingModel, deep: bool) -> Result<(EdgeInput, EdgeConfig), String> {
    let input = build_input(model)?;
    let cfg = EdgeConfig { include_plastic: deep, bushing: bushing_spec(model), ..EdgeConfig::default() };
    Ok((input, cfg))
}

/// Runs the models: ~0.3 s quick, ~3 s with the contact FE (`edge-check` is
/// compiled optimised even in dev builds). Blocking; call from a worker.
pub fn execute(input: EdgeInput, cfg: &EdgeConfig) -> EdgeCheckRun {
    let report = run(&default_models(cfg), &input, cfg);
    EdgeCheckRun { input, report }
}

/// `deep` also runs the elastic-plastic contact FE. Blocking (tests, scripts);
/// the UI goes through [`prepare`] + [`execute`] on a worker thread.
pub fn run_check(model: &BushingModel, deep: bool) -> Result<EdgeCheckRun, String> {
    let (input, cfg) = prepare(model, deep)?;
    Ok(execute(input, &cfg))
}

/// What a finished run says, one line per model, for the completion tooltip:
/// does the actual edge distance carry the bearing-limit load (so the edge
/// outlasts the bearing) and the applied load, and what e/D that needs.
pub fn completion_lines(run: &EdgeCheckRun) -> Vec<String> {
    let r = &run.report;
    let (si, qi) = (target_index("Strength"), target_index("Bearing"));
    let mut out = vec![format!("Actual e/D {:.2}; applied load {:.0} lbf; bearing limit {:.0} lbf.", r.edge / r.bore_diameter, r.applied_load, r.bearing_limit_load)];
    for m in &r.models {
        if EdgeTopic::for_model(m.id).is_none() {
            continue;
        }
        if let Some(e) = &m.error {
            out.push(format!("{}: not evaluated ({e}).", short_name(m.id)));
            continue;
        }
        let at = |i: usize| m.targets.get(i).and_then(|t| t.as_ref());
        let verdict = match (at(si), at(qi)) {
            (Some(s), Some(q)) => {
                let ok = s.margin >= 0.0 && q.margin >= 0.0;
                let need = match q.e_min {
                    EdgeMin::Value(v) => format!(", needs e/D {:.2}", v / r.bore_diameter),
                    EdgeMin::AtMost(v) => format!(", needs e/D under {:.2}", v / r.bore_diameter),
                    EdgeMin::Exceeds(v) => format!(", needs e/D over {:.1}", v / r.bore_diameter),
                    EdgeMin::NotSearched => String::new(),
                };
                format!("{} (bearing margin {:+.3}{need})", if ok { "OK" } else { "SHORT" }, q.margin)
            }
            _ => "no result".to_string(),
        };
        out.push(format!("{}: {verdict}.", short_name(m.id)));
    }
    if let Some((e, who)) = r.recommended(2) {
        let (text, short) = fmt_cell(r, Some(e));
        out.push(format!("Recommended (P99, conservative): e/D {text}, set by {}; actual {:.2} is {}.", short_name(who), r.edge / r.bore_diameter, if short { "SHORT" } else { "enough" }));
        if let (Some((p90, _)), Some((p95, _))) = (r.recommended(0), r.recommended(1)) {
            out.push(format!("P90 {}, P95 {}.", fmt_cell(r, Some(p90)).0, fmt_cell(r, Some(p95)).0));
        }
    }
    out.push("Hover a row in Results for the numbers behind it.".to_string());
    out
}

fn fmt_margin(m: f64) -> String {
    if m.is_finite() {
        format!("{m:+.2}")
    } else {
        "n/a".to_string()
    }
}

fn fmt_e_min(report: &EdgeReport, m: EdgeMin) -> String {
    let d = report.bore_diameter;
    match m {
        EdgeMin::Value(v) => format!("e/D {:.2}", v / d),
        EdgeMin::AtMost(v) => format!("e/D <{:.2}", v / d),
        EdgeMin::Exceeds(v) => format!("e/D >{:.1}", v / d),
        EdgeMin::NotSearched => "e/D --".to_string(),
    }
}

fn fmt_cell(report: &EdgeReport, m: Option<EdgeMin>) -> (String, bool) {
    let d = report.bore_diameter;
    let eps = 1e-9 * report.edge;
    match m {
        Some(EdgeMin::Value(v)) => (format!("{:.2}", v / d), v > report.edge + eps),
        Some(EdgeMin::AtMost(v)) => (format!("<{:.2}", v / d), false),
        Some(EdgeMin::Exceeds(v)) => (format!(">{:.1}", v / d), true),
        _ => ("-".to_string(), false),
    }
}

/// One frame of the busy glyph for a run that has been going `secs` seconds.
pub fn spinner_glyph(theme: &Theme, secs: f64) -> char {
    let tick = (secs * 10.0) as u64;
    if theme.reduced_color {
        crate::widgets::spinner::ascii_frame(tick)
    } else {
        crate::widgets::spinner::frame(tick)
    }
}

fn fmt_lbf(v: f64) -> String {
    if !v.is_finite() {
        "no limit".to_string()
    } else if v <= 0.0 {
        "none".to_string()
    } else {
        format!("{v:.0}")
    }
}

/// Share of the no-fit capacity the interference consumes, when it matters
/// (below 2 % either way is "no measurable effect": solver scatter).
fn fmt_fit_share(t: &TargetResult) -> Option<String> {
    if !(t.capacity_no_fit_lbf.is_finite() && t.capacity_no_fit_lbf > 0.0) {
        return None;
    }
    let share = 1.0 - t.capacity_lbf / t.capacity_no_fit_lbf;
    Some(if share.abs() < 0.02 { "~0%".to_string() } else { format!("{:.0}%", 100.0 * share) })
}

fn target_index(prefix: &str) -> usize {
    TARGETS.iter().position(|t| t.label.starts_with(prefix)).unwrap_or(0)
}

/// One table row: a name column then right-aligned cells; a cell is
/// coloured only when that check needs more edge distance than provided.
fn table_row<'a>(theme: &Theme, name: &str, cells: [(String, bool); 3], tail: &str) -> Line<'a> {
    let mut spans = vec![Span::raw(format!("  {name:<14}"))];
    for (i, (text, short)) in cells.into_iter().enumerate() {
        let width = [8usize, 9, 10][i];
        let style = if short { theme.status_style(StatusTone::Danger) } else { ratatui::style::Style::default() };
        spans.push(Span::styled(format!("{text:>width$}"), style));
    }
    spans.push(Span::styled(format!("{tail:>16}"), theme.disabled_style()));
    Line::from(spans)
}

/// The Results-pane section: a compact comparison table, one row per check.
/// Everything else (margins, governing mode, fit effect, notes, strengths
/// and limits) is in the row's tooltip (hover, or click to pin). `stale` =
/// the live input no longer matches the run.
pub fn section_lines<'a>(theme: &Theme, run: Option<&EdgeCheckRun>, stale: bool, model: &BushingModel) -> (Vec<Line<'a>>, Vec<(usize, EdgeTopic)>) {
    section_lines_with(theme, run, stale, model, None)
}

/// [`section_lines`] plus, while a run is in flight, a status line under the
/// title (`running` = seconds since it started and whether it is the deep run).
pub fn section_lines_with<'a>(theme: &Theme, run: Option<&EdgeCheckRun>, stale: bool, model: &BushingModel, running: Option<(f64, bool)>) -> (Vec<Line<'a>>, Vec<(usize, EdgeTopic)>) {
    let mut tags: Vec<(usize, EdgeTopic)> = Vec::new();
    let mut lines = vec![Line::from(vec![
        Span::styled("Edge-Distance Cross-Check", theme.title_style(false)),
        Span::styled("   c quick \u{b7} C +contact FE \u{b7} hover a row", theme.disabled_style()),
    ])];
    if let Some((secs, deep)) = running {
        let what = if deep { "contact FE (about 3 s)" } else { "quick set" };
        lines.push(Line::from(Span::styled(format!("  {} running: {what}, {secs:.1} s - {}", spinner_glyph(theme, secs), if run.is_some() { "the table below is the previous run" } else { "results appear here" }), theme.status_style(StatusTone::Warning))));
    }
    let Some(run) = run else {
        lines.push(Line::from(Span::styled("  Not run. c compares stress analysis, FE and tabulated allowables; C adds the bushing + housing contact FE (1-3 s).", theme.disabled_style())));
        return (lines, tags);
    };
    let r = &run.report;
    if stale {
        lines.push(Line::from(Span::styled("  inputs changed since this run - press c (or C) to refresh", theme.status_style(StatusTone::Warning))));
    }
    lines.push(Line::from(Span::styled(
        format!("  e/D {:.2} \u{b7} load {:.0} lbf \u{b7} bearing limit {:.0} lbf \u{b7} fit {:.0} psi", r.edge / r.bore_diameter, r.applied_load, r.bearing_limit_load, r.fit_pressure),
        theme.disabled_style(),
    )));
    lines.push(Line::from(Span::styled(format!("  {:<14}{:>8}{:>9}{:>10}{:>16}", "min e/D", "Strength", "Bearing", "1st yield", "P(fail) Bearing"), theme.disabled_style())));

    let (si, qi, yi) = (target_index("Strength"), target_index("Bearing"), target_index("First yield"));
    for m in &r.models {
        let Some(topic) = EdgeTopic::for_model(m.id) else { continue };
        tags.push((lines.len(), topic));
        if let Some(err) = &m.error {
            lines.push(Line::from(Span::styled(format!("  {:<14}not evaluated: {err}", short_name(m.id)), theme.status_style(StatusTone::Warning))));
            continue;
        }
        let at = |i: usize| m.targets.get(i).and_then(|t| t.as_ref());
        let cells = [fmt_cell(r, at(si).map(|t| t.e_min)), fmt_cell(r, at(qi).map(|t| t.e_min)), fmt_cell(r, at(yi).map(|t| t.e_min))];
        let tail = at(qi).and_then(|t| t.mc.as_ref()).map_or("-".to_string(), |mc| format!("{:.0}%", mc.p_fail * 100.0));
        lines.push(table_row(theme, short_name(m.id), cells, &tail));
    }
    // The solver's own check, for direct comparison.
    tags.push((lines.len(), EdgeTopic::Legacy));
    let out = &model.output;
    let legacy = |min_e_over_d: f64| (format!("{min_e_over_d:.2}"), min_e_over_d.is_finite() && min_e_over_d > r.edge / r.bore_diameter + 1e-9);
    lines.push(table_row(theme, "Legacy (solver)", [legacy(out.ed_min_strength), legacy(out.ed_min_sequence), ("-".to_string(), false)], "-"));

    // The conservative recommendation: e/D at which the failure probability
    // under the full variability is 10 / 5 / 1 %.
    let level_cell = |m: &edge_check::runner::ModelReport, l: usize| fmt_cell(r, m.level(l));
    lines.push(Line::from(Span::styled(format!("  {:<14}{:>8}{:>9}{:>10}{:>16}", "recommended", "P90", "P95", "P99", "variability"), theme.disabled_style())));
    tags.push((lines.len() - 1, EdgeTopic::Recommended));
    for m in r.models.iter().filter(|m| m.error.is_none()) {
        let Some(topic) = EdgeTopic::for_model(m.id) else { continue };
        if m.level(0).is_none() {
            continue;
        }
        tags.push((lines.len(), topic));
        let tail = format!("{:.0}% str {:.0}% mod", 100.0 * r.strength_cv, 100.0 * m.model_cv);
        lines.push(table_row(theme, short_name(m.id), [level_cell(m, 0), level_cell(m, 1), level_cell(m, 2)], &tail));
    }
    if let Some((e, who)) = r.recommended(2) {
        tags.push((lines.len(), EdgeTopic::Recommended));
        let (text, short) = fmt_cell(r, Some(e));
        let actual = r.edge / r.bore_diameter;
        let inches = match e {
            EdgeMin::Value(v) | EdgeMin::AtMost(v) | EdgeMin::Exceeds(v) => format!("{:.3} in", v),
            EdgeMin::NotSearched => "-".to_string(),
        };
        let verdict = if short { format!("actual {actual:.2} is SHORT") } else { format!("actual {actual:.2} is enough") };
        let style = if short { theme.status_style(StatusTone::Danger) } else { theme.status_style(StatusTone::Success) };
        lines.push(Line::from(Span::styled(format!("  Recommended e/D {text} ({inches}) at P99, set by {}; {verdict}", short_name(who)), style.add_modifier(ratatui::style::Modifier::BOLD))));
    }

    // What each check can carry at the actual edge distance, against the
    // load it must carry (the first row), and how much of it the fit uses.
    lines.push(Line::from(Span::styled(format!("  {:<14}{:>8}{:>9}{:>10}{:>16}", "capacity lbf", "Strength", "Bearing", "1st yield", "fit uses"), theme.disabled_style())));
    let need = [r.applied_load, r.bearing_limit_load, r.applied_load];
    lines.push(table_row(theme, "must carry", [(fmt_lbf(need[0]), false), (fmt_lbf(need[1]), false), (fmt_lbf(need[2]), false)], ""));
    for m in r.models.iter().filter(|m| m.error.is_none()) {
        let Some(topic) = EdgeTopic::for_model(m.id) else { continue };
        tags.push((lines.len(), topic));
        let cell = |i: usize| match m.targets.get(i).and_then(|t| t.as_ref()) {
            Some(t) => (fmt_lbf(t.capacity_lbf), t.capacity_lbf < need[i] * (1.0 - 1e-9)),
            None => ("-".to_string(), false),
        };
        let tail = m.targets.get(si).and_then(|t| t.as_ref()).map_or("-".to_string(), |t| fmt_fit_share(t).unwrap_or_else(|| if t.capacity_no_fit_lbf > 0.0 { "0%".to_string() } else { "-".to_string() }));
        lines.push(table_row(theme, short_name(m.id), [cell(si), cell(qi), cell(yi)], &tail));
    }
    (lines, tags)
}

/// "This run" paragraphs for a tooltip: the numbers behind a table row.
pub fn detail_lines(topic: EdgeTopic, run: Option<&EdgeCheckRun>, model: &BushingModel) -> Vec<String> {
    if topic == EdgeTopic::Legacy {
        let o = &model.output;
        return vec![
            format!("Bearing (sequencing): needs e/D {:.2}, actual {:.2} (margin {}).", o.ed_min_sequence, o.ed_actual, fmt_margin(o.sequence_margin)),
            format!("Strength: needs e/D {:.2} (margin {}).", o.ed_min_strength, fmt_margin(o.strength_margin)),
        ];
    }
    let Some(run) = run else { return vec!["Not run yet: press c (or C).".to_string()] };
    let r = &run.report;
    if topic == EdgeTopic::Recommended {
        let mut out = vec![format!("Fit pressure band {:.0}-{:.0} psi (nominal {:.0}); strength CV {:.0} %.", run.input.fit_pressure_min, run.input.fit_pressure_max, r.fit_pressure, 100.0 * r.strength_cv)];
        for (l, name) in ["P90", "P95", "P99"].iter().enumerate() {
            if let Some((e, who)) = r.recommended(l) {
                out.push(format!("{name}: e/D {} (set by {}); actual {:.2}.", fmt_cell(r, Some(e)).0, short_name(who), r.edge / r.bore_diameter));
            }
        }
        return out;
    }
    let Some(m) = r.models.iter().find(|m| EdgeTopic::for_model(m.id) == Some(topic)) else {
        return vec!["Not part of the last run (press C to include the plastic FE).".to_string()];
    };
    if let Some(e) = &m.error {
        return vec![format!("Not evaluated: {e}")];
    }
    let mut out = vec![format!("{} ms.", m.elapsed.as_millis())];
    for t in m.targets.iter().flatten() {
        let e = match t.e_min {
            EdgeMin::Value(v) => format!("{:.2}", v / r.bore_diameter),
            EdgeMin::AtMost(v) => format!("<{:.2}", v / r.bore_diameter),
            EdgeMin::Exceeds(v) => format!(">{:.1}", v / r.bore_diameter),
            EdgeMin::NotSearched => "-".to_string(),
        };
        let load = match t.target.case {
            Case::Applied => r.applied_load,
            Case::BearingLimit => r.bearing_limit_load,
        };
        out.push(format!("{}: can carry {} lbf vs {} lbf needed, margin {} ({}), min e/D {e}.", t.target.label, fmt_lbf(t.capacity_lbf), fmt_lbf(load), fmt_margin(t.margin), t.governing.label()));
        if let Some(share) = fmt_fit_share(t) {
            out.push(if share == "~0%" {
                format!("  The fit changes that capacity by under 2% ({} lbf without interference).", fmt_lbf(t.capacity_no_fit_lbf))
            } else {
                format!("  The fit uses {share} of that capacity ({} lbf without interference).", fmt_lbf(t.capacity_no_fit_lbf))
            });
        }
        if let Some(mc) = t.mc.as_ref().filter(|mc| mc.mean.is_finite()) {
            out.push(format!("  Over the fit-pressure band: mean capacity/load {:.2}, 5th percentile {:.2}, worst {:.2}; P(fail) {:.1}%.", 1.0 + mc.mean, 1.0 + mc.p05, 1.0 + mc.min, mc.p_fail * 100.0));
        }
    }
    out.extend(m.notes.iter().map(|n| format!("Note: {n}.")));
    out
}

/// Plain-text version for the exported report.
pub fn report_text(run: &EdgeCheckRun) -> String {
    let r = &run.report;
    let mut s = String::from("\nEdge-Distance Cross-Check:\n");
    s.push_str(&format!(
        "  e/D {:.3}, applied load {:.0} lbf, bearing-limit load {:.0} lbf, fit pressure {:.0} psi\n",
        r.edge / r.bore_diameter,
        r.applied_load,
        r.bearing_limit_load,
        r.fit_pressure
    ));
    for m in &r.models {
        s.push_str(&format!("  {} ({} ms)\n", m.label, m.elapsed.as_millis()));
        if let Some(err) = &m.error {
            s.push_str(&format!("    not evaluated: {err}\n"));
            continue;
        }
        for t in m.targets.iter().flatten() {
            s.push_str(&format!(
                "    {:<34} margin {} ({}), min {}{}\n",
                t.target.label,
                fmt_margin(t.margin),
                t.governing.label(),
                fmt_e_min(r, t.e_min),
                t.mc.as_ref().map_or(String::new(), |mc| format!(", P(fail) {:.1}%", mc.p_fail * 100.0))
            ));
        }
        if let (Some(a), Some(b), Some(c)) = (m.level(0), m.level(1), m.level(2)) {
            s.push_str(&format!("    recommended e/D (fail prob <= 10%/5%/1%): {} / {} / {}  (strength CV {:.0}%, model CV {:.0}%)\n", fmt_e_min(r, a), fmt_e_min(r, b), fmt_e_min(r, c), 100.0 * r.strength_cv, 100.0 * m.model_cv));
        }
        for n in &m.notes {
            s.push_str(&format!("    note: {n}\n"));
        }
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_model_runs_and_every_model_reports_something() {
        let model = BushingModel::default();
        let run = run_check(&model, false).expect("default bushing is checkable");
        assert!(run.report.models.len() >= 2, "superposition and the allowables");
        for m in &run.report.models {
            assert!(m.error.is_none(), "{}: {:?}", m.label, m.error);
            assert!(m.targets.iter().any(|t| t.is_some()), "{} produced no target", m.label);
        }
        let (lines, tags) = section_lines(&Theme::default_palette(), Some(&run), false, &model);
        let n = run.report.models.len();
        assert!(lines.len() >= 3 * n + 7, "title, context, three headers, the 'must carry' row, a row per model in each of three tables, the legacy row and the recommendation");
        assert_eq!(tags.len(), 3 * n + 3, "every model row (three tables), the legacy row, the recommended header and its line are hoverable");
        let text: Vec<String> = lines.iter().map(|l| l.spans.iter().map(|s| s.content.to_string()).collect::<String>()).collect();
        assert!(text.iter().any(|l| l.contains("Bearing") && l.contains("P(fail) Bearing")), "{text:?}");
        assert!(!text.iter().any(|l| l.contains("Sequencing")), "{text:?}");
        assert!(text.iter().any(|l| l.contains("capacity lbf")) && text.iter().any(|l| l.contains("must carry")));
        assert!(text.iter().any(|l| l.contains("P90") && l.contains("P95") && l.contains("P99")), "{text:?}");
        assert!(text.iter().any(|l| l.contains("Recommended e/D") && l.contains("at P99")), "{text:?}");
        assert!(report_text(&run).contains("recommended e/D (fail prob"));
        let d = detail_lines(EdgeTopic::Recommended, Some(&run), &model).join("\n");
        assert!(d.contains("P99:") && d.contains("strength CV"), "{d}");
        // Tooltip: capacity vs need, and the fit's share of it (stress models charge the fit).
        let d = detail_lines(EdgeTopic::StressSuperposition, Some(&run), &model).join("\n");
        assert!(d.contains("can carry") && d.contains("lbf needed") && d.contains("The fit uses") && d.contains("Bearing (bearing-limit load)"), "{d}");
        assert!(report_text(&run).contains("Edge-Distance Cross-Check"));
    }

    #[test]
    fn the_deep_run_adds_the_plastic_model() {
        let model = BushingModel::default();
        let quick = run_check(&model, false).unwrap();
        let deep = run_check(&model, true).unwrap();
        assert!(!quick.report.models.iter().any(|m| m.id == "contact"));
        let plastic = deep.report.models.iter().find(|m| m.id == "contact").expect("deep run includes the contact model");
        assert!(plastic.error.is_none(), "{:?}", plastic.error);
        assert!(plastic.targets.iter().flatten().any(|t| t.governing == edge_check::types::Mode::Collapse));
    }

    #[test]
    fn advisories_name_the_non_stress_model_that_needs_more_edge_distance_and_vanish_when_stale() {
        let mut model = BushingModel::default();
        model.recompute();
        let run = run_check(&model, true).unwrap();
        let adv = advisories(Some(&run), false);
        assert_eq!(adv.len(), 1, "one line however many checks are short: {:?}", adv.iter().map(|a| &a.text).collect::<Vec<_>>());
        assert!(adv[0].text.starts_with("Edge advisory") && adv[0].text.contains("Allowables") && adv[0].text.contains("Contact FE") && adv[0].text.contains("need more edge distance"), "{}", adv[0].text);
        assert_eq!(adv[0].topic, EdgeTopic::ContactFe, "hover shows the most demanding model first");
        assert!(advisories(Some(&run), true).is_empty(), "a stale run must not warn");
        assert!(advisories(None, false).is_empty());
        // Plenty of edge: no advisories.
        let mut far = BushingModel::default();
        far.edge_dist = 3.0;
        far.recompute();
        let run = run_check(&far, true).unwrap();
        assert!(advisories(Some(&run), false).is_empty(), "{:?}", advisories(Some(&run), false).iter().map(|a| &a.text).collect::<Vec<_>>());
    }

    #[test]
    fn a_handbook_material_with_fbru_at_e_over_d_1p5_removes_the_missing_allowable_note() {
        let mut model = BushingModel::default();
        // Default 7075 has no e/D 1.5 value -> the tabulated check says so.
        let note = |run: &EdgeCheckRun| run.report.models.iter().find(|m| m.id == "allowable").unwrap().notes.iter().any(|n| n.contains("no Fbru allowable"));
        assert!(note(&run_check(&model, false).unwrap()));
        let catalog = model.material_catalog();
        let idx = catalog.iter().position(|m| m.name.starts_with("7075") && m.name.contains("T651 Plate") && m.fbru_e15_ksi > 0.0).expect("handbook has 7075-T651 plate with an e/D 1.5 value");
        model.housing_material_index = idx;
        model.recompute();
        let run = run_check(&model, false).unwrap();
        assert!(!note(&run), "the e/D 1.5 value must be picked up from the material record");
    }

    #[test]
    fn an_impossible_edge_distance_is_reported_not_run() {
        let mut model = BushingModel::default();
        model.edge_dist = 0.01;
        model.recompute();
        assert!(run_check(&model, false).is_err());
    }
}
