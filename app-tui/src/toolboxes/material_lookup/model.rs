//! Pure data layer of Material Lookup: the searchable catalog, filters,
//! sorting, the side-by-side comparison and the plain-text export. No UI
//! types, so every rule here is unit-tested without a terminal.

use mechanics_core::materials::{builtin_catalog, Dir, Material};
use std::sync::OnceLock;

/// Group label of the small curated table (typical values, no handbook record).
pub const CURATED_GROUP: &str = "Curated (typical values)";

/// Most materials the comparison view holds at once.
pub const MAX_COMPARE: usize = 4;

/// Every built-in material, curated typicals first then the MIL-HDBK-5J
/// conditions; indices are stable for the life of the process.
pub fn catalog() -> &'static [&'static Material] {
    static CATALOG: OnceLock<Vec<&'static Material>> = OnceLock::new();
    CATALOG.get_or_init(|| builtin_catalog().collect())
}

pub fn group_of(m: &Material) -> &'static str {
    m.extra.map_or(CURATED_GROUP, |x| x.group)
}

/// `"All"`, the curated group, then every handbook group alphabetically.
pub fn groups() -> &'static [String] {
    static GROUPS: OnceLock<Vec<String>> = OnceLock::new();
    GROUPS.get_or_init(|| {
        let mut g: Vec<String> = catalog().iter().filter_map(|m| m.extra.map(|x| x.group.to_string())).filter(|g| !g.is_empty()).collect();
        g.sort();
        g.dedup();
        let mut out = vec!["All".to_string(), CURATED_GROUP.to_string()];
        out.extend(g);
        out
    })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Basis {
    Any,
    A,
    B,
    S,
}

impl Basis {
    pub fn next(self) -> Basis {
        match self {
            Basis::Any => Basis::A,
            Basis::A => Basis::B,
            Basis::B => Basis::S,
            Basis::S => Basis::Any,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Basis::Any => "any basis",
            Basis::A => "A-basis",
            Basis::B => "B-basis",
            Basis::S => "S-basis",
        }
    }

