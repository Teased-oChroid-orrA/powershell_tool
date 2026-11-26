//! Axial strength screen for Unified threads and threaded inserts.
//!
//! FED-STD-H28/2A gives distinct external and internal thread shear areas.
//! This module uses supplied dimensions, not an assumed thread-class limit.
//! Insert assembly ratings must come from the selected manufacturer's data.

use crate::thread_catalog::{
    basic_external_minor_diameter, basic_minor_diameter, basic_pitch_diameter,
    unified_tensile_stress_area,
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Family {
    Tapped,
    HelicalCoil,
    Keensert,
}

/// Class 2B STI tapped-hole limits (inches). These do not describe the bolt thread.
/// Stanley Heli-Coil HC-2000 Table VII, PDF p. 7.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StiLimits {
    pub pitch_min: f64,
    pub pitch_max: f64,
    pub minor_min: f64,
    pub minor_max: f64,
}
/// Suggested pilot drills for an STI tap, not for a regular Unified tap.
/// Heli-Coil Bulletin 2003 Table V, PDF p. 16; inch decimal diameters only.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StiInstallation {
    pub limits: StiLimits,
    pub drill_aluminum: f64,
    pub drill_steel_magnesium_plastic: f64,
}
// (bolt major, bolt TPI, STI pitch min/max, tapped minor min/max, pilot drills).
// HC-2000 prints 5/8-11's decimal nominal as .65250; the fraction is .6250.
const STI_INCH: &[(f64, f64, f64, f64, f64, f64, f64, f64)] = &[
    (0.0730, 64.0, 0.0832, 0.0850, 0.0764, 0.0823, 0.0785, 0.0810),
    (0.0860, 56.0, 0.0976, 0.0996, 0.0899, 0.0961, 0.0938, 0.0960),
    (0.0990, 48.0, 0.1126, 0.1148, 0.1036, 0.1104, 0.1065, 0.1094),
    (0.1120, 40.0, 0.1283, 0.1308, 0.1175, 0.1252, 0.1200, 0.1200),
    (0.1250, 40.0, 0.1413, 0.1438, 0.1305, 0.1373, 0.1339, 0.1360),
    (0.1380, 32.0, 0.1583, 0.1611, 0.1448, 0.1527, 0.1470, 0.1495),
    (0.1640, 32.0, 0.1843, 0.1872, 0.1708, 0.1781, 0.1730, 0.1770),
    (0.1900, 24.0, 0.2170, 0.2203, 0.1990, 0.2080, 0.2031, 0.2055),
    (0.2160, 24.0, 0.2430, 0.2464, 0.2250, 0.2340, 0.2280, 0.2280),
    (0.2500, 20.0, 0.2825, 0.2864, 0.2608, 0.2704, 0.2660, 0.2660),
    (0.3125, 18.0, 0.3486, 0.3529, 0.3245, 0.3342, 0.3320, 0.3320),
    (0.3750, 16.0, 0.4156, 0.4203, 0.3885, 0.3987, 0.3970, 0.3970),
    (0.4375, 14.0, 0.4839, 0.4890, 0.4530, 0.4639, 0.4531, 0.4531),
    (0.5000, 13.0, 0.5499, 0.5554, 0.5166, 0.5273, 0.5156, 0.5312),
    (0.5625, 12.0, 0.6167, 0.6225, 0.5806, 0.5918, 0.5781, 0.5938),
    (0.6250, 11.0, 0.6841, 0.6903, 0.6447, 0.6564, 0.6562, 0.6562),
    (0.7500, 10.0, 0.8149, 0.8216, 0.7716, 0.7838, 0.7812, 0.7812),
    (0.8750, 9.0, 0.9471, 0.9543, 0.8990, 0.9119, 0.9062, 0.9062),
    (1.0000, 8.0, 1.0812, 1.0890, 1.0271, 1.0421, 1.0312, 1.0312),
    (1.1250, 7.0, 1.2178, 1.2262, 1.1559, 1.1730, 1.1719, 1.1719),
    (1.2500, 7.0, 1.3428, 1.3514, 1.2809, 1.2980, 1.2969, 1.2969),
    (1.3750, 6.0, 1.4832, 1.4926, 1.4110, 1.4310, 1.4219, 1.4219),
    (1.5000, 6.0, 1.6082, 1.6177, 1.5360, 1.5560, 1.5469, 1.5469),
    (0.0860, 64.0, 0.0962, 0.0981, 0.0894, 0.0947, 0.0925, 0.0925),
    (0.0990, 56.0, 0.1106, 0.1126, 0.1029, 0.1086, 0.1040, 0.1065),
    (0.1120, 48.0, 0.1256, 0.1279, 0.1166, 0.1229, 0.1181, 0.1200),
    (0.1380, 40.0, 0.1543, 0.1569, 0.1435, 0.1503, 0.1470, 0.1495),
    (0.1640, 36.0, 0.1821, 0.1849, 0.1701, 0.1771, 0.1730, 0.1770),
    (0.1900, 32.0, 0.2103, 0.2133, 0.1968, 0.2041, 0.2010, 0.2031),
    (0.2500, 28.0, 0.2732, 0.2765, 0.2577, 0.2646, 0.2610, 0.2638),
    (0.3125, 24.0, 0.3395, 0.3433, 0.3215, 0.3288, 0.3281, 0.3281),
    (0.3750, 24.0, 0.4020, 0.4059, 0.3840, 0.3910, 0.3906, 0.3906),
    (0.4375, 20.0, 0.4700, 0.4744, 0.4483, 0.4561, 0.4531, 0.4531),
    (0.5000, 20.0, 0.5325, 0.5371, 0.5108, 0.5186, 0.5156, 0.5156),
    (0.5625, 18.0, 0.5986, 0.6035, 0.5745, 0.5826, 0.5781, 0.5781),
    (0.6250, 18.0, 0.6611, 0.6661, 0.6370, 0.6451, 0.6406, 0.6406),
    (0.7500, 16.0, 0.7906, 0.7961, 0.7635, 0.7720, 0.7656, 0.7656),
    (0.8750, 14.0, 0.9214, 0.9274, 0.8905, 0.8994, 0.8906, 0.8906),
    (1.0000, 14.0, 1.0464, 1.0527, 1.0155, 1.0243, 1.0156, 1.0312),
    (1.0000, 12.0, 1.0542, 1.0608, 1.0181, 1.0281, 1.0156, 1.0312),
    (1.1250, 12.0, 1.1792, 1.1860, 1.1431, 1.1531, 1.1406, 1.1562),
    (1.2500, 12.0, 1.3042, 1.3112, 1.2681, 1.2781, 1.2656, 1.2812),
    (1.3750, 12.0, 1.4292, 1.4364, 1.3931, 1.4031, 1.3906, 1.4062),
    (1.5000, 12.0, 1.5542, 1.5615, 1.5181, 1.5281, 1.5156, 1.5312),
];
pub fn sti_installation(major: f64, tpi: f64) -> Option<StiInstallation> {
    STI_INCH.iter().copied().find(|r| r.0 == major && r.1 == tpi).map(|r| StiInstallation {
        limits: StiLimits { pitch_min: r.2, pitch_max: r.3, minor_min: r.4, minor_max: r.5 },
        drill_aluminum: r.6,
        drill_steel_magnesium_plastic: r.7,
    })
}
pub fn sti_limits(major: f64, tpi: f64) -> Option<StiLimits> {
    sti_installation(major, tpi).map(|r| r.limits)
}
/// Howmet carbon-steel Keysert installation, inch series (catalog PDF p. 4).
/// The parent-hole tap is a regular Unified class 2B tap matching `external`.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct KeensertInstallation {
    pub carbon_part: &'static str,
    pub internal: (f64, f64),
    pub external: (f64, f64),
    pub length: f64,
    pub regular_tap_drill: f64,
    pub countersink: f64,
    pub min_tap_depth: f64,
}

