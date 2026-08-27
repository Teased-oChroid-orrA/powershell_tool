//! Aircraft reamer catalog, ported from engineering.toolbox's
//! `src/lib/core/bushing/reamerCatalog.ts` +
//! `catalogs/aircraftReamerCatalogData.ts` - the real, sourced CSV data
//! (Pan American Tool / Rock River Tool / Omega Technologies catalogs,
//! see each entry's `notes`/`source_urls`) embedded verbatim via
//! `include_str!`, not re-derived or approximated.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AvailabilityTier {
    Preferred,
    Common,
    Special,
}

#[derive(Debug, Clone)]
pub struct ReamerEntry {
    pub size_label: String,
    pub nominal_in: f64,
    pub tool_tolerance_plus_in: f64,
    pub tool_tolerance_minus_in: f64,
    pub availability_tier: AvailabilityTier,
    pub preferred_rank: Option<u32>,
    pub notes: String,
}

const CATALOG_CSV: &str = include_str!("../data/aircraft_reamer_catalog.csv");

/// Minimal RFC-4180-ish CSV row splitter (quoted fields, `""` as an
/// escaped quote) - the same shape engineering.toolbox's own
/// `parseCsvRows` hand-rolls rather than pulling in a CSV crate for one
/// small, well-known-shape embedded file.
fn parse_csv_rows(text: &str) -> Vec<Vec<String>> {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut cell = String::new();
    let mut in_quotes = false;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '"' {
            if in_quotes && chars.peek() == Some(&'"') {
                cell.push('"');
                chars.next();
            } else {
                in_quotes = !in_quotes;
            }
            continue;
        }
        if !in_quotes && ch == ',' {
            row.push(std::mem::take(&mut cell));
            continue;
        }
        if !in_quotes && (ch == '\n' || ch == '\r') {
            if ch == '\r' && chars.peek() == Some(&'\n') {
                chars.next();
            }
            row.push(std::mem::take(&mut cell));
            if row.iter().any(|c| !c.is_empty()) {
                rows.push(std::mem::take(&mut row));
            } else {
                row.clear();
            }
            continue;
        }
        cell.push(ch);
    }
    if !cell.is_empty() || !row.is_empty() {
        row.push(cell);
        rows.push(row);
    }
    rows
}

fn parse_tier(s: &str) -> AvailabilityTier {
    match s {
        "preferred" => AvailabilityTier::Preferred,
        "special" => AvailabilityTier::Special,
        _ => AvailabilityTier::Common,
    }
}

fn catalog() -> &'static [ReamerEntry] {
    use std::sync::OnceLock;
    static CATALOG: OnceLock<Vec<ReamerEntry>> = OnceLock::new();
    CATALOG.get_or_init(|| {
        let rows = parse_csv_rows(CATALOG_CSV);
        rows.into_iter()
            .skip(1) // header
            .filter_map(|r| {
                if r.len() < 6 {
                    return None;
                }
                Some(ReamerEntry {
                    size_label: r[0].clone(),
                    nominal_in: r[1].parse().ok()?,
                    tool_tolerance_plus_in: r[2].parse().unwrap_or(0.0),
                    tool_tolerance_minus_in: r[3].parse().unwrap_or(0.0),
                    availability_tier: parse_tier(&r[4]),
                    preferred_rank: r[5].parse().ok(),
                    notes: r.get(8).cloned().unwrap_or_default(),
                })
            })
            .collect()
    })
}

/// All catalog entries, sorted by nominal size ascending.
pub fn all_reamers() -> Vec<&'static ReamerEntry> {
    let mut v: Vec<&'static ReamerEntry> = catalog().iter().collect();
    v.sort_by(|a, b| a.nominal_in.partial_cmp(&b.nominal_in).unwrap());
    v
}

/// The `count` reamers whose nominal size is closest to `target_in`,
/// nearest first - the actual "pick a reamer for this bore" query the
/// UI's picker uses. Ties (equally close above/below) favor the larger
/// size, since reaming to the next size up is the safe direction for a
/// clearance/interference bore.
pub fn nearest(target_in: f64, count: usize) -> Vec<&'static ReamerEntry> {
    let mut v: Vec<&'static ReamerEntry> = catalog().iter().collect();
    v.sort_by(|a, b| {
        let da = (a.nominal_in - target_in).abs();
        let db = (b.nominal_in - target_in).abs();
        da.partial_cmp(&db).unwrap().then(b.nominal_in.partial_cmp(&a.nominal_in).unwrap())
    });
    v.truncate(count);
    v
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_loads_real_entries() {
        let all = all_reamers();
        assert!(all.len() > 30, "expected the real ~48-row catalog, got {}", all.len());
    }

    #[test]
    fn catalog_is_sorted_ascending() {
        let all = all_reamers();
        for w in all.windows(2) {
            assert!(w[0].nominal_in <= w[1].nominal_in);
        }
    }

    #[test]
    fn nearest_finds_the_closest_real_size_to_a_target() {
        // 3/16" = 0.1875, a real preferred entry in the catalog - target
        // it exactly so a real, distinctly-closer neighbor (0.1870, also
        // in the catalog) can't tie or win.
        let results = nearest(0.1875, 3);
        assert_eq!(results.len(), 3);
        assert!((results[0].nominal_in - 0.1875).abs() < 1e-9, "closest match should be 0.1875, got {}", results[0].nominal_in);
    }

    #[test]
    fn preferred_entries_have_a_rank() {
        let preferred = nearest(0.1875, 1);
        assert_eq!(preferred[0].availability_tier, AvailabilityTier::Preferred);
        assert_eq!(preferred[0].preferred_rank, Some(1));
    }
}
