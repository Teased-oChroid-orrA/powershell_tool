//! Measures approximate nearest-neighbour search against the exact scan the semantic snapshot does, on a large
//! synthetic corpus built from real chunk embeddings (see `docs/search-benchmarks.md`).
//!
//!   cargo run -p search-core --release --example ann_probe -- <total chunks> <semantic.sqlite>...
//!
//! Corpus: the stored vectors of the given indexes are the seeds. Each synthetic chunk is a random mix of two seeds
//! (weight 0.3 to 0.7) plus Gaussian noise, normalised. Real vectors are never copied verbatim, so the nearest
//! neighbours of a held-out seed are not planted. `ANN_CLUSTERED=1` instead makes each synthetic chunk one seed plus
//! noise of norm about 0.8 (tight topical clusters, the easy case for IVF); `ANN_LISTS=<n>` fixes the list count. Queries are 200 held-out real seeds (not in the corpus).
//! Compared: exact f32 scan (1 thread and all cores), exact int8 scan, and IVF (spherical k-means centroids, probe the
//! `nprobe` nearest lists). Recall is recall@100 against the exact f32 top 100.
use std::time::Instant;

const D: usize = 384;

struct Rng(u64);
impl Rng {
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 << 13;
        self.0 ^= self.0 >> 7;
        self.0 ^= self.0 << 17;
        self.0
    }
    fn unit(&mut self) -> f32 {
        (self.next() >> 40) as f32 / (1u64 << 24) as f32
    }
    fn gauss(&mut self) -> f32 {
        let (a, b) = (self.unit().max(1e-7), self.unit());
        (-2.0 * a.ln()).sqrt() * (std::f32::consts::TAU * b).cos()
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n as u64) as usize
    }
}

/// Eight independent lanes, as `semantic_cache::dot` does, so the compiler vectorises it.
fn dot(a: &[f32], b: &[f32]) -> f32 {
    let mut lanes = [0f32; 8];
    for (x, y) in a.chunks_exact(8).zip(b.chunks_exact(8)) {
        for i in 0..8 {
            lanes[i] += x[i] * y[i];
        }
    }
    lanes.iter().sum()
}

fn normalize(v: &mut [f32]) {
    let n = dot(v, v).sqrt().max(1e-12);
    v.iter_mut().for_each(|x| *x /= n);
}

fn load(paths: &[String]) -> Vec<f32> {
    let mut all = Vec::new();
    for path in paths {
        let conn = rusqlite::Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).expect("open");
        let mut stmt = conn.prepare("SELECT embedding FROM semantic_chunks").expect("prepare");
        let rows = stmt.query_map([], |r| r.get::<_, Vec<u8>>(0)).expect("query");
        for bytes in rows.flatten() {
            if bytes.len() == D * 4 {
                all.extend(bytes.chunks_exact(4).map(|c| f32::from_le_bytes(c.try_into().unwrap())));
            }
        }
    }
    all
}

/// Top `k` (index, score) by score, descending.
fn top_k(scores: impl Iterator<Item = (u32, f32)>, k: usize) -> Vec<(u32, f32)> {
    let mut best: Vec<(u32, f32)> = Vec::with_capacity(k + 1);
    let mut floor = f32::NEG_INFINITY;
    for (i, s) in scores {
        if s > floor || best.len() < k {
            best.push((i, s));
            if best.len() > k {
                let worst = best.iter().enumerate().min_by(|a, b| a.1 .1.total_cmp(&b.1 .1)).unwrap().0;
                best.swap_remove(worst);
                floor = best.iter().map(|b| b.1).fold(f32::INFINITY, f32::min);
            }
        }
    }
    best.sort_by(|a, b| b.1.total_cmp(&a.1));
    best
}

