//! The requirement rows of "Minimum wall by requirement": which failure criteria the person checks, each with the
//! factor and the required margin of safety their program asks for. The criteria that apply follow the material's class
//! (`MaterialKind`), the allowables come from the selected material, and the factor and required margin are NEVER
//! program values here: a seeded row is neutral (factor 1, required MS 0) until the person enters theirs.
use mechanics_core::materials::Material;
use pressure_vessel_solver::criterion_thickness::ThicknessCriterion;
use pressure_vessel_solver::line_run::Criterion;
use serde::{Deserialize, Serialize};

/// Which failure criteria apply to a material: the class the line-run solver evaluates (`line_run::MaterialClass`).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MaterialKind {
    Ductile,
    Brittle,
    /// No criterion is guessed for a laminate (`line_run` refuses it too).
    Composite,
}

/// Elongation (percent) under which a catalog record counts as brittle: the usual 5 % dividing line. A record without an
/// elongation (the typical-value rows) is a ductile metal unless it is a laminate; the person can switch the class.
const BRITTLE_ELONGATION_PERCENT: f64 = 5.0;

impl MaterialKind {
    pub fn default_for(m: &Material) -> Self {
        if m.id.starts_with("cfrp") {
            return Self::Composite;
        }
        let shortest = m.extra.map(|x| [x.elong.l, x.elong.lt, x.elong.st].into_iter().filter(|v| *v > 0.0).fold(f64::INFINITY, f64::min));
        match shortest {
            Some(e) if e < BRITTLE_ELONGATION_PERCENT => Self::Brittle,
            _ => Self::Ductile,
        }
    }
    pub fn label(self) -> &'static str {
        match self {
            Self::Ductile => "Ductile",
            Self::Brittle => "Brittle",
            Self::Composite => "Composite",
        }
    }
    /// Ductile and brittle swap; a laminate goes to ductile (the person overrides the guess, never the other way).
    pub fn toggled(self) -> Self {
        if self == Self::Ductile { Self::Brittle } else { Self::Ductile }
    }
}

/// A requirement's failure criterion: the cylinder's (`ThicknessCriterion`) and the line run's (`Criterion`) in one list,
/// so a row survives switching between the two analyses.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Req {
    VonMisesYield,
    TrescaYield,
    YieldMaxPrincipal,
    UltimateMaxPrincipal,
    UltimateVonMises,
    Buckling,
    Rankine,
    CoulombMohr,
}

impl Req {
    pub fn label(self) -> &'static str {
        match self {
            Self::VonMisesYield => "Yield, von Mises",
            Self::TrescaYield => "Yield, Tresca",
            Self::YieldMaxPrincipal => "Yield, max principal",
            Self::UltimateMaxPrincipal => "Ultimate, max principal",
            Self::UltimateVonMises => "Ultimate, von Mises",
            Self::Buckling => "Buckling",
            Self::Rankine => "Max principal vs ultimate",
            Self::CoulombMohr => "Coulomb-Mohr",
        }
    }
    /// The criteria offered for a class, in the order a click cycles them.
    pub fn choices(kind: MaterialKind) -> &'static [Req] {
        match kind {
            MaterialKind::Ductile => &[Self::VonMisesYield, Self::TrescaYield, Self::YieldMaxPrincipal, Self::UltimateMaxPrincipal, Self::UltimateVonMises, Self::Buckling],
            MaterialKind::Brittle => &[Self::Rankine, Self::CoulombMohr, Self::Buckling],
            MaterialKind::Composite => &[Self::Buckling],
        }
    }
    /// The next criterion of the class (a criterion of another class starts the list).
    pub fn next(self, kind: MaterialKind) -> Self {
        let c = Self::choices(kind);
        c.iter().position(|r| *r == self).map_or(c[0], |i| c[(i + 1) % c.len()])
    }
    /// The allowable the criterion is measured against, from the material in use (`sy`, `ftu`, `fcu` in ksi, overrides applied):
    /// its name and value, so a row shows where its number comes from. Buckling reads the modulus, not a strength.
    pub fn allowable_text(self, sy: f64, ftu: f64, fcu: f64) -> Option<String> {
        Some(match self {
            Self::VonMisesYield | Self::TrescaYield | Self::YieldMaxPrincipal => format!("Yield {sy} ksi"),
            Self::UltimateMaxPrincipal | Self::UltimateVonMises | Self::Rankine => format!("Ultimate {ftu} ksi"),
            Self::CoulombMohr => format!("Ultimate {ftu} ksi tension, {fcu} ksi compression"),
            Self::Buckling => return None,
        })
    }
    /// The cylinder solver's criterion; `None` when the cylinder does not evaluate it (Coulomb-Mohr needs a compression
    /// ultimate and a shear state a plain Lame cylinder has no use for). Rankine is the maximum principal against `Ftu`.
    pub fn cylinder(self) -> Option<ThicknessCriterion> {
        Some(match self {
            Self::VonMisesYield => ThicknessCriterion::VonMisesYield,
            Self::TrescaYield => ThicknessCriterion::TrescaYield,
            Self::YieldMaxPrincipal => ThicknessCriterion::YieldMaxPrincipal,
            Self::UltimateMaxPrincipal | Self::Rankine => ThicknessCriterion::UltimateMaxPrincipal,
            Self::UltimateVonMises => ThicknessCriterion::UltimateVonMises,
            Self::Buckling => ThicknessCriterion::Buckling,
            Self::CoulombMohr => return None,
        })
    }
    /// The line run's criterion; `None` for the two it does not evaluate.
    pub fn line(self) -> Option<Criterion> {
        Some(match self {
            Self::VonMisesYield => Criterion::VonMisesYield,
            Self::TrescaYield => Criterion::TrescaYield,
            Self::UltimateMaxPrincipal => Criterion::UltimateMaxPrincipal,
            Self::UltimateVonMises => Criterion::UltimateVonMises,
            Self::Rankine => Criterion::Rankine,
            Self::CoulombMohr => Criterion::CoulombMohr,
            Self::YieldMaxPrincipal | Self::Buckling => return None,
        })
    }
}

