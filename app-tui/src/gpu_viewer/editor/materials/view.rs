//! What the Material Lookup workbench shows: the page (search bar, results table, property or comparison card), the
//! tree of groups and marks, the ribbon and the menus. Pure reads of the session.
use super::{act, MaterialsAct, MaterialsSession, INPUT_SEARCH};
use crate::gpu_viewer::{
    controller::UiAction,
    editor::session::Tone,
    menu::{Entry, Item, Menu},
    ui::{DocColumn, DocDetail, DocGrid, DocGridRow, DocInput, DocRow, DocTable, Document, RibbonTab, TreeNode},
};
use crate::toolboxes::material_lookup::model::{self, compare_rows, format_value, Basis, SortKey};
use crate::widgets::material_detail;
use mechanics_core::materials::Material;

pub(super) struct RowsCache {
    query: super::Query,
    hits: Vec<usize>,
    cursor: usize,
    marked: Vec<usize>,
    rows: Vec<DocRow>,
}

impl RowsCache {
    fn current(&self, s: &MaterialsSession) -> bool {
        self.query == s.st.query && self.hits == s.st.hits && self.cursor == s.st.cursor && self.marked == s.st.marked
    }
}

fn e(label: &str, shortcut: &str, a: MaterialsAct) -> Entry {
    Entry::new(label, shortcut, act(a))
}
fn when(entry: Entry, ok: bool, why: &str) -> Entry {
    entry.when(if ok { Ok(()) } else { Err(why.to_string()) })
}

/// "1,392".
pub(super) fn group_digits(n: usize) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i).is_multiple_of(3) {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// A list value: one decimal, "-" when not given (the toolbox's list rule: zero means not given).
fn value(v: f64, decimals: usize) -> String {
    format_value((v > 0.0).then_some(v), decimals)
}

fn basis_of(m: &Material) -> &'static str {
    m.extra.map_or("typ.", |x| if x.basis.is_empty() { "-" } else { x.basis })
}

fn sort_words(s: &MaterialsSession) -> String {
    let q = &s.st.query;
    let dir = if q.sort == SortKey::Name { if q.descending { "Z-A" } else { "A-Z" } } else if q.descending { "high first" } else { "low first" };
    format!("{} ({dir})", q.sort.label())
}

// ---------------------------------------------------------------------- page

fn mark_entry(s: &MaterialsSession) -> Entry {
    let marked = s.st.current().is_some_and(|i| s.st.marked.contains(&i));
    when(e(if marked { "Unmark" } else { "Mark for Compare" }, "Enter", MaterialsAct::MarkCurrent), s.st.current().is_some(), "no material is listed")
}
fn compare_entry(s: &MaterialsSession) -> Entry {
    let n = s.st.marked.len();
    when(Entry::new(if n == 0 { "Compare".to_string() } else { format!("Compare ({n})") }, "", act(MaterialsAct::Compare)).checked(s.comparing()), n > 0, "mark materials first (double-click a row, or Enter)")
}
fn clear_marks_entry(s: &MaterialsSession) -> Entry {
    when(e("Clear marks", "", MaterialsAct::ClearMarks), !s.st.marked.is_empty(), "nothing is marked")
}
fn export_entry() -> Entry {
    e("Export report", "Ctrl+E", MaterialsAct::Export)
}

