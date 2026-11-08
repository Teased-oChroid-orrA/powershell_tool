//! cargo run -p search-llm --features llama --release --example parse_probe -- model.gguf "sentence"
use search_llm::{ConditionModel, LlamaEngine};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let t = std::time::Instant::now();
    let engine = LlamaEngine::load(std::path::Path::new(&args[1])).expect("load");
    println!("load {:?}", t.elapsed());
    let t = std::time::Instant::now();
    let conditions = engine.parse(&args[2]).expect("parse");
    println!("parse {:?}", t.elapsed());
    for c in &conditions {
        println!("{:?} review={}", c, c.needs_review());
    }
    if let (Some(cond), Some(passage)) = (args.get(3), args.get(4)) {
        let t = std::time::Instant::now();
        println!(
            "judge {:?} in {:?}",
            engine.judge(cond, passage),
            t.elapsed()
        );
    }
}
