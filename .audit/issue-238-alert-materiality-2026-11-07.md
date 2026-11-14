# Saved-alert materiality probe

Goal: determine whether bundled MiniLM cosine distance separates material engineering edits from review-only edits. A separating result would justify a candidate threshold for later held-out validation. A single hand-authored sample cannot justify a production threshold.

Command: `cargo run -p search-ml --example alert_materiality_probe --release`

One run on eight manually labeled material pairs and eight immaterial pairs produced distances 0.0162–0.3310 and 0.0028–0.0218, respectively. The ranges overlap: minimum material distance 0.0162; maximum immaterial distance 0.0218. A clearance change from 0.001 in to 0.010 in scored 0.0162, while a one-day inspection-date change scored 0.0028.

Bundled Flan-T5-small also failed as a materiality classifier. Its prompt probe predicted `IMMATERIAL` for all 16 pairs (8/16 accuracy, equal to the majority baseline).

Conclusion: neither bundled model supports a materiality verdict or threshold. Keep alerts exact and show cosine as an uncalibrated review signal. Numeric engineering changes remain a failure case for sentence embeddings. This probe is illustrative, manually labeled, and single-run; it does not estimate corpus performance.
