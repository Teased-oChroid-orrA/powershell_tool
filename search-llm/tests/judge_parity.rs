//! Real-model judge check against the shared eval cases (same data as `.audit/llm_bakeoff.py`).
//! Skipped unless `SEARCH_LLM_GGUF` names a GGUF file. Needs `--features llama`.
#![cfg(feature = "llama")]

use search_llm::{ConditionModel, LlamaEngine};

#[test]
fn judge_matches_bakeoff_accuracy() {
    let Some(path) = std::env::var_os("SEARCH_LLM_GGUF") else {
        eprintln!("SEARCH_LLM_GGUF not set; skipping real-model judge parity");
        return;
    };
    let data = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../search-ml/examples/data/condition_judge_cases.json"
    );
    let cases: Vec<serde_json::Value> =
        serde_json::from_str(&std::fs::read_to_string(data).unwrap()).unwrap();
    let engine = LlamaEngine::load(std::path::Path::new(&path)).unwrap();
    let (mut right, mut false_pos) = (0usize, 0usize);
    for case in &cases {
        let gold = case["gold"].as_bool().unwrap();
        let started = std::time::Instant::now();
        let verdict = engine
            .judge(
                case["cond"].as_str().unwrap(),
                case["passage"].as_str().unwrap(),
            )
            .unwrap();
        eprintln!(
            "{:?} gold={gold} got={} in {:?}",
            case["cond"],
            verdict.matches,
            started.elapsed()
        );
        right += (verdict.matches == gold) as usize;
        false_pos += (verdict.matches && !gold) as usize;
    }
    let accuracy = right as f32 / cases.len() as f32;
    eprintln!("judge accuracy {accuracy:.3}, false positives {false_pos}");
    // Python bake-off: 0.951 with 0 false positives on Qwen3-4B-Instruct-2507 Q4_K_M.
    assert!(
        accuracy >= 0.93 && false_pos == 0,
        "accuracy {accuracy}, false positives {false_pos}"
    );
}
