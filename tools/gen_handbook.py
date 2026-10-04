#!/usr/bin/env python3
"""Generates mechanics-core/src/handbook.rs from mechanics-core/data/mil-hdbk-5j.json.

Source data: MIL-HDBK-5J design mechanical and physical property tables (a
US-government document, public domain), as digitised and published by
KasperCalc (https://kaspercalc.com/MaterialPropertyLookup.html, file
js/data/design-allowables.json, retrieved 2026-10-03). Re-run after replacing
the JSON:   python3 tools/gen_handbook.py

Conversion rules (also documented in docs/material-handbook.md):
  * strengths in ksi; E/Ec/G are published in Msi and are stored here in ksi;
  * the solver `Material` takes the LOWEST of the in-plane (L, LT) values of
    Ftu/Fty/Fsu (ST excluded) - design-conservative; every directional value
    is kept in `MaterialExtras`;
  * Fbru/Fbry at e/D = 2.0 and 1.5 are the handbook values (0 = not tabulated);
  * conditions with neither Ftu nor Fty (4 in the source) are omitted - see
    docs/material-handbook.md;
  * fields the handbook does not give for a condition (E, Poisson's ratio) and
    the thermal expansion coefficient (never in these tables) are filled with
    GROUP-TYPICAL engineering values and listed in `estimated`.
"""
import json, collections, pathlib

ROOT = pathlib.Path(__file__).resolve().parent.parent
SRC = ROOT / "mechanics-core/data/mil-hdbk-5j.json"
OUT = ROOT / "mechanics-core/src/handbook.rs"

# group -> (E ksi, nu, alpha microstrain/degF) typical values, used ONLY where the
# handbook table is silent. Not handbook data.
TYPICAL = {
    "Aluminum Alloys": (10500.0, 0.33, 12.8),
    "Magnesium Alloys": (6500.0, 0.35, 14.5),
    "Unalloyed Titanium": (14900.0, 0.32, 4.8),
    "Alpha and Near-Alpha Titanium Alloys": (16000.0, 0.31, 4.9),
    "Alpha-Beta Titanium Alloys": (16000.0, 0.31, 4.9),
    "Beta, Near-Beta and Metastable-Beta Titanium Alloys": (15500.0, 0.32, 4.9),
    "Carbon Steels": (29000.0, 0.29, 6.5),
    "Low-Alloy Steels": (29000.0, 0.29, 6.3),
    "Intermediate Alloy Steels": (29000.0, 0.29, 6.3),
    "High-Alloy Steels": (29000.0, 0.29, 6.3),
    "Austenitic Stainless Steels": (28000.0, 0.29, 9.2),
    "Precipitation and Transformation-Hardening Stainless Steels": (28500.0, 0.29, 6.0),
    "Iron-Chromium-Nickel-Base Alloys": (28500.0, 0.29, 7.5),
    "Nickel-Base Alloys": (29000.0, 0.30, 7.0),
    "Cobalt-Base Alloys": (30000.0, 0.30, 7.0),
}
FALLBACK = (29000.0, 0.30, 7.0)


def rs(s):
    return json.dumps(s if s is not None else "", ensure_ascii=False)


def f(v):
    return repr(float(v))


def direction(v):
    """-> (l, lt, st), 0.0 = not given."""
    if v is None:
        return (0.0, 0.0, 0.0)
    if isinstance(v, dict):
        return (v.get("L", 0.0), v.get("LT", 0.0), v.get("ST", 0.0))
    return (float(v), float(v), 0.0)


def inplane_min(v):
    l, lt, st = direction(v)
    vals = [x for x in (l, lt) if x]
    if not vals and st:
        vals = [st]
    return min(vals) if vals else 0.0


def shear_min(v):
    l, lt, st = direction(v)
    vals = [x for x in (l, lt, st) if x]
    return min(vals) if vals else 0.0


def dir_lit(v):
    l, lt, st = direction(v)
    return f"Dir {{ l: {f(l)}, lt: {f(lt)}, st: {f(st)} }}"


