//! The edge-distance matrix of the Cross-checks card: one table per target (applied load, bearing limit), one row per
//! model, one column per level (nominal, P90, P95, P99). Plain data from `edge_check::matrix::EdgeMatrix`; no numbers
//! are computed here. The roles are the owner's: the legacy solver check is *governing* (it decides PASS / REVIEW), the
//! contact FE is the *best estimate*, everything else is for comparison.
use super::cards::plain;
use crate::gpu_viewer::{
    editor::session::Tone,
    page::{Cell, Column, DockTable, TableRow},
    ui::DockItem,
};
use crate::toolboxes::bushing::edge_check::{matrix_name, EdgeCheckRun};
use edge_check::matrix::{EdgeMatrix, MatrixRow, Role, LEVELS, TARGET_LABELS, VALIDATION};

fn text(t: impl Into<String>, tone: Tone) -> Cell {
    Cell::Text { text: t.into(), tone, action: None, editing: false }
}

fn name_cell(r: &MatrixRow) -> Cell {
    let name = matrix_name(r);
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
        footer: format!("Actual e/D {:.2}. {}", m.actual, if short { "Amber: that check asks for more edge distance than the housing has." } else { "Every check is met at the actual edge distance." }),
        footer_tone: Some(if short { Tone::Warn } else { Tone::On }),
        hint: String::new(),
    }))
}

/// The matrix, what it means, and what the best estimate rests on. `stale`: the inputs changed since the run.
pub(super) fn items(m: &EdgeMatrix, run: &EdgeCheckRun) -> Vec<DockItem> {
    let r = &run.report;
    let dim = |s: String| DockItem::text(plain(&s), Tone::Dim);
    let mut out = vec![dim(format!("Minimum e/D to carry the load. Nominal is the check as stated; P90, P95 and P99 are the e/D at which the chance of failing falls to 10, 5 and 1 % over the fit-pressure band, strength scatter and each model's own error. Applied load {:.0} lbf; bearing limit {:.0} lbf.", r.applied_load, r.bearing_limit_load))];
    for (t, label) in TARGET_LABELS.iter().enumerate() {
        out.push(DockItem::Heading(label.to_string()));
        out.push(table(m, t));
    }
    for e in m.rows.iter().filter_map(|r| r.error.as_ref().map(|e| (r, e))) {
        out.push(DockItem::text(format!("{}: not evaluated ({})", matrix_name(e.0), e.1), Tone::Warn));
    }
    match m.best_estimate() {
        Some(b) => {
            let cell = |t: usize, l: usize| b.cells[t][l].map(|c| c.text()).unwrap_or_else(|| "-".into());
            out.push(DockItem::text(format!("Best estimate (contact FE): bearing limit needs e/D {} nominal, {} at P99; applied load {} nominal, {} at P99. The housing has {:.2}.", cell(1, 0), cell(1, 3), cell(0, 0), cell(0, 3), m.actual), Tone::Title));
        }
        None => out.push(dim("Best estimate: not run. Use \"Run with contact FE\" below; the contact FE meshes the bushing and the housing together.".into())),
    }
    out.push(dim("Governing: the legacy solver check decides PASS / REVIEW. This table never changes it. Best estimate: the contact FE is the closest to what the joint does, but it stays advisory until it is correlated with edge-distance test data and the plane-strain assumption is bracketed by a 3D check.".into()));
    out.push(dim(VALIDATION.into()));
    let fy: Vec<String> = m.rows.iter().filter_map(|r| r.first_yield.map(|c| format!("{} {}", matrix_name(r), c.text()))).collect();
    if !fy.is_empty() {
        out.push(dim(format!("First yield at the applied load (informational), e/D: {}.", fy.join(", "))));
    }
    let pf: Vec<String> = m.rows.iter().filter_map(|r| r.p_fail_bearing.map(|p| format!("{} {:.0} %", matrix_name(r), p * 100.0))).collect();
    if !pf.is_empty() {
        out.push(dim(format!("Chance of failing the bearing-limit load at the actual e/D: {}.", pf.join(", "))));
    }
    out
}
