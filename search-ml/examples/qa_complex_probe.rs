use search_ml::generate_answer;

fn main() {
    let cases: [(&str, &[&str]); 4] = [
        (
            "What causes torsional stress and what limits the shaft temperature?",
            &[
                "Applied torque causes torsional stress in the shaft.",
                "The shaft temperature limit is 120 C.",
            ],
        ),
        (
            "What causes torsional stress?",
            &["Applied torque causes torsional stress in the shaft."],
        ),
        (
            "What limits the shaft temperature?",
            &["The shaft temperature limit is 120 C."],
        ),
        (
            "What is the shaft's fatigue life?",
            &["Applied torque causes torsional stress in the shaft."],
        ),
    ];
    for (question, context) in cases {
        println!("{question}: {:?}", generate_answer(question, context));
    }
}
