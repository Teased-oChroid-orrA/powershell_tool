//! Does the recogniser's own probability predict its errors? Runs the bundled OCR over the labelled scan set written by
//! `scripts/make_ocr_confidence_set.py`, matches every recognised line to a truth line by position, and reports how well
//! each line score separates well-read lines from misread ones, per condition and as threshold tables.
//!
//!   python3 scripts/make_ocr_confidence_set.py /tmp/ocrset
//!   cargo run -p search-core --release --features ocr --example ocr_confidence_eval -- /tmp/ocrset [--lines lines.tsv]
//!
//! Scores per line (all from the per-character probabilities `ocr::OcrLine::char_confidence`): `mean`, `min`,
//! `geo` (geometric mean) and `low` (share of characters under 0.9, a doubt score). The error is the character error
//! rate of the line against its truth text (whitespace collapsed). A truth line with no recognised line counts as a
//! miss (no score); a recognised line matching no truth line is spurious.
use std::collections::BTreeMap;
use std::path::Path;

struct Truth {
    x0: f32,
    y0: f32,
    x1: f32,
    y1: f32,
    height: f32,
    text: String,
}

struct PageAll {
    condition: String,
    p_sum: f32,
    chars: usize,
    edits: usize,
    truth_chars: usize,
    lines: usize,
    low_lines: usize,
}

struct Scored {
    condition: String,
    page: usize,
    truth_len: usize,
    edits: usize,
    err: f32,
    mean: f32,
    min: f32,
    geo: f32,
    low: f32,
}

fn read_pgm(path: &Path) -> (u32, u32, Vec<u8>) {
    let bytes = std::fs::read(path).unwrap();
    let mut fields = Vec::new();
    let mut at = 0;
    while fields.len() < 4 {
        while bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        let start = at;
        while !bytes[at].is_ascii_whitespace() {
            at += 1;
        }
        fields.push(std::str::from_utf8(&bytes[start..at]).unwrap().to_string());
    }
    assert_eq!(fields[0], "P5");
    (fields[1].parse().unwrap(), fields[2].parse().unwrap(), bytes[at + 1..].to_vec())
}

fn read_truth(path: &Path) -> Vec<Truth> {
    std::fs::read_to_string(path)
        .unwrap()
        .lines()
        .map(|l| {
            let f: Vec<&str> = l.splitn(6, '\t').collect();
            Truth { x0: f[0].parse().unwrap(), y0: f[1].parse().unwrap(), x1: f[2].parse().unwrap(), y1: f[3].parse().unwrap(), height: f[4].parse().unwrap(), text: f[5].to_string() }
        })
        .collect()
}

fn collapse(s: &str) -> Vec<char> {
    s.split_whitespace().collect::<Vec<_>>().join(" ").chars().collect()
}

fn levenshtein(a: &[char], b: &[char]) -> usize {
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.iter().enumerate() {
        let mut cur = vec![i + 1];
        for (j, cb) in b.iter().enumerate() {
            cur.push((prev[j] + usize::from(ca != cb)).min(prev[j + 1] + 1).min(cur[j] + 1));
        }
        prev = cur;
    }
    prev[b.len()]
}

/// Truth line whose (possibly tilted) centre line passes closest to the point, if within a line height.
fn nearest_truth(truth: &[Truth], cx: f32, cy: f32) -> Option<usize> {
    let mut best: Option<(usize, f32)> = None;
    for (i, t) in truth.iter().enumerate() {
        let slack = t.height * 0.5;
        if cx < t.x0 - slack || cx > t.x1 + slack {
            continue;
        }
        let along = if (t.x1 - t.x0).abs() < 1.0 { 0.0 } else { ((cx - t.x0) / (t.x1 - t.x0)).clamp(0.0, 1.0) };
        let line_y = t.y0 + (t.y1 - t.y0) * along;
        let d = (cy - line_y).abs();
        if d < t.height * 0.9 && best.is_none_or(|(_, bd)| d < bd) {
            best = Some((i, d));
        }
    }
    best.map(|(i, _)| i)
}

fn ranks(values: &[f32]) -> Vec<f32> {
    let mut order: Vec<usize> = (0..values.len()).collect();
    order.sort_by(|&a, &b| values[a].total_cmp(&values[b]));
    let mut out = vec![0.0; values.len()];
    let mut i = 0;
    while i < order.len() {
        let mut j = i;
        while j + 1 < order.len() && values[order[j + 1]] == values[order[i]] {
            j += 1;
        }
        let rank = (i + j) as f32 / 2.0 + 1.0;
        for &k in &order[i..=j] {
            out[k] = rank;
        }
        i = j + 1;
    }
    out
}