pub(super) fn document(s: &MaterialsSession) -> Document {
    let q = &s.st.query;
    let group = model::groups().get(q.group).map_or("All", String::as_str);
    let mut note = format!("{} of {} shown  ·  group {group}  ·  sorted by {}", group_digits(s.st.hits.len()), group_digits(model::catalog().len()), sort_words(s));
    if !s.st.marked.is_empty() {
        note.push_str(&format!("  ·  {} marked", s.st.marked.len()));
    }
    let search = DocInput {
        id: INPUT_SEARCH,
        label: "Search".into(),
        text: q.text.clone(),
        hint: "Alloy, temper, form, spec or table, e.g. 7075 t6 plate".into(),
        error: None,
        note: Some(note),
        submit: None,
        commit_on_blur: false,
        grow: 1.0,
        trailing: (!q.text.is_empty()).then(|| e("Clear", "Esc", MaterialsAct::ClearSearch)),
        help: None,
        choices: [Basis::Any, Basis::A, Basis::B, Basis::S]
            .into_iter()
            .map(|b| e(b.label(), "", MaterialsAct::Basis(b)).checked(q.basis == b))
            .collect(),
    };
    let chips = vec![compare_entry(s), clear_marks_entry(s)];
    let mut stats = vec![("Shown".to_string(), group_digits(s.st.hits.len())), ("Marked".to_string(), format!("{} of {}", s.st.marked.len(), model::MAX_COMPARE)), ("Basis".to_string(), q.basis.label().to_string())];
    if group != "All" {
        stats.push(("Group".to_string(), group.to_string()));
    }
    Document { precision: None, policy: None, inputs: vec![search], extra_inputs: Vec::new(), buttons: vec![export_entry()], chips, notice: None, progress: None, stats, table: table(s), detail: detail(s), panels: Vec::new(), modal: None }
}

fn table(s: &MaterialsSession) -> DocTable {
    let q = &s.st.query;
    let col = |title: &str, width: f32, right: bool, key: Option<SortKey>| DocColumn { title: title.into(), width, right, sort: key.map(|k| act(MaterialsAct::SortBy(k))), sorted: key.filter(|k| *k == q.sort).map(|_| !q.descending) };
    let cat = model::catalog();
    let cached = s.rows_cache.borrow().as_ref().filter(|c| c.current(s)).map(|c| c.rows.clone());
    let rows = cached.unwrap_or_else(|| {
        let rows: Vec<DocRow> = s
            .st
            .hits
            .iter()
            .enumerate()
            .map(|(row, &i)| {
                let m = cat[i];
                let mut sub = model::group_of(m).to_string();
                if let Some(x) = m.extra.filter(|x| !x.spec.is_empty()) {
                    sub.push_str(&format!("  ·  {}", x.spec));
                }
                DocRow {
                    cells: vec![
                        m.name.to_string(),
                        basis_of(m).to_string(),
                        value(m.ftu_ksi, 1),
                        value(m.sy_ksi, 1),
                        value(m.fbru_ksi, 1),
                        value(m.e_ksi, 0),
                        format_value(model::density(m), 3),
                        format_value(model::specific_yield_in(m), 0),
                    ],
                    sub,
                    selected: row == s.st.cursor,
                    select: act(MaterialsAct::Select(row)),
                    open: Some(act(MaterialsAct::ToggleMark(i))),
                    badge: s.st.marked.iter().position(|&x| x == i).map(|p| format!("#{}", p + 1)),
                }
            })
            .collect();
        *s.rows_cache.borrow_mut() = Some(RowsCache { query: q.clone(), hits: s.st.hits.clone(), cursor: s.st.cursor, marked: s.st.marked.clone(), rows: rows.clone() });
        rows
    });
    let total = model::catalog().len();
    let shown = s.st.hits.len();
    DocTable {
        title: if shown == total { format!("Materials  ·  {}", group_digits(total)) } else { format!("Materials  ·  {} of {}", group_digits(shown), group_digits(total)) },
        filter: None,
        columns: vec![
            col("Material", 0.0, false, Some(SortKey::Name)),
            col("Basis", 46.0, false, None),
            col("Ftu ksi", 60.0, true, Some(SortKey::Ftu)),
            col("Fty ksi", 60.0, true, Some(SortKey::Fty)),
            col("Fbru ksi", 66.0, true, Some(SortKey::Fbru)),
            col("E ksi", 62.0, true, Some(SortKey::Modulus)),
            col("lb/in^3", 60.0, true, Some(SortKey::Density)),
            col("Fty/dens in", 84.0, true, Some(SortKey::SpecificYield)),
        ],
        rows,
        empty_title: "No materials match".into(),
        empty_body: "Clear the search (Esc), choose Any basis, or pick All in the Groups of the tree.".into(),
    }
}