// Carbon part, internal major/TPI, external major/TPI, length, drill, countersink, depth.
const KEENSERT_INCH: &[KeensertInstallation] = &[
    KeensertInstallation { carbon_part: "73041", internal: (0.25, 20.0), external: (0.375, 16.0), length: 0.37, regular_tap_drill: 0.332, countersink: 0.38, min_tap_depth: 0.43 },
    KeensertInstallation { carbon_part: "74041", internal: (0.25, 28.0), external: (0.375, 16.0), length: 0.37, regular_tap_drill: 0.332, countersink: 0.38, min_tap_depth: 0.43 },
    KeensertInstallation { carbon_part: "73042", internal: (0.25, 20.0), external: (0.4375, 14.0), length: 0.37, regular_tap_drill: 0.397, countersink: 0.44, min_tap_depth: 0.43 },
    KeensertInstallation { carbon_part: "74042", internal: (0.25, 28.0), external: (0.4375, 14.0), length: 0.37, regular_tap_drill: 0.397, countersink: 0.44, min_tap_depth: 0.43 },
    KeensertInstallation { carbon_part: "73051", internal: (0.3125, 18.0), external: (0.4375, 14.0), length: 0.43, regular_tap_drill: 0.397, countersink: 0.44, min_tap_depth: 0.50 },
    KeensertInstallation { carbon_part: "74051", internal: (0.3125, 24.0), external: (0.4375, 14.0), length: 0.43, regular_tap_drill: 0.397, countersink: 0.44, min_tap_depth: 0.50 },
    KeensertInstallation { carbon_part: "73052", internal: (0.3125, 18.0), external: (0.5, 13.0), length: 0.43, regular_tap_drill: 0.4531, countersink: 0.51, min_tap_depth: 0.50 },
    KeensertInstallation { carbon_part: "74052", internal: (0.3125, 24.0), external: (0.5, 13.0), length: 0.43, regular_tap_drill: 0.4531, countersink: 0.51, min_tap_depth: 0.50 },
    KeensertInstallation { carbon_part: "73061", internal: (0.375, 16.0), external: (0.5, 13.0), length: 0.50, regular_tap_drill: 0.4531, countersink: 0.51, min_tap_depth: 0.56 },
    KeensertInstallation { carbon_part: "74061", internal: (0.375, 24.0), external: (0.5, 13.0), length: 0.50, regular_tap_drill: 0.4531, countersink: 0.51, min_tap_depth: 0.56 },
    KeensertInstallation { carbon_part: "73062", internal: (0.375, 16.0), external: (0.5625, 12.0), length: 0.50, regular_tap_drill: 0.5156, countersink: 0.57, min_tap_depth: 0.56 },
    KeensertInstallation { carbon_part: "74062", internal: (0.375, 24.0), external: (0.5625, 12.0), length: 0.50, regular_tap_drill: 0.5156, countersink: 0.57, min_tap_depth: 0.56 },
    KeensertInstallation { carbon_part: "73081", internal: (0.5, 13.0), external: (0.625, 11.0), length: 0.62, regular_tap_drill: 0.5781, countersink: 0.63, min_tap_depth: 0.68 },
    KeensertInstallation { carbon_part: "74081", internal: (0.5, 20.0), external: (0.625, 11.0), length: 0.62, regular_tap_drill: 0.5781, countersink: 0.63, min_tap_depth: 0.68 },
    KeensertInstallation { carbon_part: "73082", internal: (0.5, 13.0), external: (0.75, 16.0), length: 0.62, regular_tap_drill: 0.7031, countersink: 0.76, min_tap_depth: 0.68 },
    KeensertInstallation { carbon_part: "74082", internal: (0.5, 20.0), external: (0.75, 16.0), length: 0.62, regular_tap_drill: 0.7031, countersink: 0.76, min_tap_depth: 0.68 },
];
pub fn keensert_installation(internal: (f64, f64), external: (f64, f64), length: f64) -> Option<&'static KeensertInstallation> {
    KEENSERT_INCH.iter().find(|r| r.internal == internal && r.external == external && r.length == length)
}

