//! Aerospace/military fastener thread catalogs - AN (Army-Navy Aeronautical
//! Standard), NAS (National Aerospace Standard), MS (Military Standard), and
//! Hi-Lok pin geometry. Every dimension here is sourced (see each catalog's
//! own doc comment for its citation); nothing is invented. Two entire
//! series that could not be fully sourced were deliberately omitted rather
//! than partially fabricated - see `lib.rs`'s scope comment.
//!
//! ## Thread form: UN/UNF vs UNJ
//!
//! Most of these standards use plain 60-degree UN/UNF threads, but the
//! aerospace-specific SAE AS8879 (formerly MIL-S-8879, superseded 2003)
//! **UNJ** profile - required by many of the standards below - has a
//! larger, controlled root radius (0.15011p-0.18042p, mandatory,
//! continuous, tangent to the flanks - ASME B1.15) and consequently a
//! larger basic minor diameter than plain UN/UNF:
//!
//! ```text
//! UN/UNF (ASME B1.1):  pitch_diameter = D - 0.649519*p   minor_diameter = D - 1.082532*p
//! UNJ    (ASME B1.15):  pitch_diameter = D - 0.649519*p   minor_diameter = D - 0.974279*p
//! ```
//!
//! Pitch diameter and flank angle (60 deg) are unchanged between the two
//! forms. The UNJ minor-diameter coefficient is derived from ASME B1.15's
//! sourced basic thread height (`0.5625*H`, `H = 0.866025*p` - the
//! tangency point of UNJ's *maximum* root radius) rather than published
//! directly by any standard as a single constant: `d3 = D - 2*0.5625*H =
//! D - 0.974279*p`. [`ThreadForm`] selects which coefficient a given
//! catalog entry uses; `basic_minor_diameter`/`basic_minor_diameter_unj`
//! implement both.
//!
//! **UNJ tensile/stress area is NOT `(pi/4)*((d2+d3)/2)^2`** for aerospace
//! procurement specs - NAS4002 tabulates "thread pitch area at max
//! diameter" directly per size rather than deriving it from a generic
//! formula. Where a sourced NAS4002 value exists for a catalog entry, it is
//! stored in `tabulated_tensile_stress_area` and `BoltCatalogEntry::tensile_stress_area`
//! prefers it; entries without a sourced value fall back to the generic
//! `ThreadGeometry::tensile_stress_area()` formula, which is NOT exact for
//! UNJ (documented, not silently wrong).

