use search_core::entities::{extract_entities, EntityKind};
use tokio_util::sync::CancellationToken;

fn evaluate(label: &str, samples: &[(EntityKind, &str, &str)]) {
    let mut neural_correct = 0;
    let mut top_two_correct = 0;
    let mut pattern_covered = 0;
    let mut pattern_unambiguous = 0;
    let mut pattern_first_correct = 0;
    let mut neural_within_pattern_correct = 0;
    for &(expected, text, needle) in samples {
        let result = extract_entities(text, &CancellationToken::new()).expect("entity extraction");
        let candidate = result
            .candidates
            .iter()
            .find(|candidate| text.get(candidate.bytes.clone()) == Some(needle))
            .expect("pattern candidate");
        let predicted = candidate.category_scores[0].kind;
        top_two_correct += usize::from(
            candidate
                .category_scores
                .iter()
                .take(2)
                .any(|score| score.kind == expected),
        );
        neural_correct += usize::from(predicted == expected);
        let rule_has_label = candidate.shape_hints.contains(&expected);
        pattern_covered += usize::from(rule_has_label);
        pattern_unambiguous += usize::from(rule_has_label && candidate.shape_hints.len() == 1);
        pattern_first_correct += usize::from(candidate.shape_hints.first() == Some(&expected));
        neural_within_pattern_correct += usize::from(
            candidate
                .neural_suggestion()
                .is_some_and(|score| score.kind == expected),
        );
        println!(
            "{label} expected={expected:?} neural={predicted:?} rule_hints={:?} span={needle}",
            candidate.shape_hints
        );
    }
    println!(
        "{label}: neural_top1={neural_correct}/{} neural_top2={top_two_correct}/{} rule_coverage={pattern_covered}/{} rule_first={pattern_first_correct}/{} neural_within_rule_candidates={neural_within_pattern_correct}/{} unambiguous_rule={pattern_unambiguous}/{}",
        samples.len(), samples.len(), samples.len(), samples.len(), samples.len(), samples.len()
    );
}

