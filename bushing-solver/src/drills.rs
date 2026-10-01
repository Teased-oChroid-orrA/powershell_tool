//! Standard drill bit size catalog in inches: fractional, number (#107-#1)
//! and letter (A-Z) drills (metric sizes are deliberately not included), from the AFT Fasteners drill bit size chart
//! (https://www.aftfasteners.com/drill-bit-size-chart/), embedded verbatim
//! from `data/drill_bit_catalog.csv`.
//!
//! Two source-chart typos were corrected rather than copied: the `43/64 in`
//! row listed its millimetre value as 7.0656 (it is 17.0656), and a bogus
//! `21/23 in` row (not a drill size) was dropped. Metric (mm) rows were
//! removed - this app works in inches.
//!
//! The `common` flag marks the sizes RapidDirect's drill size chart calls
//! the most common standard drill sizes
//! (https://www.rapiddirect.com/blog/drill-size-chart/), keeping the
//! inch-based ones: 1/16, 1/8, 1/4, 3/8, 1/2, 3/4 and 1 in; and #60, #47,
//! #39, #31, #21, #8, A, E, O and X.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DrillKind {
    Fraction,
    Number,
    Letter,
}

impl DrillKind {
    pub fn name(self) -> &'static str {
        match self {
            DrillKind::Fraction => "fractional",
            DrillKind::Number => "number",
            DrillKind::Letter => "letter",
        }
    }
}

#[derive(Debug, Clone)]
pub struct DrillEntry {
    /// `#60`, `E`, `1/4 in`.
    pub label: String,
    pub nominal_in: f64,
    pub kind: DrillKind,
    /// One of the "most common" standard sizes.
    pub common: bool,
}

const CATALOG_CSV: &str = include_str!("../data/drill_bit_catalog.csv");

fn catalog() -> &'static [DrillEntry] {
    use std::sync::OnceLock;
    static CATALOG: OnceLock<Vec<DrillEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        CATALOG_CSV
            .lines()
            .skip(1)
            .filter_map(|line| {
                let c: Vec<&str> = line.split(',').collect();
                if c.len() < 4 {
                    return None;
                }
                let kind = match c[2] {
                    "number" => DrillKind::Number,
                    "letter" => DrillKind::Letter,
                    _ => DrillKind::Fraction,
                };
                Some(DrillEntry { label: c[0].to_string(), nominal_in: c[1].parse().ok()?, kind, common: c[3] == "common" })
            })
            .collect()
    })
}

/// Every drill size, ascending by diameter (ties keep the chart's order).
pub fn all_drills() -> Vec<&'static DrillEntry> {
    let mut v: Vec<&'static DrillEntry> = catalog().iter().collect();
    v.sort_by(|a, b| a.nominal_in.partial_cmp(&b.nominal_in).unwrap());
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_inch_chart_is_loaded_and_has_no_metric_sizes() {
        assert_eq!(all_drills().len(), 225);
        let kinds = |k: DrillKind| all_drills().iter().filter(|d| d.kind == k).count();
        assert_eq!(kinds(DrillKind::Letter), 26, "A-Z");
        assert_eq!(kinds(DrillKind::Number), 107, "#1-#107");
        assert_eq!(kinds(DrillKind::Fraction), 92);
        assert!(all_drills().iter().all(|d| !d.label.contains("mm")), "inches only");
    }

    #[test]
    fn spot_checks_against_the_published_chart() {
        let find = |l: &str| all_drills().into_iter().find(|d| d.label == l).unwrap_or_else(|| panic!("{l}"));
        assert_eq!(find("#60").nominal_in, 0.04);
        assert_eq!(find("E").nominal_in, 0.25);
        assert_eq!(find("Q").nominal_in, 0.332);
        assert_eq!(find("3/8 in").nominal_in, 0.375);
        assert_eq!(find("43/64 in").nominal_in, 0.6719);
        assert!(all_drills().iter().all(|d| d.label != "21/23 in"));
    }

    #[test]
    fn exactly_the_inch_based_common_sizes_are_flagged() {
        let mut common: Vec<&str> = all_drills().into_iter().filter(|d| d.common).map(|d| d.label.as_str()).collect();
        common.sort_unstable();
        let mut expected = vec!["1/16 in", "1/8 in", "1/4 in", "3/8 in", "1/2 in", "3/4 in", "1 in", "#60", "#47", "#39", "#31", "#21", "#8", "A", "E", "O", "X"];
        expected.sort_unstable();
        assert_eq!(common, expected);
    }

    #[test]
    fn sorted_ascending() {
        let v = all_drills();
        assert!(v.windows(2).all(|w| w[0].nominal_in <= w[1].nominal_in));
    }
}
