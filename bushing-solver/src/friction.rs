//! Typical installation friction coefficients for a bushing press/shrink
//! fit - reference values only (same trust level as
//! `mechanics_core::materials::MATERIALS`'s own "known constants" table),
//! each with a plain-language note on when it is and isn't the right
//! choice. Selecting one is always a starting point, never a lock - the
//! resulting `BushingInputs::friction`/`BushingModel::friction` field stays
//! an ordinary editable number afterward, same as every other input in
//! this crate's UI heads.

#[derive(Debug, Clone, Copy)]
pub struct FrictionTypical {
    pub label: &'static str,
    pub value: f64,
    pub note: &'static str,
}

pub static FRICTION_TYPICALS: &[FrictionTypical] = &[
    FrictionTypical {
        label: "Dry, steel-on-steel",
        value: 0.15,
        note: "General-purpose dry press-fit default for ferrous-on-ferrous. Not appropriate once any lubricant is present at install - use a lubricated value instead, or the install/retained force will be overestimated.",
    },
    FrictionTypical {
        label: "Dry, steel-on-bronze",
        value: 0.19,
        note: "Typical for a steel bore with a bronze bushing, dry. Bronze-on-steel dry friction runs slightly higher than steel-on-steel - don't reuse the steel-on-steel value for a bronze bushing.",
    },
    FrictionTypical {
        label: "Dry, steel-on-aluminum",
        value: 0.17,
        note: "Typical for a steel bore with an aluminum-bushing or aluminum-housing pairing, dry. Aluminum's own oxide layer affects this more than bulk material friction would predict - treat as a starting point, not a precise value.",
    },
    FrictionTypical {
        label: "Lubricated, general install",
        value: 0.08,
        note: "General assembly lubricant (e.g. anti-seize or a light oil) applied at install, any common metal pairing. Do not use for a service condition where the lubricant is expected to dry out or be washed away in service - use a dry value for that case instead.",
    },
    FrictionTypical {
        label: "Lubricated, PTFE or similar dry-film coating",
        value: 0.05,
        note: "A dry-film lubricant coating (PTFE, MoS2) rather than a wet lubricant - lower and more consistent over time than a wet lubricant that can migrate or dry out. Only applicable when the actual part is coated; do not assume this value for an uncoated part that merely receives assembly grease.",
    },
    FrictionTypical {
        label: "Dry, titanium-on-titanium",
        value: 0.30,
        note: "Titanium-on-titanium dry friction runs notably higher than steel pairings and is prone to galling - a shrink fit (thermal assist) or a lubricant is usually preferred over a straight dry press for this pairing; use this value only when a dry press is genuinely the specified install method.",
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_entry_has_a_positive_value_and_a_nonempty_note() {
        for entry in FRICTION_TYPICALS {
            assert!(entry.value > 0.0 && entry.value < 1.0, "{}: value {} out of a physically sane 0..1 range", entry.label, entry.value);
            assert!(!entry.note.is_empty(), "{} has no usage note", entry.label);
        }
    }
}
