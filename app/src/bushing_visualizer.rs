//! Bushing/parent-material cross-section visualizer for the Bushing
//! Workbench - an axial (side) section cut through the housing bore
//! axis, analogous to `~/Claude/Projects/profile_capabilities`'s
//! `joint_section_view.rs` ("a side view ... you see along the length
//! of the fastener/thickness of members"). That project draws by
//! building an SVG string in Rust from a plain geometry struct, rendered
//! inside a real WebView (`dioxus::desktop`/wry) where inline SVG is
//! native - this app deliberately has no WebView (see `CLAUDE.md`'s "Why
//! dioxus-native" section), so the same SVG-string approach is reused
//! but embedded as a `data:image/svg+xml;base64,...` `<img>` src instead
//! of inline `<svg>` markup - `blitz-paint`'s SVG support
//! (`usvg`+`anyrender_svg`, confirmed default-on in its `Cargo.toml`,
//! and confirmed as a real working code path by reading
//! `blitz-dom-0.2.4/src/net.rs`'s `ImageHandler::bytes`: it tries a
//! raster decode first, falls back to `usvg::parse_svg` on the same
//! bytes) treats SVG as a decoded image resource, the same category as
//! a raster image, not as live reactive DOM nodes - exactly the
//! mechanism this app already uses for the derivation-view formula PNGs
//! (`bushing_workbench.rs`'s `formula_img_src`).
//!
//! Drawing conventions researched and applied (not guessed):
//! - **45-degree cross-hatching for cut solid material** (ANSI31-style
//!   section lining) is the standard way to represent a sectioned solid
//!   in an engineering drawing - hatched areas are cut-through material,
//!   unhatched areas are cavities/bores. Adjacent parts in an assembly
//!   section conventionally hatch at *different* angles so they read as
//!   distinct parts - housing hatches at +45 deg, the bushing at -45 deg.
//! - **A shoulder/step (flange face) is drawn as two perpendicular
//!   segments - a radial line then an axial line - never a single
//!   diagonal.** A diagonal between two different radii is reserved for
//!   an actual taper (a countersink chamfer is a real cone, correctly
//!   drawn as one diagonal). Conflating the two was a real bug in the
//!   first version of this file: sampling a handful of (z, radius)
//!   breakpoints and connecting all of them with straight lines drew a
//!   flange's sharp step as a taper, because no sample point existed at
//!   "z just past the step" to hold the line square. Fixed by building
//!   each profile as an explicit, per-geometry-type point list
//!   (`outer_profile_points`/`inner_profile_points`) instead of
//!   generically sampling `evaluate_bushing_outer_radius`/
//!   `evaluate_bushing_inner_radius` (which are correct for *finding a
//!   minimum wall thickness* - `bushing-solver::solve` still uses them
//!   for exactly that - but wrong for *drawing a profile outline*, since
//!   they don't distinguish "this segment is a step" from "this segment
//!   is a taper" the way a real profile point list needs to).
//! - **Interference is called out as a dimension, not drawn to scale.**
//!   A real interference (thousandths of an inch) is invisible at any
//!   sane drawing scale, and there is no dedicated ANSI symbol for it
//!   beyond a tolerance callout (ISO 286-style fit codes or a plain
//!   dimension) - so it's rendered here as text, not as an exaggerated
//!   geometric overlap (which would misrepresent the actual fit).
//! - **Centerline uses a chain (long-dash/dot/long-dash) pattern**, the
//!   standard ANSI centerline convention, not a plain even dash.
//!
//! Orientation: the view is drawn with the axial (length) direction
//! vertical and the head end (the flange/countersink face - `z_top` and,
//! when flanged, `z_flange_top`) at the top of the image, radius
//! mirrored left/right around a vertical centerline - a bushing "standing
//! upright," per explicit request, rather than lying on its side.

use base64::Engine;
use bushing_solver::geometry::{resolve_bushing_section_params, BushingSectionInput, BushingSectionParams, BushingType, IdType};
use bushing_solver::lame::LameSample;

/// Real-world scale, in pixels per inch. The SVG's own `viewBox`/
/// intrinsic size is derived directly from the actual geometry at this
/// scale (not fitted into a fixed-aspect box) - `<img>` in `main.rs`
/// (`width:100%; height:auto`) then displays it at whatever size fits
/// the panel while preserving this exact aspect ratio, so the drawing is
/// never stretched, squeezed, or surrounded by wasted canvas margin
/// regardless of how elongated or squat a particular bushing's real
/// proportions are - the fixed-viewBox version of this file had exactly
/// that bug (a short, wide bushing rendered as a sliver in a mostly-empty
/// frame because a landscape viewBox was forced regardless of content).
const PX_PER_IN: f64 = 170.0;
const MARGIN_IN: f64 = 0.18;
/// Extra vertical margin reserved for the interference callout text.
const CALLOUT_IN: f64 = 0.22;