def build():
    all_entries = json.load(open(SRC))["entries"]
    # A condition with neither Ftu nor Fty cannot drive a solver (it would divide
    # by a zero yield strength): leave it out of the selectable library and say so.
    entries = [e for e in all_entries if inplane_min(e["props"].get("Ftu")) or inplane_min(e["props"].get("Fty"))]
    skipped = [e["id"] for e in all_entries if e not in entries]
    print("skipped (no tension strength in the source):", skipped)
    # unique display names
    def base_name(e):
        parts = [e["alloy"]]
        if e.get("temper"):
            parts.append(e["temper"])
        if e.get("form"):
            parts.append(e["form"])
        th = e.get("thickness") or {}
        if th.get("raw") and th["raw"] != "…":
            parts.append(th["raw"] + " in")
        n = " ".join(parts)
        if e.get("basis"):
            n += f" [{e['basis']}]"
        return n

    names = [base_name(e) for e in entries]
    cnt = collections.Counter(names)
    for i, e in enumerate(entries):
        if cnt[names[i]] > 1:
            names[i] += f" (T{e['table']})"
    cnt2 = collections.Counter(names)
    for i, e in enumerate(entries):
        if cnt2[names[i]] > 1:
            names[i] += f" #{e['id'].split('#')[-1]}"
    assert len(set(names)) == len(names)

    extras, mats = [], []
    for i, e in enumerate(entries):
        p = e["props"]
        te, tn, ta = TYPICAL.get(e["group"], FALLBACK)
        est = []
        ftu = inplane_min(p.get("Ftu"))
        fty = inplane_min(p.get("Fty"))
        if not fty and ftu:
            fty = ftu
            est.append("Fty(=Ftu)")
        if not ftu and fty:
            ftu = fty
            est.append("Ftu(=Fty)")
        e_msi = inplane_min(p.get("E"))
        if e_msi:
            e_ksi = e_msi * 1000.0
        else:
            e_ksi = te
            est.append("E")
        nu = p.get("mu")
        if not nu:
            nu = tn
            est.append("nu")
        est.append("alpha")
        fb = p.get("Fbru") or {}
        fby = p.get("Fbry") or {}
        fsu = shear_min(p.get("Fsu"))
        ec = inplane_min(p.get("Ec")) * 1000.0
        g = inplane_min(p.get("G")) * 1000.0
        th = e.get("thickness") or {}
        thk = th.get("raw") if th.get("raw") and th["raw"] != "…" else ""
        extras.append(
            "MaterialExtras { "
            f"alloy: {rs(e['alloy'])}, group: {rs(e['group'])}, form: {rs(e.get('form'))}, temper: {rs(e.get('temper'))}, "
            f"spec: {rs(e.get('spec'))}, thickness: {rs(thk)}, basis: {rs(e.get('basis'))}, clad: {str(bool(e.get('clad'))).lower()}, "
            f"table: {rs(e['table'])}, caption: {rs(e['caption'])}, "
            f"ftu: {dir_lit(p.get('Ftu'))}, fty: {dir_lit(p.get('Fty'))}, fcy: {dir_lit(p.get('Fcy'))}, fsu: {dir_lit(p.get('Fsu'))}, elong: {dir_lit(p.get('elong'))}, "
            f"fbru_e20_ksi: {f(fb.get('eD2.0', 0.0))}, fbru_e15_ksi: {f(fb.get('eD1.5', 0.0))}, fbry_e20_ksi: {f(fby.get('eD2.0', 0.0))}, fbry_e15_ksi: {f(fby.get('eD1.5', 0.0))}, "
            f"ec_ksi: {f(ec)}, g_ksi: {f(g)}, density_lb_in3: {f(p.get('density', 0.0))}, "
            f"estimated: {rs(', '.join(est))} }}"
        )
        mats.append(
            "Material { "
            f"id: {rs('mh5:' + e['id'])}, name: {rs(names[i])}, e_ksi: {f(e_ksi)}, sy_ksi: {f(fty)}, "
            f"fbru_ksi: {f(fb.get('eD2.0', 0.0))}, fbru_e15_ksi: {f(fb.get('eD1.5', 0.0))}, fsu_ksi: {f(fsu)}, ftu_ksi: {f(ftu)}, nu: {f(nu)}, alpha_u_f: {f(ta)}, "
            f"extra: Some(&EXTRAS[{i}]) }}"
        )
    n = len(entries)
    out = [
        "//! GENERATED by tools/gen_handbook.py from data/mil-hdbk-5j.json - do not edit by hand.",
        "//!",
        "//! MIL-HDBK-5J design mechanical and physical properties: every tabulated",
        f"//! condition ({n}), as published by KasperCalc",
        "//! (https://kaspercalc.com/MaterialPropertyLookup.html). See the generator's",
        "//! docstring and docs/material-handbook.md for the conversion rules.",
        "",
        "use crate::materials::{Dir, Material, MaterialExtras};",
        "",
        f"static EXTRAS: [MaterialExtras; {n}] = [",
    ]
    out += ["    " + x + "," for x in extras]
    out += ["];", "", f"pub static HANDBOOK: [Material; {n}] = ["]
    out += ["    " + x + "," for x in mats]
    out += ["];", ""]
    OUT.write_text("\n".join(out))
    print("wrote", OUT, n, "entries", OUT.stat().st_size, "bytes")


build()
