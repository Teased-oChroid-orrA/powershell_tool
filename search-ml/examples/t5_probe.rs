use rten::{Model, ValueView};
use tokenizers::Tokenizer;

fn main() {
    const MAX_NEW_TOKENS: usize = 32;
    let tokenizer = Tokenizer::from_file("/tmp/flan-t5-small-probe/tokenizer.json").unwrap();
    let prompt = "Answer the question using only this source. If the answer is absent, say so. Source: The shaft experiences torsional stress under applied torque. Question: What creates torsional stress? Answer:";
    let encoded = tokenizer.encode(prompt, true).unwrap();
    let ids: Vec<i32> = encoded.get_ids().iter().map(|&id| i32::try_from(id).unwrap()).collect();
    let mask: Vec<i32> = encoded.get_attention_mask().iter().map(|&id| i32::try_from(id).unwrap()).collect();

    let encoder = Model::load_file("/tmp/flan-t5-small-probe/encoder_model.onnx").unwrap();
    let hidden = encoder.run(
        vec![
            (encoder.node_id("input_ids").unwrap(), ValueView::from_shape([1, ids.len()], &ids).unwrap().into()),
            (encoder.node_id("attention_mask").unwrap(), ValueView::from_shape([1, mask.len()], &mask).unwrap().into()),
        ],
        &[encoder.node_id("last_hidden_state").unwrap()],
        None,
    ).unwrap().remove(0);

    let decoder = Model::load_file("/tmp/flan-t5-small-probe/decoder_model.onnx").unwrap();
    let mut generated = vec![0i32];
    for _ in 0..MAX_NEW_TOKENS {
        let outputs = decoder.run(
            vec![
                (decoder.node_id("input_ids").unwrap(), ValueView::from_shape([1, generated.len()], &generated).unwrap().into()),
                (decoder.node_id("encoder_attention_mask").unwrap(), ValueView::from_shape([1, mask.len()], &mask).unwrap().into()),
                (decoder.node_id("encoder_hidden_states").unwrap(), (&hidden).into()),
            ],
            &[decoder.node_id("logits").unwrap()],
            None,
        ).unwrap();
        let (shape, logits) = outputs.into_iter().next().unwrap().into_shape_vec::<f32, 3>().unwrap();
        let offset = (shape[1] - 1) * shape[2];
        let next = logits[offset..offset + shape[2]].iter().enumerate().max_by(|a,b| a.1.total_cmp(b.1)).unwrap().0 as i32;
        if next == 1 { break; }
        generated.push(next);
    }
    let generated: Vec<u32> = generated.into_iter().skip(1).map(|id| id as u32).collect();
    println!("{}", tokenizer.decode(&generated, true).unwrap());
}