use crate::thread::ThreadGeometry;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ThreadForm {
    /// Plain 60-degree UN/UNF thread (ASME B1.1).
    UnUnf,
    /// Controlled-root-radius UNJ thread (ASME B1.15 / SAE AS8879).
    Unj,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BoltCatalogEntry {
    pub designation: &'static str,
    /// Major (nominal shank) diameter, in. For Hi-Lok entries this is the
    /// reduced major diameter ("TD" on the manufacturer drawing), not the
    /// nominal fractional size - see [`HI_LOK_CATALOG`]'s own doc comment.
    pub major_diameter: f64,
    pub threads_per_inch: f64,
    pub thread_form: ThreadForm,
    /// A sourced NAS4002 "thread pitch area at max diameter" value, in^2,
    /// where available - `None` falls back to the generic
    /// `ThreadGeometry::tensile_stress_area()` formula (not exact for UNJ;
    /// see this module's own doc comment).
    pub tabulated_tensile_stress_area: Option<f64>,
}

/// ASME B1.1 basic pitch diameter for a 60-degree external UN/UNF thread -
/// also correct for UNJ (pitch diameter is unchanged between the two forms;
/// see this module's doc comment).
pub fn basic_pitch_diameter(major_diameter: f64, pitch: f64) -> f64 {
    major_diameter - 0.649_519 * pitch
}

/// ASME B1.1 basic minor (root) diameter for a 60-degree external UN/UNF
/// thread.
pub fn basic_minor_diameter(major_diameter: f64, pitch: f64) -> f64 {
    major_diameter - 1.082_532 * pitch
}

/// ASME B1.15 basic minor (root) diameter for a UNJ thread - derived from
/// the standard's sourced basic thread height (`0.5625*H`); see this
/// module's own doc comment.
pub fn basic_minor_diameter_unj(major_diameter: f64, pitch: f64) -> f64 {
    major_diameter - 0.974_279 * pitch
}

impl BoltCatalogEntry {
    pub fn pitch(&self) -> f64 {
        1.0 / self.threads_per_inch
    }

    /// Derives the full [`ThreadGeometry`] from this entry's major
    /// diameter/TPI/thread form via the ASME B1.1 or B1.15 basic-dimension
    /// formulas above - single-start, standard 60 deg thread.
    pub fn thread_geometry(&self) -> ThreadGeometry {
        let pitch = self.pitch();
        let d2 = basic_pitch_diameter(self.major_diameter, pitch);
        let d3 = match self.thread_form {
            ThreadForm::UnUnf => basic_minor_diameter(self.major_diameter, pitch),
            ThreadForm::Unj => basic_minor_diameter_unj(self.major_diameter, pitch),
        };
        ThreadGeometry { d: self.major_diameter, d2, d3, pitch, starts: 1, thread_angle_deg: 60.0 }
    }

    /// Tensile/stress area - the sourced NAS4002 tabulated value when one
    /// exists for this entry, else the generic formula (see this module's
    /// own doc comment for why that fallback is not exact for UNJ).
    pub fn tensile_stress_area(&self) -> f64 {
        self.tabulated_tensile_stress_area.unwrap_or_else(|| self.thread_geometry().tensile_stress_area())
    }
}

/// AN3-AN20, the standard aerospace structural bolt series. Real, sourced
/// diameter/TPI values (not invented): AN3 .1900-32 UNF, AN4 .2500-28 UNF,
/// AN5 .3125-24 UNF, AN6 .3750-24 UNF, AN7 .4375-20 UNF, AN8 .5000-20 UNF,
/// AN10 .6250-18 UNF, AN12 .7500-16 UNF, AN14 .8750-14 UNF, AN16
/// 1.0000-12 UNF, AN20 1.2500-12 UNF - the standard AN3-AN20 table
/// published in aircraft hardware references (e.g. Aircraft Spruce's AN
/// bolt selector, AC43.13-1B-derived mechanic references). AN9/AN11/AN13/
/// AN15/AN17-19 do not exist in the real series - this catalog does not
/// invent them. Plain UN/UNF thread form (not UNJ).
pub static AN_BOLT_CATALOG: &[BoltCatalogEntry] = &[
    BoltCatalogEntry { designation: "AN3 (.1900-32 UNF)", major_diameter: 0.1900, threads_per_inch: 32.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN4 (.2500-28 UNF)", major_diameter: 0.2500, threads_per_inch: 28.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN5 (.3125-24 UNF)", major_diameter: 0.3125, threads_per_inch: 24.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN6 (.3750-24 UNF)", major_diameter: 0.3750, threads_per_inch: 24.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN7 (.4375-20 UNF)", major_diameter: 0.4375, threads_per_inch: 20.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN8 (.5000-20 UNF)", major_diameter: 0.5000, threads_per_inch: 20.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN10 (.6250-18 UNF)", major_diameter: 0.6250, threads_per_inch: 18.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN12 (.7500-16 UNF)", major_diameter: 0.7500, threads_per_inch: 16.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN14 (.8750-14 UNF)", major_diameter: 0.8750, threads_per_inch: 14.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN16 (1.0000-12 UNF)", major_diameter: 1.0000, threads_per_inch: 12.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "AN20 (1.2500-12 UNF)", major_diameter: 1.2500, threads_per_inch: 12.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
];

/// NAS6203-6220 (alloy steel, 160-180 ksi) / NAS6303-6320 (A286 CRES,
/// 160 ksi) tension bolts and NAS1303-1320 (shear bolts, inactive for new
/// design) share this exact diameter/thread table - UNJF-3A,
/// .1900-32 through 1.2500-12 (source: Fastener Dimensions NAS6203-6220
/// spec sheet). The long-thread counterparts (NAS6603-6620/NAS6703-6720)
/// share the same table too - this solver doesn't model grip/thread length
/// as a catalog dimension (see `FastenerSegment`), so no separate entries
/// are needed for them. Note NAS6203 itself is .1900 major diameter, not
/// .1875 - the diameter-code-times-1/16 pattern breaks at this one size;
/// this is the sourced, correct value, not an error to "fix". Tensile
/// stress area is tabulated (NAS4002) only for the two sizes it was
/// possible to source (.1900-32 and .2500-28); the rest fall back to the
/// generic formula.
pub static NAS_UNJF_BOLT_CATALOG: &[BoltCatalogEntry] = &[
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-03 (.1900-32 UNJF-3A)", major_diameter: 0.1900, threads_per_inch: 32.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: Some(0.0226) },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-04 (.2500-28 UNJF-3A)", major_diameter: 0.2500, threads_per_inch: 28.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: Some(0.0404) },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-05 (.3125-24 UNJF-3A)", major_diameter: 0.3125, threads_per_inch: 24.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-06 (.3750-24 UNJF-3A)", major_diameter: 0.3750, threads_per_inch: 24.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-07 (.4375-20 UNJF-3A)", major_diameter: 0.4375, threads_per_inch: 20.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-08 (.5000-20 UNJF-3A)", major_diameter: 0.5000, threads_per_inch: 20.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-09 (.5625-18 UNJF-3A)", major_diameter: 0.5625, threads_per_inch: 18.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-10 (.6250-18 UNJF-3A)", major_diameter: 0.6250, threads_per_inch: 18.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-12 (.7500-16 UNJF-3A)", major_diameter: 0.7500, threads_per_inch: 16.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-14 (.8750-14 UNJF-3A)", major_diameter: 0.8750, threads_per_inch: 14.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-16 (1.0000-12 UNJF-3A)", major_diameter: 1.0000, threads_per_inch: 12.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-18 (1.1250-12 UNJF-3A)", major_diameter: 1.1250, threads_per_inch: 12.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS62xx/63xx/13xx-20 (1.2500-12 UNJF-3A)", major_diameter: 1.2500, threads_per_inch: 12.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
];

/// NAS1003-1020 hex-head machine bolts (A286 CRES, 140 ksi) - same
/// dia-code-to-thread mapping as the AN3-AN20/NAS62xx series, but plain
/// **UNF-3A**, not UNJF (source: military-fasteners NAS1003-10 spec
/// sheet). Kept as its own catalog because the thread form genuinely
/// differs, not merged into [`AN_BOLT_CATALOG`].
pub static NAS_UNF_BOLT_CATALOG: &[BoltCatalogEntry] = &[
    BoltCatalogEntry { designation: "NAS1003-03 (.1900-32 UNF-3A)", major_diameter: 0.1900, threads_per_inch: 32.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1004-04 (.2500-28 UNF-3A)", major_diameter: 0.2500, threads_per_inch: 28.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1005-05 (.3125-24 UNF-3A)", major_diameter: 0.3125, threads_per_inch: 24.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1006-06 (.3750-24 UNF-3A)", major_diameter: 0.3750, threads_per_inch: 24.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1007-07 (.4375-20 UNF-3A)", major_diameter: 0.4375, threads_per_inch: 20.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1008-08 (.5000-20 UNF-3A)", major_diameter: 0.5000, threads_per_inch: 20.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1010-10 (.6250-18 UNF-3A)", major_diameter: 0.6250, threads_per_inch: 18.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1012-12 (.7500-16 UNF-3A)", major_diameter: 0.7500, threads_per_inch: 16.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1014-14 (.8750-14 UNF-3A)", major_diameter: 0.8750, threads_per_inch: 14.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1016-16 (1.0000-12 UNF-3A)", major_diameter: 1.0000, threads_per_inch: 12.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "NAS1020-20 (1.2500-12 UNF-3A)", major_diameter: 1.2500, threads_per_inch: 12.0, thread_form: ThreadForm::UnUnf, tabulated_tensile_stress_area: None },
];

/// MS21250 - tension bolt, external wrenching, flanged 12-point, 180 ksi,
/// UNJF-3A, 450 deg F (source: MS21250L on EverySpec / MW Components
/// MS21250-08034 spec sheet). Only the 7 sizes actually sourced (10-32
/// through 3/4-16) - `MS20004`-`MS20024` (internal wrenching bolts) are
/// deliberately NOT included: only `MS20004` (1/4-28) could be sourced,
/// and shipping a "complete" 20-entry catalog that is 19/20 invented would
/// violate this crate's own sourcing discipline - see `lib.rs`.
pub static MS_BOLT_CATALOG: &[BoltCatalogEntry] = &[
    BoltCatalogEntry { designation: "MS21250-03 (.1900-32 UNJF-3A)", major_diameter: 0.1900, threads_per_inch: 32.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "MS21250-04 (.2500-28 UNJF-3A)", major_diameter: 0.2500, threads_per_inch: 28.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "MS21250-05 (.3125-24 UNJF-3A)", major_diameter: 0.3125, threads_per_inch: 24.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "MS21250-06 (.3750-24 UNJF-3A)", major_diameter: 0.3750, threads_per_inch: 24.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "MS21250-08 (.5000-20 UNJF-3A)", major_diameter: 0.5000, threads_per_inch: 20.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "MS21250-10 (.6250-18 UNJF-3A)", major_diameter: 0.6250, threads_per_inch: 18.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "MS21250-12 (.7500-16 UNJF-3A)", major_diameter: 0.7500, threads_per_inch: 16.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
];

/// Hi-Lok (HL18) pin thread geometry - source: LISI/Hi-Shear HL18
/// engineering drawing (rev. 2016). `major_diameter` here is the pin's
/// **reduced major diameter** ("TD" column on the drawing), NOT the
/// nominal fractional size (e.g. dash-6 is nominally 3/16in but threads at
/// .1890, not .1875) - Hi-Lok pins genuinely thread undersize of nominal,
/// exactly the "standard's threads are unique, not derivable from a round
/// nominal" case this catalog exists to get right rather than approximate.
/// The drawing gives TD as a min/max pair (e.g. .1895/.1885 for dash-6);
/// each entry here stores the midpoint, not one bound picked as if it were
/// exact - the true value can differ by up to the published tolerance.
/// Every entry is UNJF-3A except dash-5, which the source drawing lists as
/// UNJC-3A (coarse) - both are UNJ form, only the TPI differs, so
/// [`ThreadForm::Unj`] applies uniformly. Oversize collars (HL79/HL84/
/// HL82) and the titanium HL10/HL11 shear-head variant share this same
/// thread table (head style/material only) - no separate catalog entries.
/// No sourced clamp-up/preload-vs-diameter table exists publicly (only
/// double-shear/tension ULTIMATE strength is published) - this crate does
/// NOT suggest a torque or preload value for a Hi-Lok selection; see
/// `preload_analysis/bolt_picker.rs`'s own doc comment.
pub static HI_LOK_CATALOG: &[BoltCatalogEntry] = &[
    BoltCatalogEntry { designation: "HL18-5 (5/32 nom, TD .1630 UNJC-3A)", major_diameter: 0.1630, threads_per_inch: 32.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-6 (3/16 nom, TD .1890 UNJF-3A)", major_diameter: 0.1890, threads_per_inch: 32.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-8 (1/4 nom, TD .2490 UNJF-3A)", major_diameter: 0.2490, threads_per_inch: 28.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-10 (5/16 nom, TD .3115 UNJF-3A)", major_diameter: 0.3115, threads_per_inch: 24.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-12 (3/8 nom, TD .3740 UNJF-3A)", major_diameter: 0.3740, threads_per_inch: 24.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-14 (7/16 nom, TD .4365 UNJF-3A)", major_diameter: 0.4365, threads_per_inch: 20.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-16 (1/2 nom, TD .4990 UNJF-3A)", major_diameter: 0.4990, threads_per_inch: 20.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-18 (9/16 nom, TD .5608 UNJF-3A)", major_diameter: 0.5608, threads_per_inch: 18.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-20 (5/8 nom, TD .6235 UNJF-3A)", major_diameter: 0.6235, threads_per_inch: 18.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-24 (3/4 nom, TD .7485 UNJF-3A)", major_diameter: 0.7485, threads_per_inch: 16.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-28 (7/8 nom, TD .8735 UNJF-3A)", major_diameter: 0.8735, threads_per_inch: 14.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
    BoltCatalogEntry { designation: "HL18-32 (1 nom, TD .9985 UNJF-3A)", major_diameter: 0.9985, threads_per_inch: 12.0, thread_form: ThreadForm::Unj, tabulated_tensile_stress_area: None },
];

/// Every catalog this module exposes, in picker-display order - used by
/// [`find_matching`] and by the UI picker to enumerate sections without
/// duplicating the list of catalogs in two places.
pub static ALL_CATALOGS: &[&[BoltCatalogEntry]] = &[AN_BOLT_CATALOG, NAS_UNJF_BOLT_CATALOG, NAS_UNF_BOLT_CATALOG, MS_BOLT_CATALOG, HI_LOK_CATALOG];

/// The catalog entry (from any catalog) whose derived geometry most
/// closely matches `geometry` (by major diameter, TPI, and thread form) -
/// used by a UI to show a real designation instead of "Custom" when the
/// current field values happen to already equal a standard size, without
/// requiring the UI to track which catalog entry was last picked. `None`
/// when nothing matches within a tight tolerance (i.e. genuinely
/// custom/edited values).
pub fn find_matching(geometry: &ThreadGeometry) -> Option<&'static BoltCatalogEntry> {
    ALL_CATALOGS.iter().flat_map(|catalog| catalog.iter()).find(|e| {
        let g = e.thread_geometry();
        // Minor diameter (`d3`) is compared, not just major diameter/TPI,
        // specifically so a UN/UNF entry and a UNJ entry that happen to
        // share the same nominal size/TPI (e.g. AN6 vs the NAS62xx
        // .3750-24 UNJF entry) are never confused with each other - their
        // derived minor diameters differ, and that difference is the only
        // signal available once a caller only has a `ThreadGeometry`.
        (e.major_diameter - geometry.d).abs() < 1e-6
            && (e.threads_per_inch - (1.0 / geometry.pitch.max(1e-9))).abs() < 1e-3
            && (g.d3 - geometry.d3).abs() < 1e-4
            && g.thread_angle_deg == geometry.thread_angle_deg
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn catalog_has_the_eleven_real_an_sizes_not_the_missing_ones() {
        assert_eq!(AN_BOLT_CATALOG.len(), 11);
        let designations: Vec<&str> = AN_BOLT_CATALOG.iter().map(|e| e.designation).collect();
        assert!(designations.iter().any(|d| d.starts_with("AN3 ")));
        assert!(!designations.iter().any(|d| d.starts_with("AN9 ")), "AN9 does not exist in the real series");
    }

    #[test]
    fn an4_derives_the_correct_pitch_and_minor_diameter() {
        let an4 = AN_BOLT_CATALOG.iter().find(|e| e.designation.starts_with("AN4")).unwrap();
        let g = an4.thread_geometry();
        assert!((g.d - 0.2500).abs() < 1e-9);
        assert!((g.pitch - 1.0 / 28.0).abs() < 1e-9);
        // Basic pitch/minor diameter formulas, hand-checked:
        // d2 = 0.25 - 0.649519/28 = 0.226805..., d3 = 0.25 - 1.082532/28 = 0.211339...
        assert!((g.d2 - 0.226_805).abs() < 1e-5);
        assert!((g.d3 - 0.211_339).abs() < 1e-5);
    }

    #[test]
    fn unj_minor_diameter_is_larger_than_the_un_unf_minor_diameter_for_the_same_size() {
        // Controlled UNJ root radius means a larger minor diameter than
        // plain UN/UNF at the identical major diameter/pitch - the
        // documented, sourced difference this catalog exists to capture.
        let nas = NAS_UNJF_BOLT_CATALOG.iter().find(|e| e.designation.contains("-06 (.3750-24")).unwrap();
        let an = AN_BOLT_CATALOG.iter().find(|e| e.designation.starts_with("AN6")).unwrap();
        assert_eq!(nas.major_diameter, an.major_diameter);
        assert_eq!(nas.threads_per_inch, an.threads_per_inch);
        assert!(nas.thread_geometry().d3 > an.thread_geometry().d3);
        // Pitch diameter must be identical between the two forms.
        assert!((nas.thread_geometry().d2 - an.thread_geometry().d2).abs() < 1e-12);
    }

    #[test]
    fn nas4002_tabulated_tensile_stress_area_is_used_when_available() {
        let entry = NAS_UNJF_BOLT_CATALOG.iter().find(|e| e.designation.contains("-03 (.1900-32")).unwrap();
        assert_eq!(entry.tensile_stress_area(), 0.0226);
        // A sourced tabulated value must differ from the generic formula
        // (that's the whole point of this module's documented caveat).
        assert_ne!(entry.tensile_stress_area(), entry.thread_geometry().tensile_stress_area());
    }

    #[test]
    fn an_entry_without_a_tabulated_area_falls_back_to_the_generic_formula() {
        let an6 = AN_BOLT_CATALOG.iter().find(|e| e.designation.starts_with("AN6")).unwrap();
        assert_eq!(an6.tensile_stress_area(), an6.thread_geometry().tensile_stress_area());
    }

    #[test]
    fn derived_geometry_is_internally_consistent_for_every_catalog() {
        for catalog in ALL_CATALOGS {
            for entry in *catalog {
                let g = entry.thread_geometry();
                assert!(g.d > g.d2, "{}: major must exceed pitch diameter", entry.designation);
                assert!(g.d2 > g.d3, "{}: pitch diameter must exceed minor diameter", entry.designation);
                assert!(g.validate().is_ok(), "{}: derived geometry must validate", entry.designation);
            }
        }
    }

    #[test]
    fn find_matching_recognizes_an_unmodified_catalog_selection() {
        let an6 = AN_BOLT_CATALOG.iter().find(|e| e.designation.starts_with("AN6")).unwrap();
        let g = an6.thread_geometry();
        let found = find_matching(&g).unwrap();
        assert_eq!(found.designation, an6.designation);
    }

    #[test]
    fn find_matching_distinguishes_un_unf_from_unj_at_the_same_nominal_size() {
        // AN6 (UN/UNF) and the NAS62xx .3750-24 entry (UNJ) share major
        // diameter and TPI but derive different minor diameters - matching
        // must not confuse the two.
        let an6 = AN_BOLT_CATALOG.iter().find(|e| e.designation.starts_with("AN6")).unwrap();
        let nas = NAS_UNJF_BOLT_CATALOG.iter().find(|e| e.designation.contains("-06 (.3750-24")).unwrap();
        assert_eq!(find_matching(&an6.thread_geometry()).unwrap().designation, an6.designation);
        assert_eq!(find_matching(&nas.thread_geometry()).unwrap().designation, nas.designation);
    }

    #[test]
    fn find_matching_returns_none_for_a_hand_edited_geometry() {
        let mut g = AN_BOLT_CATALOG[0].thread_geometry();
        g.d += 0.01; // no longer matches any real size in any catalog
        assert!(find_matching(&g).is_none());
    }

    #[test]
    fn hi_lok_catalog_uses_reduced_major_diameter_not_nominal_fraction() {
        // Dash-6 is nominally 3/16in (.1875) but the sourced TD midpoint is
        // .1890 - this catalog must store the real reduced dimension, not
        // the round nominal fraction.
        let dash6 = HI_LOK_CATALOG.iter().find(|e| e.designation.starts_with("HL18-6")).unwrap();
        assert!((dash6.major_diameter - 0.1890).abs() < 1e-9);
        assert!(dash6.major_diameter != 0.1875);
    }
}