/// One row: the criterion must reach `required_ms` with `factor` applied to the demand (`MS = allowable / (factor * demand)
/// - 1`). `edited`: the person touched the row (or added it), so a change of material leaves it alone.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Requirement {
    pub label: String,
    pub criterion: Req,
    pub factor: f64,
    pub required_ms: f64,
    #[serde(default)]
    pub edited: bool,
}

impl Requirement {
    /// A neutral row: no factor, no margin, which the table says in plain words.
    pub fn neutral(label: &str, criterion: Req) -> Self {
        Self { label: label.to_string(), criterion, factor: 1.0, required_ms: 0.0, edited: false }
    }
    /// True while the row still has the neutral numbers (the table hints to enter the program's own).
    pub fn is_neutral(&self) -> bool {
        self.factor == 1.0 && self.required_ms == 0.0
    }
}

/// The rows a material class starts with: the criteria that apply, neutral. Ductile: yield by von Mises and by Tresca, and
/// the ultimate; brittle: the maximum principal stress and Coulomb-Mohr against the ultimates.
pub fn seeds(kind: MaterialKind) -> Vec<Requirement> {
    match kind {
        MaterialKind::Ductile => vec![
            Requirement::neutral("Yield (von Mises)", Req::VonMisesYield),
            Requirement::neutral("Yield (Tresca)", Req::TrescaYield),
            Requirement::neutral("Ultimate", Req::UltimateMaxPrincipal),
        ],
        MaterialKind::Brittle => vec![Requirement::neutral("Ultimate (max principal)", Req::Rankine), Requirement::neutral("Ultimate (Coulomb-Mohr)", Req::CoulombMohr)],
        MaterialKind::Composite => Vec::new(),
    }
}