fn main() {
    let integration_samples = [
        (
            EntityKind::PartNumber,
            "P/N AX-42-7 identifies the replacement bracket.",
            "AX-42-7",
        ),
        (
            EntityKind::Material,
            "Machine the fitting from Ti-6Al-4V.",
            "Ti-6Al-4V",
        ),
        (EntityKind::Dimension, "The shaft length is 12 mm.", "12 mm"),
        (EntityKind::Unit, "The shaft length is 12 mm.", "mm"),
        (
            EntityKind::FastenerCallout,
            "Install four 1/4-28 UNF bolts.",
            "1/4-28 UNF",
        ),
        (
            EntityKind::Standard,
            "Design the piping to ASME B31.3.",
            "ASME B31.3",
        ),
        (
            EntityKind::Material,
            "Material callout: 2024-T3 aluminum sheet.",
            "2024-T3",
        ),
        (
            EntityKind::PartNumber,
            "Part number 2024-T3 replaces the old fitting.",
            "2024-T3",
        ),
        (
            EntityKind::FastenerCallout,
            "Select an M8x1.25 fastener.",
            "M8x1.25",
        ),
        (
            EntityKind::PartNumber,
            "P/N M6x1 is the listed replacement screw.",
            "M6x1",
        ),
        (
            EntityKind::Dimension,
            "The wall is one half inch thick, 1/2 in.",
            "1/2 in",
        ),
        (
            EntityKind::Standard,
            "Use connector MIL-DTL-38999.",
            "MIL-DTL-38999",
        ),
    ];
    let held_out_samples = [
        (
            EntityKind::PartNumber,
            "The sourced catalog lists MS21250-04 for the joint.",
            "MS21250-04",
        ),
        (
            EntityKind::PartNumber,
            "Use catalog designation NAS1005-05 for this location.",
            "NAS1005-05",
        ),
        (
            EntityKind::Material,
            "The sourced fastener uses A286 corrosion-resistant steel.",
            "A286",
        ),
        (
            EntityKind::Material,
            "Select 15-5PH stainless for the replacement fitting.",
            "15-5PH",
        ),
        (
            EntityKind::Dimension,
            "The thick-wall vessel has an ID of 4 in.",
            "4 in",
        ),
        (
            EntityKind::Dimension,
            "Measured shell wall is 0.5 in.",
            "0.5 in",
        ),
        (
            EntityKind::Unit,
            "The pressure was 5000 psi at the inner wall.",
            "psi",
        ),
        (
            EntityKind::Unit,
            "Allowable tensile stress is 275 ksi.",
            "ksi",
        ),
        (
            EntityKind::FastenerCallout,
            "Catalog bolt NAS1003-03 has a 0.1900 in diameter.",
            "NAS1003",
        ),
        (
            EntityKind::FastenerCallout,
            "Install an MS21250-08 tension bolt.",
            "MS21250",
        ),
        (
            EntityKind::Standard,
            "UNJ thread root dimensions follow ASME B1.15.",
            "ASME B1.15",
        ),
        (
            EntityKind::Standard,
            "The material properties are sourced from AMS 6414.",
            "AMS 6414",
        ),
        (
            EntityKind::PartNumber,
            "Drawing item ZX-811-03 replaces the cracked hinge.",
            "ZX-811-03",
        ),
        (
            EntityKind::PartNumber,
            "Use assembly code WR-61-K for the aft bracket.",
            "WR-61-K",
        ),
        (
            EntityKind::PartNumber,
            "The replacement is designated BQ-907-2.",
            "BQ-907-2",
        ),
        (
            EntityKind::PartNumber,
            "The bill lists component RT-204-A.",
            "RT-204-A",
        ),
        (
            EntityKind::PartNumber,
            "Order part PN 7301-GH for this repair.",
            "7301-GH",
        ),
        (
            EntityKind::PartNumber,
            "The approved spare number is XD-7009.",
            "XD-7009",
        ),
        (
            EntityKind::PartNumber,
            "Supplier label KJ-18-44 identifies the fitting.",
            "KJ-18-44",
        ),
        (
            EntityKind::Material,
            "The bearing retainer uses Inconel 718.",
            "Inconel 718",
        ),
        (
            EntityKind::Material,
            "Switch to 316L for the corrosion barrier.",
            "316L",
        ),
        (
            EntityKind::Material,
            "Specify 6061-T6 stock for the fixture.",
            "6061-T6",
        ),
        (
            EntityKind::Material,
            "The flange is machined from 7050-T7.",
            "7050-T7",
        ),
        (
            EntityKind::Material,
            "Magnesium alloy protects the casing.",
            "Magnesium",
        ),
        (
            EntityKind::Material,
            "Use stainless steel for the cover.",
            "stainless steel",
        ),
        (
            EntityKind::Material,
            "Titanium is specified for the lug.",
            "Titanium",
        ),
        (
            EntityKind::Material,
            "The clamp is made from carbon steel.",
            "carbon steel",
        ),
        (
            EntityKind::Dimension,
            "Set flange face thickness to 25.4 mm.",
            "25.4 mm",
        ),
        (
            EntityKind::Dimension,
            "The mounting bore measures 3/4 in diameter.",
            "3/4 in",
        ),
        (
            EntityKind::Dimension,
            "Keep the support height at 10 ft.",
            "10 ft",
        ),
        (
            EntityKind::Dimension,
            "Set the roller radius to 0.75 in.",
            "0.75 in",
        ),
        (
            EntityKind::Dimension,
            "The vessel wall is 2.0 mm thick.",
            "2.0 mm",
        ),
        (
            EntityKind::Dimension,
            "Test at 20°C ambient temperature.",
            "20°C",
        ),
        (
            EntityKind::Dimension,
            "The drive applies 24 lbf·in torque.",
            "24 lbf·in",
        ),
        (
            EntityKind::Dimension,
            "Maintain a gap of 1 1/4 inches.",
            "1 1/4 inches",
        ),
        (
            EntityKind::Unit,
            "Pressure is recorded in 40 ksi at proof load.",
            "ksi",
        ),
        (
            EntityKind::Unit,
            "Set flange face thickness to 25.4 mm.",
            "mm",
        ),
        (
            EntityKind::Unit,
            "The mounting bore measures 3/4 in diameter.",
            "in",
        ),
        (EntityKind::Unit, "Keep the support height at 10 ft.", "ft"),
        (EntityKind::Unit, "Set the roller radius to 0.75 in.", "in"),
        (EntityKind::Unit, "The vessel wall is 2.0 mm thick.", "mm"),
        (EntityKind::Unit, "Test at 20°C ambient temperature.", "°C"),
        (
            EntityKind::Unit,
            "The drive applies 24 lbf·in torque.",
            "lbf·in",
        ),
        (
            EntityKind::FastenerCallout,
            "Install five 5/16-24 UNC set screws.",
            "5/16-24 UNC",
        ),
        (
            EntityKind::FastenerCallout,
            "Select an M10x1.5 bolt for the joint.",
            "M10x1.5",
        ),
        (
            EntityKind::FastenerCallout,
            "Use a NAS6206-series pin.",
            "NAS6206",
        ),
        (
            EntityKind::FastenerCallout,
            "Replace the damaged screw with MS24693.",
            "MS24693",
        ),
        (
            EntityKind::FastenerCallout,
            "Choose a 3/8-16 UNC fastener.",
            "3/8-16 UNC",
        ),
        (
            EntityKind::FastenerCallout,
            "Install M4x0.7 screws in the cover.",
            "M4x0.7",
        ),
        (
            EntityKind::FastenerCallout,
            "The bracket uses a 1/2-13 UNC bolt.",
            "1/2-13 UNC",
        ),
        (
            EntityKind::FastenerCallout,
            "Fit an M12x1.75 pin at this location.",
            "M12x1.75",
        ),
        (
            EntityKind::Standard,
            "Qualification follows MIL-STD-810H.",
            "MIL-STD-810H",
        ),
        (
            EntityKind::Standard,
            "The tensile test uses ASTM E8.",
            "ASTM E8",
        ),
        (
            EntityKind::Standard,
            "Set furnace uniformity per AMS 2750.",
            "AMS 2750",
        ),
        (
            EntityKind::Standard,
            "Apply coating under MIL-PRF-23377.",
            "MIL-PRF-23377",
        ),
        (
            EntityKind::Standard,
            "The joint design follows SAE J2343.",
            "SAE J2343",
        ),
        (
            EntityKind::Standard,
            "Use ASME B16.5 for flange dimensions.",
            "ASME B16.5",
        ),
        (
            EntityKind::Standard,
            "Fatigue coupons follow ASTM D3039.",
            "ASTM D3039",
        ),
        (
            EntityKind::Standard,
            "Material heat treatment follows AMS 6415.",
            "AMS 6415",
        ),
    ];
    evaluate("integration", &integration_samples);
    evaluate("held-out wording", &held_out_samples);
}
