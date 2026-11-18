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
    /// Indices into [`FORMS`] to keep; empty = every form (a material with no stated form never matches a chosen one).
    pub forms: Vec<usize>,
    /// Property limits (see [`Floor`]); `None` = no limit. A material with no value for the property never passes one.
    pub min_fty: Option<f64>,
    pub max_density: Option<f64>,
    pub min_specific_yield: Option<f64>,
}

impl Default for Query {
    fn default() -> Self {
        Self { text: String::new(), group: 0, basis: Basis::Any, sort: SortKey::Name, descending: false, forms: Vec::new(), min_fty: None, max_density: None, min_specific_yield: None }
    }
}

impl Query {
    /// True when a group, basis, form or property limit narrows the list (the text and the sort are not facets).
    pub fn has_facets(&self) -> bool {
        self.group != 0 || self.basis != Basis::Any || !self.forms.is_empty() || Floor::ALL.iter().any(|f| self.limit(*f).is_some())
    }

    /// Drops every facet and keeps the text and the sort.
    pub fn clear_facets(&mut self) {
        let keep = Query { text: std::mem::take(&mut self.text), sort: self.sort, descending: self.descending, ..Query::default() };
        *self = keep;
    }

    pub fn limit(&self, f: Floor) -> Option<f64> {
        match f {
            Floor::Fty => self.min_fty,
            Floor::Density => self.max_density,
            Floor::SpecificYield => self.min_specific_yield,
        }
    }

    pub fn set_limit(&mut self, f: Floor, v: Option<f64>) {
        match f {
            Floor::Fty => self.min_fty = v,
            Floor::Density => self.max_density = v,
            Floor::SpecificYield => self.min_specific_yield = v,
        }
    }
}

/// The form classes of the facet, in display order. The handbook states 60-odd spellings ("Flat sheet and plate",
/// "Extrusion (rod, bar, and shapes)", "Die forging"); [`form_class`] maps each to one of these.
pub const FORMS: [&str; 7] = ["Sheet", "Plate", "Sheet and plate", "Bar and rod", "Extrusion", "Forging", "Casting"];

/// Index into [`FORMS`] of a handbook form string: the first rule that matches wins, so "Bar, forging, tubing" is a
/// forging. `None` for no stated form, "All wrought forms", tubing and the rest.
pub fn form_class(form: &str) -> Option<usize> {
    let f = form.to_lowercase();
    let has = |w: &str| f.contains(w);
    let class = if has("casting") {
        "Casting"
    } else if has("forging") {
        "Forging"
    } else if has("extru") {
        "Extrusion"
    } else if has("plate") && (has("sheet") || has("strip")) {
        "Sheet and plate"
    } else if has("plate") {
        "Plate"
    } else if has("sheet") || has("strip") {
        "Sheet"
    } else if has("bar") || has("rod") {
        "Bar and rod"
    } else {
        return None;
    };
    FORMS.iter().position(|x| *x == class)
}

fn form_of(m: &Material) -> Option<usize> {
    m.extra.and_then(|x| form_class(x.form))
}

/// A property limit of the sidebar: Fty at least, density at most, Fty/density at least.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Floor {
    Fty,
    Density,
    SpecificYield,
}

impl Floor {
    pub const ALL: [Floor; 3] = [Floor::Fty, Floor::Density, Floor::SpecificYield];

    pub fn label(self) -> &'static str {
        match self {
            Floor::Fty => "Fty at least",
            Floor::Density => "Density at most",
            Floor::SpecificYield => "Fty / density at least",
        }
    }

    pub fn unit(self) -> &'static str {
        match self {
            Floor::Fty => "ksi",
            Floor::Density => "lb/in^3",
            Floor::SpecificYield => "in",
        }
    }

    pub fn decimals(self) -> usize {
        if self == Floor::Density {
            3
        } else {
            0
        }
    }

    /// Distance between the slider's stops.
    pub fn step(self) -> f64 {
        match self {
            Floor::Fty => 1.0,
            Floor::Density => 0.001,
            // About 100,000 to 1,000,000: a stop per 1,000 is already finer than the figure means.
            Floor::SpecificYield => 1_000.0,
        }
    }

    fn value(self, m: &Material) -> Option<f64> {
        match self {
            Floor::Fty => SortKey::Fty.value(m),
            Floor::Density => density(m),
            Floor::SpecificYield => specific_yield_in(m),
        }
    }

    /// Smallest and largest value in the catalog, widened to the step: the slider's ends (both are "no limit" ends).
    pub fn range(self) -> (f64, f64) {
        static RANGES: OnceLock<[(f64, f64); 3]> = OnceLock::new();
        let r = RANGES.get_or_init(|| {
            Floor::ALL.map(|f| {
                let (lo, hi) = catalog().iter().filter_map(|m| f.value(m)).fold((f64::INFINITY, f64::NEG_INFINITY), |(lo, hi), v| (lo.min(v), hi.max(v)));
                let s = f.step();
                ((lo / s).floor() * s, (hi / s).ceil() * s)
            })
        });
        r[Floor::ALL.iter().position(|x| *x == self).unwrap_or(0)]
    }

    fn passes(self, q: &Query, m: &Material) -> bool {
        let Some(limit) = q.limit(self) else { return true };
        // Half a step of slack: a limit read back from a slider position is the stop, not the exact catalog value.
        let slack = self.step() * 1e-6;
        self.value(m).is_some_and(|v| if self == Floor::Density { v <= limit + slack } else { v >= limit - slack })
    }
}

