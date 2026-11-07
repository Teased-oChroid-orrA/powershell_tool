use search_ml::{embed_batch, embed_tokens, embed_tokens_batch};

fn main() {
    let text = "µ material: Ti-6Al-4V; bore diameter 0.25 in; ASTM A36.";
    let tokens = embed_tokens(text).expect("bundled token inference");
    assert!(!tokens.is_empty());
    assert!(tokens.iter().all(|token| {
        text.get(token.byte_range.clone()).is_some() && token.embedding.values().len() == 384
    }));
    let long = "engineering ".repeat(300);
    assert!(
        embed_tokens(&long).is_err(),
        "over-limit input must not truncate silently"
    );
    let batch = embed_tokens_batch(&[text, "Use an M6x1 screw."]).expect("batched token inference");
    assert_eq!(batch.len(), 2);
    assert!(batch
        .iter()
        .flatten()
        .all(|token| token.embedding.values().len() == 384));
    let sentence = embed_batch(&[text]).expect("sentence embedding");
    assert_eq!(sentence.len(), 1);
    assert!(
        (sentence[0]
            .values()
            .iter()
            .map(|value| value * value)
            .sum::<f32>()
            .sqrt()
            - 1.0)
            .abs()
            < 1e-4
    );
    println!("{} contextual token vectors; UTF-8 offsets valid; over-limit input rejected; sentence embedding remains normalized", tokens.len());
}
