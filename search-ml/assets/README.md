# Search model assets

`all-minilm-l6-v2.onnx` and `all-minilm-l6-v2-tokenizer.json` come from
[`sentence-transformers/all-MiniLM-L6-v2`](https://huggingface.co/sentence-transformers/all-MiniLM-L6-v2)
at revision `855bcf54d21334332f75db10bdd48970ff0ef547`.

The model is Apache-2.0 licensed. See `LICENSE-2.0.txt`. The encoder returns
384-dimensional sentence vectors and truncates inputs beyond 256 WordPiece
tokens. It supports semantic similarity and retrieval; it does not generate
text, validate engineering facts, or replace exact matching.

Search shows cited source lines only; the bundled assets are the embedding model and nothing generates text. The
Flan-T5-small answer generator that used to live here (358 MB) was removed on 2026-11-08: it dropped parts of two-part
questions and invented answers to unsupported ones (see `.audit/llm-condition-bakeoff-2026-11-07.md`).

SHA-256:
- Model: `6fd5d72fe4589f189f8ebc006442dbb529bb7ce38f8082112682524616046452`
- Tokenizer: `be50c3628f2bf5bb5e3a7f17b1f74611b2561a3a27eeab05e5aa30f411572037`