fn outer_profile_points(input: &BushingSectionInput, p: &BushingSectionParams) -> Vec<(f64, f64)> {
    match input.bushing_type {
        BushingType::Straight => vec![(p.z_top, p.r_outer), (p.z_bottom, p.r_outer)],
        // Shoulder: flange OD is constant out to the shank face (z_top),
        // then a step straight down to the shank OD - two perpendicular
        // segments, not a taper.
        BushingType::Flanged => vec![(p.z_flange_top, p.flange_r), (p.z_top, p.flange_r), (p.z_top, p.r_outer), (p.z_bottom, p.r_outer)],
        // Countersink: a genuine cone from the counterbore mouth
        // (z_top, ext_top) down to where it meets the shank OD (z_ext,
        // r_outer) - this one *is* a real diagonal.
        BushingType::Countersink => vec![(p.z_top, p.ext_top), (p.z_ext, p.r_outer), (p.z_bottom, p.r_outer)],
    }
}

fn inner_profile_points(input: &BushingSectionInput, p: &BushingSectionParams) -> Vec<(f64, f64)> {
    match input.id_type {
        // `inner_top_z` already accounts for a flange (the bore runs
        // through it) - see `geometry.rs`'s own doc comment.
        IdType::Straight => vec![(p.inner_top_z, p.r_inner), (p.z_bottom, p.r_inner)],
        IdType::Countersink => vec![(p.inner_top_z, p.int_top), (p.z_int, p.r_inner), (p.z_bottom, p.r_inner)],
    }
}

/// The housing's own bore profile - what the parent material's hole
/// actually looks like, which is NOT simply "whatever the bushing's
/// widest point is." Real fix for a bug in the first version of this
/// file, which sized the housing's hole off `r_outer.max(flange_r).max(ext_top)`
/// - the bushing's outermost feature, regardless of what that feature
/// actually does mechanically:
///
/// - **Flanged**: the shank (`r_outer`) is what's in interference with
///   the bore. The flange rests externally against the housing's flat
///   face - it does not enlarge the hole at all (drawing the hole as
///   wide as the flange would mean the flange has nothing to rest
///   against). Same hole shape as `Straight`.
/// - **Countersink (external OD)**: the parent material's mouth is
///   machined to *mirror* the bushing's own countersink exactly (a
///   matching nested seat, not a wider cylindrical hole clearing the
///   widest point) - so this reuses the bushing's own outer countersink
///   taper verbatim. Interference is with the shank OD (`r_outer`) below
///   the taper, same as everywhere else - the taper itself is a seated
///   fit, not a press fit.
fn housing_inner_points(input: &BushingSectionInput, p: &BushingSectionParams) -> Vec<(f64, f64)> {
    match input.bushing_type {
        BushingType::Straight | BushingType::Flanged => vec![(p.z_top, p.r_outer), (p.z_bottom, p.r_outer)],
        BushingType::Countersink => outer_profile_points(input, p),
    }
}

/// One profile's full closed ring (both mirrored sides), in (z, radius)
/// space - not yet projected to screen coordinates.
fn full_ring(points: &[(f64, f64)]) -> Vec<(f64, f64)> {
    let mut ring: Vec<(f64, f64)> = points.iter().map(|&(z, r)| (z, -r)).collect();
    ring.extend(points.iter().rev().map(|&(z, r)| (z, r)));
    ring
}

fn path_from_ring(ring: &[(f64, f64)], z_top_draw: f64, cx: f64, margin_px: f64, px_per_in: f64) -> String {
    let mut d = String::new();
    for (i, &(z, r)) in ring.iter().enumerate() {
        let x = cx + r * px_per_in;
        let y = margin_px + (z - z_top_draw) * px_per_in;
        d.push_str(&format!("{}{x:.2},{y:.2} ", if i == 0 { "M" } else { "L" }));
    }
    d.push('Z');
    d
}

/// Theme-dependent stroke/fill colors, shared by the full combined render
/// (`section_svg_data_uri`) and the geometry-only overview/detail panes
/// (`geometry_crop_svg_data_uri`) so the two always agree visually.
struct ThemeColors {
    bg: &'static str,
    fg: &'static str,
    housing_hatch: &'static str,
    housing_edge: &'static str,
    bushing_hatch: &'static str,
    bushing_edge: &'static str,
    centerline: &'static str,
}

fn theme_colors(dark: bool) -> ThemeColors {
    if dark {
        ThemeColors { bg: "#1b1e25", fg: "#c7ccd6", housing_hatch: "#7d8798", housing_edge: "#4a5060", bushing_hatch: "#3fbfe8", bushing_edge: "#8fd8f2", centerline: "#4a505f" }
    } else {
        ThemeColors { bg: "#ffffff", fg: "#2a2e35", housing_hatch: "#8a94a3", housing_edge: "#c3ccd6", bushing_hatch: "#1c7fae", bushing_edge: "#0f5a80", centerline: "#aab3bf" }
    }
}

/// Which axial (z) window the "detail" pane zooms into, and how tall
/// that window is (inches) - auto-picked by the caller from whichever
/// wall margin is currently tighter (`out.wall_neck` vs.
/// `out.wall_straight`, the same governing comparison
/// `bushing-solver::solve` itself makes), so the drawing zooms exactly
/// where the numbers say to look rather than a fixed/arbitrary crop.
#[derive(Debug, Clone, Copy)]
pub struct DetailCrop {
    pub z_center: f64,
    pub z_span: f64,
}