    fn accepts(self, m: &Material) -> bool {
        match self {
            Basis::Any => true,
            Basis::A => m.extra.is_some_and(|x| x.basis == "A"),
            Basis::B => m.extra.is_some_and(|x| x.basis == "B"),
            Basis::S => m.extra.is_some_and(|x| x.basis == "S"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortKey {
    Name,
    Ftu,
    Fty,
    Fbru,
    Modulus,
    Density,
    SpecificYield,
    SpecificModulus,
}

impl SortKey {
    pub const ALL: [SortKey; 8] = [SortKey::Name, SortKey::Ftu, SortKey::Fty, SortKey::Fbru, SortKey::Modulus, SortKey::Density, SortKey::SpecificYield, SortKey::SpecificModulus];

    pub fn next(self) -> SortKey {
        let i = Self::ALL.iter().position(|k| *k == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }

    pub fn label(self) -> &'static str {
        match self {
            SortKey::Name => "name",
            SortKey::Ftu => "Ftu",
            SortKey::Fty => "Fty",
            SortKey::Fbru => "Fbru",
            SortKey::Modulus => "E",
            SortKey::Density => "density",
            SortKey::SpecificYield => "Fty/density",
            SortKey::SpecificModulus => "E/density",
        }
    }

    /// Value to sort by; `None` (not given) always sorts last.
    pub fn value(self, m: &Material) -> Option<f64> {
        let positive = |v: f64| (v > 0.0).then_some(v);
        match self {
            SortKey::Name => None,
            SortKey::Ftu => positive(m.ftu_ksi),
            SortKey::Fty => positive(m.sy_ksi),
            SortKey::Fbru => positive(m.fbru_ksi),
            SortKey::Modulus => positive(m.e_ksi),
            SortKey::Density => density(m),
            SortKey::SpecificYield => specific_yield_in(m),
            SortKey::SpecificModulus => specific_modulus_in(m),
        }
    }
}

/// Density (lb/in^3) when the handbook gives one.
pub fn density(m: &Material) -> Option<f64> {
    m.extra.map(|x| x.density_lb_in3).filter(|d| *d > 0.0)
}

/// Yield strength over weight density, in inches (`Fty / rho`).
pub fn specific_yield_in(m: &Material) -> Option<f64> {
    density(m).filter(|_| m.sy_ksi > 0.0).map(|d| m.sy_ksi * 1000.0 / d)
}

/// Modulus over weight density, in inches (`E / rho`).
pub fn specific_modulus_in(m: &Material) -> Option<f64> {
    density(m).filter(|_| m.e_ksi > 0.0).map(|d| m.e_ksi * 1000.0 / d)
}

#[derive(Debug, Clone, PartialEq)]
pub struct Query {
    pub text: String,
    /// Index into [`groups`] (0 = All).
    pub group: usize,
    pub basis: Basis,
    pub sort: SortKey,
    pub descending: bool,
}

impl Default for Query {
    fn default() -> Self {
        Self { text: String::new(), group: 0, basis: Basis::Any, sort: SortKey::Name, descending: false }
    }
}

/// Catalog indices matching the query, in display order.
pub fn filter_sort(q: &Query) -> Vec<usize> {
    let needle = q.text.to_lowercase();
    let want_group = groups().get(q.group).map(String::as_str).filter(|g| *g != "All");
    let cat = catalog();
    let mut hits: Vec<usize> = (0..cat.len())
        .filter(|&i| {
            let m = cat[i];
            want_group.is_none_or(|g| group_of(m) == g) && q.basis.accepts(m) && (needle.is_empty() || crate::widgets::material_detail::matches(m, &needle))
        })
        .collect();
    if q.sort == SortKey::Name {
        hits.sort_by(|&a, &b| cat[a].name.to_lowercase().cmp(&cat[b].name.to_lowercase()).then(a.cmp(&b)));
        if q.descending {
            hits.reverse();
        }
    } else {
        hits.sort_by(|&a, &b| {
            let (va, vb) = (q.sort.value(cat[a]), q.sort.value(cat[b]));
            match (va, vb) {
                (Some(x), Some(y)) => {
                    let o = x.total_cmp(&y);
                    (if q.descending { o.reverse() } else { o }).then(a.cmp(&b))
                }
                (Some(_), None) => std::cmp::Ordering::Less,
                (None, Some(_)) => std::cmp::Ordering::Greater,
                (None, None) => a.cmp(&b),
            }
        });
    }
    hits
}

/// One row of the side-by-side comparison.
#[derive(Debug, Clone, PartialEq)]
pub struct CompareRow {
    pub label: &'static str,
    pub unit: &'static str,
    /// One value per compared material (`None` = not given).
    pub values: Vec<Option<f64>>,
    /// Columns holding the row's largest value, when the row has a "better" and
    /// the values differ (a tie marks every tied column; all-equal marks none).
    pub best: Vec<usize>,
    pub decimals: usize,
}

fn lowest(d: &Dir) -> f64 {
    [d.l, d.lt, d.st].into_iter().filter(|v| *v > 0.0).fold(f64::INFINITY, f64::min)
}

fn dir_value(f: impl Fn(&mechanics_core::materials::MaterialExtras) -> Dir, m: &Material, fallback: f64) -> Option<f64> {
    let v = m.extra.map(|x| lowest(&f(x))).filter(|v| v.is_finite());
    v.or_else(|| (fallback > 0.0).then_some(fallback))
}

/// Strength / stiffness / weight rows for `materials`; the larger value wins a row.
pub fn compare_rows(materials: &[&Material]) -> Vec<CompareRow> {
    type Getter = Box<dyn Fn(&Material) -> Option<f64>>;
    let rows: Vec<(&'static str, &'static str, usize, Getter)> = vec![
        ("Ftu", "ksi", 1, Box::new(|m| dir_value(|x| x.ftu, m, m.ftu_ksi))),
        ("Fty", "ksi", 1, Box::new(|m| dir_value(|x| x.fty, m, m.sy_ksi))),
        ("Fcy", "ksi", 1, Box::new(|m| m.extra.and_then(|x| Some(lowest(&x.fcy)).filter(|v| v.is_finite())))),
        ("Fsu", "ksi", 1, Box::new(|m| dir_value(|x| x.fsu, m, m.fsu_ksi))),
        ("Fbru (e/D 2.0)", "ksi", 1, Box::new(|m| (m.fbru_ksi > 0.0).then_some(m.fbru_ksi))),
        ("Fbru (e/D 1.5)", "ksi", 1, Box::new(|m| (m.fbru_e15_ksi > 0.0).then_some(m.fbru_e15_ksi))),
        ("E", "ksi", 0, Box::new(|m| (m.e_ksi > 0.0).then_some(m.e_ksi))),
        ("G", "ksi", 0, Box::new(|m| m.extra.map(|x| x.g_ksi).filter(|v| *v > 0.0))),
        ("Poisson ratio", "", 2, Box::new(|m| Some(m.nu))),
        ("Density", "lb/in^3", 3, Box::new(density)),
        ("Fty / density", "in", 0, Box::new(specific_yield_in)),
        ("E / density", "in", 0, Box::new(specific_modulus_in)),
        ("Thermal expansion", "uin/in/F", 1, Box::new(|m| Some(m.alpha_u_f))),
    ];
    rows.into_iter()
        .map(|(label, unit, decimals, get)| {
            let values: Vec<Option<f64>> = materials.iter().map(|m| get(m)).collect();
            // Larger is better for strengths, stiffness and specific properties only.
            let has_better = !matches!(label, "Density" | "Poisson ratio" | "Thermal expansion" | "G");
            let given: Vec<f64> = values.iter().flatten().copied().collect();
            let max = given.iter().copied().fold(f64::NEG_INFINITY, f64::max);
            let min = given.iter().copied().fold(f64::INFINITY, f64::min);
            let best: Vec<usize> = if has_better && given.len() >= 2 && max - min > 1e-9 * max.abs().max(1.0) {
                values.iter().enumerate().filter(|(_, v)| v.is_some_and(|v| (v - max).abs() <= 1e-9 * max.abs().max(1.0))).map(|(i, _)| i).collect()
            } else {
                Vec::new()
            };
            CompareRow { label, unit, values, best, decimals }
        })
        .collect()
}

pub fn format_value(v: Option<f64>, decimals: usize) -> String {
    v.map_or("-".to_string(), |v| format!("{v:.decimals$}"))
}

/// Plain-text export of the current result list (and the comparison, when
/// materials are marked), written by `Effect::WriteTextFileAndOpen`.
pub fn report_text(q: &Query, hits: &[usize], marked: &[usize]) -> String {
    let cat = catalog();
    let mut s = String::from("Material Lookup (MIL-HDBK-5J and curated typical values)\n");
    s.push_str(&format!(
        "Filter: text \"{}\", group {}, {}, sorted by {}{}\n{} material(s)\n\n",
        q.text,
        groups().get(q.group).map_or("All", String::as_str),
        q.basis.label(),
        q.sort.label(),
        if q.descending { " (descending)" } else { "" },
        hits.len()
    ));
    if !marked.is_empty() {
        let ms: Vec<&Material> = marked.iter().map(|&i| cat[i]).collect();
        s.push_str("Comparison\n");
        for (i, m) in ms.iter().enumerate() {
            s.push_str(&format!("  [{}] {}\n", i + 1, m.name));
        }
        for r in compare_rows(&ms) {
            let vals: Vec<String> = r.values.iter().enumerate().map(|(i, v)| format!("{}{}", format_value(*v, r.decimals), if r.best.contains(&i) { "*" } else { "" })).collect();
            s.push_str(&format!("  {:<18} {:<9} {}\n", r.label, r.unit, vals.join("   ")));
        }
        s.push_str("  (* = best of the row; strengths use the lowest grain direction)\n\n");
    }
    s.push_str("name, group, Ftu ksi, Fty ksi, Fbru ksi, E ksi, density lb/in^3\n");
    for &i in hits {
        let m = cat[i];
        s.push_str(&format!(
            "{}, {}, {}, {}, {}, {:.0}, {}\n",
            m.name.replace(',', ";"),
            group_of(m),
            format_value((m.ftu_ksi > 0.0).then_some(m.ftu_ksi), 1),
            format_value((m.sy_ksi > 0.0).then_some(m.sy_ksi), 1),
            format_value((m.fbru_ksi > 0.0).then_some(m.fbru_ksi), 1),
            m.e_ksi,
            format_value(density(m), 3)
        ));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn idx_of(name_part: &str) -> usize {
        catalog().iter().position(|m| m.name.contains(name_part)).unwrap_or_else(|| panic!("no material {name_part}"))
    }

    #[test]
    fn catalog_has_the_curated_table_first_then_every_handbook_condition() {
        assert_eq!(catalog().len(), mechanics_core::materials::builtin_len());
        assert!(catalog()[0].extra.is_none() && catalog().last().unwrap().extra.is_some());
    }

    #[test]
    fn groups_start_with_all_then_curated_and_are_unique() {
        let g = groups();
        assert_eq!(g[0], "All");
        assert_eq!(g[1], CURATED_GROUP);
        let mut sorted = g[2..].to_vec();
        sorted.sort();
        sorted.dedup();
        assert_eq!(sorted.len(), g.len() - 2);
        assert!(g.iter().any(|x| x.contains("Aluminum")), "{g:?}");
    }

    #[test]
    fn text_group_and_basis_filters_narrow_the_list() {
        let all = filter_sort(&Query::default()).len();
        assert_eq!(all, catalog().len());
        let t = filter_sort(&Query { text: "7075 t6".into(), ..Query::default() }).len();
        assert!(t > 0 && t < all);
        let g = groups().iter().position(|g| g == CURATED_GROUP).unwrap();
        let curated = filter_sort(&Query { group: g, ..Query::default() });
        assert_eq!(curated.len(), mechanics_core::materials::builtin_len() - mechanics_core::handbook::HANDBOOK.len());
        let a = filter_sort(&Query { basis: Basis::A, ..Query::default() });
        assert!(!a.is_empty() && a.iter().all(|&i| catalog()[i].extra.is_some_and(|x| x.basis == "A")));
        assert!(filter_sort(&Query { text: "zzzz".into(), ..Query::default() }).is_empty());
    }

    #[test]
    fn sorting_by_a_property_orders_largest_first_and_puts_missing_values_last() {
        let q = Query { sort: SortKey::Fty, descending: true, ..Query::default() };
        let hits = filter_sort(&q);
        let vals: Vec<f64> = hits.iter().filter_map(|&i| SortKey::Fty.value(catalog()[i])).collect();
        assert!(vals.windows(2).all(|w| w[0] >= w[1]), "descending");
        let q = Query { sort: SortKey::Density, descending: true, ..Query::default() };
        let hits = filter_sort(&q);
        let first_missing = hits.iter().position(|&i| density(catalog()[i]).is_none()).unwrap_or(hits.len());
        assert!(hits[first_missing..].iter().all(|&i| density(catalog()[i]).is_none()), "missing values are contiguous at the end");
    }

    #[test]
    fn name_sort_is_alphabetical_and_reversible() {
        let asc = filter_sort(&Query::default());
        let desc = filter_sort(&Query { descending: true, ..Query::default() });
        assert_eq!(asc.first(), desc.last());
        let names: Vec<String> = asc.iter().map(|&i| catalog()[i].name.to_lowercase()).collect();
        assert!(names.windows(2).all(|w| w[0] <= w[1]));
    }

    #[test]
    fn specific_properties_are_over_weight_density() {
        let m = catalog()[idx_of("7075")];
        if let Some(d) = density(m) {
            assert!((specific_yield_in(m).unwrap() - m.sy_ksi * 1000.0 / d).abs() < 1e-6);
        }
        let curated = catalog()[0];
        assert!(specific_yield_in(curated).is_none(), "curated typicals carry no density");
    }

    #[test]
    fn comparison_marks_the_best_value_of_each_row() {
        let ti = catalog()[idx_of("Ti-6Al-4V")];
        let al = catalog()[0];
        let rows = compare_rows(&[al, ti]);
        let fty = rows.iter().find(|r| r.label == "Fty").unwrap();
        assert_eq!(fty.best, vec![1], "titanium beats aluminum on yield: {fty:?}");
        let nu = rows.iter().find(|r| r.label == "Poisson ratio").unwrap();
        assert!(nu.best.is_empty(), "no 'better' Poisson ratio");
        let dens = rows.iter().find(|r| r.label == "Density").unwrap();
        assert!(dens.best.is_empty());
    }

    #[test]
    fn a_missing_value_never_wins_and_one_value_is_not_a_comparison() {
        let rows = compare_rows(&[catalog()[0]]);
        assert!(rows.iter().all(|r| r.best.is_empty()));
    }

    #[test]
    fn equal_values_mark_no_winner_and_a_tie_for_the_top_marks_both() {
        let a = catalog()[0];
        let rows = compare_rows(&[a, a]);
        assert!(rows.iter().all(|r| r.best.is_empty()), "identical materials tie everywhere");
        let ti = catalog()[idx_of("Ti-6Al-4V")];
        let rows = compare_rows(&[a, ti, ti]);
        let fty = rows.iter().find(|r| r.label == "Fty").unwrap();
        assert_eq!(fty.best, vec![1, 2], "both titanium columns tie for the best yield");
    }

    #[test]
    fn report_lists_the_filter_the_comparison_and_every_hit() {
        let q = Query { text: "4340".into(), ..Query::default() };
        let hits = filter_sort(&q);
        let text = report_text(&q, &hits, &[hits[0], hits[hits.len() - 1]]);
        assert!(text.contains("text \"4340\"") && text.contains("Comparison") && text.contains("Fty"), "{text}");
        let rows = text.lines().skip_while(|l| !l.starts_with("name, group")).skip(1).filter(|l| !l.is_empty()).count();
        assert_eq!(rows, hits.len(), "one CSV row per hit");
    }
}