fn spearman(x: &[f32], y: &[f32]) -> f32 {
    let (rx, ry) = (ranks(x), ranks(y));
    let n = x.len() as f32;
    let (mx, my) = (rx.iter().sum::<f32>() / n, ry.iter().sum::<f32>() / n);
    let cov: f32 = rx.iter().zip(&ry).map(|(a, b)| (a - mx) * (b - my)).sum();
    let (vx, vy): (f32, f32) = (rx.iter().map(|a| (a - mx).powi(2)).sum(), ry.iter().map(|b| (b - my).powi(2)).sum());
    cov / (vx * vy).sqrt()
}

/// Probability that a randomly chosen bad line has a higher doubt score than a randomly chosen good one.
fn auc(doubt: &[f32], bad: &[bool]) -> f32 {
    let r = ranks(doubt);
    let n_bad = bad.iter().filter(|b| **b).count() as f32;
    let n_good = bad.len() as f32 - n_bad;
    if n_bad == 0.0 || n_good == 0.0 {
        return f32::NAN;
    }
    let sum_bad: f32 = r.iter().zip(bad).filter(|(_, b)| **b).map(|(r, _)| r).sum();
    (sum_bad - n_bad * (n_bad + 1.0) / 2.0) / (n_bad * n_good)
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let dir = Path::new(args.get(1).expect("folder written by make_ocr_confidence_set.py"));
    let lines_out = args.iter().position(|a| a == "--lines").and_then(|i| args.get(i + 1));
    let manifest = std::fs::read_to_string(dir.join("manifest.tsv")).unwrap();
    let mut scored = Vec::<Scored>::new();
    let (mut missed, mut spurious, mut truth_total) = (BTreeMap::<String, usize>::new(), BTreeMap::<String, usize>::new(), BTreeMap::<String, usize>::new());
    let mut order: Vec<String> = Vec::new();
    let mut recognised_total = 0usize;
    let mut spurious_p: Vec<f32> = Vec::new();
    // Per page, over everything the page yielded: probability sum and characters of every recognised line (matched or
    // spurious), edits against the truth (a missed truth line costs its length, a spurious line its length), truth
    // characters and the count of recognised lines whose mean probability is under 0.9.
    let mut page_all: Vec<PageAll> = Vec::new();
    let started = std::time::Instant::now();
    for (page, row) in manifest.lines().skip(1).enumerate() {
        let stem = row.split('\t').next().unwrap();
        let condition = row.split('\t').nth(1).unwrap().to_string();
        if !order.contains(&condition) {
            order.push(condition.clone());
        }
        let (w, h, gray) = read_pgm(&dir.join(format!("{stem}.pgm")));
        let truth = read_truth(&dir.join(format!("{stem}.tsv")));
        let lines = search_core::ocr::ocr_gray_lines(w, h, &gray).expect("ocr");
        recognised_total += lines.len();
        *truth_total.entry(condition.clone()).or_default() += truth.len();
        // Recognised pieces per truth line, kept in reading order along the line.
        let mut all = PageAll { condition: condition.clone(), p_sum: 0.0, chars: 0, edits: 0, truth_chars: truth.iter().map(|t| collapse(&t.text).len()).sum(), lines: lines.len(), low_lines: 0 };
        for l in &lines {
            all.p_sum += l.char_confidence.iter().sum::<f32>();
            all.chars += l.char_confidence.len();
            all.low_lines += usize::from(l.mean_confidence() < 0.9);
        }
        let mut pieces: Vec<Vec<(f32, usize)>> = vec![Vec::new(); truth.len()];
        for (li, l) in lines.iter().enumerate() {
            match nearest_truth(&truth, (l.x0 + l.x1) / 2.0, (l.y0 + l.y1) / 2.0) {
                Some(t) => pieces[t].push(((l.x0 + l.x1) / 2.0, li)),
                None => {
                    *spurious.entry(condition.clone()).or_default() += 1;
                    all.edits += collapse(&l.text).len();
                    spurious_p.push(l.mean_confidence());
                }
            }
        }
        for (t, mut found) in pieces.into_iter().enumerate() {
            if found.is_empty() {
                *missed.entry(condition.clone()).or_default() += 1;
                all.edits += collapse(&truth[t].text).len();
                continue;
            }
            found.sort_by(|a, b| a.0.total_cmp(&b.0));
            let text = found.iter().map(|(_, i)| lines[*i].text.as_str()).collect::<Vec<_>>().join(" ");
            let conf: Vec<f32> = found.iter().flat_map(|(_, i)| lines[*i].char_confidence.iter().copied()).collect();
            let (rec, tru) = (collapse(&text), collapse(&truth[t].text));
            let n = conf.len().max(1) as f32;
            let edits = levenshtein(&rec, &tru);
            all.edits += edits;
            scored.push(Scored {
                condition: condition.clone(),
                page,
                truth_len: tru.len(),
                edits,
                err: edits as f32 / tru.len().max(1) as f32,
                mean: conf.iter().sum::<f32>() / n,
                min: conf.iter().copied().fold(1.0, f32::min),
                geo: (conf.iter().map(|p| p.max(1e-6).ln()).sum::<f32>() / n).exp(),
                low: conf.iter().filter(|p| **p < 0.9).count() as f32 / n,
            });
        }
        page_all.push(all);
    }
    println!("{} pages, {} truth lines, {} recognised lines, {} scored, in {:.0} s\n", manifest.lines().count() - 1, truth_total.values().sum::<usize>(), recognised_total, scored.len(), started.elapsed().as_secs_f64());

    println!("condition\ttruth\tmissed\tspurious\tmean CER\tmean p\tlines CER>0.1");
    for c in &order {
        let rows: Vec<&Scored> = scored.iter().filter(|s| &s.condition == c).collect();
        let n = rows.len().max(1) as f32;
        println!(
            "{c}\t{}\t{}\t{}\t{:.4}\t{:.3}\t{}",
            truth_total[c],
            missed.get(c).copied().unwrap_or(0),
            spurious.get(c).copied().unwrap_or(0),
            rows.iter().map(|s| s.err).sum::<f32>() / n,
            rows.iter().map(|s| s.mean).sum::<f32>() / n,
            rows.iter().filter(|s| s.err > 0.1).count()
        );
    }

    let errs: Vec<f32> = scored.iter().map(|s| s.err).collect();
    println!("\nscore\tSpearman(doubt, CER)\tAUC CER>0.02\tAUC CER>0.10\tAUC CER>0.25");
    let doubts: [(&str, Vec<f32>); 4] = [
        ("1-mean", scored.iter().map(|s| 1.0 - s.mean).collect()),
        ("1-min", scored.iter().map(|s| 1.0 - s.min).collect()),
        ("1-geo", scored.iter().map(|s| 1.0 - s.geo).collect()),
        ("low<0.9", scored.iter().map(|s| s.low).collect()),
    ];
    for (name, d) in &doubts {
        let bad = |limit: f32| auc(d, &errs.iter().map(|e| *e > limit).collect::<Vec<_>>());
        println!("{name}\t{:.3}\t{:.3}\t{:.3}\t{:.3}", spearman(d, &errs), bad(0.02), bad(0.10), bad(0.25));
    }

    // Threshold table on the mean score: lines below the threshold are flagged as doubtful.
    let total_bad = scored.iter().filter(|s| s.err > 0.10).count().max(1);
    println!("\nthreshold on line mean p\tflagged\tprecision (CER>0.10)\trecall of CER>0.10\tCER of unflagged lines");
    for t in [0.80, 0.85, 0.90, 0.93, 0.95, 0.97, 0.98, 0.99] {
        let flagged: Vec<&Scored> = scored.iter().filter(|s| s.mean < t).collect();
        let rest: Vec<&Scored> = scored.iter().filter(|s| s.mean >= t).collect();
        let hit = flagged.iter().filter(|s| s.err > 0.10).count();
        println!(
            "{t:.2}\t{:.1}%\t{:.1}%\t{:.1}%\t{:.4}",
            100.0 * flagged.len() as f32 / scored.len().max(1) as f32,
            100.0 * hit as f32 / flagged.len().max(1) as f32,
            100.0 * hit as f32 / total_bad as f32,
            rest.iter().map(|s| s.err).sum::<f32>() / rest.len().max(1) as f32
        );
    }

    // Page level: a page's character error rate (edits over truth characters) against its mean doubt (1 - mean p, weighted
    // by truth characters).
    let mut pages: BTreeMap<usize, (usize, usize, f32)> = BTreeMap::new();
    for s in &scored {
        let e = pages.entry(s.page).or_default();
        e.0 += s.edits;
        e.1 += s.truth_len;
        e.2 += (1.0 - s.mean) * s.truth_len as f32;
    }
    let (page_err, page_doubt): (Vec<f32>, Vec<f32>) = pages.values().map(|(edits, len, doubt)| (*edits as f32 / (*len).max(1) as f32, doubt / (*len).max(1) as f32)).unzip();
    println!("\npages: Spearman(page doubt, page CER) = {:.3} over {} pages", spearman(&page_doubt, &page_err), pages.len());

    if !spurious_p.is_empty() {
        let n = spurious_p.len() as f32;
        println!(
            "\nspurious lines (recognised, no truth line nearby): {} with mean line p {:.3}; {:.1}% under 0.9",
            spurious_p.len(),
            spurious_p.iter().sum::<f32>() / n,
            100.0 * spurious_p.iter().filter(|p| **p < 0.9).count() as f32 / n
        );
    }

    // Page level over everything the page yielded. Error rate = (edits on matched lines + length of missed and spurious
    // lines) / truth characters. A page is "bad" when that exceeds 5%.
    let page_rows: Vec<(f32, f32, f32, &str)> = page_all
        .iter()
        .map(|p| (p.p_sum / p.chars.max(1) as f32, p.edits as f32 / p.truth_chars.max(1) as f32, p.low_lines as f32 / p.lines.max(1) as f32, p.condition.as_str()))
        .collect();
    let (pa_doubt, pa_err): (Vec<f32>, Vec<f32>) = page_rows.iter().map(|(p, e, _, _)| (1.0 - p, *e)).unzip();
    println!("\nall-lines pages: Spearman(1 - page mean p, page error) = {:.3}; AUC for page error > 5% = {:.3}", spearman(&pa_doubt, &pa_err), auc(&pa_doubt, &pa_err.iter().map(|e| *e > 0.05).collect::<Vec<_>>()));
    println!("threshold on page mean p\tflagged pages\tprecision (error>5%)\trecall of error>5%\terror of unflagged pages");
    let bad_pages = pa_err.iter().filter(|e| **e > 0.05).count().max(1);
    for t in [0.80, 0.85, 0.90, 0.92, 0.94, 0.95, 0.96, 0.97, 0.98, 0.99] {
        let flagged: Vec<&(f32, f32, f32, &str)> = page_rows.iter().filter(|r| r.0 < t).collect();
        let rest: Vec<&(f32, f32, f32, &str)> = page_rows.iter().filter(|r| r.0 >= t).collect();
        let hit = flagged.iter().filter(|r| r.1 > 0.05).count();
        println!(
            "{t:.2}\t{}/{}\t{:.1}%\t{:.1}%\t{:.4}",
            flagged.len(),
            page_rows.len(),
            100.0 * hit as f32 / flagged.len().max(1) as f32,
            100.0 * hit as f32 / bad_pages as f32,
            rest.iter().map(|r| r.1).sum::<f32>() / rest.len().max(1) as f32
        );
    }
    println!("\ncondition\tpage mean p (min..max)\tpage error (min..max)");
    for c in &order {
        let rows: Vec<&(f32, f32, f32, &str)> = page_rows.iter().filter(|r| r.3 == c.as_str()).collect();
        let (pl, ph) = rows.iter().fold((1.0f32, 0.0f32), |a, r| (a.0.min(r.0), a.1.max(r.0)));
        let (el, eh) = rows.iter().fold((f32::MAX, 0.0f32), |a, r| (a.0.min(r.1), a.1.max(r.1)));
        println!("{c}\t{pl:.3}..{ph:.3}\t{el:.3}..{eh:.3}");
    }

    if let Some(out) = lines_out {
        let mut text = String::from("condition\tpage\ttruth_len\tcer\tmean\tmin\tgeo\tlow\n");
        for s in &scored {
            text += &format!("{}\t{}\t{}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\t{:.4}\n", s.condition, s.page, s.truth_len, s.err, s.mean, s.min, s.geo, s.low);
        }
        std::fs::write(out, text).unwrap();
    }
}