/// Picks the detail-pane crop window: centered on the internal
/// countersink's neck transition (`p.z_int`) when the neck wall governs,
/// otherwise centered on the parallel shank between the head end and the
/// housing face.
pub fn detail_crop(input: &BushingSectionInput, p: &BushingSectionParams, neck_governs: bool) -> DetailCrop {
    if neck_governs && input.id_type == IdType::Countersink {
        let span = (p.z_bottom - p.inner_top_z).max(0.06) * 0.65;
        DetailCrop { z_center: p.z_int, z_span: span.max(0.12) }
    } else {
        let top = p.z_top.max(p.inner_top_z);
        let span = (p.z_bottom - top).max(0.06);
        DetailCrop { z_center: (top + p.z_bottom) / 2.0, z_span: (span * 0.6).max(0.15) }
    }
}

struct GeometryPaths {
    housing_outer_path: String,
    housing_fill_path: String,
    outer_path: String,
    inner_path: String,
    bushing_fill_path: String,
}

fn build_geometry_paths(input: &BushingSectionInput, p: &BushingSectionParams, z_top_draw: f64, cx: f64, margin_px: f64, px_per_in: f64, housing_half: f64) -> GeometryPaths {
    let housing_outer_ring = full_ring(&[(p.z_top, housing_half), (p.z_bottom, housing_half)]);
    let housing_bore_pts = housing_inner_points(input, p);
    let housing_inner_ring = full_ring(&housing_bore_pts);
    let housing_outer_path = path_from_ring(&housing_outer_ring, z_top_draw, cx, margin_px, px_per_in);
    let housing_inner_path = path_from_ring(&housing_inner_ring, z_top_draw, cx, margin_px, px_per_in);
    let housing_fill_path = format!("{housing_outer_path} {housing_inner_path}");

    let outer_pts = outer_profile_points(input, p);
    let inner_pts = inner_profile_points(input, p);
    let outer_path = path_from_ring(&full_ring(&outer_pts), z_top_draw, cx, margin_px, px_per_in);
    let inner_path = path_from_ring(&full_ring(&inner_pts), z_top_draw, cx, margin_px, px_per_in);
    let bushing_fill_path = format!("{outer_path} {inner_path}");

    GeometryPaths { housing_outer_path, housing_fill_path, outer_path, inner_path, bushing_fill_path }
}

/// Renders just the housing/bushing section geometry (hatching + outline,
/// no interference/stress callout text, no stress-distribution plot) -
/// used for the compact "overview" pane (`crop: None`, whole part) and
/// the "detail" pane (`crop: Some(...)`, zoomed on the governing wall) in
/// the Bushing Workbench's visualizer dock. `px_per_in` is independent of
/// `section_svg_data_uri`'s fixed scale specifically so the detail pane
/// can render the same geometry at a much larger effective zoom without
/// needing a second copy of the drawing logic that diverges over time -
/// see `build_geometry_paths`, shared by both.
pub fn geometry_crop_svg_data_uri(input: &BushingSectionInput, dark: bool, crop: Option<DetailCrop>, px_per_in: f64) -> String {
    let p = resolve_bushing_section_params(input);
    let z_top_full = p.z_flange_top.min(p.z_top);

    let (z_top_draw, z_span) = match crop {
        Some(c) => {
            let half = c.z_span / 2.0;
            let top = (c.z_center - half).max(z_top_full);
            let bottom = (c.z_center + half).min(p.z_bottom);
            (top, (bottom - top).max(1e-3))
        }
        None => (z_top_full, (p.z_bottom - z_top_full).max(1e-3)),
    };

    let r_half = [p.r_outer, p.r_inner, p.ext_top, p.int_top, p.flange_r, input.housing_width / 2.0]
        .into_iter()
        .fold(0.0_f64, f64::max)
        .max(1e-3);
    let housing_half = (input.housing_width / 2.0).max(p.r_outer.max(p.flange_r).max(p.ext_top));

    let margin_px = 0.12 * px_per_in;
    let view_w = 2.0 * r_half * px_per_in + 2.0 * margin_px;
    let view_h = z_span * px_per_in + 2.0 * margin_px;
    let cx = view_w / 2.0;

    let c = theme_colors(dark);
    let paths = build_geometry_paths(input, &p, z_top_draw, cx, margin_px, px_per_in, housing_half);

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_w:.1} {view_h:.1}">
<defs>
<pattern id="hHouse" width="7" height="7" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
<line x1="0" y1="0" x2="0" y2="7" stroke="{housing_hatch}" stroke-width="1"/>
</pattern>
<pattern id="hBush" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(-45)">
<line x1="0" y1="0" x2="0" y2="6" stroke="{bushing_hatch}" stroke-width="1.1"/>
</pattern>
</defs>
<rect x="0" y="0" width="{view_w:.1}" height="{view_h:.1}" fill="{bg}"/>
<path d="{housing_fill_path}" fill="url(#hHouse)" fill-rule="evenodd"/>
<path d="{housing_outer_path}" fill="none" stroke="{housing_edge}" stroke-width="1"/>
<path d="{bushing_fill_path}" fill="url(#hBush)" fill-rule="evenodd"/>
<path d="{outer_path}" fill="none" stroke="{bushing_edge}" stroke-width="1.4"/>
<path d="{inner_path}" fill="none" stroke="{bushing_edge}" stroke-width="1.2" stroke-dasharray="2,2.4"/>
</svg>"##,
        bg = c.bg,
        housing_hatch = c.housing_hatch,
        housing_edge = c.housing_edge,
        bushing_hatch = c.bushing_hatch,
        bushing_edge = c.bushing_edge,
        housing_fill_path = paths.housing_fill_path,
        housing_outer_path = paths.housing_outer_path,
        bushing_fill_path = paths.bushing_fill_path,
        outer_path = paths.outer_path,
        inner_path = paths.inner_path,
    );

    format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

