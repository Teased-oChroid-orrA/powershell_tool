//! Pairwise precision and recall of `neural_search::group_by_similarity` against labelled engineering terms
//! (`examples/data/term_clusters.tsv`: `group<TAB>term`, `#` lines ignored; a group is one idea, `s*` groups are
//! single-term distractors). Each threshold is run over several shuffled input orders (grouping is order dependent).
//!
//!   cargo run -p search-core --release --features neural-search --example term_cluster_eval -- [terms.tsv]
use search_core::neural_search::group_by_similarity;

fn main() {
    let path = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "search-core/examples/data/term_clusters.tsv".into());
    let text = std::fs::read_to_string(&path).expect("terms file");
    let rows: Vec<(String, String)> = text
        .lines()
        .filter(|l| !l.trim().is_empty() && !l.starts_with('#'))
        .filter_map(|l| {
            l.split_once('\t')
                .map(|(g, t)| (g.to_string(), t.to_string()))
        })
        .collect();
    let terms: Vec<&str> = rows.iter().map(|(_, t)| t.as_str()).collect();
    let embeddings = search_ml::embed_batch(&terms).expect("embed");
    let truth = |a: usize, b: usize| rows[a].0 == rows[b].0;
    let total_true: usize = (0..rows.len())
        .map(|a| (a + 1..rows.len()).filter(|&b| truth(a, b)).count())
        .sum();
    println!(
        "{} terms, {} true pairs, 20 shuffled orders per threshold",
        rows.len(),
        total_true
    );
    println!("threshold  precision  recall  false-merges  groups");
    let mut seed = 0x9E3779B97F4A7C15u64;
    let orders: Vec<Vec<usize>> = (0..20)
        .map(|_| {
            let mut order: Vec<usize> = (0..rows.len()).collect();
            for i in (1..order.len()).rev() {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                order.swap(i, (seed % (i as u64 + 1)) as usize);
            }
            order
        })
        .collect();
    for step in 6..=18 {
        let threshold = step as f32 * 0.05;
        let (mut tp, mut fp, mut groups) = (0usize, 0usize, 0usize);
        for order in &orders {
            let shuffled: Vec<_> = order.iter().map(|&i| embeddings[i].clone()).collect();
            let found = group_by_similarity(&shuffled, threshold);
            groups += found.len();
            for members in &found {
                for (x, &m) in members.iter().enumerate() {
                    for &n in &members[x + 1..] {
                        if truth(order[m], order[n]) {
                            tp += 1;
                        } else {
                            fp += 1;
                        }
                    }
                }
            }
        }
        let runs = orders.len() as f32;
        let precision = if tp + fp == 0 {
            1.0
        } else {
            tp as f32 / (tp + fp) as f32
        };
        println!(
            "{threshold:>8.2}  {precision:>9.3}  {:>6.3}  {:>12.1}  {:>6.1}",
            tp as f32 / runs / total_true as f32,
            fp as f32 / runs,
            groups as f32 / runs
        );
    }
}