fn detail(s: &MaterialsSession) -> Option<DocDetail> {
    let cat = model::catalog();
    if s.comparing() {
        let ms: Vec<&Material> = s.st.marked.iter().map(|&i| cat[i]).collect();
        let subtitle = ms.iter().enumerate().map(|(k, m)| format!("#{} {}", k + 1, m.name)).collect::<Vec<_>>().join("  ·  ");
        return Some(DocDetail {
            drawing: None,
            grid: Some(compare_grid(&ms)),
            title: format!("Compare  ·  {} marked", ms.len()),
            subtitle,
            actions: vec![e("Properties", "Esc", MaterialsAct::Compare), clear_marks_entry(s), export_entry()],
            blocks: Vec::new(),
            text: None,
            empty: String::new(),
        });
    }
    let i = s.st.current()?;
    let m = cat[i];
    let subtitle = match material_detail::facts(m) {
        Some(f) => format!("{}  ·  {}", f.heading, f.source),
        None => format!("{}  ·  typical values, no handbook record", model::CURATED_GROUP),
    };
    Some(DocDetail { drawing: None, grid: Some(property_grid(m)), title: m.name.to_string(), subtitle, actions: vec![mark_entry(s), compare_entry(s)], blocks: Vec::new(), text: None, empty: String::new() })
}

fn row(label: &str, value: String, unit: &str) -> DocGridRow {
    DocGridRow { cells: vec![label.to_string(), value, unit.to_string()], best: Vec::new(), section: false }
}
fn section(title: &str) -> DocGridRow {
    DocGridRow { cells: vec![title.to_string()], best: Vec::new(), section: true }
}

/// The highlighted material: every handbook value (or the curated solver fields), then the weight-specific figures.
pub(super) fn property_grid(m: &Material) -> DocGrid {
    let mut rows = Vec::new();
    let note = match material_detail::facts(m) {
        Some(f) => {
            rows.push(section("Strength"));
            rows.push(row("Ftu", f.ftu, "ksi"));
            rows.push(row("Fty", f.fty, "ksi"));
            rows.push(row("Fcy", f.fcy, "ksi"));
            rows.push(row("Fsu", f.fsu, "ksi"));
            rows.push(row("Fbru  e/D 2.0 / 1.5", format!("{} / {}", f.fbru.0, f.fbru.1), "ksi"));
            rows.push(row("Fbry  e/D 2.0 / 1.5", format!("{} / {}", f.fbry.0, f.fbry.1), "ksi"));
            rows.push(row("Elongation", f.elongation, "%"));
            rows.push(section("Stiffness and weight"));
            rows.push(row("E", f.e, "ksi"));
            rows.push(row("Ec", f.ec, "ksi"));
            rows.push(row("G", f.g, "ksi"));
            rows.push(row("Poisson ratio", f.nu, ""));
            rows.push(row("Density", f.density, "lb/in^3"));
            f.note
        }
        None => {
            rows.push(section("Solver values"));
            for (label, v) in material_detail::curated_values(m) {
                rows.push(row(label, v, if label == "Poisson ratio" { "" } else { "ksi" }));
            }
            "Curated typical values carry no density, so the weight-specific figures are not shown.".to_string()
        }
    };
    rows.push(section("Derived"));
    rows.push(row("Fty / density", format_value(model::specific_yield_in(m), 0), "in"));
    rows.push(row("E / density", format_value(model::specific_modulus_in(m), 0), "in"));
    rows.push(row("Thermal expansion", format!("{:.1}", m.alpha_u_f), "microstrain/F"));
    DocGrid { columns: vec!["Property".into(), "Value".into(), "Unit".into()], rows, note }
}

