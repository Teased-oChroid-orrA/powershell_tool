//! The edge-distance matrix of an edge-estimate card (Bushing, Fastener Hole): one table per target (applied load, bearing limit), one row per
//! model, one column per level (nominal, P90, P95, P99). Plain data from `edge_check::matrix::EdgeMatrix`; no numbers
//! are computed here. The roles are the owner's: the legacy solver check is *governing* (it decides PASS / REVIEW), the
//! contact FE is the *best estimate*, everything else is for comparison.
use super::session::Tone;
use crate::gpu_viewer::{
    page::{Cell, Column, DockTable, TableRow},
    ui::DockItem,
};
use edge_check::matrix::{EdgeMatrix, MatrixRow, Role, BRACKET_3D, LEVELS, VALIDATION};
use edge_check::plate::{PlateEstimate, PlateInput};
use edge_check::runner::EdgeReport;

fn text(t: impl Into<String>, tone: Tone) -> Cell {
    Cell::Text { text: t.into(), tone, action: None, editing: false }
}

fn name_cell(r: &MatrixRow) -> Cell {
    // The legacy row's long name would wrap and clip in the narrow results pane; the window text spells it out.
    let name = if r.id == edge_check::matrix::LEGACY_ID { "Legacy" } else { r.name() };
    match r.role {
        Role::Comparison => text(name, Tone::Normal),
        role => text(format!("{name} \u{b7} {}", role.label()), Tone::Title),
    }
}

/// One table: the models' minimum e/D for target `t`, a chip per cell green when the housing's actual e/D carries it.
fn table(m: &EdgeMatrix, t: usize) -> DockItem {
    let rows: Vec<TableRow> = m
        .rows
        .iter()
        .filter(|r| r.error.is_none())
        .map(|r| {
            let mut cells = vec![name_cell(r)];
            cells.extend(r.cells[t].iter().map(|c| match c {
                None if r.unavailable.is_some() => text("n/a", Tone::Dim),
                Some(c) => Cell::Chip { text: c.text(), tone: if c.short { Tone::Warn } else { Tone::On } },
                None => text("-", Tone::Dim),
            }));
            TableRow { cells, governing: r.role == Role::Governing }
        })
        .collect();
    let short = m.rows.iter().filter(|r| r.error.is_none()).any(|r| r.cells[t].iter().flatten().any(|c| c.short));
    let mut columns = vec![Column::new("Model", 3.4)];
    columns.extend(LEVELS.iter().map(|l| Column::new(l, 1.2).right()));
    DockItem::Table(Box::new(DockTable {
        columns,
        rows,
        add: None,
        footer: format!("Actual e/D {:.2}. {}", m.actual, if short { "Amber: that check asks for more edge distance than the actual e/D." } else { "Every check is met at the actual edge distance." }),
        footer_tone: Some(if short { Tone::Warn } else { Tone::On }),
        hint: String::new(),
    }))
}

/// The matrix, what it means, and what the best estimate rests on. `stale`: the inputs changed since the run.
pub(in crate::gpu_viewer::editor) fn items(m: &EdgeMatrix, r: &EdgeReport) -> Vec<DockItem> {
    let dim = |s: String| DockItem::text(s, Tone::Dim);
    let mut out = vec![dim(format!("Minimum e/D to carry the load. Nominal is the check as stated; P90, P95 and P99 are the e/D at which the chance of failing falls to 10, 5 and 1 % over the fit-pressure band, strength scatter and each model's own error. Applied load {:.0} lbf; bearing limit {:.0} lbf.", r.applied_load, r.bearing_limit_load))];
    for (t, label) in m.target_labels.iter().enumerate() {
        out.push(DockItem::Heading(label.to_string()));
        out.push(table(m, t));
    }
    for e in m.rows.iter().filter_map(|r| r.error.as_ref().map(|e| (r, e))) {
        out.push(DockItem::text(format!("{}: not evaluated ({})", e.0.name(), e.1), Tone::Warn));
    }
    match m.best_estimate() {
        Some(b) => {
            let cell = |t: usize, l: usize| b.cells[t][l].map(|c| c.text()).unwrap_or_else(|| "-".into());
            out.push(DockItem::text(format!("Best estimate (contact FE): bearing limit needs e/D {} nominal, {} at P99; applied load {} nominal, {} at P99. The actual e/D is {:.2}.", cell(1, 0), cell(1, 3), cell(0, 0), cell(0, 3), m.actual), Tone::Title));
        }
        None => out.push(dim("Best estimate: not run. Use \"Run with contact FE\" below; the contact FE meshes the pin or bushing and the plate together.".into())),
    }
    out.push(dim(if m.is_advisory() {
        "Advisory: this workbench has no legacy edge check, so no row here is governing and none decides a pass or fail. Best estimate: the contact FE is the closest to what the joint does, but it stays advisory until it is correlated with edge-distance test data and the plane-strain assumption is bracketed by a 3D check.".to_string()
    } else {
        "Governing: the legacy solver check decides PASS / REVIEW. This table never changes it. Best estimate: the contact FE is the closest to what the joint does, but it stays advisory until it is correlated with edge-distance test data and the plane-strain assumption is bracketed by a 3D check.".to_string()
    }));
    out.push(dim(format!("{VALIDATION} Further validation and its limits: docs/edge-distance-crosscheck.md.")));
    out.push(dim(BRACKET_3D.into()));
    let fy: Vec<String> = m.rows.iter().filter_map(|r| r.first_yield.map(|c| format!("{} {}", r.name(), c.text()))).collect();
    if !fy.is_empty() {
        out.push(dim(format!("First yield at the applied load (informational), e/D: {}.", fy.join(", "))));
    }
    let pf: Vec<String> = m.rows.iter().filter_map(|r| r.p_fail_bearing.map(|p| format!("{} {:.0} %", r.name(), p * 100.0))).collect();
    if !pf.is_empty() {
        out.push(dim(format!("Chance of failing the bearing-limit load at the actual e/D: {}.", pf.join(", "))));
    }
    out
}

