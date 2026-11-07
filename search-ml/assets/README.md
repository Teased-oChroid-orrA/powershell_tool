# Search model assets

`all-minilm-l6-v2.onnx` and `all-minilm-l6-v2-tokenizer.json` come from
[`sentence-transformers/all-MiniLM-L6-v2`](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2)
at revision `855bcf54d21334332f75db10bdd48970ff0ef547`.

`flan-t5-small-encoder.onnx`, `flan-t5-small-decoder.onnx`, and
`flan-t5-small-tokenizer.json` are the full-precision ONNX conversion of
[`google/flan-t5-small`](https://huggingface.co/google/flan-t5-small), base
revision `0fc9ddf78a1e988dac52e2dac162b0ede4fd74ab`, from
[`onnx-community/flan-t5-small-ONNX`](https://huggingface.co/onnx-community/flan-t5-small-ONNX)
revision `76988c16f73cadb2c2e13e2d7d85608944223105`. The base model is
Apache-2.0 licensed. The ONNX graphs stay bundled so answer generation works
offline.

The model is Apache-2.0 licensed. See `LICENSE-2.0.txt`. The encoder returns
384-dimensional sentence vectors and truncates inputs beyond 256 WordPiece
tokens. It supports semantic similarity and retrieval; it does not generate
text, validate engineering facts, or replace exact matching.

Flan-T5 generates short answers from locally retrieved, MiniLM-ranked source
lines. Generated answers are not trusted conclusions; the UI shows the source
lines beside each answer for review. The full-precision encoder/decoder add
about 358 MB to the bundled model assets.

SHA-256:
- Model: `6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452`
- Tokenizer: `be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037`
- Flan-T5 encoder: `06c5a1c726c31f4bceafab64f5241a5d4a84c9ac861fa52f600632bdc2934c80`
- Flan-T5 decoder: `14d80238ad1e43e1c275cd3d28207c03901054008438779408534067cbb4344b`
- Flan-T5 tokenizer: `4d4b21a8cc7c0407dafd8ac6215269cd05c8e49a521c3580479b567879526160`