/// Approximate room-temperature assembly pull-out read from HC68-2 charts.
/// The graph's 1D and 1.5D lines are digitized to about 100 lbf; intermediate
/// effective lengths are an interpolation for damage comparison, not a qualified rating.
pub fn hc68_pullout_estimate(major: f64, tpi: f64, engagement: f64, parent_fsu: f64) -> Option<f64> {
    let (one, one_half) = match (major, tpi) {
        (0.25, 20.0) => (134.0, 228.0), // HC68-2 PDF p. 15
        (0.25, 28.0) => (140.0, 225.0), // HC68-2 PDF p. 16
        _ => return None,
    };
    let x = parent_fsu / 1000.0;
    let length_d = engagement / major;
    if !x.is_finite() || !(15.0..=50.0).contains(&x)
        || !length_d.is_finite() || !(1.0..=1.5).contains(&length_d) {
        return None;
    }
    let estimate = x * (one + (one_half - one) * (length_d - 1.0) / 0.5);
    (estimate <= 12_000.0).then_some((estimate / 100.0).floor() * 100.0)
}
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct StiMeasurement {
    pub pitch: f64,
    pub minor: f64,
}
impl StiMeasurement {
    pub fn within(self, limits: StiLimits) -> bool {
        self.pitch.is_finite()
            && self.minor.is_finite()
            && (limits.pitch_min..=limits.pitch_max).contains(&self.pitch)
            && (limits.minor_min..=limits.minor_max).contains(&self.minor)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Interface {
    pub major: f64,
    pub pitch: f64,
    pub minor: f64,
    pub female_pitch: f64,
    pub female_minor: f64,
    pub engagement: f64,
}

impl Interface {
    /// Basic 60-degree Unified dimensions. Class tolerance and allowance are not included.
    pub fn basic(major: f64, tpi: f64, engagement: f64) -> Self {
        let p = 1.0 / tpi;
        let pitch = basic_pitch_diameter(major, p);
        let minor = basic_external_minor_diameter(major, p);
        let female_minor = basic_minor_diameter(major, p);
        Self {
            major,
            pitch,
            minor,
            female_pitch: pitch,
            female_minor,
            engagement,
        }
    }
    pub fn validate(&self) -> Result<(), &'static str> {
        let a = [
            self.major,
            self.pitch,
            self.minor,
            self.female_pitch,
            self.female_minor,
            self.engagement,
        ];
        if a.iter().any(|v| !v.is_finite() || *v <= 0.0) {
            return Err("thread dimensions and engagement must be finite and positive");
        }
        if !(self.minor < self.pitch
            && self.pitch < self.major
            && self.female_minor < self.female_pitch
            && self.female_pitch < self.major
            && self.female_minor < self.pitch
            && self.female_pitch < self.major)
        {
            return Err("thread diameters must form a physically ordered mating pair");
        }
        Ok(())
    }
    /// FED-STD-H28/2A external thread stripping area at the internal minor diameter.
    pub fn external_shear_area(&self, tpi: f64) -> f64 {
        std::f64::consts::PI
            * tpi
            * self.engagement
            * self.female_minor
            * (0.5 / tpi + 0.577_350_269_189_625_8 * (self.pitch - self.female_minor))
    }
    /// FED-STD-H28/2A internal thread stripping area at the external major diameter.
    pub fn internal_shear_area(&self, tpi: f64) -> f64 {
        std::f64::consts::PI
            * tpi
            * self.engagement
            * self.major
            * (0.5 / tpi + 0.577_350_269_189_625_8 * (self.major - self.female_pitch))
    }
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Case {
    pub family: Family,
    /// Undamaged nominal bolt size; measured crest damage changes inner.major only.
    pub nominal_major: f64,
    pub tpi: f64,
    pub inner: Interface,
    /// Measured or worst-limit STI tapped hole for a helical coil.
    #[serde(default)]
    pub sti: Option<StiMeasurement>,
    /// Solid external insert thread and its tapped parent. Only used for Keenserts.
    pub outer: Option<(f64, Interface)>,
    pub axial_load: f64,
    pub bolt_tensile_strength: f64,
    pub bolt_shear_strength: f64,
    pub parent_shear_strength: f64,
    pub insert_shear_strength: Option<f64>,
    /// Manufacturer assembly pull-out rating for this size, length and parent material.
    pub assembly_rating: Option<(f64, String)>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Capacity {
    pub name: &'static str,
    pub lbf: f64,
    pub basis: &'static str,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Analysis {
    pub rows: Vec<Capacity>,
    /// None means one or more assembly failure paths lack evidence.
    pub governing: Option<Capacity>,
    pub missing: Vec<&'static str>,
    pub axial_load: f64,
}

pub fn compute(case: &Case) -> Result<Analysis, &'static str> {
    case.inner.validate()?;
    if !case.nominal_major.is_finite()
        || case.nominal_major <= 0.0
        || !case.tpi.is_finite()
        || case.tpi <= 0.0
        || !case.axial_load.is_finite()
        || case.axial_load < 0.0
        || [
            case.bolt_tensile_strength,
            case.bolt_shear_strength,
            case.parent_shear_strength,
        ]
        .iter()
        .any(|v| !v.is_finite() || *v <= 0.0)
    {
        return Err("TPI and strengths must be positive; axial load must be nonnegative");
    }
    if let Some(v) = case.insert_shear_strength {
        if !v.is_finite() || v <= 0.0 {
            return Err("insert shear strength must be positive");
        }
    }
    if let Some((rating, source)) = &case.assembly_rating {
        if !rating.is_finite() || *rating <= 0.0 || source.trim().is_empty() {
            return Err("assembly rating needs a positive value and source");
        }
    }
    let mut rows = Vec::new();
    let mut missing = Vec::new();
    let mut advisory = None;
    let pitch = 1.0 / case.tpi;
    let nominal_root = basic_external_minor_diameter(case.nominal_major, pitch);
    let area = unified_tensile_stress_area(case.nominal_major, pitch);
    let root_ratio = (case.inner.minor / nominal_root).min(1.0);
    let tensile_area = area * root_ratio * root_ratio;
    rows.push(Capacity {
        name: "Bolt tensile",
        lbf: tensile_area * case.bolt_tensile_strength,
        basis: "ASME B1.1 Unified tensile area; damaged root scaled by remaining root-section area",
    });
    if case.family != Family::HelicalCoil {
        rows.push(Capacity {
            name: "Bolt external thread stripping",
            lbf: case.inner.external_shear_area(case.tpi) * case.bolt_shear_strength,
            basis: "FED-STD-H28/2A external shear area",
        });
    }
    match case.family {
        Family::Tapped => rows.push(Capacity {
            name: "Parent internal thread stripping",
            lbf: case.inner.internal_shear_area(case.tpi) * case.parent_shear_strength,
            basis: "FED-STD-H28/2A internal shear area",
        }),
        Family::HelicalCoil => {
            // Wire insert load transfer is not a solid 60-degree thread shear plane.
            match (sti_limits(case.nominal_major, case.tpi), case.sti) {
                (Some(limits), Some(measured)) if measured.within(limits) => {
                    if let Some(lbf) = hc68_pullout_estimate(case.nominal_major, case.tpi, case.inner.engagement, case.parent_shear_strength) {
                        advisory = Some(Capacity {
                            name: "HC68-2 pull-out estimate",
                            lbf,
                            basis: "Approximate graph read; advisory, not a qualified assembly rating",
                        });
                    }
                }
                (Some(_), Some(_)) => {
                    missing.push("STI tapped hole exceeds Stanley class 2B limits")
                }
                (Some(_), None) => {
                    missing.push("Measured STI tapped-hole pitch and minor diameters")
                }
                (None, _) => missing.push("Sourced STI limits for this Unified size"),
            }
            if case.assembly_rating.is_none() {
                missing.push("Helical coil assembly pull-out rating for this installed case");
            }
        }
        Family::Keensert => {
            if let Some(strength) = case.insert_shear_strength {
                rows.push(Capacity {
                    name: "Insert internal thread stripping",
                    lbf: case.inner.internal_shear_area(case.tpi) * strength,
                    basis: "FED-STD-H28/2A internal shear area; solid insert",
                });
            } else {
                missing.push("Insert shear strength");
            }
            if let Some((outer_tpi, outer)) = case.outer {
                outer.validate()?;
                if !outer_tpi.is_finite() || outer_tpi <= 0.0 {
                    return Err("external insert TPI must be positive");
                }
                if let Some(strength) = case.insert_shear_strength {
                    rows.push(Capacity {
                        name: "Insert external thread stripping",
                        lbf: outer.external_shear_area(outer_tpi) * strength,
                        basis: "FED-STD-H28/2A external shear area; solid insert",
                    });
                }
                rows.push(Capacity {
                    name: "Parent around insert stripping",
                    lbf: outer.internal_shear_area(outer_tpi) * case.parent_shear_strength,
                    basis: "FED-STD-H28/2A internal shear area",
                });
            } else {
                missing.push("External insert and parent thread dimensions");
            }
            if case.assembly_rating.is_none() {
                missing.push("Key retention, insert body and assembly pull-out rating");
            }
        }
    }
    if let Some((rating, _)) = &case.assembly_rating {
        if case.family != Family::Tapped {
            rows.push(Capacity {
                name: "Manufacturer assembly pull-out",
                lbf: *rating,
                basis:
                    "User-supplied manufacturer rating; confirm installation and parent material",
            });
        }
    }
    if rows.iter().any(|r| !r.lbf.is_finite() || r.lbf <= 0.0) {
        return Err("computed capacity is invalid; check dimensions");
    }
    let governing = if missing.is_empty() {
        rows.iter().min_by(|a, b| a.lbf.total_cmp(&b.lbf)).cloned()
    } else {
        None
    };
    if let Some(row) = advisory { rows.push(row); }
    Ok(Analysis {
        rows,
        governing,
        missing,
        axial_load: case.axial_load,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inch_insert_installations_keep_sti_and_regular_taps_distinct() {
        assert_eq!(STI_INCH.len(), 44);
        assert!(STI_INCH.iter().all(|r| r.2 < r.3 && r.4 < r.5 && r.6 > r.0 && r.7 > r.0));
        let coil = sti_installation(0.25, 28.0).unwrap();
        assert_eq!(coil.limits.minor_min, 0.2577);
        assert_eq!(coil.drill_aluminum, 0.2610);
        assert_eq!(coil.drill_steel_magnesium_plastic, 0.2638);
        let key = keensert_installation((0.25, 28.0), (0.375, 16.0), 0.37).unwrap();
        assert_eq!(key.carbon_part, "74041");
        assert_eq!(key.regular_tap_drill, 0.3320);
        assert_ne!(key.regular_tap_drill, coil.drill_aluminum);
        assert!(keensert_installation((0.25, 28.0), (0.5, 13.0), 0.37).is_none());
        assert!(keensert_installation((0.25, 28.0), (0.375, 16.0), 0.50).is_none());
    }
    #[test]
    fn hc68_chart_estimate_tracks_parent_shear_and_engagement_but_is_not_a_rating() {
        assert_eq!(hc68_pullout_estimate(0.25, 28.0, 0.375, 30_000.0), Some(6_700.0));
        assert_eq!(hc68_pullout_estimate(0.25, 28.0, 0.25, 30_000.0), Some(4_200.0));
        assert!(hc68_pullout_estimate(0.25, 28.0, 0.375, 51_000.0).is_none());
        let mut c = direct();
        c.family = Family::HelicalCoil;
        c.sti = Some(StiMeasurement { pitch: 0.2765, minor: 0.2646 });
        c.parent_shear_strength = 30_000.0;
        let base = compute(&c).unwrap();
        assert!(base.governing.is_none());
        assert_eq!(row(&base, "HC68-2 pull-out estimate"), 6_700.0);
        c.inner.engagement -= 0.005;
        let changed = compute(&c).unwrap();
        assert!(row(&changed, "HC68-2 pull-out estimate") < row(&base, "HC68-2 pull-out estimate"));
        assert!(changed.governing.is_none());
    }
    fn direct() -> Case {
        Case {
            family: Family::Tapped,
            nominal_major: 0.25,
            tpi: 28.0,
            inner: Interface::basic(0.25, 28.0, 0.375),
            sti: None,
            outer: None,
            axial_load: 3000.0,
            bolt_tensile_strength: 120_000.0,
            bolt_shear_strength: 72_000.0,
            parent_shear_strength: 30_000.0,
            insert_shear_strength: None,
            assembly_rating: None,
        }
    }
    fn row(a: &Analysis, name: &str) -> f64 {
        a.rows.iter().find(|r| r.name == name).unwrap().lbf
    }
    #[test]
    fn diameter_damage_hits_the_correct_thread_and_not_unrelated_paths() {
        let base = direct();
        let b = compute(&base).unwrap();
        let mut minor = base.clone();
        minor.inner.female_minor += 0.005;
        let m = compute(&minor).unwrap();
        assert!(
            row(&m, "Bolt external thread stripping") < row(&b, "Bolt external thread stripping")
        );
        assert_eq!(
            row(&m, "Parent internal thread stripping"),
            row(&b, "Parent internal thread stripping")
        );
        let mut major = base.clone();
        major.inner.major -= 0.005;
        let d = compute(&major).unwrap();
        assert!(
            row(&d, "Parent internal thread stripping")
                < row(&b, "Parent internal thread stripping")
        );
        assert_eq!(
            row(&d, "Bolt external thread stripping"),
            row(&b, "Bolt external thread stripping")
        );
        assert_eq!(row(&d, "Bolt tensile"), row(&b, "Bolt tensile"));
        let mut root = base.clone();
        root.inner.minor -= 0.005;
        let damaged = row(&compute(&root).unwrap(), "Bolt tensile");
        let expected = row(&b, "Bolt tensile") * (root.inner.minor / base.inner.minor).powi(2);
        assert!((damaged - expected).abs() < 1e-9);
    }
    #[test]
    fn engagement_scales_each_thread_area_but_not_bolt_tension() {
        let b = compute(&direct()).unwrap();
        let mut longer = direct();
        longer.inner.engagement *= 2.0;
        let l = compute(&longer).unwrap();
        assert_eq!(row(&l, "Bolt tensile"), row(&b, "Bolt tensile"));
        assert!(
            (row(&l, "Parent internal thread stripping")
                / row(&b, "Parent internal thread stripping")
                - 2.0)
                .abs()
                < 1e-12
        );
    }
    #[test]
    fn inserts_do_not_claim_complete_capacity_without_assembly_evidence() {
        let mut c = direct();
        c.family = Family::HelicalCoil;
        assert!(compute(&c).unwrap().governing.is_none());
        c.sti = Some(StiMeasurement {
            pitch: 0.2765,
            minor: 0.2646,
        });
        c.assembly_rating = Some((5000.0, "Stanley chart, matched case".into()));
        assert!(compute(&c).unwrap().governing.is_some());
        c.family = Family::Keensert;
        assert!(compute(&c).unwrap().governing.is_none());
        c.insert_shear_strength = Some(60_000.0);
        c.outer = Some((16.0, Interface::basic(0.375, 16.0, 0.37)));
        assert!(compute(&c).unwrap().governing.is_some());
    }
    #[test]
    fn invalid_geometry_and_unsourced_rating_are_refused() {
        let mut c = direct();
        c.inner.female_minor = c.inner.pitch;
        assert!(compute(&c).is_err());
        c = direct();
        c.family = Family::HelicalCoil;
        c.assembly_rating = Some((5000.0, "".into()));
        assert!(compute(&c).is_err());
    }
    #[test]
    fn unified_quarter_twenty_eight_matches_tabulated_stress_area() {
        let c = direct();
        let a = compute(&c).unwrap();
        assert!((row(&a, "Bolt tensile") / c.bolt_tensile_strength - 0.0364).abs() < 0.00005);
    }
    #[test]
    fn sti_damage_removes_assembly_capability_without_misusing_solid_thread_area() {
        let mut c = direct();
        c.family = Family::HelicalCoil;
        c.sti = Some(StiMeasurement {
            pitch: 0.2765,
            minor: 0.2646,
        });
        c.assembly_rating = Some((5000.0, "Stanley matched chart".into()));
        assert!(compute(&c).unwrap().governing.is_some());
        c.sti.as_mut().unwrap().minor += 0.001;
        assert!(compute(&c).unwrap().governing.is_none());
        assert!(compute(&c)
            .unwrap()
            .missing
            .iter()
            .any(|m| m.contains("STI")));
    }
}