/// Stress results to overlay on the drawing - the same numbers already
/// shown in the results panel's `MarginRow`s
/// (`out.stress_hoop_housing`/`out.housing_ms`, `out.bushing_stress_field`
/// etc.), not separately derived here. Two things are overlaid, not one:
/// a margin-of-safety severity (fail/marginal/pass) tint on each
/// material's fill, using this app's own `--danger`/`--warning`/`--good`
/// palette (`main.rs`) so the drawing and the margin rows agree on what
/// counts as marginal; and the actual hoop-stress *distribution* across
/// each region's wall (`bushing_field`/`housing_field` -
/// `bushing-solver::lame`'s real Lame closed-form solution sampled
/// at 41 points, not an interpolation between the two boundary values) -
/// what was actually requested: "the plot of stress distribution," not
/// just the two boundary numbers annotated as text.
#[derive(Debug, Clone)]
pub struct StressOverlay {
    pub housing_stress_psi: f64,
    pub housing_ms: f64,
    pub bushing_stress_psi: f64,
    pub bushing_ms: f64,
    pub bushing_field: Vec<LameSample>,
    pub housing_field: Vec<LameSample>,
}

fn fmt_ms(ms: f64) -> String {
    if ms.is_finite() {
        format!("{ms:+.2}")
    } else {
        "\u{2014}".to_string()
    }
}

fn severity_color(ms: f64, dark: bool) -> &'static str {
    if !ms.is_finite() {
        if dark { "#4a5060" } else { "#c3ccd6" }
    } else if ms < 0.0 {
        if dark { "#e2657a" } else { "#c2394f" }
    } else if ms < 0.15 {
        if dark { "#e0b355" } else { "#a1751f" }
    } else if dark {
        "#52c98a"
    } else {
        "#29875a"
    }
}

/// Draws hoop-stress (sigma_theta) vs. radius as a real line plot, radius
/// ascending left-to-right across `bushing_field` then `housing_field`
/// (they share a radius at the interface, so the two curves join there,
/// not a gap) - a standard distribution-vs-position chart, not an
/// attempt to align its x-axis with the mirrored cross-section above it
/// (that would need mirroring the plot too, which reads as two separate,
/// harder-to-compare curves for no real benefit over one continuous one).
fn stress_plot_svg(bushing_field: &[LameSample], housing_field: &[LameSample], width: f64, height: f64, fg: &str, bushing_color: &str, housing_color: &str, grid_color: &str) -> String {
    if bushing_field.is_empty() || housing_field.is_empty() {
        return String::new();
    }
    let r_min = bushing_field.first().unwrap().r;
    let r_interface = bushing_field.last().unwrap().r;
    let r_max = housing_field.last().unwrap().r;
    let r_span = (r_max - r_min).max(1e-9);

    let s_min = bushing_field
        .iter()
        .chain(housing_field.iter())
        .map(|s| s.sigma_theta)
        .fold(0.0_f64, f64::min);
    let s_max = bushing_field
        .iter()
        .chain(housing_field.iter())
        .map(|s| s.sigma_theta)
        .fold(0.0_f64, f64::max);
    let s_span = (s_max - s_min).max(1.0);

    let plot_x = 34.0; // left gutter for the stress-axis labels
    let plot_w = width - plot_x;
    let x_for_r = |r: f64| plot_x + (r - r_min) / r_span * plot_w;
    let y_for_s = |s: f64| height - (s - s_min) / s_span * height;

    let poly = |field: &[LameSample]| -> String { field.iter().map(|s| format!("{:.1},{:.1}", x_for_r(s.r), y_for_s(s.sigma_theta))).collect::<Vec<_>>().join(" ") };

    let zero_y = y_for_s(0.0);
    let interface_x = x_for_r(r_interface);

    format!(
        r#"<line x1="{plot_x:.1}" y1="{zero_y:.1}" x2="{width:.1}" y2="{zero_y:.1}" stroke="{grid_color}" stroke-width="1" stroke-dasharray="3,3"/>
<line x1="{interface_x:.1}" y1="0" x2="{interface_x:.1}" y2="{height:.1}" stroke="{grid_color}" stroke-width="1" stroke-dasharray="2,2"/>
<polyline points="{bpoly}" fill="none" stroke="{bushing_color}" stroke-width="1.6"/>
<polyline points="{hpoly}" fill="none" stroke="{housing_color}" stroke-width="1.6"/>
<text x="0" y="9" font-size="9" fill="{fg}">{s_max:.0}</text>
<text x="0" y="{bottom_label_y:.1}" font-size="9" fill="{fg}">{s_min:.0}</text>
<text x="0" y="{zero_label_y:.1}" font-size="9" fill="{fg}">0</text>"#,
        bpoly = poly(bushing_field),
        hpoly = poly(housing_field),
        bottom_label_y = height - 2.0,
        zero_label_y = zero_y + 3.0,
    )
}

