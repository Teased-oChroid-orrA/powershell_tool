//! Small text and colour helpers shared by the property panels and the legend: word wrapping, number
//! formatting, the contour colour map (mirrored from the shader) and the key-help lines.

/// The shader's contour colour map, mirrored on the CPU for the legend (`shader.wgsl::fs`).
pub fn colormap(t: f32) -> [u8; 3] {
    let t = t.clamp(0.0, 1.0);
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(t), q(0.3 + 0.6 * (1.0 - (2.0 * t - 1.0).abs())), q(1.0 - t)]
}

pub fn wrap(text: &str, width: usize) -> Vec<String> {
    let mut lines = Vec::new();
    let mut line = String::new();
    for word in text.split_whitespace() {
        let mut word = word.to_string();
        while word.chars().count() > width {
            if !line.is_empty() {
                lines.push(std::mem::take(&mut line));
            }
            lines.push(word.chars().take(width).collect());
            word = word.chars().skip(width).collect();
        }
        if !line.is_empty() && line.chars().count() + 1 + word.chars().count() > width {
            lines.push(std::mem::take(&mut line));
        }
        if !line.is_empty() {
            line.push(' ');
        }
        line.push_str(&word);
    }
    if !line.is_empty() {
        lines.push(line);
    }
    lines
}

pub fn fmt_value(v: f32) -> String {
    let a = v.abs();
    if v == 0.0 {
        "0".into()
    } else if !(1e-3..1e6).contains(&a) {
        format!("{v:.3e}")
    } else {
        format!("{v:.4}")
    }
}

pub const HELP: [&str; 10] = [
    "Tab or [ ] : previous / next result",
    "F / Shift+F: next / previous scalar field",
    "M mesh   C contour   D deformation",
    "Space play/pause   Home rewind",
    "Left/Right : step one frame",
    "Up/Down    : deformation gain",
    "+ / -      : playback speed",
    "Drag orbit, wheel zoom at the cursor, R reset camera",
    "Middle or right drag pans (a right click opens the menu)",
    "H or ? toggles this help; Esc closes",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn colormap_matches_shader_endpoints_and_middle() {
        assert_eq!(colormap(0.0), [0, 77, 255]);
        assert_eq!(colormap(0.5), [128, 230, 128]);
        assert_eq!(colormap(1.0), [255, 77, 0]);
        assert_eq!(colormap(-3.0), colormap(0.0));
        assert_eq!(colormap(7.0), colormap(1.0));
    }
    #[test]
    fn wrap_respects_width_and_splits_long_words() {
        for l in wrap("alpha beta gamma delta epsilon", 11) {
            assert!(l.chars().count() <= 11, "{l}");
        }
        assert_eq!(wrap("abcdefghij", 4), ["abcd", "efgh", "ij"]);
        assert!(wrap("   ", 10).is_empty());
    }
    #[test]
    fn value_formatting_switches_to_scientific_outside_plain_range() {
        assert_eq!(fmt_value(0.0), "0");
        assert_eq!(fmt_value(12.5), "12.5000");
        assert_eq!(fmt_value(2.5e7), "2.500e7");
        assert_eq!(fmt_value(-4e-5), "-4.000e-5");
    }
}