/// The plate-under-axial-load matrix: one table, a row per model and a column per criterion (nominal minimum e/D), the
/// capacities of the model that can compute them, why each other model cannot, and what the numbers rest on.
pub(in crate::gpu_viewer::editor) fn plate_items(est: &PlateEstimate, input: &PlateInput) -> Vec<DockItem> {
    let m = &est.matrix;
    let dim = |s: String| DockItem::text(s, Tone::Dim);
    let rows: Vec<TableRow> = m
        .rows
        .iter()
        .map(|r| {
            let mut cells = vec![name_cell(r)];
            for t in 0..2 {
                cells.push(match r.cells[t][0] {
                    Some(c) => Cell::Chip { text: c.text(), tone: if c.short { Tone::Warn } else { Tone::On } },
                    None => text("n/a", Tone::Dim),
                });
            }
            TableRow { cells, governing: false }
        })
        .collect();
    let short = m.rows.iter().any(|r| r.cells.iter().flatten().flatten().any(|c| c.short));
    let mut out = vec![dim(format!("Minimum e/D that carries {:.0} lbf along a strip {:.3} in wide, at the nominal geometry. The edge named is the nearer one (e at most half the width).", input.load, input.width))];
    out.push(DockItem::Table(Box::new(DockTable {
        columns: vec![Column::new("Model", 3.4), Column::new("Net section", 1.7).right(), Column::new("First yield", 1.7).right()],
        rows,
        add: None,
        footer: format!("Actual e/D {:.2}. {}", m.actual, if short { "Amber: that check asks for more edge distance than the actual e/D." } else { "Every available check is met at the actual edge distance." }),
        footer_tone: Some(if short { Tone::Warn } else { Tone::On }),
        hint: String::new(),
    })));
    for r in m.rows.iter().filter(|r| r.unavailable.is_some()) {
        out.push(DockItem::text(format!("{}: unavailable, {}.", r.name(), r.unavailable.as_deref().unwrap_or("")), Tone::Warn));
    }
    if let Some(n) = &est.numbers {
        out.push(dim(format!("Plane-stress FE carries {:.0} lbf before the ligament's mean stress reaches Ftu (net section) and {:.0} lbf before the peak stress at the hole edge reaches the yield strength; the peak is {:.2} times the gross stress.", n.net_section_capacity, n.first_yield_capacity, n.kt_gross)));
    }
    out.push(dim("Advisory: no legacy edge check exists for a plate load, so no row is governing and none decides a pass or fail. Net section is the mean stress across the ligament between the hole and the edge against Ftu; first yield is the peak elastic von Mises stress on the bore and the free edge against Sy.".into()));
    out.push(dim("What the plane-stress FE has been checked against: limiting cases only, no test data. A hole far from every edge gives the open-hole stress concentration of 3 within 3 %, a centred hole in a finite strip matches the published net-section stress concentration within 3 %, and the net-section stress of a centred hole equals the section equilibrium within 1 %. Elastic, small strain, plane stress: no hardening, no fracture, no bearing.".into()));
    out
}
