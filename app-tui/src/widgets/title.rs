//! Toolbox block titles that stay readable on narrow terminals.

/// `" Name - key1 · key2 · ... "`, dropping trailing key hints (least important last) until the
/// title fits the block's top border; the toolbox name alone is the floor.
pub fn toolbox_title(name: &str, keys: &[&str], width: u16) -> String {
    // The border takes two corner cells and the title keeps one cell of margin on each side.
    let room = (width as usize).saturating_sub(4);
    let mut shown = keys.len();
    loop {
        let title = if shown == 0 { name.to_string() } else { format!("{name} - {}", keys[..shown].join(" \u{b7} ")) };
        if shown == 0 || title.chars().count() <= room {
            return format!(" {title} ");
        }
        shown -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::toolbox_title;

    #[test]
    fn keys_drop_from_the_end_until_the_title_fits() {
        let keys = ["r Analyse", "m Max offset", "e Export"];
        assert_eq!(toolbox_title("Eccentric", &keys, 120), " Eccentric - r Analyse \u{b7} m Max offset \u{b7} e Export ");
        let narrow = toolbox_title("Eccentric", &keys, 35);
        assert_eq!(narrow, " Eccentric - r Analyse ");
        assert!(narrow.chars().count() <= 35 - 2);
        assert_eq!(toolbox_title("Eccentric", &keys, 5), " Eccentric ", "the name is the floor");
    }
}