/// Re-seed after the material (or its class) changed: every row the person has not edited is replaced by the new class's
/// seeds, except a criterion an edited row already covers. Edited rows stay as they are.
pub fn reseed(rows: &mut Vec<Requirement>, kind: MaterialKind) {
    rows.retain(|r| r.edited);
    let covered: Vec<Req> = rows.iter().map(|r| r.criterion).collect();
    let mut fresh: Vec<Requirement> = seeds(kind).into_iter().filter(|s| !covered.contains(&s.criterion)).collect();
    fresh.append(rows);
    *rows = fresh;
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_core::materials::get_material;

    #[test]
    fn seeds_are_neutral_and_follow_the_class() {
        for kind in [MaterialKind::Ductile, MaterialKind::Brittle] {
            let s = seeds(kind);
            assert!(!s.is_empty() && s.iter().all(|r| r.is_neutral() && !r.edited), "{kind:?}");
            assert!(s.iter().all(|r| Req::choices(kind).contains(&r.criterion)));
        }
        assert!(seeds(MaterialKind::Composite).is_empty());
        let d: Vec<Req> = seeds(MaterialKind::Ductile).iter().map(|r| r.criterion).collect();
        assert_eq!(d, [Req::VonMisesYield, Req::TrescaYield, Req::UltimateMaxPrincipal]);
    }

    #[test]
    fn the_class_follows_the_catalog_record() {
        assert_eq!(MaterialKind::default_for(get_material("al7075")), MaterialKind::Ductile);
        assert_eq!(MaterialKind::default_for(get_material("steel4340")), MaterialKind::Ductile);
        assert_eq!(MaterialKind::default_for(get_material("cfrp_qi")), MaterialKind::Composite);
        let mut brittle = *get_material("al7075");
        static EXTRA: std::sync::OnceLock<mechanics_core::materials::MaterialExtras> = std::sync::OnceLock::new();
        let mut extra = get_material("al7075").extra.copied().unwrap_or_else(|| {
            let h = mechanics_core::materials::builtin_catalog().find(|m| m.extra.is_some()).expect("a handbook record").extra.copied().unwrap();
            h
        });
        extra.elong = mechanics_core::materials::Dir { l: 2.0, lt: 0.0, st: 0.0 };
        brittle.extra = Some(EXTRA.get_or_init(|| extra));
        assert_eq!(MaterialKind::default_for(&brittle), MaterialKind::Brittle);
    }

    #[test]
    fn reseed_keeps_edited_rows_and_replaces_the_rest() {
        let mut rows = seeds(MaterialKind::Ductile);
        rows[1].factor = 1.5;
        rows[1].edited = true;
        rows.push(Requirement { label: "Proof".into(), criterion: Req::VonMisesYield, factor: 1.5, required_ms: 0.0, edited: true });
        reseed(&mut rows, MaterialKind::Brittle);
        let kept: Vec<_> = rows.iter().filter(|r| r.edited).map(|r| r.label.as_str()).collect();
        assert_eq!(kept, ["Yield (Tresca)", "Proof"], "edited rows survive the class change untouched");
        assert!(rows.iter().any(|r| r.criterion == Req::Rankine && !r.edited) && rows.iter().any(|r| r.criterion == Req::CoulombMohr));
        assert!(!rows.iter().any(|r| !r.edited && r.criterion == Req::VonMisesYield), "unedited ductile seeds are gone");
        // Back again: the seed whose criterion an edited row already covers is not added twice.
        reseed(&mut rows, MaterialKind::Ductile);
        assert_eq!(rows.iter().filter(|r| r.criterion == Req::TrescaYield).count(), 1);
        assert_eq!(rows.iter().filter(|r| r.criterion == Req::VonMisesYield).count(), 1, "the edited Proof row covers von Mises");
    }

    #[test]
    fn a_row_names_its_allowable() {
        assert_eq!(Req::VonMisesYield.allowable_text(70.0, 77.0, 80.0).as_deref(), Some("Yield 70 ksi"));
        assert_eq!(Req::Rankine.allowable_text(70.0, 77.0, 80.0).as_deref(), Some("Ultimate 77 ksi"));
        assert_eq!(Req::CoulombMohr.allowable_text(70.0, 77.0, 80.0).as_deref(), Some("Ultimate 77 ksi tension, 80 ksi compression"));
        assert_eq!(Req::Buckling.allowable_text(70.0, 77.0, 80.0), None);
    }

    #[test]
    fn criteria_map_to_the_solvers_and_cycle_within_the_class() {
        assert_eq!(Req::Rankine.cylinder(), Some(ThicknessCriterion::UltimateMaxPrincipal));
        assert_eq!(Req::CoulombMohr.cylinder(), None);
        assert_eq!(Req::Buckling.line(), None);
        assert_eq!(Req::YieldMaxPrincipal.line(), None);
        assert_eq!(Req::VonMisesYield.line(), Some(Criterion::VonMisesYield));
        let mut c = Req::VonMisesYield;
        for _ in 0..Req::choices(MaterialKind::Ductile).len() {
            c = c.next(MaterialKind::Ductile);
        }
        assert_eq!(c, Req::VonMisesYield, "a full cycle returns to the start");
        assert_eq!(Req::Buckling.next(MaterialKind::Brittle), Req::Rankine);
    }
}
