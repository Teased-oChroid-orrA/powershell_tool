use search_ml::{cosine_similarity, embed_batch};

fn main() {
    let pairs = [
        (true, "Fastener installation torque is 8 N·m.", "Fastener installation torque is 80 N·m."),
        (true, "The component is aluminum 6061-T6.", "The component is aluminum 2024-T3."),
        (true, "Maximum service pressure is 250 psi.", "Maximum service pressure is 75 psi."),
        (true, "The bushing diametral clearance is 0.001 in.", "The bushing diametral clearance is 0.010 in."),
        (true, "Inspection found no cracks in the fitting.", "Inspection found a crack in the fitting; remove from service."),
        (true, "The bracket supports a 12 kN limit load.", "The bracket supports a 120 kN limit load."),
        (true, "Use five bolts to attach the flange.", "Use four bolts to attach the flange."),
        (true, "Minimum fatigue life is 20,000 cycles.", "Minimum fatigue life is 2,000 cycles."),
        (false, "The fitting was inspected on 2026-10-02.", "The fitting was inspected on 2026-10-03."),
        (false, "The bracket supports a 12 kN limit load.", "The bracket supports a 12 kN limit load!"),
        (false, "Material is aluminum alloy 6061-T6.", "The material is aluminum alloy 6061-T6."),
        (false, "Pressure: 250 psi; temperature: 80 C.", "Temperature: 80 C; pressure: 250 psi."),
        (false, "Inspect the upper flange for fatigue cracks.", "Inspect upper flange for fatigue cracks."),
        (false, "Use five bolts to attach the flange.", "Use 5 bolts to attach the flange."),
        (false, "Report prepared by J. Smith.", "Report prepared by J Smith."),
        (false, "The fitting is made from Ti-6Al-4V.", "The fitting is made from Ti 6Al 4V."),
    ];
    let texts = pairs.iter().flat_map(|(_, before, after)| [*before, *after]).collect::<Vec<_>>();
    let embeddings = embed_batch(&texts).expect("bundled local MiniLM");
    let mut material = Vec::new();
    let mut immaterial = Vec::new();
    for (i, (label, _, _)) in pairs.iter().enumerate() {
        let similarity = cosine_similarity(&embeddings[i * 2], &embeddings[i * 2 + 1]).unwrap();
        let distance = 1.0 - similarity;
        if *label { material.push(distance); } else { immaterial.push(distance); }
        println!("{} {:.4}", if *label { "material" } else { "immaterial" }, distance);
    }
    let min_material = material.iter().copied().fold(f32::INFINITY, f32::min);
    let max_immaterial = immaterial.iter().copied().fold(f32::NEG_INFINITY, f32::max);
    println!("material_min={min_material:.4} immaterial_max={max_immaterial:.4} separable={}", min_material > max_immaterial);
}