/// The marked materials side by side (`model::compare_rows`); the best value of a row is drawn in the good colour.
pub(super) fn compare_grid(ms: &[&Material]) -> DocGrid {
    let mut columns = vec!["Property".to_string(), "Unit".to_string()];
    columns.extend((1..=ms.len()).map(|k| format!("#{k}")));
    let rows = compare_rows(ms)
        .into_iter()
        .map(|r| {
            let mut cells = vec![r.label.to_string(), r.unit.to_string()];
            cells.extend(r.values.iter().map(|v| format_value(*v, r.decimals)));
            DocGridRow { cells, best: r.best.iter().map(|c| c + 2).collect(), section: false }
        })
        .collect();
    DocGrid { columns, rows, note: "Strengths use the lowest grain direction (L / LT / ST); green marks the largest value of a row where larger is better.".into() }
}

// ---------------------------------------------------------------------- tree, ribbon, menus

const NODE_GROUPS: u32 = 1;
const NODE_MARKED: u32 = 2;
const NODE_GROUP0: u32 = 100;
const NODE_MARK0: u32 = 1_000;

pub(super) fn tree(s: &MaterialsSession) -> Vec<TreeNode> {
    let mut t = vec![TreeNode::new(NODE_GROUPS, 0, '+', "Groups").folder()];
    for (k, g) in model::groups().iter().enumerate() {
        t.push(TreeNode::new(NODE_GROUP0 + k as u32, 1, 'f', g.clone()).selected(s.st.query.group == k).on_click(act(MaterialsAct::Group(k))));
    }
    t.push(TreeNode::new(NODE_MARKED, 0, '+', format!("Marked ({} of {})", s.st.marked.len(), model::MAX_COMPARE)).folder().on_click(act(MaterialsAct::Compare)));
    for (k, &i) in s.st.marked.iter().enumerate() {
        t.push(TreeNode::new(NODE_MARK0 + k as u32, 1, 'M', format!("#{} {}", k + 1, model::catalog()[i].name)).tone(Tone::On).on_click(act(MaterialsAct::ToggleMark(i))));
    }
    t
}

pub(super) fn ribbon(s: &MaterialsSession) -> Vec<RibbonTab> {
    let q = &s.st.query;
    vec![
        RibbonTab::new("Browse")
            .group("Search", vec![when(e("Clear search", "Esc", MaterialsAct::ClearSearch), !q.text.is_empty(), "nothing is typed"), when(e("All groups", "", MaterialsAct::Group(0)), q.group != 0, "every group is shown")])
            .group("Basis", [Basis::Any, Basis::A, Basis::B, Basis::S].into_iter().map(|b| e(b.label(), "", MaterialsAct::Basis(b)).checked(q.basis == b)).collect())
            .group("Sort", SortKey::ALL.into_iter().map(|k| e(k.label(), "", MaterialsAct::SortBy(k)).checked(q.sort == k)).collect()),
        RibbonTab::new("Compare").group("Marks", vec![mark_entry(s), compare_entry(s), clear_marks_entry(s)]).group("Report", vec![export_entry()]),
    ]
}

pub(super) fn menus(s: &MaterialsSession) -> Vec<Menu> {
    let item = Item::Entry;
    let file = vec![item(export_entry()), Item::Separator, item(Entry::new("Close", "", UiAction::Quit))];
    let edit = vec![item(mark_entry(s)), item(compare_entry(s)), item(clear_marks_entry(s)), Item::Separator, item(when(e("Clear search", "Esc", MaterialsAct::ClearSearch), !s.st.query.text.is_empty(), "nothing is typed"))];
    vec![Menu { title: "File".into(), items: file }, Menu { title: "Edit".into(), items: edit }]
}

pub(super) fn context_menu(s: &MaterialsSession) -> Vec<Item> {
    vec![Item::Entry(mark_entry(s)), Item::Entry(compare_entry(s)), Item::Separator, Item::Entry(export_entry())]
}
