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

/// Published Stanley Heli-Coil tapped-hole limits, inch series, class 2B.
/// Source: Heli-Coil Catalog HC-2000, Table VII (inch tapped hole dimensions).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct StiLimits {
    pub pitch_min: f64,
    pub pitch_max: f64,
    pub minor_min: f64,
    pub minor_max: f64,
}
pub fn sti_limits(major: f64, tpi: f64) -> Option<StiLimits> {
    let entries = [
        (0.25, 20.0, 0.2825, 0.2864, 0.2608, 0.2704),
        (0.25, 28.0, 0.2732, 0.2765, 0.2577, 0.2646),
        (0.3125, 18.0, 0.3486, 0.3529, 0.3245, 0.3342),
        (0.3125, 24.0, 0.3395, 0.3433, 0.3215, 0.3288),
        (0.375, 16.0, 0.4156, 0.4203, 0.3885, 0.3987),
        (0.375, 24.0, 0.4020, 0.4059, 0.3840, 0.3910),
        (0.5, 13.0, 0.5499, 0.5554, 0.5166, 0.5273),
        (0.5, 20.0, 0.5325, 0.5371, 0.5108, 0.5186),
    ];
    entries
        .into_iter()
        .find(|(d, n, _, _, _, _)| *d == major && *n == tpi)
        .map(
            |(_, _, pitch_min, pitch_max, minor_min, minor_max)| StiLimits {
                pitch_min,
                pitch_max,
                minor_min,
                minor_max,
            },
        )
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
                (Some(limits), Some(measured)) if measured.within(limits) => {}
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
