//! One Find box for everything: what was typed decides how it is searched. A sentence is read as conditions; anything
//! else (including a typed question) is an exact search of the words and phrases. The user can pin the reading instead
//! of leaving it on Auto.

/// How the Find text is read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum FindAs {
    Auto,
    Words,
    Sentence,
}

impl FindAs {
    pub(super) fn next(self) -> Self {
        match self {
            Self::Auto => Self::Words,
            Self::Words => Self::Sentence,
            Self::Sentence => Self::Auto,
        }
    }
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::Auto => "Find as: auto",
            Self::Words => "Find as: words",
            Self::Sentence => "Find as: sentence",
        }
    }
}

/// What a Find text is, once Auto has looked at it (never `Auto`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Intent {
    Words,
    #[cfg_attr(not(feature = "nl-find"), allow(dead_code))]
    Sentence,
}

pub(super) fn classify(text: &str, pinned: FindAs) -> Intent {
    match pinned {
        FindAs::Words => Intent::Words,
        FindAs::Sentence => Intent::Sentence,
        FindAs::Auto => {
            #[cfg(feature = "nl-find")]
            if super::nl::looks_like_sentence(text) {
                Intent::Sentence
            } else {
                Intent::Words
            }
            #[cfg(not(feature = "nl-find"))]
            {
                let _ = text;
                Intent::Words
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auto_reads_a_sentence_or_words_and_a_pin_overrides() {
        assert_eq!(classify("how do I exclude a folder?", FindAs::Words), Intent::Words);
        assert_eq!(classify("torque, bolt", FindAs::Auto), Intent::Words);
        assert_eq!(classify("torque, bolt", FindAs::Sentence), Intent::Sentence);
        #[cfg(feature = "nl-find")]
        assert_eq!(classify("surfaces that were blended and filled and fair", FindAs::Auto), Intent::Sentence);
    }

    #[test]
    fn a_pinned_reading_cycles() {
        let mut m = FindAs::Auto;
        for _ in 0..3 {
            m = m.next();
        }
        assert_eq!(m, FindAs::Auto);
    }
}