fn exact_parallel(corpus: &[f32], q: &[f32], k: usize, threads: usize) -> Vec<(u32, f32)> {
    let n = corpus.len() / D;
    let part = n.div_ceil(threads);
    let merged: Vec<(u32, f32)> = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..threads)
            .map(|t| {
                scope.spawn(move || {
                    let (lo, hi) = (t * part, ((t + 1) * part).min(n));
                    top_k((lo..hi).map(|i| (i as u32, dot(q, &corpus[i * D..(i + 1) * D]))), k)
                })
            })
            .collect();
        handles.into_iter().flat_map(|h| h.join().unwrap()).collect()
    });
    top_k(merged.into_iter(), k)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let total: usize = args[1].parse().expect("total chunks");
    let seeds = load(&args[2..]);
    let seed_count = seeds.len() / D;
    eprintln!("{seed_count} real vectors");
    let clustered = std::env::var("ANN_CLUSTERED").is_ok_and(|v| v == "1");
    let list_counts: Vec<usize> = match std::env::var("ANN_LISTS").ok().and_then(|v| v.parse().ok()) {
        Some(n) => vec![n],
        None => vec![(total as f64).sqrt() as usize, 2 * (total as f64).sqrt() as usize],
    };
    let mut rng = Rng(0x9E3779B97F4A7C15);
    // Hold out 200 seeds as queries.
    let mut held: Vec<usize> = Vec::new();
    while held.len() < 200 {
        let i = rng.below(seed_count);
        if !held.contains(&i) {
            held.push(i);
        }
    }
    let pool: Vec<usize> = (0..seed_count).filter(|i| !held.contains(i)).collect();
    let mut corpus = vec![0f32; total * D];
    for row in corpus.chunks_exact_mut(D) {
        let (a, b) = (pool[rng.below(pool.len())], pool[rng.below(pool.len())]);
        let w = if clustered { 1.0 } else { 0.3 + 0.4 * rng.unit() };
        let noise = if clustered { 0.04 } else { 0.02 };
        for j in 0..D {
            row[j] = w * seeds[a * D + j] + (1.0 - w) * seeds[b * D + j] + noise * rng.gauss();
        }
        normalize(row);
    }
    let queries: Vec<&[f32]> = held.iter().map(|&i| &seeds[i * D..(i + 1) * D]).collect();
    let threads = std::thread::available_parallelism().map_or(1, |n| n.get()).min(8);

    // Exact f32.
    let t = Instant::now();
    let truth: Vec<Vec<(u32, f32)>> = queries.iter().map(|q| exact_parallel(&corpus, q, 100, 1)).collect();
    let exact1 = t.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64;
    let t = Instant::now();
    for q in &queries {
        exact_parallel(&corpus, q, 100, threads);
    }
    let exact_n = t.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64;
    println!("N={total}: exact f32 scan {exact1:.1} ms/query (1 thread), {exact_n:.1} ms ({threads} threads), {} MB", total * D * 4 / 1_000_000);

    let recall = |found: &[(u32, f32)], truth: &[(u32, f32)]| {
        truth.iter().filter(|t| found.iter().any(|f| f.0 == t.0)).count() as f64 / truth.len() as f64
    };

    // Exact int8 (per-vector scale), scored as a dot product against the f32 query.
    let t = Instant::now();
    let mut q8 = vec![0i8; total * D];
    let mut scale = vec![0f32; total];
    for (i, row) in corpus.chunks_exact(D).enumerate() {
        let m = row.iter().fold(0f32, |m, x| m.max(x.abs())).max(1e-12);
        scale[i] = m / 127.0;
        for (o, x) in q8[i * D..(i + 1) * D].iter_mut().zip(row) {
            *o = (x / scale[i]).round() as i8;
        }
    }
    let quant_ms = t.elapsed().as_secs_f64() * 1000.0;
    let (mut rec, t) = (0.0, Instant::now());
    for (q, tr) in queries.iter().zip(&truth) {
        let found = top_k(
            (0..total).map(|i| (i as u32, scale[i] * q8[i * D..(i + 1) * D].iter().zip(*q).map(|(a, b)| *a as f32 * b).sum::<f32>())),
            100,
        );
        rec += recall(&found, tr);
    }
    println!(
        "int8 scan (1 thread): recall@100 {:.4}, {:.1} ms/query, {} MB (quantise {:.0} ms)",
        rec / queries.len() as f64,
        t.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64,
        (total * D + total * 4) / 1_000_000,
        quant_ms
    );

    // IVF: spherical k-means on a sample, then assign every vector to its nearest centroid.
    for lists in list_counts {
        let t = Instant::now();
        let sample: Vec<usize> = (0..(lists * 40).min(total)).map(|_| rng.below(total)).collect();
        let mut centroids: Vec<f32> = sample[..lists].iter().flat_map(|&i| corpus[i * D..(i + 1) * D].to_vec()).collect();
        for _ in 0..8 {
            let mut sums = vec![0f32; lists * D];
            let mut counts = vec![0usize; lists];
            for &i in &sample {
                let row = &corpus[i * D..(i + 1) * D];
                let c = (0..lists).max_by(|&a, &b| dot(row, &centroids[a * D..(a + 1) * D]).total_cmp(&dot(row, &centroids[b * D..(b + 1) * D]))).unwrap();
                counts[c] += 1;
                sums[c * D..(c + 1) * D].iter_mut().zip(row).for_each(|(s, x)| *s += x);
            }
            for c in 0..lists {
                if counts[c] > 0 {
                    normalize(&mut sums[c * D..(c + 1) * D]);
                    centroids[c * D..(c + 1) * D].copy_from_slice(&sums[c * D..(c + 1) * D]);
                }
            }
        }
        let mut members: Vec<Vec<u32>> = vec![Vec::new(); lists];
        for i in 0..total {
            let row = &corpus[i * D..(i + 1) * D];
            let c = (0..lists).max_by(|&a, &b| dot(row, &centroids[a * D..(a + 1) * D]).total_cmp(&dot(row, &centroids[b * D..(b + 1) * D]))).unwrap();
            members[c].push(i as u32);
        }
        println!("IVF lists={lists}: build {:.1} s", t.elapsed().as_secs_f64());
        for nprobe in [4, 8, 16, 32, 64, 128, 256] {
            if nprobe > lists {
                continue;
            }
            let (mut rec, mut scanned, t) = (0.0, 0usize, Instant::now());
            for (q, tr) in queries.iter().zip(&truth) {
                let near = top_k((0..lists).map(|c| (c as u32, dot(q, &centroids[c * D..(c + 1) * D]))), nprobe);
                let found = top_k(
                    near.iter().flat_map(|&(c, _)| members[c as usize].iter()).map(|&i| {
                        scanned += 1;
                        (i, dot(q, &corpus[i as usize * D..(i as usize + 1) * D]))
                    }),
                    100,
                );
                rec += recall(&found, tr);
            }
            println!(
                "  nprobe={nprobe:<4} recall@100 {:.4}  {:.1} ms/query  scans {:.1}% of the corpus",
                rec / queries.len() as f64,
                t.elapsed().as_secs_f64() * 1000.0 / queries.len() as f64,
                100.0 * scanned as f64 / (queries.len() * total) as f64
            );
        }
    }
}
