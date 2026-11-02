//! Shared material-picker helpers: multi-word filtering and the property
//! panel for the highlighted material (every MIL-HDBK-5J value for a
//! handbook condition, the solver fields otherwise).

use mechanics_core::materials::{Dir, Material};
use ratatui::text::{Line, Span};

use crate::theme::Theme;

/// Every whitespace-separated word of `needle` (already lowercase) must
/// appear in the material's name, so "7075 t6 plate" narrows 1,400 entries.
pub fn matches(material: &Material, needle: &str) -> bool {
    let mut hay = material.name.to_lowercase();
    if let Some(x) = material.extra {
        // Also searchable by what the name leaves out: family, spec, table.
        hay.push(' ');
        hay.push_str(&format!("{} {} {} {}", x.group, x.spec, x.table, if x.clad { "clad" } else { "" }).to_lowercase());
    }
    needle.split_whitespace().all(|w| hay.contains(w))
}

/// Type-to-search editing shared by the material pickers: printable
/// characters append, Backspace removes one, Delete clears. Returns `true`
/// if the key edited the text (the caller then resets its cursor).
pub fn search_edit(text: &mut String, key: &crossterm::event::KeyEvent) -> bool {
    use crossterm::event::{KeyCode, KeyModifiers};
    match key.code {
        KeyCode::Char(c) if key.modifiers.is_empty() || key.modifiers == KeyModifiers::SHIFT => {
            text.push(c);
            true
        }
        KeyCode::Backspace => text.pop().is_some(),
        KeyCode::Delete => {
            let had = !text.is_empty();
            text.clear();
            had
        }
        _ => false,
    }
}

/// The search line every material picker shows: always visible, so the
/// feature is discoverable.
pub fn search_line<'a>(theme: &Theme, text: &str, hint: &str, width: u16) -> Line<'a> {
    let label = if text.is_empty() { "Search (just type): " } else { "Search: " };
    let mut line = crate::widgets::input_line::line(theme, label, text, "_", width.saturating_sub(hint.chars().count() as u16 + 2));
    line.spans.push(Span::styled(format!("  {hint}"), theme.disabled_style()));
    line
}

fn dir(d: &Dir) -> String {
    let mut parts = Vec::new();
    if d.l != 0.0 {
        parts.push(format!("L {}", trim(d.l)));
    }
    if d.lt != 0.0 {
        parts.push(format!("LT {}", trim(d.lt)));
    }
    if d.st != 0.0 {
        parts.push(format!("ST {}", trim(d.st)));
    }
    if parts.is_empty() {
        "-".to_string()
    } else if d.l == d.lt && d.st == 0.0 {
        trim(d.l)
    } else {
        parts.join(" / ")
    }
}

fn trim(v: f64) -> String {
    let s = format!("{v:.2}");
    s.trim_end_matches('0').trim_end_matches('.').to_string()
}

fn opt(v: f64) -> String {
    if v == 0.0 {
        "-".to_string()
    } else {
        trim(v)
    }
}

/// One handbook condition's values as plain text, shared by the terminal property panel ([`lines`]) and the native
/// Material Lookup window's property grid. Directional values read "L 75 / LT 76"; a missing value is "-".
pub struct Facts {
    /// Alloy, temper, form and thickness.
    pub heading: String,
    /// Handbook table, basis, clad and specification.
    pub source: String,
    pub ftu: String,
    pub fty: String,
    pub fcy: String,
    pub fsu: String,
    /// Bearing ultimate / yield at e/D 2.0 and 1.5 (ksi).
    pub fbru: (String, String),
    pub fbry: (String, String),
    pub elongation: String,
    /// E, with `*` when it is a group-typical estimate.
    pub e: String,
    pub ec: String,
    pub g: String,
    pub nu: String,
    pub density: String,
    /// Which values are estimates and what the solver takes.
    pub note: String,
}

/// The handbook record of `m` as plain text; `None` for a curated typical-value material (no record).
pub fn facts(m: &Material) -> Option<Facts> {
    let x = m.extra?;
    let basis = match x.basis {
        "A" => "A-basis",
        "B" => "B-basis",
        "S" => "S-basis (specification minimum)",
        _ => "no statistical basis stated",
    };
    let estimated = |name: &str| x.estimated.split(", ").any(|e| e == name);
    Some(Facts {
        heading: format!("{} - {} {} {}", x.alloy, x.temper, x.form, if x.thickness.is_empty() { String::new() } else { format!("{} in", x.thickness) }).trim().to_string(),
        source: format!("MIL-HDBK-5J Table {} - {basis}{}{}", x.table, if x.clad { " - clad" } else { "" }, if x.spec.is_empty() { String::new() } else { format!(" - {}", x.spec) }),
        ftu: dir(&x.ftu),
        fty: dir(&x.fty),
        fcy: dir(&x.fcy),
        fsu: dir(&x.fsu),
        fbru: (opt(x.fbru_e20_ksi), opt(x.fbru_e15_ksi)),
        fbry: (opt(x.fbry_e20_ksi), opt(x.fbry_e15_ksi)),
        elongation: dir(&x.elong),
        e: if estimated("E") { format!("{:.0}*", m.e_ksi) } else { format!("{:.0}", m.e_ksi) },
        ec: opt(x.ec_ksi),
        g: opt(x.g_ksi),
        nu: if estimated("nu") { format!("{}*", trim(m.nu)) } else { trim(m.nu) },
        density: if x.density_lb_in3 == 0.0 { "-".to_string() } else { format!("{:.3}", x.density_lb_in3) },
        note: format!("Solver uses the lowest of L/LT. * or alpha {} microstrain/F = group-typical, not in the handbook ({}).", trim(m.alpha_u_f), x.estimated),
    })
}

