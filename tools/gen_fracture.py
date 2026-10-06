#!/usr/bin/env python3
"""Generates mechanics-core/src/fracture.rs: the finite-strain bearing failure strain of every
library material (MIL-HDBK-5J conditions + the curated built-ins).

Rule (docs/failure-strain-library.md): eps_f = m_family * ln(1 + A), A = the condition's own
handbook elongation (lowest of L / LT, else ST), capped at 0.50 and floored at 0.03. The
multiplier `m` is bound by two kinds of evidence and never exceeds the largest value validated
against pin-bearing tests (2.0):

  * bearing-validated: wrought aluminium 7075 (m = 1.0) and 2014 / 2024 (m = 2.0) reproduce the
    NACA TN 1503 and TN 1502 bearing tests (20 points, six alloy/temper/product combinations,
    -9 % to +3 %) - lug-solver/tests/failure_strain_rule.rs;
  * tensile-anchored: published reduction of area (ASM / AMS specifications) fixes how much larger
    the true fracture strain is than ln(1 + A) for steels, PH stainless, titanium and nickel; those
    families get m = min(evidence, 2.0), others (no evidence) m = 1.0, the lower bound.

Conditions with no elongation get the flat default 0.10 (basis "default").
Run after replacing the handbook JSON:   python3 tools/gen_fracture.py
"""
import json, math, pathlib, collections

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "mechanics-core/data/mil-hdbk-5j.json"
OUT = ROOT / "mechanics-core/src/fracture.rs"

CAP, FLOOR, DEFAULT, M_MAX = 0.50, 0.03, 0.10, 2.0

# Typical elongation (%) of the curated built-ins (they carry none of their own), from the
# handbook conditions they stand for.
CURATED = {
    "al7075": ("Aluminum Alloys", "7075", 11.0), "al7075t6": ("Aluminum Alloys", "7075", 11.0),
    "al2024": ("Aluminum Alloys", "2024", 18.0), "al2024t3": ("Aluminum Alloys", "2024", 18.0),
    "al7050": ("Aluminum Alloys", "7050", 10.0),
    "steel": ("Low-Alloy Steels", "steel", 12.0), "steel4340": ("Low-Alloy Steels", "4340", 11.0),
    "ti6al4v": ("Alpha-Beta Titanium Alloys", "Ti-6Al-4V", 10.0),
    "ph157mo": ("Precipitation and Transformation-Hardening Stainless Steels", "15-7Mo", 6.0),
    "ph174": ("Precipitation and Transformation-Hardening Stainless Steels", "17-4PH", 12.0),
    "inconel718": ("Nickel-Base Alloys", "Inconel 718", 12.0), "inconel625": ("Nickel-Base Alloys", "Inconel 625", 40.0),
    "washer_steel": ("Carbon Steels", "washer", 25.0), "washer_al": ("Aluminum Alloys", "washer", 15.0),
    "bronze": (None, "Al-Bronze", 15.0), "beryllium": (None, "Be-Cu", 4.0),
}

VALIDATED_AL = {"2014": 2.0, "2024": 2.0, "7075": 1.0}


def multiplier(group, alloy):
    """-> (m, basis) with basis in validated / family / bound."""
    if group == "Aluminum Alloys":
        key = alloy.strip()[:4]
        if key in VALIDATED_AL:
            return VALIDATED_AL[key], "validated"
        if key.startswith("2"):
            return 2.0, "family"  # 2xxx: the validated 2014 / 2024 behaviour
        return 1.0, "bound"
    if group in ("Low-Alloy Steels", "Intermediate Alloy Steels", "High-Alloy Steels", "Carbon Steels"):
        return 2.0, "family"  # AMS 6414 4340: RA >= 30 % at A = 10 %  -> ratio 3.7, capped at 2.0
    if group == "Precipitation and Transformation-Hardening Stainless Steels":
        return 2.0, "family"  # AMS 5643 17-4PH: RA >= 40-45 % at A = 10-12 % -> ratio 5.3, capped
    if group == "Austenitic Stainless Steels" or group == "Iron-Chromium-Nickel-Base Alloys":
        return 2.0, "family"  # annealed austenitics: RA 50-70 %
    if group in ("Unalloyed Titanium", "Alpha and Near-Alpha Titanium Alloys", "Alpha-Beta Titanium Alloys"):
        return 2.0, "family"  # Ti-6Al-4V ASM: RA 25-36 % at A = 10-14 % -> ratio 3.0 and up, capped
    if group == "Nickel-Base Alloys":
        return 1.4, "family"  # Inconel 718 AMS 5662 minimum: RA 15 % at A = 12 % -> ratio 1.4
    return 1.0, "bound"  # magnesium, beta titanium, cobalt, other aluminium, copper alloys: no evidence