/// Renders the housing (parent material) + installed bushing as an
/// axial cross-section, colored to match the app's own dark/light theme
/// variables (`main.rs`'s `--panel-bg`/`--border`/`--accent`/`--fg` -
/// hardcoded here since an embedded SVG image can't read the host
/// document's CSS custom properties). `interference` is the achieved
/// diametral interference (`out.od_installed - bore_dia`, in.), shown as
/// a dimension callout rather than drawn to scale (see module docs).
/// `stress` overlays each material's margin-of-safety severity tint,
/// boundary stress/MS text callouts, and a real hoop-stress-vs-radius
/// distribution plot below the section (`stress_plot_svg`) - not a
/// single boundary number presented as if it applied everywhere across
/// the wall.
///
/// The bushing's own bore (its ID - the through-hole that remains once
/// it's installed in the housing) is drawn with a dotted stroke, per
/// explicit request, to read as "this is the open hole," distinct from
/// the housing/bushing interference-fit interface (a real solid-to-solid
/// boundary, drawn solid) and the housing's own outer extent (also
/// solid).
pub fn section_svg_data_uri(input: &BushingSectionInput, interference: f64, stress: StressOverlay, dark: bool) -> String {
    let p = resolve_bushing_section_params(input);

    let z_top_draw = p.z_flange_top.min(p.z_top);
    let z_span = (p.z_bottom - z_top_draw).max(1e-3);
    let r_half = [p.r_outer, p.r_inner, p.ext_top, p.int_top, p.flange_r, input.housing_width / 2.0]
        .into_iter()
        .fold(0.0_f64, f64::max)
        .max(1e-3);

    let margin_px = MARGIN_IN * PX_PER_IN;
    let callout_px = CALLOUT_IN * PX_PER_IN;
    // SVG has no text wrapping - the callout text's pixel width doesn't
    // scale with the bushing's own geometry the way the rest of the
    // canvas does, so a narrow bushing needs a floor here or its callout
    // text runs off the edge (a real bug in the first version of this
    // overlay: a 0.5in-bore drawing is only ~300px wide at this scale,
    // nowhere near enough for two lines of stress text).
    const MIN_VIEW_W: f64 = 460.0;
    const PLOT_H: f64 = 120.0;
    let view_w = (2.0 * r_half * PX_PER_IN + 2.0 * margin_px).max(MIN_VIEW_W);
    let geometry_h = z_span * PX_PER_IN + 2.0 * margin_px;
    let callout_block_h = callout_px * 2.0;
    let view_h = geometry_h + callout_block_h + PLOT_H + margin_px;
    let cx = view_w / 2.0;

    let y_for_z = |z: f64| margin_px + (z - z_top_draw) * PX_PER_IN;

    let c = theme_colors(dark);
    let (bg, fg, housing_hatch, housing_edge, bushing_hatch, bushing_edge, centerline) = (c.bg, c.fg, c.housing_hatch, c.housing_edge, c.bushing_hatch, c.bushing_edge, c.centerline);
    let housing_tint = severity_color(stress.housing_ms, dark);
    let bushing_tint = severity_color(stress.bushing_ms, dark);

    let housing_half = (input.housing_width / 2.0).max(p.r_outer.max(p.flange_r).max(p.ext_top));
    let paths = build_geometry_paths(input, &p, z_top_draw, cx, margin_px, PX_PER_IN, housing_half);
    // The housing/bushing interference-fit interface is a real solid-to-
    // solid boundary (drawn solid), separately from the bushing's own
    // bore below (drawn dotted) - so only the OUTER ring gets its own
    // stroke here; the shared interface line is stroked once, as part of
    // the bushing's own outer ring below, not duplicated.
    let housing_outer_stroke_path = paths.housing_outer_path.clone();
    let housing_fill_path = paths.housing_fill_path;
    let outer_path = paths.outer_path;
    let inner_path = paths.inner_path;
    let bushing_fill_path = paths.bushing_fill_path;

    let cy_top = y_for_z(z_top_draw) - margin_px * 0.4;
    let cy_bot = y_for_z(p.z_bottom) + margin_px * 0.4;
    // Three separate lines, not one long concatenated string - SVG
    // doesn't wrap text, and a single combined line is easily 60+
    // characters, wider than this drawing typically is even after the
    // `MIN_VIEW_W` floor above.
    let line_h = callout_px * 0.62;
    let callout_y = geometry_h + line_h * 0.9;
    let housing_label_y = callout_y + line_h;
    let bushing_label_y = housing_label_y + line_h;
    let plot_y = geometry_h + callout_block_h;
    let plot_svg = stress_plot_svg(&stress.bushing_field, &stress.housing_field, view_w, PLOT_H, fg, bushing_edge, housing_edge, centerline);

    let svg = format!(
        r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {view_w:.1} {view_h:.1}" font-family="sans-serif">
<defs>
<pattern id="hHouse" width="7" height="7" patternUnits="userSpaceOnUse" patternTransform="rotate(45)">
<line x1="0" y1="0" x2="0" y2="7" stroke="{housing_hatch}" stroke-width="1"/>
</pattern>
<pattern id="hBush" width="6" height="6" patternUnits="userSpaceOnUse" patternTransform="rotate(-45)">
<line x1="0" y1="0" x2="0" y2="6" stroke="{bushing_hatch}" stroke-width="1.1"/>
</pattern>
</defs>
<rect x="0" y="0" width="{view_w:.1}" height="{view_h:.1}" fill="{bg}"/>
<path d="{housing_fill_path}" fill="{housing_tint}" fill-opacity="0.28" fill-rule="evenodd"/>
<path d="{housing_fill_path}" fill="url(#hHouse)" fill-rule="evenodd"/>
<path d="{housing_outer_stroke_path}" fill="none" stroke="{housing_edge}" stroke-width="1"/>
<line x1="{cx:.2}" y1="{cy_top:.2}" x2="{cx:.2}" y2="{cy_bot:.2}" stroke="{centerline}" stroke-width="1" stroke-dasharray="16,3,2,3"/>
<path d="{bushing_fill_path}" fill="{bushing_tint}" fill-opacity="0.3" fill-rule="evenodd"/>
<path d="{bushing_fill_path}" fill="url(#hBush)" fill-rule="evenodd"/>
<path d="{outer_path}" fill="none" stroke="{bushing_edge}" stroke-width="1.4"/>
<path d="{inner_path}" fill="none" stroke="{bushing_edge}" stroke-width="1.2" stroke-dasharray="2,2.4"/>
<text x="{cx:.1}" y="{callout_y:.1}" font-size="11.5" fill="{fg}" text-anchor="middle">Interference: {interference:+.4} in (bore {bore:.4} / OD {od:.4})</text>
<text x="{cx:.1}" y="{housing_label_y:.1}" font-size="11.5" fill="{fg}" text-anchor="middle">Housing: {hstress:.0} psi, MS {hms}</text>
<text x="{cx:.1}" y="{bushing_label_y:.1}" font-size="11.5" fill="{fg}" text-anchor="middle">Bushing: {bstress:.0} psi, MS {bms}</text>
<g transform="translate(0,{plot_y:.1})">{plot_svg}</g>
</svg>"##,
        bore = input.bore_dia,
        od = input.od_bushing,
        hstress = stress.housing_stress_psi,
        hms = fmt_ms(stress.housing_ms),
        bstress = stress.bushing_stress_psi,
        bms = fmt_ms(stress.bushing_ms),
    );

    format!("data:image/svg+xml;base64,{}", base64::engine::general_purpose::STANDARD.encode(svg))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn straight_input() -> BushingSectionInput {
        BushingSectionInput {
            bore_dia: 0.5,
            housing_len: 0.5,
            housing_width: 1.5,
            id_bushing: 0.375,
            bushing_type: BushingType::Straight,
            id_type: IdType::Straight,
            flange_od: 0.0,
            flange_thk: 0.0,
            od_bushing: 0.5015,
            cs_external: None,
            cs_internal: None,
        }
    }

    fn decode(data_uri: &str) -> String {
        let b64 = data_uri.strip_prefix("data:image/svg+xml;base64,").expect("must be an svg data uri");
        String::from_utf8(base64::engine::general_purpose::STANDARD.decode(b64).unwrap()).unwrap()
    }

    fn sample_stress() -> StressOverlay {
        use bushing_solver::lame::sample_lame_field;
        StressOverlay {
            housing_stress_psi: 10475.0,
            housing_ms: 5.68,
            bushing_stress_psi: -31407.0,
            bushing_ms: 0.59,
            bushing_field: sample_lame_field(0.1875, 0.25, 0.0, 8794.0, 0.0, 0.34, 41),
            housing_field: sample_lame_field(0.25, 0.846, 8794.0, 0.0, 0.0, 0.33, 41),
        }
    }

    #[test]
    fn straight_bushing_renders_a_valid_svg() {
        let svg = decode(&section_svg_data_uri(&straight_input(), 0.0015, sample_stress(), false));
        assert!(svg.starts_with("<svg"));
        assert!(svg.ends_with("</svg>"));
        assert!(svg.contains("Interference"));
        assert!(svg.contains("Housing") && svg.contains("Bushing"));
    }

    /// Reproduces the EXACT parse step Blitz's own image loader performs
    /// on an embedded SVG `<img>` src (`blitz-dom-0.2.4/src/util.rs`'s
    /// `parse_svg`: `usvg::Tree::from_data` with default `Options`) -
    /// `straight_bushing_renders_a_valid_svg` above only checks the SVG
    /// *string* looks superficially sane, which is not the same claim as
    /// "Blitz can actually decode and render this image." A user reported
    /// the lightbox's full-size render (this exact function's output)
    /// showing as an empty rectangle in the real app while the geometry-
    /// only overview/detail panes rendered fine - this test is how that
    /// gets proven or ruled out with real evidence instead of guessing.
    fn assert_usvg_parses(svg: &str, label: &str) {
        let opts = usvg::Options::default();
        match usvg::Tree::from_str(svg, &opts) {
            Ok(tree) => {
                let size = tree.size();
                assert!(size.width() > 0.0 && size.height() > 0.0, "{label}: usvg parsed but reported a zero/degenerate size ({}x{})", size.width(), size.height());
            }
            Err(e) => panic!("{label}: usvg::Tree::from_str failed - this is exactly why the real app shows a blank image: {e}"),
        }
    }

    #[test]
    fn full_lightbox_svg_parses_with_usvg_the_same_way_blitz_dom_parses_it() {
        let svg = decode(&section_svg_data_uri(&straight_input(), 0.0015, sample_stress(), false));
        assert_usvg_parses(&svg, "straight, full section_svg_data_uri");

        let mut cs_input = straight_input();
        cs_input.bushing_type = BushingType::Countersink;
        cs_input.id_type = IdType::Countersink;
        cs_input.cs_external = Some((0.6, 0.06));
        cs_input.cs_internal = Some((0.45, 0.1));
        let cs_svg = decode(&section_svg_data_uri(&cs_input, 0.0015, sample_stress(), true));
        assert_usvg_parses(&cs_svg, "countersink, full section_svg_data_uri");
    }

    #[test]
    fn geometry_crop_svg_also_parses_with_usvg() {
        let overview = decode(&geometry_crop_svg_data_uri(&straight_input(), false, None, 90.0));
        assert_usvg_parses(&overview, "overview geometry_crop_svg_data_uri");
    }

    #[test]
    fn failing_margin_uses_the_danger_color_for_that_part() {
        let fail_stress = StressOverlay { housing_stress_psi: 90000.0, housing_ms: -0.2, ..sample_stress() };
        let svg = decode(&section_svg_data_uri(&straight_input(), 0.0015, fail_stress, false));
        assert!(svg.contains(severity_color(-0.2, false)), "failing housing margin must use the danger tint");
    }

    #[test]
    fn flanged_bushing_outer_profile_is_a_shoulder_not_a_taper() {
        let mut input = straight_input();
        input.bushing_type = BushingType::Flanged;
        input.flange_od = 0.75;
        input.flange_thk = 0.063;
        let p = resolve_bushing_section_params(&input);
        let pts = outer_profile_points(&input, &p);
        // Two consecutive points must share the SAME z (a pure radial
        // step) - a diagonal/taper would never have two points at
        // identical z.
        assert!(pts.windows(2).any(|w| (w[0].0 - w[1].0).abs() < 1e-9 && (w[0].1 - w[1].1).abs() > 1e-9), "flange step must have a constant-z segment: {pts:?}");
    }

    #[test]
    fn countersink_outer_profile_is_a_genuine_taper() {
        let mut input = straight_input();
        input.bushing_type = BushingType::Countersink;
        input.cs_external = Some((0.6, 0.06));
        let p = resolve_bushing_section_params(&input);
        let pts = outer_profile_points(&input, &p);
        // The first segment must change BOTH z and radius (a real cone).
        assert!((pts[0].0 - pts[1].0).abs() > 1e-9 && (pts[0].1 - pts[1].1).abs() > 1e-9, "countersink taper must move in both axes: {pts:?}");
    }

    #[test]
    fn flanged_bushing_housing_bore_is_the_shank_od_not_the_flange() {
        let mut input = straight_input();
        input.bushing_type = BushingType::Flanged;
        input.flange_od = 0.75;
        input.flange_thk = 0.063;
        let p = resolve_bushing_section_params(&input);
        let pts = housing_inner_points(&input, &p);
        // The housing's hole must be sized off the shank (r_outer) -
        // never the flange, which rests externally against the housing
        // face and does not enlarge the hole. A pre-fix bug sized the
        // hole off `r_outer.max(flange_r)`, drawing an oversized bore
        // the flange had nothing to seat against.
        assert!(pts.iter().all(|&(_, r)| (r - p.r_outer).abs() < 1e-9), "housing bore must equal the shank OD everywhere: {pts:?}");
        assert!(p.flange_r > p.r_outer, "test fixture must have a flange wider than the shank for this assertion to mean anything");
    }

    #[test]
    fn countersink_housing_bore_mirrors_the_bushings_own_countersink_taper() {
        let mut input = straight_input();
        input.bushing_type = BushingType::Countersink;
        input.cs_external = Some((0.6, 0.06));
        let p = resolve_bushing_section_params(&input);
        let housing_pts = housing_inner_points(&input, &p);
        let bushing_pts = outer_profile_points(&input, &p);
        // The parent material's mouth must be machined to the exact same
        // profile as the bushing's own countersink - a matching nested
        // seat, not a wider cylindrical clearance hole. A pre-fix bug
        // sized the whole hole off `ext_top` (the countersink's widest
        // point) as a constant radius, which is neither a seat nor a
        // correct interference bore.
        assert_eq!(housing_pts, bushing_pts);
        // And below the taper, that shared profile must settle at the
        // shank OD - the actual interference diameter.
        assert!((housing_pts.last().unwrap().1 - p.r_outer).abs() < 1e-9);
    }

    #[test]
    fn all_geometry_combinations_render_without_panicking_or_nan() {
        for bushing_type in [BushingType::Straight, BushingType::Flanged, BushingType::Countersink] {
            for id_type in [IdType::Straight, IdType::Countersink] {
                let mut input = straight_input();
                input.bushing_type = bushing_type;
                input.id_type = id_type;
                input.flange_od = 0.75;
                input.flange_thk = 0.063;
                input.cs_external = Some((0.6, 0.06));
                input.cs_internal = Some((0.45, 0.1));
                let svg = decode(&section_svg_data_uri(&input, 0.0015, sample_stress(), dark_variant(bushing_type)));
                assert!(!svg.contains("NaN") && !svg.contains("inf"), "{bushing_type:?}/{id_type:?}: {svg}");
            }
        }
    }

    fn dark_variant(t: BushingType) -> bool {
        matches!(t, BushingType::Countersink)
    }

    #[test]
    fn geometry_crop_svg_renders_without_callouts_or_plot() {
        let svg = decode(&geometry_crop_svg_data_uri(&straight_input(), false, None, 240.0));
        assert!(svg.starts_with("<svg") && svg.ends_with("</svg>"));
        assert!(!svg.contains("<text"), "geometry-only render must carry no callout/plot text");
        assert!(svg.contains("hHouse") && svg.contains("hBush"), "must still cross-hatch both materials");
    }

    #[test]
    fn detail_crop_on_a_countersink_neck_is_narrower_than_the_full_part_and_centered_on_the_transition() {
        let mut input = straight_input();
        input.id_type = IdType::Countersink;
        input.cs_internal = Some((0.45, 0.1));
        let p = resolve_bushing_section_params(&input);
        let full_span = p.z_bottom - p.z_top.min(p.z_flange_top);
        let crop = detail_crop(&input, &p, true);
        assert!(crop.z_span < full_span, "detail crop must be a real zoom, not the whole part: {crop:?} vs {full_span}");
        assert!((crop.z_center - p.z_int).abs() < 1e-9, "neck-governing crop must center on the countersink's own neck transition");
    }

    #[test]
    fn detail_crop_falls_back_to_the_shank_midpoint_when_the_straight_wall_governs() {
        let input = straight_input();
        let p = resolve_bushing_section_params(&input);
        let full_span = p.z_bottom - p.z_top.min(p.z_flange_top);
        let crop = detail_crop(&input, &p, false);
        assert!(crop.z_span < full_span, "straight-wall crop must also be a real zoom: {crop:?} vs {full_span}");
        assert!(crop.z_center > p.z_top && crop.z_center < p.z_bottom, "must center somewhere within the shank, not at an endpoint");
    }

    #[test]
    fn geometry_crop_with_a_crop_window_yields_a_taller_effective_zoom_than_the_full_overview() {
        let mut input = straight_input();
        input.id_type = IdType::Countersink;
        input.cs_internal = Some((0.45, 0.1));
        let p = resolve_bushing_section_params(&input);
        let crop = detail_crop(&input, &p, true);
        let overview = decode(&geometry_crop_svg_data_uri(&input, false, None, 240.0));
        let detail = decode(&geometry_crop_svg_data_uri(&input, false, Some(crop), 240.0));
        let view_h = |svg: &str| -> f64 {
            let vb = svg.split("viewBox=\"0 0 ").nth(1).unwrap().split('"').next().unwrap();
            vb.split(' ').nth(1).unwrap().parse().unwrap()
        };
        // Same px-per-inch scale on both, but the detail window covers
        // fewer inches of the part - its viewBox height must be smaller
        // even though the drawing itself reads bigger to the user (the
        // <img> element displays it at a similar on-screen box, so fewer
        // source inches per pixel = more zoom).
        assert!(view_h(&detail) < view_h(&overview), "cropped detail viewBox must be shorter than the full overview's");
    }

    /// Not exercised by `cargo test` - this app has no automated way to
    /// render/screenshot its own GUI (same standing limitation as every
    /// other UI-only change in this project). Writes real SVG output for
    /// a human (or an agent with an SVG rasterizer like `inkscape`/
    /// `rsvg-convert` available) to actually look at after touching this
    /// file - run with:
    /// `cargo test -p app bushing_visualizer::tests::dump_for_visual_inspection -- --ignored`
    /// then `inkscape /tmp/bushing_<case>.svg -o /tmp/bushing_<case>.png -w 500`.
    #[test]
    #[ignore]
    fn dump_for_visual_inspection() {
        let cases: Vec<(&str, BushingSectionInput, f64, StressOverlay, bool)> = vec![
            ("straight", straight_input(), 0.0015, sample_stress(), false),
            (
                "flanged",
                BushingSectionInput { bushing_type: BushingType::Flanged, flange_od: 0.75, flange_thk: 0.063, ..straight_input() },
                0.0015,
                StressOverlay { housing_ms: 0.1, ..sample_stress() }, // marginal, amber
                false,
            ),
            (
                "countersink",
                BushingSectionInput {
                    bushing_type: BushingType::Countersink,
                    id_type: IdType::Countersink,
                    cs_external: Some((0.6, 0.06)),
                    cs_internal: Some((0.45, 0.1)),
                    ..straight_input()
                },
                0.0015,
                StressOverlay { bushing_ms: -0.1, ..sample_stress() }, // failing, red
                true,
            ),
        ];
        for (name, input, interference, stress, dark) in cases {
            let svg = decode(&section_svg_data_uri(&input, interference, stress, dark));
            std::fs::write(format!("/tmp/bushing_{name}.svg"), svg).unwrap();
        }

        // Overview + auto-cropped detail panes for the visualizer dock -
        // the countersink case exercises the neck-governing crop path.
        let cs_input = BushingSectionInput {
            bushing_type: BushingType::Countersink,
            id_type: IdType::Countersink,
            cs_external: Some((0.6, 0.06)),
            cs_internal: Some((0.45, 0.1)),
            ..straight_input()
        };
        let p = resolve_bushing_section_params(&cs_input);
        let crop = detail_crop(&cs_input, &p, true);
        std::fs::write("/tmp/bushing_dock_overview.svg", decode(&geometry_crop_svg_data_uri(&cs_input, false, None, 90.0))).unwrap();
        std::fs::write("/tmp/bushing_dock_detail.svg", decode(&geometry_crop_svg_data_uri(&cs_input, false, Some(crop), 280.0))).unwrap();
    }
}
