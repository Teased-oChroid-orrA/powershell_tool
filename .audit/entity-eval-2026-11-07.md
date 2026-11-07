# MiniLM entity ranking evaluation

## Dataset and method

`cargo run -p search-core --features neural-search --example entity_rank_probe` evaluates six entity categories on 12 integration spans and 59 independently worded held-out spans. Held-out values and sentences do not repeat the MiniLM prototype anchors. Each example has one manually labeled span; evaluation measures type ranking only after the exact pattern extractor found that span. It does not measure document-level false positives or recall outside the bounded candidate set.

The benchmark compares unrestricted MiniLM top-one/top-two, the first sorted pattern hint, all pattern hints, and MiniLM top rank restricted to pattern-supported categories. The production search behavior remains exact: rules determine candidate spans and allowed categories. MiniLM only ranks these categories for the review suggestion; raw six-class cosine scores remain available for diagnostics.

## Results

| Split | Spans | MiniLM top-one | MiniLM top-two | First pattern hint | Pattern coverage | MiniLM within pattern hints |
|---|---:|---:|---:|---:|---:|---:|
| Integration | 12 | 9/12 | 11/12 | 9/12 | 12/12 | 9/12 |
| Held-out wording | 59 | 41/59 | 52/59 | 54/59 | 59/59 | 56/59 |

The pattern extractor emits one hint for 52/59 held-out spans. Constrained MiniLM improves over the first-hint baseline by 2/59 examples (3.4 percentage points). Unrestricted top-one is 13/59 below the first-hint baseline, showing that raw category similarity is not suitable as an unconstrained label.

## Decision and limits

Search displays the highest-scoring neural category only among categories found by exact patterns. This improves the held-out ranking result without changing span membership or exact search. Similarity remains a ranking signal, not a calibrated probability.

The manually labeled held-out set is modest and evaluates known candidate spans, not corpus-level precision/recall. Keep the feature as a review aid. Broader corpus evaluation remains open before using it to make automatic engineering decisions.