def elong(entry):
    e = entry["props"].get("elong")
    if e is None:
        return None
    if isinstance(e, dict):
        vals = [e.get(k) for k in ("L", "LT") if e.get(k)]
        if not vals and e.get("ST"):
            vals = [e["ST"]]
        return min(vals) if vals else None
    return float(e) if e else None


def value(group, alloy, a_pct):
    m, basis = multiplier(group, alloy)
    if not a_pct or a_pct <= 0:
        return DEFAULT, "default", 1.0, 0.0
    raw = m * math.log(1.0 + a_pct / 100.0)
    v = min(max(raw, FLOOR), CAP)
    if raw > CAP:
        basis = "capped"
    return v, basis, m, a_pct


TESTS = '\n#[cfg(test)]\nmod tests {\n    use super::*;\n    use crate::materials::builtin_catalog;\n\n    #[test]\n    fn every_library_material_has_a_failure_strain_in_range() {\n        let mut n = 0;\n        for m in builtin_catalog() {\n            let f = failure_strain(m.id).unwrap_or_else(|| panic!("no failure strain for {}", m.id));\n            if f.basis == Basis::NotApplicable {\n                assert_eq!(m.id, "cfrp_qi");\n                continue;\n            }\n            assert!((0.0299..=0.5001).contains(&f.value), "{}: {}", m.id, f.value);\n            n += 1;\n        }\n        assert_eq!(n + 1, len());\n        assert!(failure_strain("nonexistent").is_none());\n    }\n\n    #[test]\n    fn the_rule_is_multiplier_times_ln_one_plus_elongation() {\n        for m in builtin_catalog() {\n            let f = failure_strain(m.id).unwrap();\n            if matches!(f.basis, Basis::Validated | Basis::Family | Basis::Bound) {\n                let want = (f.multiplier * (1.0 + f.elongation_pct / 100.0).ln()).clamp(0.03, 0.50);\n                assert!((f.value - want).abs() < 6e-4, "{}: {} vs {want}", m.id, f.value);\n                assert!(f.multiplier <= 2.0, "{}: no multiplier beyond the bearing-validated 2.0", m.id);\n            }\n        }\n    }\n\n    #[test]\n    fn the_validated_aluminium_alloys_follow_the_naca_calibration() {\n        // 7075-T6 bar (A = 11.2 %): the NACA TN 1503 inverse calibration gave about 0.11.\n        let f = failure_strain("al7075").unwrap();\n        assert_eq!(f.basis, Basis::Typical);\n        assert!((f.value - 0.104).abs() < 0.01, "{}", f.value);\n        // Every 2024 / 7075 / 2014 handbook condition with an elongation is Validated.\n        let validated = builtin_catalog().filter(|m| m.extra.is_some_and(|x| ["2014", "2024", "7075"].iter().any(|a| x.alloy.starts_with(a))) && failure_strain(m.id).unwrap().basis == Basis::Validated).count();\n        assert!(validated > 100, "{validated}");\n    }\n}\n'