/// The facets a count leaves out of its own filter, so a section shows what choosing each option would give.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Skip {
    None,
    Group,
    Basis,
    Form,
}

fn accepts(q: &Query, m: &Material, needle: &str, skip: Skip) -> bool {
    let want_group = groups().get(q.group).map(String::as_str).filter(|g| *g != "All");
    (skip == Skip::Group || want_group.is_none_or(|g| group_of(m) == g))
        && (skip == Skip::Basis || q.basis.accepts(m))
        && (skip == Skip::Form || q.forms.is_empty() || form_of(m).is_some_and(|f| q.forms.contains(&f)))
        && Floor::ALL.iter().all(|f| f.passes(q, m))
        && (needle.is_empty() || crate::widgets::material_detail::matches(m, needle))
}

/// How many materials each option of the sidebar would show with every other choice kept.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct FacetCounts {
    /// One per [`groups`] entry (index 0, "All", counts every group).
    pub groups: Vec<usize>,
    /// A-, B-, S-basis.
    pub bases: [usize; 3],
    /// One per [`FORMS`] entry.
    pub forms: [usize; FORMS.len()],
}

pub fn facet_counts(q: &Query) -> FacetCounts {
    let needle = q.text.to_lowercase();
    let cat = catalog();
    let mut c = FacetCounts { groups: vec![0; groups().len()], ..FacetCounts::default() };
    for m in cat.iter().copied() {
        if accepts(q, m, &needle, Skip::Group) {
            c.groups[0] += 1;
            if let Some(g) = groups().iter().position(|g| g == group_of(m)) {
                c.groups[g] += 1;
            }
        }
        if accepts(q, m, &needle, Skip::Basis) {
            if let Some(b) = ["A", "B", "S"].iter().position(|b| m.extra.is_some_and(|x| x.basis == *b)) {
                c.bases[b] += 1;
            }
        }
        if accepts(q, m, &needle, Skip::Form) {
            if let Some(f) = form_of(m) {
                c.forms[f] += 1;
            }
        }
    }
    c
}