/// The solver fields of a curated typical-value material (no handbook record) as `(label, value)` pairs, ksi.
pub fn curated_values(m: &Material) -> Vec<(&'static str, String)> {
    vec![("E", format!("{:.0}", m.e_ksi)), ("Fty", trim(m.sy_ksi)), ("Ftu", trim(m.ftu_ksi)), ("Fsu", opt(m.fsu_ksi)), ("Fbru", opt(m.fbru_ksi)), ("Poisson ratio", trim(m.nu))]
}

/// Property lines for the highlighted material.
pub fn lines<'a>(theme: &Theme, m: &Material) -> Vec<Line<'a>> {
    let Some(f) = facts(m) else {
        let v = curated_values(m);
        return vec![Line::from(format!("E {} ksi  Sy {} Ftu {} Fsu {} Fbru {} (ksi)  nu {}", v[0].1, v[1].1, v[2].1, v[3].1, v[4].1, v[5].1))];
    };
    vec![
        Line::from(Span::styled(f.heading, theme.title_style(false))),
        Line::from(f.source),
        Line::from(format!("Ftu {}  Fty {}  Fcy {}  Fsu {}  (ksi)", f.ftu, f.fty, f.fcy, f.fsu)),
        Line::from(format!("Fbru {} / {}  Fbry {} / {}  (ksi, e/D 2.0 / 1.5)   elongation {} %", f.fbru.0, f.fbru.1, f.fbry.0, f.fbry.1, f.elongation)),
        Line::from(format!("E {} / Ec {} / G {} (ksi)  nu {}  density {} lb/in^3", f.e, f.ec, f.g, f.nu, f.density)),
        Line::from(Span::styled(f.note, theme.disabled_style())),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use mechanics_core::handbook::HANDBOOK;

    #[test]
    fn multi_word_filter_narrows_the_handbook() {
        let hits = |q: &str| HANDBOOK.iter().filter(|m| matches(m, q)).count();
        assert!(hits("7075") > hits("7075 t6") && hits("7075 t6") > hits("7075 t6 plate") && hits("7075 t6 plate") > 0);
        assert_eq!(hits("zzzz"), 0);
        assert_eq!(hits("PLATE 7075".to_lowercase().as_str()), hits("7075 plate"), "word order does not matter");
    }

    #[test]
    fn search_also_matches_family_spec_and_table() {
        let theme_free = |q: &str| HANDBOOK.iter().filter(|m| matches(m, q)).count();
        assert!(theme_free("titanium") > 100, "family words not in the name must still match");
        assert!(theme_free("t3.7.6.0b1") > 0);
        assert!(theme_free("magnesium") > 0 && theme_free("magnesium") < theme_free("aluminum"));
    }

    #[test]
    fn search_edit_types_backspaces_and_clears() {
        use crossterm::event::{KeyCode, KeyEvent, KeyEventKind, KeyEventState, KeyModifiers};
        let k = |code, m| KeyEvent { code, modifiers: m, kind: KeyEventKind::Press, state: KeyEventState::NONE };
        let mut t = String::new();
        assert!(search_edit(&mut t, &k(KeyCode::Char('7'), KeyModifiers::NONE)));
        assert!(search_edit(&mut t, &k(KeyCode::Char('N'), KeyModifiers::SHIFT)));
        assert_eq!(t, "7N");
        assert!(!search_edit(&mut t, &k(KeyCode::Char('n'), KeyModifiers::CONTROL)), "Ctrl chords are not text");
        assert!(search_edit(&mut t, &k(KeyCode::Backspace, KeyModifiers::NONE)));
        assert_eq!(t, "7");
        assert!(search_edit(&mut t, &k(KeyCode::Delete, KeyModifiers::NONE)));
        assert!(t.is_empty());
    }

    #[test]
    fn every_handbook_material_renders_a_property_panel() {
        let theme = Theme::default_palette();
        for m in HANDBOOK.iter() {
            let l = lines(&theme, m);
            assert!(l.len() >= 6, "{}", m.name);
        }
    }
}