def main():
    data = json.loads(SRC.read_text())["entries"]
    rows = []  # (id, value, basis, m, a)
    for mid, (group, alloy, a) in CURATED.items():
        if group is None:
            v, b, m, aa = value("copper", alloy, a)
        else:
            v, b, m, aa = value(group, alloy, a)
        rows.append((mid, v, "typical" if b in ("validated", "family", "bound", "capped") else b, m, aa))
    rows.append(("cfrp_qi", 0.0, "n/a", 0.0, 0.0))
    omitted = 0
    for e in data:
        p = e["props"]
        if not (p.get("Ftu") or p.get("Fty")):
            omitted += 1  # the generator of handbook.rs omits these conditions too
            continue
        v, b, m, a = value(e["group"], e["alloy"], elong(e))
        rows.append(("mh5:" + e["id"], v, b, m, a))
    rows.sort(key=lambda r: r[0])
    code = {"validated": 0, "family": 1, "bound": 2, "default": 3, "capped": 4, "typical": 5, "n/a": 6}
    lines = [
        "//! GENERATED by tools/gen_fracture.py - do not edit by hand.",
        "//!",
        "//! Finite-strain bearing failure strain of every library material: `m ln(1 + A)` with `A` the",
        "//! condition's own handbook elongation and `m` the family multiplier. Method, sources and the",
        "//! validation against NACA TN 1503 / TN 1502: docs/failure-strain-library.md.",
        "",
        "/// How a failure strain was obtained, from most to least evidence.",
        "#[derive(Debug, Clone, Copy, PartialEq, Eq)]",
        "pub enum Basis {",
        "    /// Aluminium 2014, 2024, 7075: the rule reproduces published pin-bearing tests (NACA TN 1503, TN 1502).",
        "    Validated,",
        "    /// A family multiplier fixed by published tensile reduction of area (or the validated 2xxx aluminium behaviour).",
        "    Family,",
        "    /// No evidence for the family: the lower bound `ln(1 + A)` (multiplier 1).",
        "    Bound,",
        "    /// The condition has no handbook elongation: the flat default 0.10.",
        "    Default,",
        "    /// The rule gave more than the 0.50 cap (strains beyond 0.4 are unvalidated).",
        "    Capped,",
        "    /// A curated built-in: typical elongation of the condition it stands for, same rule.",
        "    Typical,",
        "    /// Not a metal; the finite-strain model does not apply.",
        "    NotApplicable,",
        "}",
        "",
        "#[derive(Debug, Clone, Copy, PartialEq)]",
        "pub struct FailureStrain {",
        "    /// Equivalent plastic strain at which the lug is taken to fail.",
        "    pub value: f64,",
        "    pub basis: Basis,",
        "    /// Family multiplier `m` and the elongation (%) used (0 when none).",
        "    pub multiplier: f64,",
        "    pub elongation_pct: f64,",
        "}",
        "",
        "/// The default used when a material is not in the library (a user-added one) and by `Default` entries.",
        f"pub const DEFAULT_FAILURE_STRAIN: f64 = {DEFAULT};",
        "",
        f"static TABLE: [(&str, f32, u8, f32, f32); {len(rows)}] = [",
    ]
    for r in rows:
        lines.append(f'    ("{r[0]}", {r[1]:.4f}, {code[r[2]]}, {r[3]:.2f}, {r[4]:.1f}),')
    lines += [
        "];",
        "",
        "/// The failure strain of the library material with this `id` (`None` for a user-added material).",
        "pub fn failure_strain(id: &str) -> Option<FailureStrain> {",
        "    let i = TABLE.binary_search_by(|r| r.0.cmp(id)).ok()?;",
        "    let (_, value, basis, m, a) = TABLE[i];",
        "    let basis = match basis {",
        "        0 => Basis::Validated,",
        "        1 => Basis::Family,",
        "        2 => Basis::Bound,",
        "        3 => Basis::Default,",
        "        4 => Basis::Capped,",
        "        5 => Basis::Typical,",
        "        _ => Basis::NotApplicable,",
        "    };",
        "    Some(FailureStrain { value: value as f64, basis, multiplier: m as f64, elongation_pct: a as f64 })",
        "}",
        "",
        "/// Number of entries.",
        "pub fn len() -> usize {",
        "    TABLE.len()",
        "}",
        "",
    ]
    OUT.write_text("\n".join(lines) + TESTS)
    c = collections.Counter(r[2] for r in rows)
    print(f"{len(rows)} entries ({omitted} source conditions without Ftu/Fty omitted): {dict(c)}")
    vals = [r[1] for r in rows if r[2] not in ("n/a",)]
    print("value range %.3f .. %.3f, median %.3f" % (min(vals), max(vals), sorted(vals)[len(vals) // 2]))


if __name__ == "__main__":
    main()