/// Catalog indices matching the query, in display order.
pub fn filter_sort(q: &Query) -> Vec<usize> {
    let needle = q.text.to_lowercase();
    let cat = catalog();
    let mut hits: Vec<usize> = (0..cat.len()).filter(|&i| accepts(q, cat[i], &needle, Skip::None)).collect();
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

/// ", form Plate or Forging, Fty >= 60 ksi, ..." for the facets the group and basis words do not cover; empty for none.
fn facet_words(q: &Query) -> String {
    let mut s = String::new();
    if !q.forms.is_empty() {
        let names: Vec<&str> = q.forms.iter().filter_map(|&f| FORMS.get(f).copied()).collect();
        s.push_str(&format!(", form {}", names.join(" or ")));
    }
    for f in Floor::ALL {
        if let Some(v) = q.limit(f) {
            let op = if f == Floor::Density { "<=" } else { ">=" };
            let name = match f {
                Floor::Fty => "Fty",
                Floor::Density => "density",
                Floor::SpecificYield => "Fty/density",
            };
            s.push_str(&format!(", {name} {op} {:.*} {}", f.decimals(), v, f.unit()));
        }
    }
    s
}

/// Plain-text export of the current result list (and the comparison, when
/// materials are marked), written by `Effect::WriteTextFileAndOpen`.
pub fn report_text(q: &Query, hits: &[usize], marked: &[usize]) -> String {
    let cat = catalog();
    let mut s = String::from("Material Lookup (MIL-HDBK-5J and curated typical values)\n");
    s.push_str(&format!(
        "Filter: text \"{}\", group {}, {}{}, sorted by {}{}\n{} material(s)\n\n",
        q.text,
        groups().get(q.group).map_or("All", String::as_str),
        q.basis.label(),
        facet_words(q),
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

    #[test]
    fn form_classes_cover_the_handbook_spellings_by_first_matching_rule() {
        let class = |f: &str| form_class(f).map(|i| FORMS[i]);
        assert_eq!(class("Flat sheet and plate"), Some("Sheet and plate"));
        assert_eq!(class("Sheet, strip, and plate"), Some("Sheet and plate"));
        assert_eq!(class("Plate"), Some("Plate"));
        assert_eq!(class("Sheet and strip"), Some("Sheet"));
        assert_eq!(class("Extrusion (rod, bar, and shapes)"), Some("Extrusion"));
        assert_eq!(class("Bar, forging, tubing"), Some("Forging"));
        assert_eq!(class("Die forging"), Some("Forging"));
        assert_eq!(class("Sand casting"), Some("Casting"));
        assert_eq!(class("Bar and rod; rolled, drawn, or cold-finished"), Some("Bar and rod"));
        assert_eq!(class(""), None);
        assert_eq!(class("All wrought forms"), None);
    }

    #[test]
    fn every_form_class_has_materials_and_the_common_forms_are_classified() {
        let c = facet_counts(&Query::default());
        assert!(c.forms.iter().all(|&n| n > 0), "{:?}", c.forms);
        // Almost every handbook condition names a form this facet understands.
        let classified: usize = c.forms.iter().sum();
        assert!(classified * 10 > catalog().len() * 8, "{classified} of {}", catalog().len());
    }

    #[test]
    fn form_and_property_limits_narrow_the_list_and_never_let_a_missing_value_pass() {
        let all = filter_sort(&Query::default()).len();
        let plate = FORMS.iter().position(|f| *f == "Plate").unwrap();
        let q = Query { forms: vec![plate], ..Query::default() };
        let hits = filter_sort(&q);
        assert!(!hits.is_empty() && hits.len() < all);
        assert!(hits.iter().all(|&i| form_of(catalog()[i]) == Some(plate)));
        let q = Query { min_fty: Some(100.0), ..Query::default() };
        let hits = filter_sort(&q);
        assert!(!hits.is_empty() && hits.iter().all(|&i| catalog()[i].sy_ksi >= 100.0 - 1e-6));
        let q = Query { max_density: Some(0.1), ..Query::default() };
        let hits = filter_sort(&q);
        assert!(!hits.is_empty() && hits.iter().all(|&i| density(catalog()[i]).is_some_and(|d| d <= 0.1 + 1e-6)));
        // The curated typical values have no handbook density: a density limit leaves them out.
        assert!(!hits.iter().any(|&i| catalog()[i].extra.is_none()));
        let q = Query { min_specific_yield: Some(1.0e6), ..Query::default() };
        assert!(filter_sort(&q).is_empty());
    }

    #[test]
    fn floor_ranges_run_from_the_smallest_to_the_largest_catalog_value() {
        for f in Floor::ALL {
            let (lo, hi) = f.range();
            assert!(lo < hi, "{f:?}");
            let values: Vec<f64> = catalog().iter().filter_map(|m| f.value(m)).collect();
            assert!(values.iter().all(|v| *v >= lo - 1e-9 && *v <= hi + 1e-9), "{f:?}");
            // At the "no limit" end of a limit nothing is lost that has a value.
            let mut q = Query::default();
            q.set_limit(f, Some(if f == Floor::Density { hi } else { lo }));
            assert_eq!(filter_sort(&q).len(), values.len(), "{f:?}");
        }
    }

    #[test]
    fn facet_counts_are_what_choosing_each_option_would_show() {
        // With a basis chosen, the group counts are the groups within that basis, the basis counts ignore the basis.
        let q = Query { basis: Basis::A, text: "7075".into(), ..Query::default() };
        let c = facet_counts(&q);
        assert_eq!(c.groups[0], filter_sort(&q).len());
        let alu = groups().iter().position(|g| g.contains("Aluminum")).unwrap();
        let in_alu = filter_sort(&Query { group: alu, ..q.clone() }).len();
        assert_eq!(c.groups[alu], in_alu);
        let b = filter_sort(&Query { basis: Basis::B, ..q.clone() }).len();
        assert_eq!(c.bases[1], b);
        assert!(c.bases[0] > 0 && c.bases[0] == filter_sort(&q).len());
        let plate = FORMS.iter().position(|f| *f == "Plate").unwrap();
        assert_eq!(c.forms[plate], filter_sort(&Query { forms: vec![plate], ..q.clone() }).len());
    }

    #[test]
    fn clear_facets_keeps_the_text_and_the_sort() {
        let mut q = Query { text: "7075".into(), group: 3, basis: Basis::A, sort: SortKey::Fty, descending: true, forms: vec![1], min_fty: Some(50.0), ..Query::default() };
        assert!(q.has_facets());
        q.clear_facets();
        assert!(!q.has_facets());
        assert_eq!((q.text.as_str(), q.sort, q.descending), ("7075", SortKey::Fty, true));
    }

    #[test]
    fn the_report_names_the_form_and_property_limits() {
        let plate = FORMS.iter().position(|f| *f == "Plate").unwrap();
        let q = Query { forms: vec![plate], min_fty: Some(60.0), max_density: Some(0.1), ..Query::default() };
        let text = report_text(&q, &filter_sort(&q), &[]);
        assert!(text.contains("form Plate, Fty >= 60 ksi, density <= 0.100 lb/in^3"), "{text}");
    }
}
